pub mod admin;
pub mod auth;
pub mod batches;
pub mod devices;
pub mod relay;
pub mod snapshots;
pub mod vaults;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post, put};

use crate::state::SharedState;

/// Build the full application router.
pub fn router(state: SharedState) -> Router {
    let max_blob = state.config.max_blob_bytes;

    // Auth routes (no auth required, except logout)
    let auth_routes = Router::new()
        .route("/register", post(auth::register))
        .route("/login/init", post(auth::login_init))
        .route("/login/verify", post(auth::login_verify))
        .route("/logout", post(auth::logout))
        .layer(DefaultBodyLimit::max(64 * 1024)); // 64 KB

    // Vault management (auth required)
    let vault_routes =
        Router::new().route("/", post(vaults::create_vault).get(vaults::list_vaults));

    // Batch blob routes (auth required)
    let batch_routes = Router::new()
        .route(
            "/{device_id}/{batch_id}",
            put(batches::put_batch).get(batches::get_batch),
        )
        .route("/", get(batches::list_batches))
        .layer(DefaultBodyLimit::max(max_blob));

    // Snapshot blob routes (auth required)
    let snapshot_routes = Router::new()
        .route(
            "/{snapshot_id}",
            put(snapshots::put_snapshot).get(snapshots::get_snapshot),
        )
        .route("/", get(snapshots::list_snapshots))
        .layer(DefaultBodyLimit::max(max_blob));

    // Device management (auth required)
    let device_routes = Router::new()
        .route("/", get(devices::list_devices))
        .route(
            "/{device_id}",
            put(devices::put_device).delete(devices::delete_device),
        )
        .layer(DefaultBodyLimit::max(64 * 1024));

    // Key exchange relay (auth required)
    let relay_routes = Router::new()
        .route("/offer", post(relay::create_offer))
        .route("/{offer_id}", get(relay::get_offer))
        .route("/{offer_id}/respond", post(relay::post_response))
        .route("/{offer_id}/response", get(relay::get_response))
        .layer(DefaultBodyLimit::max(64 * 1024));

    // Admin API (admin role required — guarded per-handler by AdminUser).
    let admin_routes = Router::new()
        .route("/users", get(admin::list_users).post(admin::create_user))
        .route(
            "/users/{id}",
            axum::routing::patch(admin::update_user).delete(admin::delete_user),
        )
        .route(
            "/invites",
            get(admin::list_invites).post(admin::create_invite),
        )
        .route(
            "/invites/{token}",
            axum::routing::delete(admin::delete_invite),
        )
        .route(
            "/settings",
            get(admin::get_settings).patch(admin::update_settings),
        )
        .route("/stats", get(admin::stats))
        .layer(DefaultBodyLimit::max(64 * 1024));

    Router::new()
        .route("/health", get(health))
        .nest("/api/v1/auth", auth_routes)
        .nest("/api/v1/vaults", vault_routes)
        .nest("/api/v1/vaults/{vault_id}/batches", batch_routes)
        .nest("/api/v1/vaults/{vault_id}/snapshots", snapshot_routes)
        .nest("/api/v1/vaults/{vault_id}/devices", device_routes)
        .nest("/api/v1/relay", relay_routes)
        .nest("/api/v1/admin", admin_routes)
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}
