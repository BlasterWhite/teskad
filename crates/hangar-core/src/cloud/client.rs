//! The calls to the account server, one function per route of
//! docs/native/cloud.md, with hangar-cloud-api's types in and out.
//!
//! Blocking, over ureq and rustls like every other call Hangar makes; the
//! callers run them off the interface's thread. Statuses are not ureq errors
//! here: a refusal's body is the server's code and sentence, and that is what
//! comes back, with a `401` set apart as "signed out".
//!
//! A token only ever travels in the `Authorization` header, a link code and
//! its verifier in a body: never in a URL, and never in an error.

use std::time::Duration;

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::de::DeserializeOwned;
use serde::Serialize;
use ureq::http::Response;

use super::api::{
    self, paths, ApiError, Device, DevicesResponse, ErrorCode, ExchangeRequest, ExchangeResponse,
    Health, MeResponse, PutStatsRequest, PutStatsResponse, RenameDeviceRequest, StatsQuery,
    StatsResponse, StatsRow,
};
use super::Error;

/// How long the server gets to accept a connection, and to answer a call
/// altogether: a sync runs in the background, but a click on "sign out" or a
/// rename waits on this.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// The longest answer read: the statistics of years, on many machines.
const MAX_ANSWER_BYTES: u64 = 64 * 1024 * 1024;

/// What a value put in a URL keeps as it is (RFC 3986's unreserved).
pub(super) const URL_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub(super) fn url_value(text: &str) -> String {
    utf8_percent_encode(text, URL_VALUE).to_string()
}

enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

pub(super) struct Client {
    base: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(base_url: &str) -> Self {
        let base = base_url.trim().trim_end_matches('/').to_string();
        let agent = ureq::Agent::config_builder()
            // The public server is HTTPS and nothing else; only an address
            // that says `http://` itself (a local server) is reached in clear.
            .https_only(!base.starts_with("http://"))
            // No route of the protocol redirects the desktop.
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent("Hangar")
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(CALL_TIMEOUT))
            .build()
            .new_agent();
        Self { base, agent }
    }

    /// The server's address, without a trailing slash.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// `GET /health`: which version of the protocol the server speaks. The
    /// one call that needs no token.
    pub fn health(&self) -> Result<Health, Error> {
        read(self.call(Method::Get, paths::HEALTH, None, None)?)
    }

    /// `POST /v1/auth/exchange`: the link code and its verifier for a token.
    pub fn exchange(&self, request: &ExchangeRequest) -> Result<ExchangeResponse, Error> {
        read(self.call(
            Method::Post,
            paths::AUTH_EXCHANGE,
            None,
            Some(body(request)?),
        )?)
    }

    /// `GET /v1/me`.
    pub fn me(&self, token: &str) -> Result<MeResponse, Error> {
        read(self.call(Method::Get, paths::ME, Some(token), None)?)
    }

    /// `GET /v1/devices`.
    pub fn devices(&self, token: &str) -> Result<DevicesResponse, Error> {
        read(self.call(Method::Get, paths::DEVICES, Some(token), None)?)
    }

    /// `PATCH /v1/devices/{id}`.
    pub fn rename_device(&self, token: &str, id: &str, name: &str) -> Result<Device, Error> {
        let request = RenameDeviceRequest {
            name: name.to_string(),
        };
        read(self.call(
            Method::Patch,
            &paths::device(&url_value(id)),
            Some(token),
            Some(body(&request)?),
        )?)
    }

    /// `DELETE /v1/devices/{id}`: signed out, or with `forget` deleted along
    /// with its rows.
    pub fn delete_device(&self, token: &str, id: &str, forget: bool) -> Result<(), Error> {
        let mut path = paths::device(&url_value(id));
        if forget {
            path.push_str("?forget=true");
        }
        done(self.call(Method::Delete, &path, Some(token), None)?)
    }

    /// `PUT /v1/stats`: this device's rows, each replacing the one of the
    /// same day, model and project.
    pub fn put_stats(&self, token: &str, rows: &[StatsRow]) -> Result<PutStatsResponse, Error> {
        let request = PutStatsRequest {
            rows: rows.to_vec(),
        };
        read(self.call(
            Method::Put,
            paths::STATS,
            Some(token),
            Some(body(&request)?),
        )?)
    }

    /// `GET /v1/stats`: the account's rows in the range, from all its
    /// machines.
    pub fn get_stats(&self, token: &str, query: &StatsQuery) -> Result<StatsResponse, Error> {
        let mut path = paths::STATS.to_string();
        let ends = [("from", &query.from), ("to", &query.to)];
        for (name, day) in ends {
            if let Some(day) = day {
                path.push(if path.contains('?') { '&' } else { '?' });
                path.push_str(name);
                path.push('=');
                path.push_str(&url_value(day));
            }
        }
        read(self.call(Method::Get, &path, Some(token), None)?)
    }

    /// `DELETE /v1/account`.
    pub fn delete_account(&self, token: &str) -> Result<(), Error> {
        done(self.call(Method::Delete, paths::ACCOUNT, Some(token), None)?)
    }

    fn call(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Vec<u8>>,
    ) -> Result<Response<ureq::Body>, Error> {
        let url = format!("{}{path}", self.base);
        let body = body.unwrap_or_default();
        let json = "application/json";
        // ureq types requests by whether they carry a body, so the methods
        // cannot share one builder chain.
        let sent = match method {
            Method::Get => signed(self.agent.get(&url), token).call(),
            Method::Delete => signed(self.agent.delete(&url), token).call(),
            Method::Post => signed(self.agent.post(&url), token)
                .content_type(json)
                .send(body.as_slice()),
            Method::Put => signed(self.agent.put(&url), token)
                .content_type(json)
                .send(body.as_slice()),
            Method::Patch => signed(self.agent.patch(&url), token)
                .content_type(json)
                .send(body.as_slice()),
        };
        // ureq's errors name what failed (a timeout, a refused connection, a
        // certificate), never a header or a body.
        sent.map_err(|err| Error::Unreachable(err.to_string()))
    }
}

