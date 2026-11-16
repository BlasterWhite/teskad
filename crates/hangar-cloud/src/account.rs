//! The account: an id, and the identities a provider vouched for
//! (docs/native/cloud.md, "What the server stores").
//!
//! `GET /v1/me` and `DELETE /v1/account` are here, with what the sign-in
//! routes call once a provider has said who the person is.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use hangar_cloud_api::{Account, MeResponse, Provider};
use rusqlite::{Connection, params};

use crate::auth::Caller;
use crate::refusal::Refusal;
use crate::state::AppState;

/// What a provider said about the person. `email` only when the provider
/// says it verified the address.
// Built by the sign-in routes, which come next; the tests build accounts
// with it meanwhile.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIdentity {
    pub provider: Provider,
    pub provider_user_id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

/// A new account, with the identity that signed in as its first. Gives the
/// account's id.
#[allow(dead_code)]
pub fn create(conn: &mut Connection, identity: &NewIdentity, now: i64) -> Result<String, Refusal> {
    let id = uuid::Uuid::new_v4().to_string();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO accounts (id, created_at) VALUES (?1, ?2)",
        params![id, now],
    )?;
    add_identity(&tx, &id, identity, now)?;
    tx.commit()?;
    Ok(id)
}

/// One more way to sign in to an account that exists.
#[allow(dead_code)]
pub fn add_identity(
    conn: &Connection,
    account_id: &str,
    identity: &NewIdentity,
    now: i64,
) -> Result<(), Refusal> {
    conn.execute(
        "INSERT INTO identities (provider, provider_user_id, account_id, email, display_name, \
         avatar_url, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            identity.provider.as_str(),
            identity.provider_user_id,
            account_id,
            identity.email,
            identity.display_name,
            identity.avatar_url,
            now
        ],
    )?;
    Ok(())
}

/// The account as the protocol shows it: the profile of its first identity,
/// and every provider it can sign in with. Nothing when there is no such
/// account.
pub fn load(conn: &Connection, account_id: &str) -> Result<Option<Account>, Refusal> {
    let mut statement = conn.prepare_cached(
        "SELECT provider, display_name, email, avatar_url FROM identities \
         WHERE account_id = ?1 ORDER BY created_at, rowid",
    )?;
    let mut rows = statement.query(params![account_id])?;
    let mut account: Option<Account> = None;
    while let Some(row) = rows.next()? {
        let provider: String = row.get(0)?;
        let (display_name, email, avatar_url) = (row.get(1)?, row.get(2)?, row.get(3)?);
        let account = account.get_or_insert_with(|| Account {
            id: account_id.to_string(),
            display_name,
            email,
            avatar_url,
            providers: Vec::new(),
        });
        if let Some(provider) = Provider::parse(&provider)
            && !account.providers.contains(&provider)
        {
            account.providers.push(provider);
        }
    }
    Ok(account)
}

/// `GET /v1/me`: the account, and the device asking.
pub async fn me(
    State(state): State<AppState>,
    caller: Caller,
) -> Result<Json<MeResponse>, Refusal> {
    let account_id = caller.account_id;
    let account = state
        .db
        .call(move |conn| load(conn, &account_id))
        .await?
        // Deleted between the token's check and here: the token is dead.
        .ok_or_else(Refusal::unauthorized)?;
    Ok(Json(MeResponse {
        account,
        device: caller.device,
    }))
}

/// `DELETE /v1/account`: the account, its identities, its devices and their
/// tokens, every row they sent, its link codes. One statement, so one
/// transaction: the schema's cascades take everything or nothing.
pub async fn delete(State(state): State<AppState>, caller: Caller) -> Result<StatusCode, Refusal> {
    let account_id = caller.account_id;
    state
        .db
        .call(move |conn| {
            conn.execute("DELETE FROM accounts WHERE id = ?1", params![account_id])?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
