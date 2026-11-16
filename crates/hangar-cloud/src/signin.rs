//! Signing in with GitHub or Google (docs/native/cloud.md, "Signing in"):
//! the two routes a browser goes through, between the desktop that opened it
//! and the desktop's loopback listener it ends at.
//!
//! `GET /v1/auth/start` stores what the desktop sent and sends the browser to
//! the provider. `GET /v1/auth/callback/{provider}` is where the provider
//! sends it back: the server learns who the person is, finds or makes their
//! account, and sends the browser to the desktop with a link code.
//!
//! What holds this together:
//!
//! - Two states and two PKCE pairs, never mixed. The desktop's state and
//!   challenge are stored and handed back (the state on the loopback, the
//!   challenge to the link code); the server draws its own state and verifier
//!   for the provider. The provider never sees anything of the desktop's.
//! - The server's state is kept as its SHA-256, works once, and for ten
//!   minutes: the callback deletes the row it finds before it does anything
//!   else, so a callback replayed, forged or late finds nothing.
//! - The browser is only ever sent to the provider's consent page (an address
//!   of this server's configuration) or to `http://127.0.0.1:<port>/callback`
//!   with the port the start stored. No address, host or path of a request is
//!   ever redirected to.
//! - Nothing of a request is written into a page or a log.

use std::time::Instant;

