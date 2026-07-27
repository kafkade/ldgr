//! Scoped CORS layer for the browser-based admin panel and web client.
//!
//! The admin UI (`apps/web`, Apache-2.0) is a static export that can be served
//! from a different origin than this API. Browsers then block its `fetch` calls
//! unless the server returns CORS headers. We grant access **only** to an
//! explicit, operator-configured allowlist (`LDGR_ALLOWED_ORIGINS`) — never a
//! wildcard/permissive default — because this is a zero-knowledge product where
//! the safe posture is to deny cross-origin access unless deliberately enabled.
//!
//! A same-origin deployment (the web app reverse-proxied behind the same host as
//! the API) needs no allowlist entry at all and therefore no CORS headers.

use axum::http::{HeaderValue, Method, header};
use tower_http::cors::{AllowOrigin, CorsLayer};

/// Build a scoped [`CorsLayer`] from an explicit origin allowlist.
///
/// Returns `None` when the allowlist is empty (or contains no parseable
/// origins), in which case no CORS layer should be attached and cross-origin
/// browser requests remain blocked by default. Entries that are not valid
/// header values are skipped with a warning rather than aborting startup.
///
/// The layer allows only the methods and headers the admin + sync API needs and
/// does **not** enable credentials: authentication is a `Bearer` token carried
/// in the `Authorization` header (never a cookie), so `Access-Control-Allow-
/// Credentials` is unnecessary and deliberately omitted.
#[must_use]
pub fn cors_layer(allowed_origins: &[String]) -> Option<CorsLayer> {
    let origins: Vec<HeaderValue> = allowed_origins
        .iter()
        .filter_map(|origin| {
            HeaderValue::from_str(origin)
                .inspect_err(|_| {
                    tracing::warn!("ignoring invalid LDGR_ALLOWED_ORIGINS entry: {origin:?}");
                })
                .ok()
        })
        .collect();

    if origins.is_empty() {
        return None;
    }

    // Cache preflight results for an hour. Kept in seconds (rather than
    // `Duration::from_hours`, which is newer than our MSRV); the pedantic
    // "use a larger unit" lint is intentional here.
    #[allow(clippy::duration_suboptimal_units)]
    let preflight_max_age = std::time::Duration::from_secs(3600);

    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
            .max_age(preflight_max_age),
    )
}
