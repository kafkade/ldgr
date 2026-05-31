//! Vault management endpoints.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::auth::middleware::AuthUser;
use crate::error::ServerError;
use crate::state::SharedState;

#[derive(Deserialize)]
pub struct CreateVaultRequest {
    pub vault_id: String,
}

#[derive(Serialize)]
pub struct VaultResponse {
    pub id: String,
    pub created_at: String,
}

pub async fn create_vault(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Json(req): Json<CreateVaultRequest>,
) -> Result<(StatusCode, Json<VaultResponse>), ServerError> {
    if req.vault_id.is_empty() || req.vault_id.len() > 128 {
        return Err(ServerError::BadRequest(
            "vault_id must be 1-128 characters".into(),
        ));
    }

    let created_at = state.db.create_vault(&req.vault_id, &user_id).await?;

    Ok((
        StatusCode::CREATED,
        Json(VaultResponse {
            id: req.vault_id,
            created_at,
        }),
    ))
}

pub async fn list_vaults(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
) -> Result<Json<Vec<VaultResponse>>, ServerError> {
    let vaults = state.db.list_user_vaults(&user_id).await?;
    let response = vaults
        .into_iter()
        .map(|v| VaultResponse {
            id: v.id,
            created_at: v.created_at,
        })
        .collect();
    Ok(Json(response))
}

/// Verify the authenticated user owns the vault, or return `NotFound`.
pub async fn require_vault_access(
    state: &SharedState,
    user_id: &str,
    vault_id: &str,
) -> Result<(), ServerError> {
    if !state.db.user_owns_vault(user_id, vault_id).await? {
        return Err(ServerError::NotFound);
    }
    Ok(())
}
