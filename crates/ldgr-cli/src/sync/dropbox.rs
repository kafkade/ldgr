//! Dropbox blob transport implementation.
//!
//! Uses the Dropbox HTTP API v2 for file operations in the app folder.
//! `OAuth2` PKCE authorization for CLI headless flow.

// OAuth helpers and API types are used during `sync auth` flow, not all paths active yet.
#![allow(dead_code)]

use ldgr_core::sync::transport::{
    BlobEntry, BlobPath, BlobPrefix, ListResult, PutResult, TransportErrorKind,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::{BlobTransport, TransportError};

/// Dropbox API endpoints.
const API_BASE: &str = "https://api.dropboxapi.com/2";
const CONTENT_BASE: &str = "https://content.dropboxapi.com/2";

/// Dropbox transport configuration.
#[derive(Debug, Clone)]
pub struct DropboxTransport {
    client: Client,
    access_token: String,
    root_path: String,
}

impl DropboxTransport {
    /// Create a new Dropbox transport with the given access token.
    ///
    /// `root_path` is the base path within the app folder (e.g., "" for root).
    pub fn new(access_token: String, root_path: String) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to create HTTP client");

        Self {
            client,
            access_token,
            root_path,
        }
    }

    /// Build the full Dropbox path for a blob.
    fn full_path(&self, path: &BlobPath) -> String {
        if self.root_path.is_empty() {
            format!("/{}", path.as_str())
        } else {
            format!("/{}/{}", self.root_path, path.as_str())
        }
    }

    /// Classify a reqwest error into a transport error kind.
    fn classify_error(status: Option<reqwest::StatusCode>) -> TransportErrorKind {
        match status {
            Some(s) if s == reqwest::StatusCode::UNAUTHORIZED => TransportErrorKind::Auth,
            Some(s) if s == reqwest::StatusCode::NOT_FOUND => TransportErrorKind::NotFound,
            Some(s) if s == reqwest::StatusCode::CONFLICT => TransportErrorKind::Conflict,
            Some(s) if s == reqwest::StatusCode::TOO_MANY_REQUESTS => {
                TransportErrorKind::RateLimited
            }
            Some(s) if s.is_server_error() => TransportErrorKind::Server,
            Some(s) if s == reqwest::StatusCode::PAYLOAD_TOO_LARGE => {
                TransportErrorKind::PayloadTooLarge
            }
            None => TransportErrorKind::Network,
            _ => TransportErrorKind::Other,
        }
    }
}

// ── Dropbox API types ──────────────────────────────────────────────────────────

#[derive(Serialize)]
struct UploadArg {
    path: String,
    mode: String,
    autorename: bool,
    mute: bool,
}

#[derive(Serialize)]
struct DownloadArg {
    path: String,
}

#[derive(Serialize)]
struct ListFolderArg {
    path: String,
    recursive: bool,
    limit: Option<u32>,
}

#[derive(Serialize)]
struct ListFolderContinueArg {
    cursor: String,
}

#[derive(Serialize)]
struct DeleteArg {
    path: String,
}

#[derive(Serialize)]
struct GetMetadataArg {
    path: String,
}

#[derive(Serialize)]
struct CreateFolderArg {
    path: String,
    autorename: bool,
}

#[derive(Deserialize)]
struct FileMetadata {
    name: String,
    path_display: Option<String>,
    size: Option<u64>,
    content_hash: Option<String>,
    server_modified: Option<String>,
    #[serde(rename = ".tag")]
    tag: Option<String>,
}

#[derive(Deserialize)]
struct ListFolderResult {
    entries: Vec<FileMetadata>,
    cursor: String,
    has_more: bool,
}

// ── OAuth2 PKCE Flow ───────────────────────────────────────────────────────────

/// `OAuth2` authorization URL for Dropbox PKCE flow.
pub fn oauth2_authorize_url(app_key: &str, redirect_uri: &str, code_challenge: &str) -> String {
    format!(
        "https://www.dropbox.com/oauth2/authorize\
         ?client_id={app_key}\
         &response_type=code\
         &code_challenge={code_challenge}\
         &code_challenge_method=S256\
         &token_access_type=offline\
         &redirect_uri={redirect_uri}"
    )
}

/// Token response from Dropbox `OAuth2`.
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
    pub token_type: String,
}

