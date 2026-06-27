//! Runtime-updatable server settings (ADR-008, #176).
//!
//! Environment variables (loaded into [`Config`](crate::config::Config)) provide
//! the bootstrap defaults. The admin API persists overrides into the `settings`
//! table; a persisted value always wins over the env/config fallback, so
//! operators can change the registration policy, default quota, and max blob
//! size at runtime without restarting. Absent (or unparseable) keys fall back to
//! the env default, keeping a misconfigured row from bricking the server.

use crate::config::RegistrationPolicy;
use crate::error::ServerError;
use crate::state::SharedState;

pub const KEY_REGISTRATION_POLICY: &str = "registration_policy";
pub const KEY_DEFAULT_QUOTA_BYTES: &str = "default_quota_bytes";
pub const KEY_MAX_BLOB_BYTES: &str = "max_blob_bytes";

/// Snapshot of effective settings (persisted overrides resolved against env).
pub struct EffectiveSettings {
    pub registration_policy: RegistrationPolicy,
    pub default_quota_bytes: i64,
    pub max_blob_bytes: i64,
}

/// Effective registration policy: persisted value if present, else env/config.
pub async fn registration_policy(state: &SharedState) -> Result<RegistrationPolicy, ServerError> {
    match state.db.get_setting(KEY_REGISTRATION_POLICY).await? {
        Some(v) => Ok(RegistrationPolicy::parse(&v)),
        None => Ok(state.config.registration_policy),
    }
}

/// Effective default per-user quota in bytes: persisted value if present and
/// parseable, else env/config.
pub async fn default_quota_bytes(state: &SharedState) -> Result<i64, ServerError> {
    Ok(parse_i64_setting(state, KEY_DEFAULT_QUOTA_BYTES)
        .await?
        .unwrap_or(state.config.default_user_quota_bytes))
}

/// Effective max blob size in bytes: persisted value if present and parseable,
/// else env/config. Note the request body-limit layer is built at startup from
/// the env value; runtime changes to this setting apply on the next restart.
pub async fn max_blob_bytes(state: &SharedState) -> Result<i64, ServerError> {
    #[allow(clippy::cast_possible_wrap)]
    let env_default = state.config.max_blob_bytes as i64;
    Ok(parse_i64_setting(state, KEY_MAX_BLOB_BYTES)
        .await?
        .unwrap_or(env_default))
}

/// Resolve all effective settings in one place (for `GET /settings`).
pub async fn effective(state: &SharedState) -> Result<EffectiveSettings, ServerError> {
    Ok(EffectiveSettings {
        registration_policy: registration_policy(state).await?,
        default_quota_bytes: default_quota_bytes(state).await?,
        max_blob_bytes: max_blob_bytes(state).await?,
    })
}

/// Read an integer setting, ignoring (with a warning) a malformed persisted row.
async fn parse_i64_setting(state: &SharedState, key: &str) -> Result<Option<i64>, ServerError> {
    match state.db.get_setting(key).await? {
        Some(v) => {
            if let Ok(n) = v.trim().parse::<i64>() {
                Ok(Some(n))
            } else {
                tracing::warn!("ignoring malformed setting {key}={v:?}; using env default");
                Ok(None)
            }
        }
        None => Ok(None),
    }
}
