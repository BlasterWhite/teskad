//! The router. `GET /health` for now, and the protocol's way of refusing for
//! everything else: `{"error": code, "message": text}` with the code's status
//! (docs/native/cloud.md, "Errors"), so that a desktop never has to read an
//! answer that is not JSON.

use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use hangar_cloud_api::{API_VERSION, ApiError, ErrorCode, Health, MAX_BODY_BYTES, paths};

pub fn router() -> Router {
    Router::new()
        .route(paths::HEALTH, get(health))
        .fallback(no_such_route)
        .method_not_allowed_fallback(wrong_method)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
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

/// A refusal, as the protocol writes it.
pub fn refuse(code: ErrorCode, message: &str) -> Response {
    let status = StatusCode::from_u16(code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(ApiError::new(code, message))).into_response()
}

async fn no_such_route() -> Response {
    refuse(ErrorCode::NotFound, "no such route")
}

async fn wrong_method() -> Response {
    refuse(
        ErrorCode::MethodNotAllowed,
        "this route does not take this method",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request, header};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn call(method: Method, path: &str) -> (StatusCode, String, Vec<u8>) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let response = router().oneshot(request).await.unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, content_type, body.to_vec())
    }

    #[tokio::test]
    async fn health_says_ok_and_which_protocol_the_server_speaks() {
        let (status, content_type, body) = call(Method::GET, paths::HEALTH).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, "application/json");
        let health: Health = serde_json::from_slice(&body).unwrap();
        assert_eq!(health.status, "ok");
        assert_eq!(health.api_version, API_VERSION);
        assert_eq!(health.version, env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn a_route_that_does_not_exist_is_refused_in_the_protocol_s_words() {
        // The protocol's own routes too, until they are written.
        for path in ["/", "/v1/nothing", paths::ME] {
            let (status, content_type, body) = call(Method::GET, path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            assert_eq!(content_type, "application/json", "{path}");
            let error: ApiError = serde_json::from_slice(&body).unwrap();
            assert_eq!(error.error, ErrorCode::NotFound, "{path}");
        }
    }

    #[tokio::test]
    async fn a_method_a_route_does_not_take_is_refused_the_same_way() {
        let (status, content_type, body) = call(Method::POST, paths::HEALTH).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(content_type, "application/json");
        let error: ApiError = serde_json::from_slice(&body).unwrap();
        assert_eq!(error.error, ErrorCode::MethodNotAllowed);
    }
}
