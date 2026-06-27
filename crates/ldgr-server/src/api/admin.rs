//! Admin API endpoints (ADR-008, #176).
//!
//! Every route in this module is guarded by the [`AdminUser`] extractor, so a
//! missing/invalid session is rejected with `401` and a valid non-admin session
//! with `403`. The server stays headless and API-only (ADR-005/008): all
//! responses are JSON for the web admin UI (#179) to consume.
//!
//! Each mutation emits a `tracing::info!` audit line recording who did what to
//! whom.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rand::Rng;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::auth::middleware::AdminUser;
use crate::auth::{hex_decode, hex_encode};
use crate::config::RegistrationPolicy;
use crate::error::ServerError;
use crate::settings;
use crate::state::SharedState;
use crate::storage::{AdminUserRecord, InviteRecord, NewUser};

// ── Views ───────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct UserView {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub role: String,
    pub status: String,
    /// Per-user quota override in bytes; `null` means "use the server default".
    pub quota_bytes: Option<i64>,
    pub usage_bytes: i64,
    pub created_at: String,
}

impl From<AdminUserRecord> for UserView {
    fn from(u: AdminUserRecord) -> Self {
        Self {
            id: u.id,
            username: u.username,
            email: u.email,
            role: u.role,
            status: u.status,
            quota_bytes: u.storage_quota_bytes,
            usage_bytes: u.usage_bytes,
            created_at: u.created_at,
        }
    }
}

#[derive(Serialize)]
pub struct InviteView {
    /// The invite's stable id (its token hash). The raw token is only ever
    /// returned once, at creation time.
    pub id: String,
    pub email: Option<String>,
    pub role: String,
    pub created_by: Option<String>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub redeemed_at: Option<String>,
    pub redeemed_by: Option<String>,
    /// Derived lifecycle: `pending`, `redeemed`, or `expired`.
    pub status: String,
}

impl From<InviteRecord> for InviteView {
    fn from(i: InviteRecord) -> Self {
        let status = if i.redeemed_at.is_some() {
            "redeemed"
        } else if i
            .expires_at
            .as_deref()
            .is_some_and(|e| e <= chrono::Utc::now().to_rfc3339().as_str())
        {
            "expired"
        } else {
            "pending"
        }
        .to_string();
        Self {
            id: i.token_hash,
            email: i.email,
            role: i.role,
            created_by: i.created_by,
            created_at: i.created_at,
            expires_at: i.expires_at,
            redeemed_at: i.redeemed_at,
            redeemed_by: i.redeemed_by,
            status,
        }
    }
}

// ── Validation helpers ──────────────────────────────────────────────────────

fn validate_role(role: &str) -> Result<(), ServerError> {
    if matches!(role, "admin" | "user") {
        Ok(())
    } else {
        Err(ServerError::BadRequest(format!(
            "invalid role: {role} (expected 'admin' or 'user')"
        )))
    }
}

fn validate_status(status: &str) -> Result<(), ServerError> {
    if matches!(status, "active" | "disabled") {
        Ok(())
    } else {
        Err(ServerError::BadRequest(format!(
            "invalid status: {status} (expected 'active' or 'disabled')"
        )))
    }
}

/// Reject an action that would remove the final active admin, locking out the
/// instance. The guard only bites when `target` is itself an active admin and
/// no other active admin remains.
async fn ensure_not_last_admin(
    state: &SharedState,
    target: &AdminUserRecord,
    action: &str,
) -> Result<(), ServerError> {
    if target.role == "admin"
        && target.status == "active"
        && state.db.count_active_admins_excluding(&target.id).await? == 0
    {
        return Err(ServerError::Forbidden(format!(
            "cannot {action} the last active admin"
        )));
    }
    Ok(())
}

// ── Users ───────────────────────────────────────────────────────────────────

pub async fn list_users(
    State(state): State<SharedState>,
    AdminUser(_admin): AdminUser,
) -> Result<Json<Vec<UserView>>, ServerError> {
    let users = state.db.list_users().await?;
    Ok(Json(users.into_iter().map(UserView::from).collect()))
}

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub salt: String,     // hex-encoded
    pub verifier: String, // hex-encoded
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub quota_bytes: Option<i64>,
    #[serde(default)]
    pub auth_scheme: Option<String>,
}

