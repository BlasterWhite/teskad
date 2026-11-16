//! The router: `GET /health`, the protocol's routes under `/v1`, and the
//! protocol's way of refusing for everything else, `{"error": code,
//! "message": text}` with the code's status (docs/native/cloud.md, "Errors"),
//! so that a desktop never has to read an answer that is not JSON.
//!
//! Around every route: a limit on the body's size, and the headers that tell
//! a browser or a cache to do nothing with an answer. Around the routes of
//! `/v1`: the limit per address. The limit per device and the token's check
//! are in `auth::Caller`, which each route with a token takes.

use axum::extract::{DefaultBodyLimit, Request};
use axum::http::header::{self, HeaderName, HeaderValue};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use hangar_cloud_api::{API_VERSION, ErrorCode, Health, MAX_BODY_BYTES, paths};

use crate::refusal::Refusal;
use crate::state::AppState;
use crate::{account, auth, devices, limit, stats};

pub fn router(state: AppState) -> Router {
    let v1 = Router::new()
        .route(
            paths::AUTH_EXCHANGE,
            post(auth::exchange).route_layer(middleware::from_fn_with_state(
                state.clone(),
                auth::limit_exchange,
            )),
        )
        .route(paths::ME, get(account::me))
        .route(paths::DEVICES, get(devices::list))
        .route(
            paths::DEVICE,
            patch(devices::rename).delete(devices::delete),
        )
        .route(paths::STATS, get(stats::get).put(stats::put))
        .route(paths::ACCOUNT, delete(account::delete))
        .route_layer(middleware::from_fn_with_state(state.clone(), limit::per_ip));

    Router::new()
        .route(paths::HEALTH, get(health))
        .merge(v1)
        .fallback(no_such_route)
        .method_not_allowed_fallback(wrong_method)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

/// What the reverse proxy and the deployment ask: the process is up and
/// answering, and which protocol it speaks.
async fn health() -> Json<Health> {
    Json(Health {
        status: "ok".into(),
        api_version: API_VERSION,
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

async fn no_such_route() -> Response {
    Refusal::new(ErrorCode::NotFound, "no such route").into_response()
}

async fn wrong_method() -> Response {
    Refusal::new(
        ErrorCode::MethodNotAllowed,
        "this route does not take this method",
    )
    .into_response()
}

/// On every answer, the refusals included. An answer of this server is JSON
/// for a program: nothing may cache it (some carry a token), a browser must
/// not guess another type for it, frame it, or load anything because of it.
/// A route that sets one of these itself (a page of the sign-in, with a
/// policy of its own) keeps its value.
async fn security_headers(request: Request, next: Next) -> Response {
    const HEADERS: [(HeaderName, &str); 6] = [
        (header::CACHE_CONTROL, "no-store"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::REFERRER_POLICY, "no-referrer"),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; frame-ancestors 'none'",
        ),
        // Only honoured over HTTPS, which is how the server is reached
        // outside of development.
        (header::STRICT_TRANSPORT_SECURITY, "max-age=63072000"),
    ];
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in HEADERS {
        headers
            .entry(name)
            .or_insert(HeaderValue::from_static(value));
    }
    response
}
