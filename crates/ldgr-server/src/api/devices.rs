//! Device management endpoints (list / register / remove).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Serialize;

use super::vaults::require_vault_access;
use crate::auth::hex_encode;
use crate::auth::middleware::AuthUser;
use crate::error::ServerError;
use crate::state::SharedState;

#[derive(Serialize)]
pub struct DeviceResponse {
    pub id: String,
    pub updated_at: String,
    /// Hex-encoded encrypted device info (opaque to server).
    pub encrypted_info: String,
}

/// `GET /api/v1/vaults/:vault_id/devices`
pub async fn list_devices(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Path(vault_id): Path<String>,
) -> Result<Json<Vec<DeviceResponse>>, ServerError> {
    require_vault_access(&state, &user_id, &vault_id).await?;

    let devices = state.db.list_devices(&vault_id).await?;
    let response = devices
        .into_iter()
        .map(|d| DeviceResponse {
            id: d.id,
            updated_at: d.updated_at,
            encrypted_info: hex_encode(&d.encrypted_info),
        })
        .collect();

    Ok(Json(response))
}

/// `PUT /api/v1/vaults/:vault_id/devices/:device_id`
///
/// Register or update a device. Body is raw encrypted device info bytes.
pub async fn put_device(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Path((vault_id, device_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<StatusCode, ServerError> {
    require_vault_access(&state, &user_id, &vault_id).await?;

    if body.is_empty() {
        return Err(ServerError::BadRequest("empty body".into()));
    }

    state
        .db
        .put_device(&device_id, &vault_id, body.to_vec())
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/vaults/:vault_id/devices/:device_id`
pub async fn delete_device(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Path((vault_id, device_id)): Path<(String, String)>,
) -> Result<StatusCode, ServerError> {
    require_vault_access(&state, &user_id, &vault_id).await?;

    if state.db.delete_device(&device_id, &vault_id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ServerError::NotFound)
    }
}