/// Admin-created account. Bypasses the registration policy (this *is* the admin
/// path). The client still supplies the SRP `(salt, verifier)` — the server
/// never sees the password.
pub async fn create_user(
    State(state): State<SharedState>,
    AdminUser(admin): AdminUser,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserView>), ServerError> {
    if req.username.is_empty() || req.username.len() > 128 {
        return Err(ServerError::BadRequest(
            "username must be 1-128 characters".into(),
        ));
    }
    let salt =
        hex_decode(&req.salt).map_err(|e| ServerError::BadRequest(format!("invalid salt: {e}")))?;
    if salt.len() < 16 {
        return Err(ServerError::BadRequest(
            "salt must be at least 16 bytes".into(),
        ));
    }
    let verifier = hex_decode(&req.verifier)
        .map_err(|e| ServerError::BadRequest(format!("invalid verifier: {e}")))?;
    if verifier.is_empty() {
        return Err(ServerError::BadRequest("verifier must not be empty".into()));
    }

    let role = req.role.as_deref().unwrap_or("user");
    validate_role(role)?;

    let auth_scheme = req.auth_scheme.as_deref().unwrap_or("srp-1secret");
    if !matches!(auth_scheme, "srp-1secret" | "srp-2skd-v1") {
        return Err(ServerError::BadRequest(format!(
            "unsupported auth_scheme: {auth_scheme}"
        )));
    }

    let email = req
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(req.username.as_str())
        .to_string();

    if let Some(q) = req.quota_bytes
        && q < 0
    {
        return Err(ServerError::BadRequest(
            "quota_bytes must be non-negative".into(),
        ));
    }

    let user_id = uuid::Uuid::now_v7().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();

    state
        .db
        .create_user(&NewUser {
            id: &user_id,
            username: &req.username,
            email: Some(&email),
            salt: &salt,
            verifier: &verifier,
            role,
            auth_scheme,
            invited_by: Some(&admin),
            created_at: &created_at,
        })
        .await?;

    if let Some(q) = req.quota_bytes {
        state.db.set_user_quota(&user_id, Some(q)).await?;
    }

    tracing::info!(
        admin = %admin,
        target = %user_id,
        username = %req.username,
        role = %role,
        "admin created user"
    );

    let view = state
        .db
        .get_user_by_id(&user_id)
        .await?
        .ok_or_else(|| ServerError::Internal("created user vanished".into()))?;
    Ok((StatusCode::CREATED, Json(view.into())))
}

/// Deserialize a field as `Some(value)` even when explicitly `null`, so callers
/// can distinguish "field absent" (leave unchanged) from "field is null" (clear
/// the quota override).
#[allow(clippy::option_option)]
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    /// Absent → unchanged; `null` → clear override (use server default); value →
    /// set override.
    #[serde(default, deserialize_with = "double_option")]
    #[allow(clippy::option_option)]
    pub quota_bytes: Option<Option<i64>>,
}