/// Exchange an authorization code for tokens.
pub async fn exchange_code(
    client: &Client,
    app_key: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, TransportError> {
    let resp = client
        .post("https://api.dropboxapi.com/oauth2/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", app_key),
            ("code_verifier", code_verifier),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await
        .map_err(|e| {
            TransportError::with_source(TransportErrorKind::Network, "token exchange failed", e)
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(TransportError::new(
            TransportErrorKind::Auth,
            format!("token exchange failed ({status}): {body}"),
        ));
    }

    resp.json::<TokenResponse>().await.map_err(|e| {
        TransportError::with_source(
            TransportErrorKind::InvalidResponse,
            "invalid token response",
            e,
        )
    })
}

/// Refresh an access token using a refresh token.
pub async fn refresh_token(
    client: &Client,
    app_key: &str,
    refresh_tok: &str,
) -> Result<TokenResponse, TransportError> {
    let resp = client
        .post("https://api.dropboxapi.com/oauth2/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_tok),
            ("client_id", app_key),
        ])
        .send()
        .await
        .map_err(|e| {
            TransportError::with_source(TransportErrorKind::Network, "token refresh failed", e)
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(TransportError::new(
            TransportErrorKind::Auth,
            format!("token refresh failed ({status}): {body}"),
        ));
    }

    resp.json::<TokenResponse>().await.map_err(|e| {
        TransportError::with_source(
            TransportErrorKind::InvalidResponse,
            "invalid refresh response",
            e,
        )
    })
}

// ── BlobTransport Implementation ───────────────────────────────────────────────

#[async_trait::async_trait]
impl BlobTransport for DropboxTransport {
    async fn put_blob(&self, path: &BlobPath, data: &[u8]) -> Result<PutResult, TransportError> {
        let arg = UploadArg {
            path: self.full_path(path),
            mode: "add".to_string(), // put-if-absent
            autorename: false,
            mute: true,
        };

        let arg_json = serde_json::to_string(&arg).map_err(|e| {
            TransportError::with_source(TransportErrorKind::Other, "failed to serialize arg", e)
        })?;

        let resp = self
            .client
            .post(format!("{CONTENT_BASE}/files/upload"))
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Dropbox-API-Arg", &arg_json)
            .header("Content-Type", "application/octet-stream")
            .body(data.to_vec())
            .send()
            .await
            .map_err(|e| {
                TransportError::with_source(TransportErrorKind::Network, "upload failed", e)
            })?;

        let status = resp.status();
        if status == reqwest::StatusCode::CONFLICT {
            // File already exists (put-if-absent semantics)
            return Err(TransportError::new(
                TransportErrorKind::Conflict,
                "blob already exists",
            ));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(TransportError::new(
                Self::classify_error(Some(status)),
                format!("upload failed ({status}): {body}"),
            ));
        }

        let meta: FileMetadata = resp.json().await.map_err(|e| {
            TransportError::with_source(
                TransportErrorKind::InvalidResponse,
                "invalid upload response",
                e,
            )
        })?;

        Ok(PutResult {
            content_hash: meta.content_hash,
            etag: None,
        })
    }

    async fn get_blob(&self, path: &BlobPath) -> Result<Vec<u8>, TransportError> {
        let arg = DownloadArg {
            path: self.full_path(path),
        };
        let arg_json = serde_json::to_string(&arg).map_err(|e| {
            TransportError::with_source(TransportErrorKind::Other, "failed to serialize arg", e)
        })?;

        let resp = self
            .client
            .post(format!("{CONTENT_BASE}/files/download"))
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Dropbox-API-Arg", &arg_json)
            .send()
            .await
            .map_err(|e| {
                TransportError::with_source(TransportErrorKind::Network, "download failed", e)
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(TransportError::new(
                Self::classify_error(Some(status)),
                format!("download failed ({status}): {body}"),
            ));
        }

        resp.bytes().await.map(|b| b.to_vec()).map_err(|e| {
            TransportError::with_source(TransportErrorKind::Network, "failed to read response", e)
        })
    }

    async fn list_blobs(
        &self,
        prefix: &BlobPrefix,
        cursor: Option<&str>,
    ) -> Result<ListResult, TransportError> {
        let resp = if let Some(cur) = cursor {
            let arg = ListFolderContinueArg {
                cursor: cur.to_string(),
            };
            self.client
                .post(format!("{API_BASE}/files/list_folder/continue"))
                .header("Authorization", format!("Bearer {}", self.access_token))
                .header("Content-Type", "application/json")
                .json(&arg)
                .send()
                .await
        } else {
            let path = if self.root_path.is_empty() {
                format!("/{}", prefix.as_str().trim_end_matches('/'))
            } else {
                format!(
                    "/{}/{}",
                    self.root_path,
                    prefix.as_str().trim_end_matches('/')
                )
            };
            let arg = ListFolderArg {
                path,
                recursive: true,
                limit: Some(2000),
            };
            self.client
                .post(format!("{API_BASE}/files/list_folder"))
                .header("Authorization", format!("Bearer {}", self.access_token))
                .header("Content-Type", "application/json")
                .json(&arg)
                .send()
                .await
        };

        let resp = resp.map_err(|e| {
            TransportError::with_source(TransportErrorKind::Network, "list failed", e)
        })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(TransportError::new(
                Self::classify_error(Some(status)),
                format!("list failed ({status}): {body}"),
            ));
        }

        let result: ListFolderResult = resp.json().await.map_err(|e| {
            TransportError::with_source(
                TransportErrorKind::InvalidResponse,
                "invalid list response",
                e,
            )
        })?;

        let entries = result
            .entries
            .into_iter()
            .filter(|e| e.tag.as_deref() == Some("file"))
            .map(|e| BlobEntry {
                path: e.path_display.unwrap_or(e.name),
                size: e.size.unwrap_or(0),
                content_hash: e.content_hash,
                modified_at: e.server_modified,
            })
            .collect();

        Ok(ListResult {
            entries,
            cursor: Some(result.cursor),
            has_more: result.has_more,
        })
    }

    async fn delete_blob(&self, path: &BlobPath) -> Result<(), TransportError> {
        let arg = DeleteArg {
            path: self.full_path(path),
        };

        let resp = self
            .client
            .post(format!("{API_BASE}/files/delete_v2"))
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Content-Type", "application/json")
            .json(&arg)
            .send()
            .await
            .map_err(|e| {
                TransportError::with_source(TransportErrorKind::Network, "delete failed", e)
            })?;

        let status = resp.status();
        // 409 with path/not_found is fine (idempotent delete)
        if status == reqwest::StatusCode::CONFLICT {
            let body = resp.text().await.unwrap_or_default();
            if body.contains("not_found") {
                return Ok(());
            }
            return Err(TransportError::new(
                TransportErrorKind::Conflict,
                format!("delete conflict: {body}"),
            ));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(TransportError::new(
                Self::classify_error(Some(status)),
                format!("delete failed ({status}): {body}"),
            ));
        }

        Ok(())
    }

    async fn exists(&self, path: &BlobPath) -> Result<bool, TransportError> {
        let arg = GetMetadataArg {
            path: self.full_path(path),
        };

        let resp = self
            .client
            .post(format!("{API_BASE}/files/get_metadata"))
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Content-Type", "application/json")
            .json(&arg)
            .send()
            .await
            .map_err(|e| {
                TransportError::with_source(TransportErrorKind::Network, "metadata check failed", e)
            })?;

        if resp.status() == reqwest::StatusCode::CONFLICT {
            // Dropbox returns 409 for path/not_found
            return Ok(false);
        }
        if resp.status().is_success() {
            return Ok(true);
        }

        let body = resp.text().await.unwrap_or_default();
        if body.contains("not_found") {
            return Ok(false);
        }

        Err(TransportError::new(
            Self::classify_error(None),
            format!("metadata check failed: {body}"),
        ))
    }

    async fn ensure_directory(&self, prefix: &BlobPrefix) -> Result<(), TransportError> {
        let path = if self.root_path.is_empty() {
            format!("/{}", prefix.as_str().trim_end_matches('/'))
        } else {
            format!(
                "/{}/{}",
                self.root_path,
                prefix.as_str().trim_end_matches('/')
            )
        };

        let arg = CreateFolderArg {
            path,
            autorename: false,
        };

        let resp = self
            .client
            .post(format!("{API_BASE}/files/create_folder_v2"))
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Content-Type", "application/json")
            .json(&arg)
            .send()
            .await
            .map_err(|e| {
                TransportError::with_source(TransportErrorKind::Network, "create folder failed", e)
            })?;

        // 409 conflict means folder exists — that's fine
        if resp.status() == reqwest::StatusCode::CONFLICT || resp.status().is_success() {
            return Ok(());
        }

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(TransportError::new(
            Self::classify_error(Some(status)),
            format!("create folder failed ({status}): {body}"),
        ))
    }
}
