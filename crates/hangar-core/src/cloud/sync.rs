//! The statistics sync: this machine's totals sent to the account, and the
//! account's read back (docs/native/cloud.md, `PUT` and `GET /v1/stats`).
//!
//! It runs only for a person who signed in **and** turned the sync on: the
//! flag is passed to every call here, and a call made with it off, or without
//! a token, returns before anything is read or sent.
//!
//! What leaves is rows of totals per day, model and project name, built from
//! the answer the statistics page is drawn from (stats.rs). A project is the
//! last component of its folder: the path stays here, and so do the sessions'
//! titles and ids, which no row has a field for.
//!
//! A row replaces the server's row of the same key, so sending is idempotent.
//! The last rows sent are remembered in a file of the app data folder, and a
//! sync sends only the rows that differ from it. Once a day, and at the first
//! sync of a run, what the server holds for this machine is compared with
//! that memory: rows it lost are sent again.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};

use super::api::{self, StatsQuery};
use super::{Cloud, Error, StatsResponse, StatsRow};
use crate::stats::UsageStats;

/// How often the statistics are sent while the sync is on.
pub const SYNC_EVERY: Duration = Duration::from_secs(15 * 60);

/// How often what the server holds is compared with what was sent.
const VERIFY_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// The rows last sent, in the app data folder.
const LEDGER_FILE: &str = "cloud-sync.json";
const LEDGER_VERSION: u32 = 1;

/// The model of calls whose transcript named none, as the statistics page
/// calls it.
const UNKNOWN_MODEL: &str = "unknown";

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// A project's name: the last component of its folder, whichever way its
/// separators lean, cut to the protocol's length. Empty for a folder that has
/// no name of its own (a drive, the root).
///
/// `C:\Users\ada\Projects\hangar` and `/home/ada/Projects/hangar` are both
/// `hangar`: nothing above the last component is in the answer.
pub fn project_name(folder: &str) -> String {
    let last = folder
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or("");
    let drive = last.len() == 2 && last.ends_with(':');
    if drive || last == "." || last == ".." {
        return String::new();
    }
    last.chars()
        .filter(|c| !c.is_control())
        .take(api::MAX_PROJECT_CHARS)
        .collect()
}

/// A model's id as a row takes it: one line, within the protocol's length.
fn model_name(id: &str) -> String {
    let name: String = id
        .chars()
        .filter(|c| !c.is_control())
        .take(api::MAX_MODEL_CHARS)
        .collect();
    if name.is_empty() {
        UNKNOWN_MODEL.to_string()
    } else {
        name
    }
}

/// The calendar day a time falls on, on this machine's clock.
fn local_day(at: i64) -> Option<String> {
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|time| time.date_naive().to_string())
}

/// This machine's rows, from the statistics page's own answer (the whole
/// history: `Window::default()`), with days counted in the machine's time
/// zone as the page counts them.
///
/// Tokens and cost are each model's. Active time and sessions belong to a
/// session, not to a model: each session's day goes, whole, to the model that
/// answered most of its calls that day, so that summing a day's rows over its
/// models gives the day's real active time and its real number of sessions,
/// with nothing counted twice. A session that no model ever answered has no
/// row to sit in and is left out.
pub fn rows_from(stats: &UsageStats) -> Vec<StatsRow> {
    rows_with(stats, local_day)
}

type Key = (String, String, String);

fn key_of(row: &StatsRow) -> Key {
    (row.day.clone(), row.model.clone(), row.project.clone())
}

#[derive(Default)]
struct Totals {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    cost: f64,
    active_ms: u64,
    sessions: u64,
}

/// One session's day: how many calls each model answered, and how long the
/// session was active.
#[derive(Default)]
struct SessionDay {
    calls: BTreeMap<String, u64>,
    active_ms: u64,
}

/// The model that answered most calls; the first by name when two tie.
fn busiest(calls: &BTreeMap<String, u64>) -> Option<&String> {
    calls
        .iter()
        .filter(|(_, &count)| count > 0)
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(model, _)| model)
}

