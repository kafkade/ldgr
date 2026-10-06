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
    /// Identifier the client wants to keep. Absent when the client has none yet
    /// and wants the server to mint one (ADR-011).
    #[serde(default)]
    pub vault_id: Option<String>,
}

#[derive(Serialize)]
pub struct VaultResponse {
    pub id: String,
    pub created_at: String,
}

/// `POST /api/v1/vaults`
///
/// Claim a vault for the authenticated account. The response `id` is
/// authoritative and may differ from the requested one — see
/// [`ServerDb::claim_vault`](crate::storage::ServerDb::claim_vault) for the
/// resolution order and why a taken identifier mints a new one instead of
/// returning a conflict.
pub async fn create_vault(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Json(req): Json<CreateVaultRequest>,
) -> Result<(StatusCode, Json<VaultResponse>), ServerError> {
    // Only the length contract is enforced here, matching what pre-ADR-011
    // servers rejected. The character-set rule lives in `claim_vault` so it can
    // gate *new* identifiers without rejecting one the account already owns —
    // the iOS and web clients used to let users type anything, so identifiers
    // like `Family Vault` exist in the wild and must stay claimable.
    if let Some(requested) = req.vault_id.as_deref()
        && (requested.is_empty() || requested.len() > ldgr_core::sync::MAX_VAULT_ID_LEN)
    {
        return Err(ServerError::BadRequest(format!(
            "vault_id must be 1-{} characters",
            ldgr_core::sync::MAX_VAULT_ID_LEN
        )));
    }

    let vault = state
        .db
        .claim_vault(req.vault_id.as_deref(), &user_id)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(VaultResponse {
            id: vault.id,
            created_at: vault.created_at,
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