use axum::extract::{Path, Query, Request, State};
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use hangar_cloud_api::{
    AuthStartQuery, ErrorCode, MAX_DEVICE_NAME_CHARS, MAX_STATE_CHARS, MIN_LOOPBACK_PORT, Provider,
    paths,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;

use crate::limit::ClientIp;
use crate::pages::Page;
use crate::provider::{self, percent_encode};
use crate::refusal::Refusal;
use crate::secrets::{self, Hash};
use crate::state::AppState;
use crate::{account, auth, devices};

/// How long a sign-in may stay at the provider: the time the desktop's
/// listener waits for it.
const PENDING_TTL_MS: i64 = 10 * 60 * 1000;

/// The most sign-ins in progress the server holds at a time. The limit per
/// address is what keeps one caller from filling the table; this keeps many
/// from doing it together. Far above what real sign-ins ever reach.
const MAX_PENDING: i64 = 10_000;

/// What the desktop's listener is told when no code can be sent to it.
const ACCESS_DENIED: &str = "access_denied";
const PROVIDER_ERROR: &str = "provider_error";

/// A sign-in between the start and the provider's callback: a row of
/// `auth_states`.
struct Pending {
    provider: Provider,
    port: u16,
    desktop_state: String,
    challenge: String,
    provider_verifier: String,
}

/// `GET /v1/auth/start`: the page the desktop opens in the system browser.
/// `302` to the provider's consent page, or a page saying why not.
pub async fn start(State(state): State<AppState>, request: Request) -> Response {
    let headers = request.headers();
    let Some(&ClientIp(ip)) = request.extensions().get::<ClientIp>() else {
        // `limit::per_ip` is around every route of `/v1`.
        let _ = Refusal::internal("the start was routed without its address");
        return Page::Failed.response(headers);
    };
    if state.limits.start.check(ip, Instant::now()).is_err() {
        return Page::Busy.response(headers);
    }

    // Every field once, the provider one this protocol knows, the port a
    // number that fits a port.
    let Ok(Query(query)) = Query::<AuthStartQuery>::try_from_uri(request.uri()) else {
        return Page::Invalid.response(headers);
    };
    let valid = query.port >= MIN_LOOPBACK_PORT
        && is_desktop_state(&query.state)
        && secrets::is_challenge(&query.challenge)
        && devices::check_text("machine", &query.machine, MAX_DEVICE_NAME_CHARS).is_ok();
    if !valid {
        return Page::Invalid.response(headers);
    }
    let provider = query.provider;
    let Some(client) = state.sign_in.client(provider) else {
        return Page::Unavailable.response(headers);
    };

    // The server's own state and verifier, which the desktop never sees.
    let (server_state, verifier) = match (secrets::new_oauth_secret(), secrets::new_oauth_secret())
    {
        (Ok(server_state), Ok(verifier)) => (server_state, verifier),
        _ => return Page::Failed.response(headers),
    };
    let location = provider::authorize_url(
        &state.sign_in.endpoints,
        provider,
        client,
        &state.sign_in.redirect_uri(provider),
        &server_state,
        &secrets::challenge_of(&verifier),
    );

    let state_hash = secrets::sha256(&server_state);
    let now = state.now();
    let stored = state
        .db
        .call(move |conn| remember(conn, &state_hash, &query, &verifier, now))
        .await;
    match stored {
        Ok(true) => redirect(&location, headers),
        Ok(false) => Page::Busy.response(headers),
        Err(_) => Page::Failed.response(headers),
    }
}

/// The desktop's state: 1 to 200 of RFC 3986's unreserved characters, which
/// is what a random value in base64url or hexadecimal is made of. It is
/// handed back in a URL and nowhere else.
fn is_desktop_state(state: &str) -> bool {
    (1..=MAX_STATE_CHARS).contains(&state.len())
        && state
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// Stores the sign-in in progress. `false` when the table is full.
fn remember(
    conn: &mut Connection,
    state_hash: &Hash,
    query: &AuthStartQuery,
    verifier: &str,
    now: i64,
) -> Result<bool, Refusal> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    auth::purge_expired(&tx, now)?;
    let pending: i64 = tx.query_row("SELECT count(*) FROM auth_states", [], |row| row.get(0))?;
    if pending >= MAX_PENDING {
        tx.commit()?;
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO auth_states (state_hash, provider, port, desktop_state, challenge, \
         machine, provider_verifier, created_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            &state_hash[..],
            query.provider.as_str(),
            i64::from(query.port),
            query.state,
            query.challenge,
            query.machine,
            verifier,
            now,
            now + PENDING_TTL_MS
        ],
    )?;
    tx.commit()?;
    Ok(true)
}

/// What a provider sends the browser back with: a code, or an error, and the
/// state the server gave it.
#[derive(Deserialize)]
struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

/// `GET /v1/auth/callback/{provider}`: where the provider sends the browser
/// back. `302` to the desktop's loopback listener, with a link code or with
/// the reason there is none; a page, and no redirect at all, when the
/// server's state is not one it is waiting for.
pub async fn callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let Some(provider) = Provider::parse(&provider) else {
        return Refusal::new(ErrorCode::NotFound, "no such route").into_response();
    };
    let headers = &headers;
    // A query with a field twice is not one a provider sends.
    let query = Query::<CallbackQuery>::try_from_uri(&uri)
        .map(|Query(query)| query)
        .unwrap_or(CallbackQuery {
            state: None,
            code: None,
            error: None,
        });
    let denied = query.error.as_deref() == Some(ACCESS_DENIED);
    // Without a sign-in to tie it to, there is no desktop to tell: the
    // person is told here.
    let unknown = if denied { Page::Denied } else { Page::Expired };

    let Some(server_state) = query.state.filter(|text| secrets::is_secret_shaped(text)) else {
        return unknown.response(headers);
    };
    let state_hash = secrets::sha256(&server_state);
    let now = state.now();
    let pending = match state
        .db
        .call(move |conn| take(conn, &state_hash, provider, now))
        .await
    {
        Ok(Some(pending)) => pending,
        Ok(None) => return unknown.response(headers),
        Err(_) => return Page::Failed.response(headers),
    };
    // From here the sign-in is known, and consumed: whatever happens next is
    // said to the desktop that started it, on the port it gave.

    if query.error.is_some() {
        let error = if denied {
            ACCESS_DENIED
        } else {
            PROVIDER_ERROR
        };
        return to_desktop(&pending, "error", error, headers);
    }
    let Some(code) = query.code.filter(|code| !code.is_empty()) else {
        return to_desktop(&pending, "error", PROVIDER_ERROR, headers);
    };

    let sign_in = state.sign_in.clone();
    let verifier = pending.provider_verifier.clone();
    let identified = tokio::task::spawn_blocking(move || {
        let client = sign_in.client(provider).ok_or(provider::Failed)?;
        provider::identify(
            &sign_in.endpoints,
            provider,
            client,
            &sign_in.redirect_uri(provider),
            &code,
            &verifier,
        )
    })
    .await;
    let identity = match identified {
        Ok(Ok(identity)) => identity,
        Ok(Err(provider::Failed)) => {
            return to_desktop(&pending, "error", PROVIDER_ERROR, headers);
        }
        Err(_) => {
            let _ = Refusal::internal("the call to the provider panicked");
            return Page::Failed.response(headers);
        }
    };

    let challenge = pending.challenge.clone();
    let now = state.now();
    let linked = state
        .db
        .call(move |conn| {
            // One transaction: the account found or made, and its code.
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let account_id = account::resolve(&tx, &identity, now)?;
            let code = auth::create_link_code(&tx, &account_id, &challenge, now)?;
            tx.commit()?;
            Ok(code)
        })
        .await;
    match linked {
        Ok(code) => to_desktop(&pending, "code", &code, headers),
        // The server's own fault: said here, the desktop's listener gives up
        // by itself.
        Err(_) => Page::Failed.response(headers),
    }
}

