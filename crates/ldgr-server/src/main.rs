use std::sync::Arc;

use tower_http::trace::TraceLayer;

use ldgr_server::{api, auth, config, cors, state, storage};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ldgr_server=info,tower_http=info".parse().unwrap()),
        )
        .init();

    let config = config::Config::from_env();
    tracing::info!("starting ldgr-server on {}", config.bind_addr);

    let db = storage::ServerDb::open(&config.db_path)?;
    let srp_ttl = std::time::Duration::from_secs(config.srp_handshake_ttl_secs);

    let state = Arc::new(state::AppState {
        db,
        srp_handshakes: auth::srp::SrpHandshakeStore::new(srp_ttl),
        config,
    });

    let mut app = api::router(state.clone()).layer(TraceLayer::new_for_http());

    // Attach the scoped CORS layer only when an allowlist is configured; with no
    // `LDGR_ALLOWED_ORIGINS` set, cross-origin browser requests stay blocked
    // (the secure default). Same-origin deployments need no allowlist.
    if let Some(cors) = cors::cors_layer(&state.config.allowed_origins) {
        tracing::info!(
            "CORS enabled for {} allowed origin(s)",
            state.config.allowed_origins.len()
        );
        app = app.layer(cors);
    }

    let listener = tokio::net::TcpListener::bind(state.config.bind_addr).await?;
    tracing::info!(
        "ldgr-server ready — listening on {}",
        state.config.bind_addr
    );

    axum::serve(listener, app).await?;

    Ok(())
}
