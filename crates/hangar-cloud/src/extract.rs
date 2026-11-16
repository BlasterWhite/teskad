//! axum's extractors, refusing in the protocol's words. axum's own answer a
//! body it cannot read with a line of plain text that quotes the parser; a
//! desktop reads `{"error", "message"}` and nothing else.

use axum::Json;
use axum::extract::{FromRequest, FromRequestParts, Path, Query, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use hangar_cloud_api::ErrorCode;
use serde::de::DeserializeOwned;

use crate::refusal::Refusal;

/// A JSON body. `413 too_large` over the router's body limit, `400
/// bad_request` for anything that is not the JSON the route takes (a missing
/// field, a negative counter, another content type).
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Refusal;

    async fn from_request(request: Request, state: &S) -> Result<Self, Refusal> {
        match Json::<T>::from_request(request, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => Err(
                Refusal::new(ErrorCode::TooLarge, "the body is larger than 4 MiB"),
            ),
            Err(_) => Err(Refusal::bad_request(
                "the body is not the JSON this route takes",
            )),
        }
    }
}

/// A query string. `400 bad_request` when it is not what the route takes.
pub struct ApiQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Refusal;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Refusal> {
        Query::<T>::try_from_uri(&parts.uri)
            .map(|Query(value)| Self(value))
            .map_err(|_| Refusal::bad_request("the query is not what this route takes"))
    }
}

/// The `{id}` of `/v1/devices/{id}`. One that cannot be read (bytes that are
/// not text) names no device.
pub struct DeviceId(pub String);

impl<S: Send + Sync> FromRequestParts<S> for DeviceId {
    type Rejection = Refusal;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Refusal> {
        Path::<String>::from_request_parts(parts, state)
            .await
            .map(|Path(id)| Self(id))
            .map_err(|_| Refusal::no_such_device())
    }
}
