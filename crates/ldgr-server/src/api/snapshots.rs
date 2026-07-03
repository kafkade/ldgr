//! Encrypted snapshot blob endpoints (push / pull / list).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::batches::{ListBlobsResponse, build_list_response};
use super::vaults::require_vault_access;
use crate::auth::hex_encode;
use crate::auth::middleware::AuthUser;
use crate::error::ServerError;
use crate::state::SharedState;

#[derive(Serialize)]
pub struct PutSnapshotResponse {
    pub path: String,
    pub size: i64,
    pub content_hash: String,
}

/// `PUT /api/v1/vaults/:vault_id/snapshots/:snapshot_id`
///
/// Upload an encrypted snapshot. Put-if-absent semantics.
pub async fn put_snapshot(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Path((vault_id, snapshot_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<PutSnapshotResponse>), ServerError> {
    require_vault_access(&state, &user_id, &vault_id).await?;

    if body.is_empty() {
        return Err(ServerError::BadRequest("empty body".into()));
    }

    let path = format!("{vault_id}/snapshots/{snapshot_id}.enc");
    let content_hash = hex_encode(&Sha256::digest(&body));

    let default_quota = crate::settings::default_quota_bytes(&state).await?;
    let meta = state
        .db
        .put_blob(
            &path,
            &vault_id,
            body.to_vec(),
            &content_hash,
            default_quota,
        )
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(PutSnapshotResponse {
            path: meta.path,
            size: meta.size,
            content_hash: meta.content_hash,
        }),
    ))
}

/// `GET /api/v1/vaults/:vault_id/snapshots/:snapshot_id`
///
/// Download an encrypted snapshot.
pub async fn get_snapshot(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Path((vault_id, snapshot_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ServerError> {
    require_vault_access(&state, &user_id, &vault_id).await?;

    let path = format!("{vault_id}/snapshots/{snapshot_id}.enc");
    let data = state
        .db
        .get_blob(&path)
        .await?
        .ok_or(ServerError::NotFound)?;

    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data))
}

#[derive(Deserialize)]
pub struct ListSnapshotsQuery {
    pub since: Option<String>,
    pub limit: Option<u32>,
}

/// `GET /api/v1/vaults/:vault_id/snapshots`
///
/// List snapshot blobs.
pub async fn list_snapshots(
    State(state): State<SharedState>,
    AuthUser(user_id): AuthUser,
    Path(vault_id): Path<String>,
    Query(params): Query<ListSnapshotsQuery>,
) -> Result<Json<ListBlobsResponse>, ServerError> {
    require_vault_access(&state, &user_id, &vault_id).await?;

    let limit = params.limit.unwrap_or(100).min(1000);
    let prefix = format!("{vault_id}/snapshots/");

    let metas = state
        .db
        .list_blobs(&vault_id, Some(&prefix), params.since.as_deref(), limit + 1)
        .await?;

    Ok(Json(build_list_response(metas, limit as usize)))
}