pub async fn update_user(
    State(state): State<SharedState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<UserView>, ServerError> {
    let target = state
        .db
        .get_user_by_id(&id)
        .await?
        .ok_or(ServerError::NotFound)?;

    if let Some(ref status) = req.status {
        validate_status(status)?;
    }
    if let Some(ref role) = req.role {
        validate_role(role)?;
    }
    if let Some(Some(q)) = req.quota_bytes
        && q < 0
    {
        return Err(ServerError::BadRequest(
            "quota_bytes must be non-negative".into(),
        ));
    }

    // Last-admin protection: block a change that would strip the final active
    // admin (demotion to `user` or disabling).
    let demoting = req.role.as_deref() == Some("user") && target.role == "admin";
    let disabling = req.status.as_deref() == Some("disabled") && target.status == "active";
    if demoting || disabling {
        let action = if demoting { "demote" } else { "disable" };
        ensure_not_last_admin(&state, &target, action).await?;
    }

    if let Some(ref status) = req.status
        && *status != target.status
    {
        state.db.set_user_status_by_id(&id, status).await?;
        tracing::info!(admin = %admin, target = %id, status = %status, "admin set user status");
    }
    if let Some(ref role) = req.role
        && *role != target.role
    {
        state.db.set_user_role(&id, role).await?;
        tracing::info!(admin = %admin, target = %id, role = %role, "admin set user role");
    }
    if let Some(quota) = req.quota_bytes {
        state.db.set_user_quota(&id, quota).await?;
        tracing::info!(admin = %admin, target = %id, ?quota, "admin set user quota");
    }

    let view = state
        .db
        .get_user_by_id(&id)
        .await?
        .ok_or(ServerError::NotFound)?;
    Ok(Json(view.into()))
}

pub async fn delete_user(
    State(state): State<SharedState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ServerError> {
    let target = state
        .db
        .get_user_by_id(&id)
        .await?
        .ok_or(ServerError::NotFound)?;

    ensure_not_last_admin(&state, &target, "delete").await?;

    let removed = state.db.delete_user(&id).await?;
    if !removed {
        return Err(ServerError::NotFound);
    }
    tracing::info!(admin = %admin, target = %id, username = %target.username, "admin deleted user");
    Ok(StatusCode::NO_CONTENT)
}

// ── Invites ─────────────────────────────────────────────────────────────────

/// Generate `(raw_token, token_hash)` for an invite. The hash matches the one
/// `auth::register` computes on redemption (`SHA-256` of the raw token string),
/// so the raw token round-trips through the existing redemption path.
fn generate_invite_token() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let raw = hex_encode(&bytes);
    let token_hash = hex_encode(&Sha256::digest(raw.as_bytes()));
    (raw, token_hash)
}

#[derive(Deserialize)]
pub struct CreateInviteRequest {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    /// Optional expiry, in hours from now. Omit for a non-expiring invite.
    #[serde(default)]
    pub expires_in_hours: Option<i64>,
}

#[derive(Serialize)]
pub struct CreateInviteResponse {
    /// The raw invite token — shown exactly once. Hand this to the invitee; they
    /// pass it to `register`.
    pub token: String,
    /// The invite's stable id (its hash), for later listing/revoke.
    pub id: String,
    pub role: String,
    pub email: Option<String>,
    pub expires_at: Option<String>,
}

pub async fn create_invite(
    State(state): State<SharedState>,
    AdminUser(admin): AdminUser,
    Json(req): Json<CreateInviteRequest>,
) -> Result<(StatusCode, Json<CreateInviteResponse>), ServerError> {
    let role = req.role.as_deref().unwrap_or("user");
    validate_role(role)?;

    let email = req
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let expires_at = match req.expires_in_hours {
        Some(h) if h <= 0 => {
            return Err(ServerError::BadRequest(
                "expires_in_hours must be positive".into(),
            ));
        }
        Some(h) => Some((chrono::Utc::now() + chrono::Duration::hours(h)).to_rfc3339()),
        None => None,
    };

    let (raw_token, token_hash) = generate_invite_token();
    state
        .db
        .create_invite(
            &token_hash,
            email.as_deref(),
            role,
            Some(&admin),
            expires_at.as_deref(),
        )
        .await?;

    tracing::info!(
        admin = %admin,
        invite = %token_hash,
        role = %role,
        email = ?email,
        "admin issued invite"
    );

    Ok((
        StatusCode::CREATED,
        Json(CreateInviteResponse {
            token: raw_token,
            id: token_hash,
            role: role.to_string(),
            email,
            expires_at,
        }),
    ))
}

pub async fn list_invites(
    State(state): State<SharedState>,
    AdminUser(_admin): AdminUser,
) -> Result<Json<Vec<InviteView>>, ServerError> {
    let invites = state.db.list_invites().await?;
    Ok(Json(invites.into_iter().map(InviteView::from).collect()))
}

/// Revoke an unredeemed invite. The path segment accepts either the raw invite
/// token or its id/hash (as returned by `GET /invites`), so it works whether the
/// caller holds the token or is acting from the admin listing.
pub async fn delete_invite(
    State(state): State<SharedState>,
    AdminUser(admin): AdminUser,
    Path(token): Path<String>,
) -> Result<StatusCode, ServerError> {
    // Treat the segment as a raw token first (hash it), then fall back to
    // treating it as the hash/id directly.
    let hashed = hex_encode(&Sha256::digest(token.as_bytes()));
    let removed = state.db.delete_invite(&hashed).await? || state.db.delete_invite(&token).await?;
    if !removed {
        return Err(ServerError::NotFound);
    }
    tracing::info!(admin = %admin, "admin revoked invite");
    Ok(StatusCode::NO_CONTENT)
}

// ── Settings ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct SettingsView {
    pub registration_policy: String,
    pub default_quota_bytes: i64,
    pub max_blob_bytes: i64,
}

