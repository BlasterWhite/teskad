//! Who is asking: the link code a sign-in ends with, its exchange for a
//! device token, and the token's check on every request after that
//! (docs/native/cloud.md, "Signing in" and "With a device token").

use std::time::Instant;

use axum::Json;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use hangar_cloud_api::{
    Device, ExchangeRequest, ExchangeResponse, LINK_CODE_TTL_SECS, MAX_DEVICE_NAME_CHARS,
    MAX_MACHINE_ID_CHARS, MachineInfo,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::extract::ApiJson;
use crate::limit::ClientIp;
use crate::refusal::Refusal;
use crate::secrets::{self, Hash};
use crate::state::AppState;
use crate::{account, devices};

/// The device a request's token names, and its account. Taking this as an
/// argument is what makes a route need a token: `401 unauthorized` without a
/// valid one, before anything of the request's body is read.
#[derive(Debug, Clone)]
pub struct Caller {
    pub account_id: String,
    pub device: Device,
}

impl FromRequestParts<AppState> for Caller {
    type Rejection = Refusal;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Refusal> {
        let hash = bearer_token(parts)
            .and_then(secrets::device_token_hash)
            .ok_or_else(Refusal::unauthorized)?;
        let now = state.now();
        let caller = state
            .db
            .call(move |conn| devices::authenticate(conn, &hash, now))
            .await?
            .ok_or_else(Refusal::unauthorized)?;
        // Counted by device, once the token is known to be one: a wrong
        // token is counted by its address only.
        state
            .limits
            .device
            .check(caller.device.id.clone(), Instant::now())
            .map_err(Refusal::rate_limited)?;
        Ok(caller)
    }
}

/// The token of `Authorization: Bearer <token>`. Exactly one such header.
fn bearer_token(parts: &Parts) -> Option<&str> {
    let mut values = parts.headers.get_all(AUTHORIZATION).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim_matches(' '))
}

/// A link code for `account_id`: what the sign-in routes send the browser
/// back to the desktop with, once the provider has said who the person is.
/// `challenge` is the desktop's PKCE challenge, as it gave it at the start;
/// `issued_at` is when the provider's callback was answered, which the
/// code's five minutes are counted from. The code is returned here and
/// nowhere else: the database keeps its SHA-256.
pub fn create_link_code(
    conn: &Connection,
    account_id: &str,
    challenge: &str,
    issued_at: i64,
) -> Result<String, Refusal> {
    if !secrets::is_challenge(challenge) {
        // The start refuses such a challenge; one that reaches this is a bug
        // of the server's, and a code nobody could redeem.
        return Err(Refusal::internal(
            "a link code asked for a malformed challenge",
        ));
    }
    let code = secrets::new_link_code()?;
    conn.execute(
        "INSERT INTO link_codes (code_hash, account_id, challenge, expires_at) \
         VALUES (?1, ?2, ?3, ?4)",
        params![
            &secrets::sha256(&code)[..],
            account_id,
            challenge,
            issued_at + link_code_ttl_ms()
        ],
    )?;
    Ok(code)
}

fn link_code_ttl_ms() -> i64 {
    i64::try_from(LINK_CODE_TTL_SECS * 1000).unwrap_or(i64::MAX)
}

/// Deletes the sign-ins nobody can finish any more: pending states past
/// their ten minutes, link codes past their five. Run at each exchange and
/// once a minute.
pub fn purge_expired(conn: &Connection, now: i64) -> Result<(), Refusal> {
    conn.execute(
        "DELETE FROM link_codes WHERE expires_at <= ?1",
        params![now],
    )?;
    conn.execute(
        "DELETE FROM auth_states WHERE expires_at <= ?1",
        params![now],
    )?;
    Ok(())
}

/// The exchange's own limit, per address, counted before its body is read.
/// Runs inside `limit::per_ip`, which found the address.
pub async fn limit_exchange(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(&ClientIp(ip)) = request.extensions().get::<ClientIp>() else {
        return Refusal::internal("the exchange was routed without its address").into_response();
    };
    if let Err(secs) = state.limits.exchange.check(ip, Instant::now()) {
        return Refusal::rate_limited(secs).into_response();
    }
    next.run(request).await
}

/// `POST /v1/auth/exchange`: a link code and its verifier for a device token.
pub async fn exchange(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<ExchangeRequest>,
) -> Result<Json<ExchangeResponse>, Refusal> {
    // A machine the server would not store is refused before the code is
    // looked at: the desktop's mistake does not cost the person their
    // sign-in.
    let machine = request.machine;
    devices::check_text("machine.id", &machine.id, MAX_MACHINE_ID_CHARS)?;
    devices::check_text("machine.name", &machine.name, MAX_DEVICE_NAME_CHARS)?;
    devices::check_text("machine.os", &machine.os, MAX_MACHINE_ID_CHARS)?;

    let code_hash = secrets::sha256(&request.code);
    let verifier = request.verifier;
    let token = secrets::new_device_token()?;
    let token_hash = secrets::sha256(&token);
    let now = state.now();
    let (account, device) = state
        .db
        .call(move |conn| redeem(conn, &code_hash, &verifier, &machine, &token_hash, now))
        .await?;
    Ok(Json(ExchangeResponse {
        token,
        account,
        device,
    }))
}

/// One transaction: the code is consumed, the verifier checked, the device
/// signed in. A code is consumed by the first exchange that presents it,
/// whether or not its verifier matches: whoever holds a code without its
/// verifier gets one guess, and the right desktop then signs in again.
fn redeem(
    conn: &mut Connection,
    code_hash: &Hash,
    verifier: &str,
    machine: &MachineInfo,
    token_hash: &Hash,
    now: i64,
) -> Result<(hangar_cloud_api::Account, Device), Refusal> {
    // Immediate: the write lock is taken before the code is read, not
    // between reading it and marking it used.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    purge_expired(&tx, now)?;
    // Looked up by its hash, and marked used by the statement that finds it:
    // two exchanges of one code cannot both find it unused.
    let found: Option<(String, String)> = tx
        .query_row(
            "UPDATE link_codes SET used_at = ?1 \
             WHERE code_hash = ?2 AND used_at IS NULL AND expires_at > ?1 \
             RETURNING account_id, challenge",
            params![now, &code_hash[..]],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((account_id, challenge)) = found else {
        tx.commit()?;
        return Err(Refusal::invalid_code());
    };
    if !secrets::verifier_matches(verifier, &challenge) {
        // Committed: the code stays used.
        tx.commit()?;
        return Err(Refusal::invalid_code());
    }
    let device = devices::sign_in(&tx, &account_id, machine, token_hash, now)?;
    let account = account::load(&tx, &account_id)?
        .ok_or_else(|| Refusal::internal("a link code named an account without an identity"))?;
    tx.commit()?;
    Ok((account, device))
}
