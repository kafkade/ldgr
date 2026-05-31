//! Auth middleware — extracts authenticated user from Bearer token.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use super::session::hash_token_hex;
use crate::error::ServerError;
use crate::state::SharedState;

/// Extractor that validates the `Authorization: Bearer <token>` header
/// and provides the authenticated user's ID.
pub struct AuthUser(pub String);

impl FromRequestParts<SharedState> for AuthUser {
    type Rejection = ServerError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, Self::Rejection> {
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

        let user_id = state
            .db
            .validate_session(&token_hash)
            .await?
            .ok_or(ServerError::Unauthorized)?;

        Ok(AuthUser(user_id))
    }
}