pub async fn get_settings(
    State(state): State<SharedState>,
    AdminUser(_admin): AdminUser,
) -> Result<Json<SettingsView>, ServerError> {
    let s = settings::effective(&state).await?;
    Ok(Json(SettingsView {
        registration_policy: s.registration_policy.as_str().to_string(),
        default_quota_bytes: s.default_quota_bytes,
        max_blob_bytes: s.max_blob_bytes,
    }))
}

#[derive(Deserialize)]
pub struct UpdateSettingsRequest {
    #[serde(default)]
    pub registration_policy: Option<String>,
    #[serde(default)]
    pub default_quota_bytes: Option<i64>,
    #[serde(default)]
    pub max_blob_bytes: Option<i64>,
}

pub async fn update_settings(
    State(state): State<SharedState>,
    AdminUser(admin): AdminUser,
    Json(req): Json<UpdateSettingsRequest>,
) -> Result<Json<SettingsView>, ServerError> {
    if let Some(ref policy) = req.registration_policy {
        // Reject spellings the parser would silently coerce to the default.
        let canonical = RegistrationPolicy::parse(policy);
        if !matches!(
            policy.trim().to_ascii_lowercase().as_str(),
            "open"
                | "invite-only"
                | "invite_only"
                | "inviteonly"
                | "admin-only"
                | "admin_only"
                | "adminonly"
        ) {
            return Err(ServerError::BadRequest(format!(
                "invalid registration_policy: {policy}"
            )));
        }
        state
            .db
            .set_setting(settings::KEY_REGISTRATION_POLICY, canonical.as_str())
            .await?;
        tracing::info!(admin = %admin, policy = %canonical.as_str(), "admin updated registration policy");
    }

    if let Some(q) = req.default_quota_bytes {
        if q <= 0 {
            return Err(ServerError::BadRequest(
                "default_quota_bytes must be positive".into(),
            ));
        }
        state
            .db
            .set_setting(settings::KEY_DEFAULT_QUOTA_BYTES, &q.to_string())
            .await?;
        tracing::info!(admin = %admin, default_quota_bytes = q, "admin updated default quota");
    }

    if let Some(m) = req.max_blob_bytes {
        if m <= 0 {
            return Err(ServerError::BadRequest(
                "max_blob_bytes must be positive".into(),
            ));
        }
        state
            .db
            .set_setting(settings::KEY_MAX_BLOB_BYTES, &m.to_string())
            .await?;
        tracing::info!(admin = %admin, max_blob_bytes = m, "admin updated max blob size");
    }

    let s = settings::effective(&state).await?;
    Ok(Json(SettingsView {
        registration_policy: s.registration_policy.as_str().to_string(),
        default_quota_bytes: s.default_quota_bytes,
        max_blob_bytes: s.max_blob_bytes,
    }))
}

// ── Stats ───────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct PerUserUsage {
    pub id: String,
    pub username: String,
    pub usage_bytes: i64,
    pub quota_bytes: Option<i64>,
}

#[derive(Serialize)]
pub struct StatsView {
    pub user_count: usize,
    pub total_storage_bytes: i64,
    pub per_user: Vec<PerUserUsage>,
}

pub async fn stats(
    State(state): State<SharedState>,
    AdminUser(_admin): AdminUser,
) -> Result<Json<StatsView>, ServerError> {
    let users = state.db.list_users().await?;
    let total_storage_bytes = state.db.total_storage_used().await?;
    let per_user = users
        .iter()
        .map(|u| PerUserUsage {
            id: u.id.clone(),
            username: u.username.clone(),
            usage_bytes: u.usage_bytes,
            quota_bytes: u.storage_quota_bytes,
        })
        .collect();
    Ok(Json(StatsView {
        user_count: users.len(),
        total_storage_bytes,
        per_user,
    }))
}
