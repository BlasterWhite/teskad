//! What every route answers when it refuses: `{"error": code, "message":
//! text}`, with the HTTP status the code maps to.

use serde::{Deserialize, Serialize};

/// The body of every answer that is not a success. `message` is English, for
/// logs and bug reports; the desktop words what it shows from `error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: ErrorCode,
    pub message: String,
}

impl ApiError {
    pub fn new(error: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.error.as_str())
    }
}

impl std::error::Error for ApiError {}

/// Why a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request is not what the route takes: a field missing, a name too
    /// long, a day that is not a day.
    BadRequest,
    /// No device token, or one that is unknown or revoked. The desktop
    /// forgets its token and shows the account as signed out.
    Unauthorized,
    /// The link code is unknown, already used or expired, or the verifier
    /// does not match the challenge. Which of them is not said.
    InvalidCode,
    /// No such device on this account, or no such route.
    NotFound,
    /// The route exists and does not take this method.
    MethodNotAllowed,
    /// More than `MAX_STATS_ROWS` rows, or a body over `MAX_BODY_BYTES`.
    TooLarge,
    /// Too many requests; `Retry-After` says when to come back.
    RateLimited,
    /// The server's own fault.
    Internal,
    /// GitHub or Google refused or answered nonsense.
    ProviderError,
    /// This server has no client id for the provider asked for.
    ProviderUnavailable,
    /// A code a newer server sends and this version does not know. Never
    /// sent; read as a refusal like any other.
    #[serde(other)]
    Unknown,
}

impl ErrorCode {
    /// Every code a server sends, which leaves `Unknown` out.
    pub const ALL: [ErrorCode; 10] = [
        ErrorCode::BadRequest,
        ErrorCode::Unauthorized,
        ErrorCode::InvalidCode,
        ErrorCode::NotFound,
        ErrorCode::MethodNotAllowed,
        ErrorCode::TooLarge,
        ErrorCode::RateLimited,
        ErrorCode::Internal,
        ErrorCode::ProviderError,
        ErrorCode::ProviderUnavailable,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::BadRequest => "bad_request",
            ErrorCode::Unauthorized => "unauthorized",
            ErrorCode::InvalidCode => "invalid_code",
            ErrorCode::NotFound => "not_found",
            ErrorCode::MethodNotAllowed => "method_not_allowed",
            ErrorCode::TooLarge => "too_large",
            ErrorCode::RateLimited => "rate_limited",
            ErrorCode::Internal => "internal",
            ErrorCode::ProviderError => "provider_error",
            ErrorCode::ProviderUnavailable => "provider_unavailable",
            ErrorCode::Unknown => "unknown",
        }
    }

    /// The HTTP status the server answers this code with.
    pub fn status(self) -> u16 {
        match self {
            ErrorCode::BadRequest | ErrorCode::InvalidCode => 400,
            ErrorCode::Unauthorized => 401,
            ErrorCode::NotFound => 404,
            ErrorCode::MethodNotAllowed => 405,
            ErrorCode::TooLarge => 413,
            ErrorCode::RateLimited => 429,
            ErrorCode::Internal | ErrorCode::Unknown => 500,
            ErrorCode::ProviderError => 502,
            ErrorCode::ProviderUnavailable => 503,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::round_trip;
    use serde_json::json;

    #[test]
    fn an_error_is_a_code_and_a_message() {
        round_trip(
            &ApiError::new(ErrorCode::InvalidCode, "the link code has expired"),
            json!({ "error": "invalid_code", "message": "the link code has expired" }),
        );
    }

    #[test]
    fn every_code_is_its_name_on_the_wire_and_an_error_status() {
        for code in ErrorCode::ALL {
            round_trip(&code, json!(code.as_str()));
            assert!((400..600).contains(&code.status()), "{code:?}");
        }
    }

    #[test]
    fn a_code_from_a_newer_server_still_reads() {
        let error: ApiError =
            serde_json::from_value(json!({ "error": "quota_exceeded", "message": "later" }))
                .unwrap();
        assert_eq!(error.error, ErrorCode::Unknown);
        assert_eq!(error.message, "later");
    }
}