pub(super) fn rows_with(
    stats: &UsageStats,
    day_of: impl Fn(i64) -> Option<String>,
) -> Vec<StatsRow> {
    let projects: Vec<String> = stats
        .sessions
        .iter()
        .map(|session| project_name(&session.cwd))
        .collect();
    let models: Vec<String> = stats.models.iter().map(|m| model_name(&m.id)).collect();
    let mut days: HashMap<i64, Option<String>> = HashMap::new();

    let mut totals: BTreeMap<Key, Totals> = BTreeMap::new();
    let mut session_days: BTreeMap<(u32, String), SessionDay> = BTreeMap::new();
    let mut session_calls: HashMap<u32, BTreeMap<String, u64>> = HashMap::new();

    for bucket in &stats.buckets {
        let Some(project) = projects.get(bucket.session as usize) else {
            continue;
        };
        let Some(day) = days
            .entry(bucket.at)
            .or_insert_with(|| day_of(bucket.at))
            .clone()
        else {
            continue;
        };
        let session_day = session_days
            .entry((bucket.session, day.clone()))
            .or_default();
        // Rows without a model carry what belongs to the session: its turns,
        // which no row sends, and its active time.
        session_day.active_ms += bucket.active_ms.max(0) as u64;
        let Some(model) = usize::try_from(bucket.model)
            .ok()
            .and_then(|at| models.get(at))
        else {
            continue;
        };
        let calls = u64::from(bucket.calls);
        *session_day.calls.entry(model.clone()).or_default() += calls;
        *session_calls
            .entry(bucket.session)
            .or_default()
            .entry(model.clone())
            .or_default() += calls;
        let row = totals
            .entry((day, model.clone(), project.clone()))
            .or_default();
        row.input += bucket.input;
        row.output += bucket.output;
        row.cache_read += bucket.cache_read;
        row.cache_write += bucket.cache_write;
        row.cost += bucket.cost;
    }

    for ((session, day), session_day) in &session_days {
        if session_day.active_ms == 0 && session_day.calls.values().all(|&count| count == 0) {
            continue;
        }
        // The day's busiest model; on a day the session only waited or was
        // typed into, the model it mostly ran on.
        let Some(model) =
            busiest(&session_day.calls).or_else(|| session_calls.get(session).and_then(busiest))
        else {
            continue;
        };
        let project = &projects[*session as usize];
        let row = totals
            .entry((day.clone(), model.clone(), project.clone()))
            .or_default();
        row.active_ms += session_day.active_ms;
        row.sessions += 1;
    }

    let counter = |n: u64| n.min(api::MAX_COUNTER);
    totals
        .into_iter()
        .map(|((day, model, project), sum)| StatsRow {
            day,
            model,
            project,
            input_tokens: counter(sum.input),
            output_tokens: counter(sum.output),
            cache_read_tokens: counter(sum.cache_read),
            cache_write_tokens: counter(sum.cache_write),
            // Dollars to millionths of one. A float-to-integer cast stops at
            // the integer's bounds and reads "not a number" as zero.
            cost_usd_micros: counter((sum.cost * 1e6).round() as u64),
            active_seconds: counter((sum.active_ms + 500) / 1000),
            sessions: counter(sum.sessions),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// What was last sent
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct LedgerFile {
    version: u32,
    rows: Vec<StatsRow>,
}

/// The rows the server is believed to hold for this machine, by key.
#[derive(Default)]
struct Ledger {
    rows: BTreeMap<Key, StatsRow>,
}

impl Ledger {
    /// A missing or damaged file is an empty memory: every row is then sent
    /// again, which changes nothing on the server.
    fn load(data_dir: &Path) -> Self {
        let rows = fs::read_to_string(data_dir.join(LEDGER_FILE))
            .ok()
            .and_then(|text| serde_json::from_str::<LedgerFile>(&text).ok())
            .filter(|file| file.version == LEDGER_VERSION)
            .map(|file| file.rows)
            .unwrap_or_default();
        Self {
            rows: rows.into_iter().map(|row| (key_of(&row), row)).collect(),
        }
    }

    fn save(&self, data_dir: &Path) -> Result<(), Error> {
        let failed = |err: std::io::Error| {
            Error::Local(format!(
                "what the sync sent could not be kept ({LEDGER_FILE}): {err}"
            ))
        };
        let file = LedgerFile {
            version: LEDGER_VERSION,
            rows: self.rows.values().cloned().collect(),
        };
        let text = serde_json::to_string(&file).map_err(|err| failed(err.into()))?;
        fs::create_dir_all(data_dir).map_err(failed)?;
        let aside = data_dir.join(format!("{LEDGER_FILE}.tmp"));
        fs::write(&aside, text).map_err(failed)?;
        fs::rename(&aside, data_dir.join(LEDGER_FILE)).map_err(failed)
    }
}

/// Forgets what was sent: at sign-in and sign-out, the memory of one token
/// says nothing of the next.
pub(super) fn forget(data_dir: &Path) {
    let _ = fs::remove_file(data_dir.join(LEDGER_FILE));
}

// ---------------------------------------------------------------------------
// Sending and reading back
// ---------------------------------------------------------------------------

/// What a sync did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Synced {
    /// Rows sent: new ones, changed ones, and the ones the server had lost.
    pub sent: usize,
    /// Rows the server already had as they are.
    pub unchanged: usize,
    /// Rows the server no longer held as they were sent, found by the daily
    /// comparison. Those that still exist here are among `sent`.
    pub lost: usize,
    /// Rows the protocol does not take, left here.
    pub skipped: usize,
}

/// The rows cut into calls the server takes: within its row count and well
/// within its body size.
fn calls(rows: Vec<StatsRow>) -> Vec<Vec<StatsRow>> {
    // Room for the envelope around the rows.
    let budget = api::MAX_BODY_BYTES - 1024;
    let mut calls: Vec<Vec<StatsRow>> = Vec::new();
    let mut bytes = 0;
    for row in rows {
        let size = serde_json::to_vec(&row).map_or(0, |json| json.len()) + 1;
        let full = calls
            .last()
            .is_none_or(|call| call.len() >= api::MAX_STATS_ROWS || bytes + size > budget);
        if full {
            calls.push(Vec::new());
            bytes = 0;
        }
        bytes += size;
        if let Some(call) = calls.last_mut() {
            call.push(row);
        }
    }
    calls
}

impl Cloud {
    /// Sends this machine's statistics: reads them as the statistics page
    /// does, builds the rows and sends the ones that changed.
    ///
    /// With `enabled` off, or signed out, nothing is read and nothing is
    /// sent (`Error::SyncOff`, `Error::NotSignedIn`).
    pub async fn sync_stats(
        &self,
        host: crate::host::Host,
        enabled: bool,
    ) -> Result<Synced, Error> {
        if !enabled {
            return Err(Error::SyncOff);
        }
        let cloud = self.clone();
        if !off_thread(move || cloud.signed_in()).await? {
            return Err(Error::NotSignedIn);
        }
        let stats = crate::stats::usage_stats(host, None, None, None)
            .await
            .map_err(Error::Local)?;
        let cloud = self.clone();
        off_thread(move || cloud.sync_rows(enabled, &rows_from(&stats))).await
    }

    /// Sends the rows that differ from what was last sent. Blocking: one
    /// call to the server per 5000 rows, none when nothing changed.
    pub fn sync_rows(&self, enabled: bool, rows: &[StatsRow]) -> Result<Synced, Error> {
        if !enabled {
            return Err(Error::SyncOff);
        }
        let _one_at_a_time = self.inner.syncing.lock().unwrap_or_else(|e| e.into_inner());
        self.with_token(|token| {
            let data_dir = &self.inner.data_dir;
            let mut ledger = Ledger::load(data_dir);
            let mut synced = Synced::default();

            if self.verification_due() {
                // Nothing remembered is nothing to compare: all is sent.
                if !ledger.rows.is_empty() {
                    synced.lost = self.drop_what_the_server_lost(token, &mut ledger)?;
                    if synced.lost > 0 {
                        ledger.save(data_dir)?;
                    }
                }
                self.verified_now();
            }

            // A key given twice keeps its last row, as the server would.
            let wanted: BTreeMap<Key, &StatsRow> =
                rows.iter().map(|row| (key_of(row), row)).collect();
            let mut changed = Vec::new();
            for (key, row) in wanted {
                if row.validate().is_err() {
                    synced.skipped += 1;
                } else if ledger.rows.get(&key) == Some(row) {
                    synced.unchanged += 1;
                } else {
                    changed.push(row.clone());
                }
            }

            for call in calls(changed) {
                self.inner.client.put_stats(token, &call)?;
                synced.sent += call.len();
                // Kept call by call: a sync cut short does not send again
                // what already went through.
                ledger
                    .rows
                    .extend(call.into_iter().map(|row| (key_of(&row), row)));
                ledger.save(data_dir)?;
            }
            Ok(synced)
        })
    }

    /// The rows of the whole account, every machine's, for the days from
    /// `from` to `to` (`YYYY-MM-DD`, both included, either left open), with
    /// the machines they name.
    pub fn pull_stats(
        &self,
        enabled: bool,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<StatsResponse, Error> {
        if !enabled {
            return Err(Error::SyncOff);
        }
        for day in [from, to].into_iter().flatten() {
            if !api::is_day(day) {
                return Err(Error::Invalid(format!("{day:?} is not a YYYY-MM-DD day")));
            }
        }
        let query = StatsQuery {
            from: from.map(str::to_string),
            to: to.map(str::to_string),
        };
        self.with_token(|token| self.inner.client.get_stats(token, &query))
    }

    fn verification_due(&self) -> bool {
        self.inner
            .verified
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none_or(|at| at.elapsed() >= VERIFY_EVERY)
    }

    fn verified_now(&self) {
        *self
            .inner
            .verified
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }

    /// Compares the memory of what was sent with what the server holds for
    /// this machine, and forgets every row the server does not have as it
    /// was sent: a database restored from an older copy, say. Those rows
    /// then count as changed, and go again. Returns how many there were.
    fn drop_what_the_server_lost(&self, token: &str, ledger: &mut Ledger) -> Result<usize, Error> {
        let client = &self.inner.client;
        let device = client.me(token)?.device.id;
        let held: HashMap<Key, StatsRow> = client
            .get_stats(token, &StatsQuery::default())?
            .rows
            .into_iter()
            .filter(|row| row.device_id == device)
            .map(|row| (key_of(&row.row), row.row))
            .collect();
        let before = ledger.rows.len();
        ledger.rows.retain(|key, row| held.get(key) == Some(row));
        Ok(before - ledger.rows.len())
    }
}

async fn off_thread<T: Send + 'static>(
    task: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|err| Error::Local(err.to_string()))?
}

// ---------------------------------------------------------------------------
// When
// ---------------------------------------------------------------------------

/// When the next sync is due: every fifteen minutes, and at once when asked.
/// It decides and nothing more: whoever drives it (the interface) asks
/// `due`, runs the sync, and says `ran`.
#[derive(Debug, Clone)]
pub struct Schedule {
    every: Duration,
    last: Option<Instant>,
    asked: bool,
    not_before: Option<Instant>,
}

impl Default for Schedule {
    fn default() -> Self {
        Self::every(SYNC_EVERY)
    }
}

impl Schedule {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn every(every: Duration) -> Self {
        Self {
            every,
            last: None,
            asked: false,
            not_before: None,
        }
    }

    /// "Sync now": the next `due` says yes, whatever the clock says.
    pub fn sync_now(&mut self) {
        self.asked = true;
    }

    /// Whether a sync should start at `now`. Never without both a signed-in
    /// account and the sync turned on. The first one is due at once.
    pub fn due(&self, now: Instant, signed_in: bool, enabled: bool) -> bool {
        signed_in && enabled && (self.asked || self.wait(now).is_zero())
    }

    /// How long until the next timed sync, for a timer to sleep on.
    pub fn wait(&self, now: Instant) -> Duration {
        let timed = self.last.map(|last| last + self.every);
        let next = match (timed, self.not_before) {
            (Some(timed), Some(held)) => Some(timed.max(held)),
            (timed, held) => timed.or(held),
        };
        next.map_or(Duration::ZERO, |next| next.saturating_duration_since(now))
    }

    /// A sync started at `now`, whatever came of it: the next timed one is a
    /// full interval away.
    pub fn ran(&mut self, now: Instant) {
        self.last = Some(now);
        self.asked = false;
        self.not_before = None;
    }

    /// The server asked for patience (`429`, with its `Retry-After`): no
    /// timed sync before `delay` has passed.
    pub fn hold(&mut self, now: Instant, delay: Duration) {
        self.not_before = Some(now + delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_is_its_folder_s_last_component_and_nothing_above_it() {
        for (folder, name) in [
            ("C:\\Users\\ada\\Projects\\hangar", "hangar"),
            ("C:\\Users\\ada\\Projects\\hangar\\", "hangar"),
            ("/home/ada/Projects/hangar", "hangar"),
            ("/home/ada/Projects/hangar//", "hangar"),
            ("C:/Users/ada\\mixed/site web", "site web"),
            ("\\\\server\\share\\atelier", "atelier"),
            ("hangar", "hangar"),
            ("/home/ada/été", "été"),
            ("C:\\", ""),
            ("C:", ""),
            ("/", ""),
            ("", ""),
            ("/home/ada/..", ""),
            ("/home/ada/a\nb", "ab"),
        ] {
            let got = project_name(folder);
            assert_eq!(got, name, "{folder}");
            assert!(!got.contains(['/', '\\']), "{folder}");
        }
        // Cut to the protocol's length, in characters.
        let long = format!("/work/{}", "格".repeat(150));
        assert_eq!(project_name(&long).chars().count(), api::MAX_PROJECT_CHARS);
    }

    #[test]
    fn a_model_is_one_line_within_the_limit_and_never_empty() {
        assert_eq!(model_name("claude-opus-5-5"), "claude-opus-5-5");
        assert_eq!(model_name("a\tb"), "ab");
        assert_eq!(model_name(""), "unknown");
        assert_eq!(model_name(&"m".repeat(300)).len(), api::MAX_MODEL_CHARS);
    }

    #[test]
    fn rows_are_cut_into_calls_of_at_most_the_protocol_s_count() {
        let row = |n: usize| StatsRow {
            day: "2026-11-16".into(),
            model: "m".into(),
            project: format!("p{n}"),
            input_tokens: 1,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd_micros: 0,
            active_seconds: 0,
            sessions: 0,
        };
        assert!(calls(Vec::new()).is_empty());
        let sizes = |count: usize| -> Vec<usize> {
            calls((0..count).map(row).collect())
                .iter()
                .map(Vec::len)
                .collect()
        };
        assert_eq!(sizes(1), [1]);
        assert_eq!(sizes(api::MAX_STATS_ROWS), [api::MAX_STATS_ROWS]);
        assert_eq!(sizes(api::MAX_STATS_ROWS + 1), [api::MAX_STATS_ROWS, 1]);
        // Rows at their longest names still fit the body.
        let wide = |n: usize| StatsRow {
            model: "格".repeat(api::MAX_MODEL_CHARS),
            project: format!("{n:0>5}{}", "格".repeat(api::MAX_PROJECT_CHARS - 5)),
            ..row(n)
        };
        for call in calls((0..api::MAX_STATS_ROWS * 2).map(wide).collect()) {
            assert!(call.len() <= api::MAX_STATS_ROWS);
            let body = serde_json::to_vec(&api::PutStatsRequest { rows: call }).unwrap();
            assert!(body.len() <= api::MAX_BODY_BYTES);
        }
    }

    #[test]
    fn the_schedule_runs_at_once_then_every_interval_and_when_asked() {
        let start = Instant::now();
        let minute = Duration::from_secs(60);
        let mut schedule = Schedule::new();
        // Never without an account, never with the sync off.
        assert!(!schedule.due(start, false, true));
        assert!(!schedule.due(start, true, false));
        schedule.sync_now();
        assert!(!schedule.due(start, false, true));
        assert!(!schedule.due(start, true, false));
        let mut schedule = Schedule::new();
        // The first one right away.
        assert!(schedule.due(start, true, true));
        assert_eq!(schedule.wait(start), Duration::ZERO);
        schedule.ran(start);
        assert!(!schedule.due(start + minute, true, true));
        assert_eq!(schedule.wait(start + minute), minute * 14);
        assert!(!schedule.due(start + minute * 14, true, true));
        assert!(schedule.due(start + minute * 15, true, true));
        // "Now" does not wait for the clock, and is asked once.
        schedule.sync_now();
        assert!(schedule.due(start + minute, true, true));
        schedule.ran(start + minute);
        assert!(!schedule.due(start + minute * 2, true, true));
        assert!(schedule.due(start + minute * 16, true, true));
    }

    #[test]
    fn a_server_asking_for_patience_holds_the_timed_sync_back() {
        let start = Instant::now();
        let minute = Duration::from_secs(60);
        let mut schedule = Schedule::new();
        schedule.ran(start);
        schedule.hold(start, minute * 40);
        assert!(!schedule.due(start + minute * 20, true, true));
        assert_eq!(schedule.wait(start + minute * 20), minute * 20);
        assert!(schedule.due(start + minute * 40, true, true));
        // Asking for it now is the person's call.
        schedule.sync_now();
        assert!(schedule.due(start + minute, true, true));
        // The next run clears the hold.
        schedule.ran(start + minute * 40);
        assert!(schedule.due(start + minute * 55, true, true));
    }
}
