//! The machines of an account (docs/native/cloud.md, `/v1/devices`).
//!
//! Every statement that takes a device's id takes the caller's account id
//! beside it: a device of another account is not refused, it is not found.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use hangar_cloud_api::{
    DeleteDeviceQuery, Device, DevicesResponse, MAX_DEVICE_NAME_CHARS, MachineInfo,
    RenameDeviceRequest,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::auth::Caller;
use crate::extract::{ApiJson, ApiQuery, DeviceId};
use crate::refusal::Refusal;
use crate::secrets::Hash;
use crate::state::AppState;

/// `last_seen_at` moves at most this often: a desktop that syncs every few
/// seconds does not make the server write for each request.
const LAST_SEEN_EVERY_MS: i64 = 60_000;

/// The columns [`device`] reads, in its order.
const COLUMNS: &str = "id, name, os, created_at, last_seen_at, revoked_at";

fn device(row: &Row<'_>) -> rusqlite::Result<Device> {
    Ok(Device {
        id: row.get(0)?,
        name: row.get(1)?,
        os: row.get(2)?,
        created_at: row.get(3)?,
        last_seen_at: row.get(4)?,
        revoked_at: row.get(5)?,
    })
}

/// A name, an id or an `os` the server stores and shows: 1 to `max`
/// characters (not bytes), on one line.
pub fn check_text(field: &str, text: &str, max: usize) -> Result<(), Refusal> {
    let chars = text.chars().count();
    if chars == 0 {
        return Err(Refusal::bad_request(format!("{field} is empty")));
    }
    if chars > max {
        return Err(Refusal::bad_request(format!(
            "{field} is longer than {max} characters"
        )));
    }
    if text.chars().any(char::is_control) {
        return Err(Refusal::bad_request(format!(
            "{field} holds a control character"
        )));
    }
    Ok(())
}

/// The device a token's hash names, when it is signed in. Moves its
/// `last_seen_at`, at most once a minute.
pub fn authenticate(
    conn: &Connection,
    token_hash: &Hash,
    now: i64,
) -> Result<Option<Caller>, Refusal> {
    // A signed-out device has no hash at all (the schema's CHECK); the
    // second condition says so to the reader too.
    let found = conn
        .prepare_cached(&format!(
            "SELECT account_id, {COLUMNS} FROM devices \
             WHERE token_hash = ?1 AND revoked_at IS NULL"
        ))?
        .query_row(params![&token_hash[..]], |row| {
            let account_id: String = row.get(0)?;
            let device = Device {
                id: row.get(1)?,
                name: row.get(2)?,
                os: row.get(3)?,
                created_at: row.get(4)?,
                last_seen_at: row.get(5)?,
                revoked_at: row.get(6)?,
            };
            Ok(Caller { account_id, device })
        })
        .optional()?;
    let Some(mut caller) = found else {
        return Ok(None);
    };
    if now - caller.device.last_seen_at >= LAST_SEEN_EVERY_MS {
        conn.prepare_cached("UPDATE devices SET last_seen_at = ?1 WHERE id = ?2")?
            .execute(params![now, caller.device.id])?;
        caller.device.last_seen_at = now;
    }
    Ok(Some(caller))
}

/// Signs a machine in to an account with a new token. One device per machine:
/// when the account already has this machine, it is that device that gets
/// the token (its previous one stops working, its rows and its `created_at`
/// are kept) and its name and `os` are the ones just given.
pub fn sign_in(
    conn: &Connection,
    account_id: &str,
    machine: &MachineInfo,
    token_hash: &Hash,
    now: i64,
) -> Result<Device, Refusal> {
    let id = uuid::Uuid::new_v4().to_string();
    let device = conn.query_row(
        &format!(
            "INSERT INTO devices (id, account_id, machine_id, name, os, created_at, \
             last_seen_at, token_hash, revoked_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, NULL) \
             ON CONFLICT (account_id, machine_id) DO UPDATE SET \
             name = excluded.name, os = excluded.os, last_seen_at = excluded.last_seen_at, \
             token_hash = excluded.token_hash, revoked_at = NULL \
             RETURNING {COLUMNS}"
        ),
        params![
            id,
            account_id,
            machine.id,
            machine.name,
            machine.os,
            now,
            &token_hash[..]
        ],
        device,
    )?;
    Ok(device)
}

/// Every device of the account, the oldest first.
pub fn of_account(conn: &Connection, account_id: &str) -> Result<Vec<Device>, Refusal> {
    let devices = conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM devices WHERE account_id = ?1 ORDER BY created_at, id"
        ))?
        .query_map(params![account_id], device)?
        .collect::<Result<_, _>>()?;
    Ok(devices)
}

/// `GET /v1/devices`: every device of the account, signed-out ones included.
pub async fn list(
    State(state): State<AppState>,
    caller: Caller,
) -> Result<Json<DevicesResponse>, Refusal> {
    let account_id = caller.account_id;
    let devices = state
        .db
        .call(move |conn| of_account(conn, &account_id))
        .await?;
    Ok(Json(DevicesResponse { devices }))
}

/// `PATCH /v1/devices/{id}`: any device of the account may rename any other.
pub async fn rename(
    State(state): State<AppState>,
    caller: Caller,
    DeviceId(id): DeviceId,
    ApiJson(request): ApiJson<RenameDeviceRequest>,
) -> Result<Json<Device>, Refusal> {
    check_text("name", &request.name, MAX_DEVICE_NAME_CHARS)?;
    let account_id = caller.account_id;
    let renamed = state
        .db
        .call(move |conn| {
            Ok(conn
                .query_row(
                    &format!(
                        "UPDATE devices SET name = ?1 WHERE id = ?2 AND account_id = ?3 \
                         RETURNING {COLUMNS}"
                    ),
                    params![request.name, id, account_id],
                    device,
                )
                .optional()?)
        })
        .await?;
    renamed.map(Json).ok_or_else(Refusal::no_such_device)
}

/// `DELETE /v1/devices/{id}`: signs the device out (its token is destroyed,
/// its rows stay), or with `?forget=true` deletes it and its rows.
pub async fn delete(
    State(state): State<AppState>,
    caller: Caller,
    DeviceId(id): DeviceId,
    ApiQuery(query): ApiQuery<DeleteDeviceQuery>,
) -> Result<StatusCode, Refusal> {
    let account_id = caller.account_id;
    let now = state.now();
    let found = state
        .db
        .call(move |conn| {
            if query.forget {
                // The schema's cascade takes the device's rows with it.
                let deleted = conn.execute(
                    "DELETE FROM devices WHERE id = ?1 AND account_id = ?2",
                    params![id, account_id],
                )?;
                return Ok(deleted > 0);
            }
            let signed_out = conn.execute(
                "UPDATE devices SET token_hash = NULL, revoked_at = ?1 \
                 WHERE id = ?2 AND account_id = ?3 AND revoked_at IS NULL",
                params![now, id, account_id],
            )?;
            if signed_out > 0 {
                return Ok(true);
            }
            // Already signed out, which changes nothing, or not the
            // account's.
            let exists: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM devices WHERE id = ?1 AND account_id = ?2)",
                params![id, account_id],
                |row| row.get(0),
            )?;
            Ok(exists)
        })
        .await?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(Refusal::no_such_device())
    }
}
