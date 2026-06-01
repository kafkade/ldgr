//! Key exchange relay endpoints for device onboarding.
//!
//! Relay offers are ephemeral — they expire after a configurable TTL
//! (default 10 minutes). Both the offer creator and responder must be
//! authenticated as the same user.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::auth::hex_encode;
use crate::auth::middleware::AuthUser;
use crate::error::ServerError;
use crate::state::SharedState;

#[derive(Deserialize)]
pub struct CreateOfferRequest {
    /// Hex-encoded encrypted offer payload (opaque to server).
    pub offer_data: String,
}

#[derive(Serialize)]
pub struct CreateOfferResponse {
    pub offer_id: String,
    pub expires_at: String,
}

/// `POST /api/v1/relay/offer`
pub async fn create_offer(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Json(req): Json<CreateOfferRequest>,
) -> Result<(StatusCode, Json<CreateOfferResponse>), ServerError> {
    let offer_data = crate::auth::hex_decode(&req.offer_data)
        .map_err(|e| ServerError::BadRequest(format!("invalid offer_data: {e}")))?;

    if offer_data.is_empty() || offer_data.len() > 65536 {
        return Err(ServerError::BadRequest(
            "offer_data must be 1-65536 bytes".into(),
        ));
    }

    let offer_id = uuid::Uuid::now_v7().to_string();
    #[allow(clippy::cast_possible_wrap)]
    let expires_at = (chrono::Utc::now()
        + chrono::Duration::minutes(state.config.relay_ttl_minutes as i64))
    .to_rfc3339();

    state
        .db
        .create_relay_offer(&offer_id, &user_id, offer_data, &expires_at)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(CreateOfferResponse {
            offer_id,
            expires_at,
        }),
    ))
}

#[derive(Serialize)]
pub struct OfferResponse {
    pub offer_id: String,
    /// Hex-encoded encrypted offer payload.
    pub offer_data: String,
    pub expires_at: String,
    pub has_response: bool,
}

/// `GET /api/v1/relay/:offer_id`
pub async fn get_offer(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Path(offer_id): Path<String>,
) -> Result<Json<OfferResponse>, ServerError> {
    let offer = state
        .db
        .get_relay_offer(&offer_id, &user_id)
        .await?
        .ok_or(ServerError::NotFound)?;

    Ok(Json(OfferResponse {
        offer_id: offer.id,
        offer_data: hex_encode(&offer.offer_data),
        expires_at: offer.expires_at,
        has_response: offer.response_data.is_some(),
    }))
}

#[derive(Deserialize)]
pub struct PostResponseRequest {
    /// Hex-encoded encrypted response payload.
    pub response_data: String,
}

/// `POST /api/v1/relay/:offer_id/respond`
pub async fn post_response(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Path(offer_id): Path<String>,
    Json(req): Json<PostResponseRequest>,
) -> Result<StatusCode, ServerError> {
    let response_data = crate::auth::hex_decode(&req.response_data)
        .map_err(|e| ServerError::BadRequest(format!("invalid response_data: {e}")))?;

    if response_data.is_empty() || response_data.len() > 65536 {
        return Err(ServerError::BadRequest(
            "response_data must be 1-65536 bytes".into(),
        ));
    }

    if state
        .db
        .set_relay_response(&offer_id, &user_id, response_data)
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ServerError::NotFound)
    }
}

#[derive(Serialize)]
pub struct GetResponseResponse {
    pub response_data: String,
}

/// `GET /api/v1/relay/:offer_id/response`
pub async fn get_response(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Path(offer_id): Path<String>,
) -> Result<Json<GetResponseResponse>, ServerError> {
    let offer = state
        .db
        .get_relay_offer(&offer_id, &user_id)
        .await?
        .ok_or(ServerError::NotFound)?;

    let response_data = offer.response_data.ok_or(ServerError::NotFound)?;

    Ok(Json(GetResponseResponse {
        response_data: hex_encode(&response_data),
    }))
}
