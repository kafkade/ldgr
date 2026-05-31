pub mod schema;

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};

use crate::error::ServerError;

// ── Record Types ──────────────────────────────────────────────────────────────

pub struct User {
    pub id: String,
    pub username: String,
    pub salt: Vec<u8>,
    pub verifier: Vec<u8>,
}

pub struct Vault {
    pub id: String,
    #[allow(dead_code)]
    pub user_id: String,
    pub created_at: String,
}

pub struct BlobMeta {
    pub path: String,
    pub size: i64,
    pub content_hash: String,
    pub created_at: String,
}

pub struct DeviceRecord {
    pub id: String,
    #[allow(dead_code)]
    pub vault_id: String,
    pub encrypted_info: Vec<u8>,
    pub updated_at: String,
}

pub struct RelayOffer {
    pub id: String,
    #[allow(dead_code)]
    pub user_id: String,
    pub offer_data: Vec<u8>,
    pub response_data: Option<Vec<u8>>,
    pub expires_at: String,
}

// ── Database ──────────────────────────────────────────────────────────────────

/// Server-side `SQLite` storage. All operations run in `spawn_blocking`
/// to avoid blocking the Tokio runtime.
#[derive(Clone)]
pub struct ServerDb {
    conn: Arc<Mutex<Connection>>,
}