/// The sign-in the server's state names, deleted as it is read: found once,
/// and never after. Nothing when it is unknown, expired, or was started with
/// the other provider (and it is consumed all the same).
fn take(
    conn: &mut Connection,
    state_hash: &Hash,
    provider: Provider,
    now: i64,
) -> Result<Option<Pending>, Refusal> {
    let row: Option<(String, i64, String, String, String, i64)> = conn
        .query_row(
            "DELETE FROM auth_states WHERE state_hash = ?1 \
             RETURNING provider, port, desktop_state, challenge, provider_verifier, expires_at",
            params![&state_hash[..]],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((started_with, port, desktop_state, challenge, provider_verifier, expires_at)) = row
    else {
        return Ok(None);
    };
    if expires_at <= now || Provider::parse(&started_with) != Some(provider) {
        return Ok(None);
    }
    // The start stored a port it had checked; a row that says otherwise is
    // not one to redirect anybody with.
    let port = u16::try_from(port)
        .ok()
        .filter(|port| *port >= MIN_LOOPBACK_PORT)
        .ok_or_else(|| Refusal::internal("a pending sign-in holds a port that is not one"))?;
    Ok(Some(Pending {
        provider,
        port,
        desktop_state,
        challenge,
        provider_verifier,
    }))
}

/// Sends the browser to the desktop's listener: `http://127.0.0.1`, the port
/// the start stored, the protocol's path, and `name=value` with the desktop's
/// own state. The only redirect to the desktop there is.
fn to_desktop(pending: &Pending, name: &str, value: &str, headers: &HeaderMap) -> Response {
    debug_assert!(matches!(name, "code" | "error"));
    debug_assert!(Provider::ALL.contains(&pending.provider));
    let location = format!(
        "http://127.0.0.1:{}{}?{name}={}&state={}",
        pending.port,
        paths::LOOPBACK_CALLBACK,
        percent_encode(value),
        percent_encode(&pending.desktop_state)
    );
    redirect(&location, headers)
}

/// `302` to `location`, which is always an address this module built.
fn redirect(location: &str, headers: &HeaderMap) -> Response {
    match HeaderValue::from_str(location) {
        Ok(location) => (StatusCode::FOUND, [(header::LOCATION, location)]).into_response(),
        Err(_) => {
            let _ = Refusal::internal("a redirect that is not a header's value");
            Page::Failed.response(headers)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_state_is_one_to_two_hundred_unreserved_characters() {
        assert!(is_desktop_state("a"));
        assert!(is_desktop_state("aZ09-._~"));
        assert!(is_desktop_state(&"a".repeat(200)));
        assert!(!is_desktop_state(""));
        assert!(!is_desktop_state(&"a".repeat(201)));
        for state in [
            "a b", "a&b", "a=b", "a#b", "a/b", "a%20b", "é", "a\nb", "a+b",
        ] {
            assert!(!is_desktop_state(state), "{state:?}");
        }
    }
}
