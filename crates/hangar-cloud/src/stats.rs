//! The statistics a device sends and the account's read back
//! (docs/native/cloud.md, `/v1/stats`).
//!
//! A device writes its own rows and no other's: the device id of a write is
//! the token's, never the request's. A read is the account's, across its
//! devices.

use std::collections::HashSet;

use axum::Json;
use axum::extract::State;
use hangar_cloud_api::{
    DeviceStatsRow, ErrorCode, MAX_STATS_ROWS, PutStatsRequest, PutStatsResponse, StatsQuery,
    StatsResponse, StatsRow, is_day,
};
use rusqlite::{Connection, TransactionBehavior, params};

use crate::auth::Caller;
use crate::devices;
use crate::extract::{ApiJson, ApiQuery};
use crate::refusal::Refusal;
use crate::state::AppState;

/// `PUT /v1/stats`: each row replaces the device's row of the same (day,
/// model, project). All or nothing.
pub async fn put(
    State(state): State<AppState>,
    caller: Caller,
    ApiJson(request): ApiJson<PutStatsRequest>,
) -> Result<Json<PutStatsResponse>, Refusal> {
    let rows = request.rows;
    if rows.len() > MAX_STATS_ROWS {
        return Err(Refusal::new(
            ErrorCode::TooLarge,
            format!("more than {MAX_STATS_ROWS} rows"),
        ));
    }
    // Every row is checked before any is written.
    for (index, row) in rows.iter().enumerate() {
        row.validate()
            .map_err(|err| Refusal::bad_request(format!("rows[{index}]: {err}")))?;
    }
    let distinct = rows.iter().map(StatsRow::key).collect::<HashSet<_>>().len();

    let device_id = caller.device.id;
    let now = state.now();
    state
        .db
        .call(move |conn| upsert(conn, &device_id, &rows, now))
        .await?;
    Ok(Json(PutStatsResponse {
        upserted: distinct as u64,
    }))
}

fn upsert(
    conn: &mut Connection,
    device_id: &str,
    rows: &[StatsRow],
    now: i64,
) -> Result<(), Refusal> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // The token was checked a moment ago, outside this transaction: a device
    // signed out or forgotten since then writes nothing.
    let signed_in: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM devices WHERE id = ?1 AND revoked_at IS NULL)",
        params![device_id],
        |row| row.get(0),
    )?;
    if !signed_in {
        return Err(Refusal::unauthorized());
    }
    {
        // In the request's order, so that a key sent twice keeps its last
        // row.
        let mut statement = tx.prepare_cached(
            "INSERT INTO stats (device_id, day, model, project, input_tokens, output_tokens, \
             cache_read_tokens, cache_write_tokens, cost_usd_micros, active_seconds, sessions, \
             updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
             ON CONFLICT (device_id, day, model, project) DO UPDATE SET \
             input_tokens = excluded.input_tokens, output_tokens = excluded.output_tokens, \
             cache_read_tokens = excluded.cache_read_tokens, \
             cache_write_tokens = excluded.cache_write_tokens, \
             cost_usd_micros = excluded.cost_usd_micros, \
             active_seconds = excluded.active_seconds, sessions = excluded.sessions, \
             updated_at = excluded.updated_at",
        )?;
        for row in rows {
            statement.execute(params![
                device_id,
                row.day,
                row.model,
                row.project,
                counter(row.input_tokens)?,
                counter(row.output_tokens)?,
                counter(row.cache_read_tokens)?,
                counter(row.cache_write_tokens)?,
                counter(row.cost_usd_micros)?,
                counter(row.active_seconds)?,
                counter(row.sessions)?,
                now
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// A counter as SQLite holds it. `StatsRow::validate` refused what does not
/// fit before this is reached.
fn counter(value: u64) -> Result<i64, Refusal> {
    i64::try_from(value).map_err(|_| Refusal::bad_request("a counter is larger than 2^63 - 1"))
}

/// `GET /v1/stats?from=&to=`: every row of the account in the range, and the
/// devices those rows name.
pub async fn get(
    State(state): State<AppState>,
    caller: Caller,
    ApiQuery(query): ApiQuery<StatsQuery>,
) -> Result<Json<StatsResponse>, Refusal> {
    for (name, day) in [("from", &query.from), ("to", &query.to)] {
        if day.as_deref().is_some_and(|day| !is_day(day)) {
            return Err(Refusal::bad_request(format!(
                "{name} is not a YYYY-MM-DD date"
            )));
        }
    }
    let account_id = caller.account_id;
    let response = state
        .db
        .call(move |conn| {
            read(
                conn,
                &account_id,
                query.from.as_deref(),
                query.to.as_deref(),
            )
        })
        .await?;
    Ok(Json(response))
}

fn read(
    conn: &mut Connection,
    account_id: &str,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<StatsResponse, Refusal> {
    // One transaction, so that the rows and their devices are of the same
    // moment.
    let tx = conn.transaction()?;
    // Days written YYYY-MM-DD sort as text.
    let rows: Vec<DeviceStatsRow> = tx
        .prepare_cached(
            "SELECT s.device_id, s.day, s.model, s.project, s.input_tokens, s.output_tokens, \
             s.cache_read_tokens, s.cache_write_tokens, s.cost_usd_micros, s.active_seconds, \
             s.sessions \
             FROM stats s JOIN devices d ON d.id = s.device_id \
             WHERE d.account_id = ?1 AND (?2 IS NULL OR s.day >= ?2) \
             AND (?3 IS NULL OR s.day <= ?3) \
             ORDER BY s.day, s.device_id, s.model, s.project",
        )?
        .query_map(params![account_id, from, to], |row| {
            // Never negative: the schema checks it.
            let count = |index: usize| row.get::<_, i64>(index).map(|n| n.max(0) as u64);
            Ok(DeviceStatsRow {
                device_id: row.get(0)?,
                row: StatsRow {
                    day: row.get(1)?,
                    model: row.get(2)?,
                    project: row.get(3)?,
                    input_tokens: count(4)?,
                    output_tokens: count(5)?,
                    cache_read_tokens: count(6)?,
                    cache_write_tokens: count(7)?,
                    cost_usd_micros: count(8)?,
                    active_seconds: count(9)?,
                    sessions: count(10)?,
                },
            })
        })?
        .collect::<Result<_, _>>()?;
    let named: HashSet<&str> = rows.iter().map(|row| row.device_id.as_str()).collect();
    let devices = devices::of_account(&tx, account_id)?
        .into_iter()
        .filter(|device| named.contains(device.id.as_str()))
        .collect();
    tx.commit()?;
    Ok(StatsResponse { devices, rows })
}