fn signed<B>(request: ureq::RequestBuilder<B>, token: Option<&str>) -> ureq::RequestBuilder<B> {
    match token {
        Some(token) => request.header("authorization", format!("Bearer {token}")),
        None => request,
    }
}

fn body(value: &impl Serialize) -> Result<Vec<u8>, Error> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| Error::Local("a request could not be written".into()))?;
    if bytes.len() > api::MAX_BODY_BYTES {
        return Err(Error::Invalid(format!(
            "a request of {} bytes is over the server's {}",
            bytes.len(),
            api::MAX_BODY_BYTES
        )));
    }
    Ok(bytes)
}

/// A success's body as `T`, or the refusal.
fn read<T: DeserializeOwned>(response: Response<ureq::Body>) -> Result<T, Error> {
    let text = text_of(response)?;
    // serde's own message can quote the value it tripped on (a name, an
    // email): it is not passed on.
    serde_json::from_str(&text)
        .map_err(|_| Error::Answer(format!("not the JSON of {}", short_type::<T>())))
}

/// A success with nothing to read (`204`), or the refusal.
fn done(response: Response<ureq::Body>) -> Result<(), Error> {
    text_of(response).map(|_| ())
}

fn short_type<T>() -> &'static str {
    let name = std::any::type_name::<T>();
    name.rsplit("::").next().unwrap_or(name)
}

/// The body of a success; a refusal as the error it is.
fn text_of(mut response: Response<ureq::Body>) -> Result<String, Error> {
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let text = response
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER_BYTES)
        .lossy_utf8(true)
        .read_to_string()
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    if (200..300).contains(&status) {
        return Ok(text);
    }
    if status == 401 {
        return Err(Error::SignedOut);
    }
    // A proxy's page rather than the server's JSON is a refusal all the
    // same, with nothing known of it but its status.
    let error = serde_json::from_str::<ApiError>(&text)
        .unwrap_or_else(|_| ApiError::new(ErrorCode::Unknown, format!("HTTP {status}")));
    Err(Error::Refused {
        status,
        error,
        retry_after,
    })
}
