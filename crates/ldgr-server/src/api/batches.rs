//! Encrypted batch blob endpoints (push / pull / list).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::vaults::require_vault_access;
use crate::auth::hex_encode;
use crate::auth::middleware::AuthUser;
use crate::error::ServerError;
use crate::state::SharedState;

#[derive(Serialize)]
pub struct PutBlobResponse {
    pub path: String,
    pub size: i64,
    pub content_hash: String,
}

/// `PUT /api/v1/vaults/:vault_id/batches/:device_id/:batch_id`
///
/// Upload an encrypted batch blob. Put-if-absent semantics — returns 409 if
/// the blob already exists.
pub async fn put_batch(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Path((vault_id, device_id, batch_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<PutBlobResponse>), ServerError> {
    require_vault_access(&state, &user_id, &vault_id).await?;

    if body.is_empty() {
        return Err(ServerError::BadRequest("empty body".into()));
    }

    let path = format!("{vault_id}/batches/{device_id}/{batch_id}.enc");
    let content_hash = hex_encode(&Sha256::digest(&body));

    let meta = state
        .db
        .put_blob(
            &path,
            &vault_id,
            body.to_vec(),
            &content_hash,
            state.config.default_user_quota_bytes,
        )
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(PutBlobResponse {
            path: meta.path,
            size: meta.size,
            content_hash: meta.content_hash,
        }),
    ))
}

/// `GET /api/v1/vaults/:vault_id/batches/:device_id/:batch_id`
///
/// Download an encrypted batch blob.
pub async fn get_batch(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Path((vault_id, device_id, batch_id)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, ServerError> {
    require_vault_access(&state, &user_id, &vault_id).await?;

    let path = format!("{vault_id}/batches/{device_id}/{batch_id}.enc");
    let data = state
        .db
        .get_blob(&path)
        .await?
        .ok_or(ServerError::NotFound)?;

    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data))
}

// ── List ──────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ListQuery {
    pub since: Option<String>,
    pub device_id: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Serialize)]
pub struct BlobEntry {
    pub path: String,
    pub size: i64,
    pub content_hash: String,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct ListBlobsResponse {
    pub entries: Vec<BlobEntry>,
    pub has_more: bool,
}

/// `GET /api/v1/vaults/:vault_id/batches`
///
/// List batch blobs with optional filters.
pub async fn list_batches(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Path(vault_id): Path<String>,
    Query(params): Query<ListQuery>,
) -> Result<Json<ListBlobsResponse>, ServerError> {
    require_vault_access(&state, &user_id, &vault_id).await?;

    let limit = params.limit.unwrap_or(100).min(1000);

    let prefix = params.device_id.as_ref().map_or_else(
        || format!("{vault_id}/batches/"),
        |did| format!("{vault_id}/batches/{did}/"),
    );

    let entries = state
        .db
        .list_blobs(
            &vault_id,
            Some(&prefix),
            params.since.as_deref(),
            limit + 1, // Fetch one extra to detect has_more
        )
        .await?;

    let has_more = entries.len() > limit as usize;
    let entries: Vec<BlobEntry> = entries
        .into_iter()
        .take(limit as usize)
        .map(|m| BlobEntry {
            path: m.path,
            size: m.size,
            content_hash: m.content_hash,
            created_at: m.created_at,
        })
        .collect();

    Ok(Json(ListBlobsResponse { entries, has_more }))
}
