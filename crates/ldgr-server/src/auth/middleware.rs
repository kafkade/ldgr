//! Auth middleware — extracts authenticated user from Bearer token.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use super::session::hash_token_hex;
use crate::error::ServerError;
use crate::state::SharedState;

/// Validate the `Authorization: Bearer <token>` header against an active
/// session and return the authenticated user's id. Shared by the [`AuthUser`]
/// and [`AdminUser`] extractors.
async fn authenticate(parts: &Parts, state: &SharedState) -> Result<String, ServerError> {
    let header = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(ServerError::Unauthorized)?;

    let token = header
        .strip_prefix("Bearer ")
        .ok_or(ServerError::Unauthorized)?;

    if token.is_empty() {
        return Err(ServerError::Unauthorized);
    }

    let token_hash = hash_token_hex(token).map_err(|_| ServerError::Unauthorized)?;

    state
        .db
        .validate_session(&token_hash)
        .await?
        .ok_or(ServerError::Unauthorized)
}

/// Extractor that validates the `Authorization: Bearer <token>` header
/// and provides the authenticated user's ID.
pub struct AuthUser(pub String);

impl FromRequestParts<SharedState> for AuthUser {
    type Rejection = ServerError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, Self::Rejection> {
        Ok(AuthUser(authenticate(parts, state).await?))
    }
}

/// Extractor that requires the authenticated user to be an active `admin`.
///
/// Rejects a missing/invalid session with `401` and a valid non-admin (or a
/// disabled admin) session with `403`, so every admin route can simply take an
/// `AdminUser` argument to be guarded (ADR-008, #176).
pub struct AdminUser(pub String);

impl FromRequestParts<SharedState> for AdminUser {
    type Rejection = ServerError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, Self::Rejection> {
        let user_id = authenticate(parts, state).await?;
        let user = state
            .db
            .get_user_by_id(&user_id)
            .await?
            .ok_or(ServerError::Unauthorized)?;
        if user.role != "admin" || user.status != "active" {
            return Err(ServerError::Forbidden(
                "administrator privileges required".into(),
            ));
        }
        Ok(AdminUser(user_id))
    }
}
