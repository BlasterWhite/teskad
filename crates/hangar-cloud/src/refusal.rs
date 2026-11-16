//! A refusal, as the protocol writes it: `{"error": code, "message": text}`
//! with the code's status (docs/native/cloud.md, "Errors").
//!
//! Every handler and every extractor of this server fails with this type, so
//! that a desktop never has to read an answer that is not JSON, and so that
//! nothing of the server's inside (an SQL error, a path, a panic's text) is
//! ever in an answer: what went wrong inside is written to stderr, and the
//! answer says `internal`.

use std::borrow::Cow;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use hangar_cloud_api::{ApiError, ErrorCode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: ErrorCode,
    pub message: Cow<'static, str>,
    /// Seconds, for `Retry-After` on a `rate_limited`.
    pub retry_after: Option<u64>,
}

impl Refusal {
    pub fn new(code: ErrorCode, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            code,
            message: message.into(),
            retry_after: None,
        }
    }

    pub fn bad_request(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCode::BadRequest, message)
    }

    /// One sentence for every way a token can be wrong: which one it was is
    /// not said.
    pub fn unauthorized() -> Self {
        Self::new(
            ErrorCode::Unauthorized,
            "no device token, or one that is unknown or revoked",
        )
    }

    /// One sentence for every way an exchange can fail, as the protocol asks.
    pub fn invalid_code() -> Self {
        Self::new(
            ErrorCode::InvalidCode,
            "the link code is unknown, used or expired, or the verifier does not match",
        )
    }

    pub fn no_such_device() -> Self {
        Self::new(ErrorCode::NotFound, "no such device on this account")
    }

    pub fn rate_limited(retry_after_secs: u64) -> Self {
        Self {
            code: ErrorCode::RateLimited,
            message: "too many requests".into(),
            retry_after: Some(retry_after_secs.max(1)),
        }
    }

    /// The server's own fault. `what` goes to stderr and never to the caller;
    /// it must not carry a token, a code or an email, and what calls this
    /// hands in the name of a step or a library's error, never a request's
    /// content.
    pub fn internal(what: impl std::fmt::Display) -> Self {
        eprintln!("hangar-cloud: internal error: {what}");
        Self::new(ErrorCode::Internal, "the server failed to answer")
    }
}

/// SQLite's errors name a constraint or a column, never a value.
impl From<rusqlite::Error> for Refusal {
    fn from(err: rusqlite::Error) -> Self {
        Self::internal(format_args!("database: {err}"))
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut response = (status, Json(ApiError::new(self.code, self.message))).into_response();
        if let Some(secs) = self.retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(secs));
        }
        response
    }
}