impl ServerDb {
    /// Open (or create) the database at `path` and run migrations.
    pub fn open(path: &str) -> Result<Self, ServerError> {
        let conn = Connection::open(path).map_err(ServerError::from)?;
        conn.execute_batch("PRAGMA journal_mode = WAL;")
            .map_err(ServerError::from)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(ServerError::from)?;
        conn.execute_batch("PRAGMA busy_timeout = 5000;")
            .map_err(ServerError::from)?;
        conn.execute_batch(schema::SCHEMA)
            .map_err(ServerError::from)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    // ── Users ─────────────────────────────────────────────────────────────────

    pub async fn create_user(
        &self,
        id: &str,
        username: &str,
        salt: &[u8],
        verifier: &[u8],
        created_at: &str,
    ) -> Result<(), ServerError> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let username = username.to_string();
        let salt = salt.to_vec();
        let verifier = verifier.to_vec();
        let created_at = created_at.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            conn.execute(
                "INSERT INTO users (id, username, salt, verifier, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, username, salt, verifier, created_at],
            ).map_err(|e| match e {
                rusqlite::Error::SqliteFailure(err, _)
                    if err.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    ServerError::Conflict("username already exists".into())
                }
                other => ServerError::from(other),
            })?;
            Ok(())
        })
        .await?
    }

    pub async fn get_user_by_username(&self, username: &str) -> Result<Option<User>, ServerError> {
        let conn = self.conn.clone();
        let username = username.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            let mut stmt =
                conn.prepare("SELECT id, username, salt, verifier FROM users WHERE username = ?1")?;
            let user = stmt
                .query_row(params![username], |row| {
                    Ok(User {
                        id: row.get(0)?,
                        username: row.get(1)?,
                        salt: row.get(2)?,
                        verifier: row.get(3)?,
                    })
                })
                .optional()?;
            Ok(user)
        })
        .await?
    }

    // ── Sessions ──────────────────────────────────────────────────────────────

    pub async fn create_session(
        &self,
        token_hash: &str,
        user_id: &str,
        created_at: &str,
        expires_at: &str,
    ) -> Result<(), ServerError> {
        let conn = self.conn.clone();
        let token_hash = token_hash.to_string();
        let user_id = user_id.to_string();
        let created_at = created_at.to_string();
        let expires_at = expires_at.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            conn.execute(
                "INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
                params![token_hash, user_id, created_at, expires_at],
            )?;
            Ok(())
        })
        .await?
    }

    /// Validate a session token hash and return the user ID if valid and not expired.
    pub async fn validate_session(&self, token_hash: &str) -> Result<Option<String>, ServerError> {
        let conn = self.conn.clone();
        let token_hash = token_hash.to_string();
        let now = chrono::Utc::now().to_rfc3339();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            let user_id: Option<String> = conn
                .query_row(
                    "SELECT user_id FROM sessions WHERE token_hash = ?1 AND expires_at > ?2",
                    params![token_hash, now],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(user_id)
        })
        .await?
    }

    #[allow(dead_code)]
    pub async fn delete_expired_sessions(&self) -> Result<usize, ServerError> {
        let conn = self.conn.clone();
        let now = chrono::Utc::now().to_rfc3339();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            let count =
                conn.execute("DELETE FROM sessions WHERE expires_at <= ?1", params![now])?;
            Ok(count)
        })
        .await?
    }

    // ── Vaults ────────────────────────────────────────────────────────────────

    pub async fn create_vault(&self, id: &str, user_id: &str) -> Result<String, ServerError> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let user_id = user_id.to_string();
        let created_at = chrono::Utc::now().to_rfc3339();
        let ts = created_at.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            conn.execute(
                "INSERT INTO vaults (id, user_id, created_at) VALUES (?1, ?2, ?3)",
                params![id, user_id, ts],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(err, _)
                    if err.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    ServerError::Conflict("vault already exists".into())
                }
                other => ServerError::from(other),
            })?;
            Ok(created_at)
        })
        .await?
    }

    pub async fn list_user_vaults(&self, user_id: &str) -> Result<Vec<Vault>, ServerError> {
        let conn = self.conn.clone();
        let user_id = user_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            let mut stmt =
                conn.prepare("SELECT id, user_id, created_at FROM vaults WHERE user_id = ?1")?;
            let vaults = stmt
                .query_map(params![user_id], |row| {
                    Ok(Vault {
                        id: row.get(0)?,
                        user_id: row.get(1)?,
                        created_at: row.get(2)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(vaults)
        })
        .await?
    }

    pub async fn user_owns_vault(
        &self,
        user_id: &str,
        vault_id: &str,
    ) -> Result<bool, ServerError> {
        let conn = self.conn.clone();
        let user_id = user_id.to_string();
        let vault_id = vault_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            let exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM vaults WHERE id = ?1 AND user_id = ?2)",
                params![vault_id, user_id],
                |row| row.get(0),
            )?;
            Ok(exists)
        })
        .await?
    }

    // ── Blobs ─────────────────────────────────────────────────────────────────

    /// Insert a blob (put-if-absent: fails on duplicate path).
    pub async fn put_blob(
        &self,
        path: &str,
        vault_id: &str,
        data: Vec<u8>,
        content_hash: &str,
    ) -> Result<BlobMeta, ServerError> {
        let conn = self.conn.clone();
        let path = path.to_string();
        let vault_id = vault_id.to_string();
        let content_hash = content_hash.to_string();
        let created_at = chrono::Utc::now().to_rfc3339();

        #[allow(clippy::cast_possible_wrap)]
        let size = data.len() as i64;
        let ts = created_at.clone();
        let hash = content_hash.clone();
        let p = path.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            conn.execute(
                "INSERT INTO blobs (path, vault_id, data, size, content_hash, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![p, vault_id, data, size, hash, ts],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(err, _)
                    if err.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    ServerError::Conflict("blob already exists".into())
                }
                other => ServerError::from(other),
            })?;
            Ok(BlobMeta {
                path,
                size,
                content_hash,
                created_at,
            })
        })
        .await?
    }

    pub async fn get_blob(&self, path: &str) -> Result<Option<Vec<u8>>, ServerError> {
        let conn = self.conn.clone();
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            let data: Option<Vec<u8>> = conn
                .query_row(
                    "SELECT data FROM blobs WHERE path = ?1",
                    params![path],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(data)
        })
        .await?
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_blobs(
        &self,
        vault_id: &str,
        prefix: Option<&str>,
        since: Option<&str>,
        limit: u32,
    ) -> Result<Vec<BlobMeta>, ServerError> {
        let conn = self.conn.clone();
        let vault_id = vault_id.to_string();
        let prefix = prefix.map(String::from);
        let since = since.map(String::from);
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;

            let mut sql = String::from(
                "SELECT path, size, content_hash, created_at FROM blobs WHERE vault_id = ?1",
            );
            let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(vault_id)];

            if let Some(ref p) = prefix {
                sql.push_str(" AND path LIKE ?2");
                params_vec.push(Box::new(format!("{p}%")));
            }
            if let Some(ref s) = since {
                let idx = params_vec.len() + 1;
                let _ = write!(sql, " AND created_at > ?{idx}");
                params_vec.push(Box::new(s.clone()));
            }
            sql.push_str(" ORDER BY created_at ASC");
            let idx = params_vec.len() + 1;
            let _ = write!(sql, " LIMIT ?{idx}");
            params_vec.push(Box::new(limit));

            let mut stmt = conn.prepare(&sql)?;
            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(AsRef::as_ref).collect();
            let entries = stmt
                .query_map(params_refs.as_slice(), |row| {
                    Ok(BlobMeta {
                        path: row.get(0)?,
                        size: row.get(1)?,
                        content_hash: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(entries)
        })
        .await?
    }

    #[allow(dead_code)]
    pub async fn delete_blob(&self, path: &str) -> Result<bool, ServerError> {
        let conn = self.conn.clone();
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            let count = conn.execute("DELETE FROM blobs WHERE path = ?1", params![path])?;
            Ok(count > 0)
        })
        .await?
    }

    /// Check if a blob path belongs to a given vault.
    #[allow(dead_code)]
    pub async fn blob_belongs_to_vault(
        &self,
        path: &str,
        vault_id: &str,
    ) -> Result<bool, ServerError> {
        let conn = self.conn.clone();
        let path = path.to_string();
        let vault_id = vault_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            let exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM blobs WHERE path = ?1 AND vault_id = ?2)",
                params![path, vault_id],
                |row| row.get(0),
            )?;
            Ok(exists)
        })
        .await?
    }

    // ── Devices ───────────────────────────────────────────────────────────────

    pub async fn put_device(
        &self,
        id: &str,
        vault_id: &str,
        encrypted_info: Vec<u8>,
    ) -> Result<(), ServerError> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let vault_id = vault_id.to_string();
        let updated_at = chrono::Utc::now().to_rfc3339();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            conn.execute(
                "INSERT INTO devices (id, vault_id, encrypted_info, updated_at) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (vault_id, id) DO UPDATE SET encrypted_info = ?3, updated_at = ?4",
                params![id, vault_id, encrypted_info, updated_at],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn list_devices(&self, vault_id: &str) -> Result<Vec<DeviceRecord>, ServerError> {
        let conn = self.conn.clone();
        let vault_id = vault_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            let mut stmt = conn.prepare(
                "SELECT id, vault_id, encrypted_info, updated_at FROM devices WHERE vault_id = ?1",
            )?;
            let devices = stmt
                .query_map(params![vault_id], |row| {
                    Ok(DeviceRecord {
                        id: row.get(0)?,
                        vault_id: row.get(1)?,
                        encrypted_info: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(devices)
        })
        .await?
    }

    pub async fn delete_device(&self, id: &str, vault_id: &str) -> Result<bool, ServerError> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let vault_id = vault_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            let count = conn.execute(
                "DELETE FROM devices WHERE id = ?1 AND vault_id = ?2",
                params![id, vault_id],
            )?;
            Ok(count > 0)
        })
        .await?
    }

    // ── Relay ─────────────────────────────────────────────────────────────────

    pub async fn create_relay_offer(
        &self,
        id: &str,
        user_id: &str,
        offer_data: Vec<u8>,
        expires_at: &str,
    ) -> Result<(), ServerError> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let user_id = user_id.to_string();
        let created_at = chrono::Utc::now().to_rfc3339();
        let expires_at = expires_at.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            // Clean expired offers first
            let now = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "DELETE FROM relay_offers WHERE expires_at <= ?1",
                params![now],
            )?;
            conn.execute(
                "INSERT INTO relay_offers (id, user_id, offer_data, created_at, expires_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, user_id, offer_data, created_at, expires_at],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn get_relay_offer(
        &self,
        id: &str,
        user_id: &str,
    ) -> Result<Option<RelayOffer>, ServerError> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let user_id = user_id.to_string();
        let now = chrono::Utc::now().to_rfc3339();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            let offer = conn
                .query_row(
                    "SELECT id, user_id, offer_data, response_data, expires_at \
                     FROM relay_offers WHERE id = ?1 AND user_id = ?2 AND expires_at > ?3",
                    params![id, user_id, now],
                    |row| {
                        Ok(RelayOffer {
                            id: row.get(0)?,
                            user_id: row.get(1)?,
                            offer_data: row.get(2)?,
                            response_data: row.get(3)?,
                            expires_at: row.get(4)?,
                        })
                    },
                )
                .optional()?;
            Ok(offer)
        })
        .await?
    }

    pub async fn set_relay_response(
        &self,
        id: &str,
        user_id: &str,
        response_data: Vec<u8>,
    ) -> Result<bool, ServerError> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let user_id = user_id.to_string();
        let now = chrono::Utc::now().to_rfc3339();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ServerError::Internal(format!("lock poisoned: {e}")))?;
            let count = conn.execute(
                "UPDATE relay_offers SET response_data = ?1 \
                 WHERE id = ?2 AND user_id = ?3 AND expires_at > ?4 AND response_data IS NULL",
                params![response_data, id, user_id, now],
            )?;
            Ok(count > 0)
        })
        .await?
    }
}

/// Extension trait to make `optional()` available on `rusqlite::Result`.
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
