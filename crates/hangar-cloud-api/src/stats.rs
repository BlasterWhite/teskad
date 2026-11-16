//! The statistics a machine sends, and the account's read back.
//!
//! A row is the totals of one day, one model and one project on one machine.
//! The project is a name, the folder's last component: never a path. Nothing
//! of a transcript is in a row, and no type here has a field to put it in.

use serde::{Deserialize, Serialize};

use crate::Device;
use crate::limits::{MAX_COUNTER, MAX_MODEL_CHARS, MAX_PROJECT_CHARS};

/// The totals of one (day, model, project) on one machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsRow {
    /// `YYYY-MM-DD`, the day in the machine's own time zone, as the desktop's
    /// statistics page counts days.
    pub day: String,
    /// The model's id, as the transcripts name it.
    pub model: String,
    /// The project's name: the last component of its folder. Empty for work
    /// done outside any folder that has a name.
    pub project: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// What the API would have charged, in millionths of a US dollar.
    pub cost_usd_micros: u64,
    pub active_seconds: u64,
    pub sessions: u64,
}

/// Why the server refuses a row, and what the desktop checks before sending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsRowError {
    Day,
    ModelEmpty,
    ModelTooLong,
    ProjectTooLong,
    /// A name with a control character in it, or a project with a path
    /// separator: a name is one line and one folder.
    Name,
    CounterTooLarge,
}

impl std::fmt::Display for StatsRowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            StatsRowError::Day => "day is not a YYYY-MM-DD date",
            StatsRowError::ModelEmpty => "model is empty",
            StatsRowError::ModelTooLong => "model is longer than 100 characters",
            StatsRowError::ProjectTooLong => "project is longer than 100 characters",
            StatsRowError::Name => {
                "model or project holds a control character, or project a path separator"
            }
            StatsRowError::CounterTooLarge => "a counter is larger than 2^63 - 1",
        })
    }
}

impl std::error::Error for StatsRowError {}

impl StatsRow {
    /// The key a row is upserted by, on its device.
    pub fn key(&self) -> (&str, &str, &str) {
        (&self.day, &self.model, &self.project)
    }

    /// Whether the server takes this row.
    pub fn validate(&self) -> Result<(), StatsRowError> {
        if !is_day(&self.day) {
            return Err(StatsRowError::Day);
        }
        if self.model.is_empty() {
            return Err(StatsRowError::ModelEmpty);
        }
        if self.model.chars().count() > MAX_MODEL_CHARS {
            return Err(StatsRowError::ModelTooLong);
        }
        if self.project.chars().count() > MAX_PROJECT_CHARS {
            return Err(StatsRowError::ProjectTooLong);
        }
        // A separator in a project would be a path, which never leaves the
        // PC: refused here so that a mistake on the desktop cannot send one.
        if self.model.chars().any(char::is_control)
            || self
                .project
                .chars()
                .any(|c| c.is_control() || c == '/' || c == '\\')
        {
            return Err(StatsRowError::Name);
        }
        let counters = [
            self.input_tokens,
            self.output_tokens,
            self.cache_read_tokens,
            self.cache_write_tokens,
            self.cost_usd_micros,
            self.active_seconds,
            self.sessions,
        ];
        if counters.iter().any(|&n| n > MAX_COUNTER) {
            return Err(StatsRowError::CounterTooLarge);
        }
        Ok(())
    }
}

/// Whether `text` is a calendar day written `YYYY-MM-DD`: ten ASCII
/// characters, a real month, a day that month has (leap years included).
/// Days written this way sort as text, which the server's range relies on.
pub fn is_day(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u32> {
        bytes[range].iter().try_fold(0u32, |n, b| {
            b.is_ascii_digit().then(|| n * 10 + u32::from(b - b'0'))
        })
    };
    let (Some(year), Some(month), Some(day)) = (number(0..4), number(5..7), number(8..10)) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

/// The body of `PUT /v1/stats`: rows of the device whose token signs the
/// request, each replacing the row of the same (day, model, project).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutStatsRequest {
    pub rows: Vec<StatsRow>,
}

/// The answer to `PUT /v1/stats`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutStatsResponse {
    /// How many rows the device now has for the keys it sent: the number of
    /// distinct keys in the request.
    pub upserted: u64,
}

/// The query of `GET /v1/stats`. Both ends are days, inclusive; either may be
/// left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

/// A row with the device it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStatsRow {
    pub device_id: String,
    #[serde(flatten)]
    pub row: StatsRow,
}

/// The answer to `GET /v1/stats`: the account's rows across its machines, and
/// the machines they name (signed-out ones included, while they have rows).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsResponse {
    pub devices: Vec<Device>,
    pub rows: Vec<DeviceStatsRow>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::round_trip;
    use serde_json::json;

    fn row() -> StatsRow {
        StatsRow {
            day: "2026-11-16".into(),
            model: "claude-opus-5-5".into(),
            project: "hangar".into(),
            input_tokens: 1200,
            output_tokens: 3400,
            cache_read_tokens: 560_000,
            cache_write_tokens: 78_000,
            cost_usd_micros: 1_234_567,
            active_seconds: 5400,
            sessions: 3,
        }
    }

    fn wire_row() -> serde_json::Value {
        json!({
            "day": "2026-11-16",
            "model": "claude-opus-5-5",
            "project": "hangar",
            "input_tokens": 1200,
            "output_tokens": 3400,
            "cache_read_tokens": 560_000,
            "cache_write_tokens": 78_000,
            "cost_usd_micros": 1_234_567,
            "active_seconds": 5400,
            "sessions": 3,
        })
    }

    #[test]
    fn the_upload_round_trips() {
        round_trip(
            &PutStatsRequest { rows: vec![row()] },
            json!({ "rows": [wire_row()] }),
        );
        round_trip(&PutStatsResponse { upserted: 1 }, json!({ "upserted": 1 }));
    }

    #[test]
    fn the_read_back_names_each_row_s_device_beside_its_totals() {
        let mut wire = wire_row();
        wire["device_id"] = json!("dev_1");
        round_trip(
            &StatsResponse {
                devices: vec![Device {
                    id: "dev_1".into(),
                    name: "Atelier".into(),
                    os: "linux".into(),
                    created_at: 1,
                    last_seen_at: 2,
                    revoked_at: None,
                }],
                rows: vec![DeviceStatsRow {
                    device_id: "dev_1".into(),
                    row: row(),
                }],
            },
            json!({
                "devices": [{
                    "id": "dev_1",
                    "name": "Atelier",
                    "os": "linux",
                    "created_at": 1,
                    "last_seen_at": 2,
                }],
                "rows": [wire],
            }),
        );
    }

    #[test]
    fn the_range_is_optional_at_both_ends() {
        round_trip(&StatsQuery::default(), json!({}));
        round_trip(
            &StatsQuery {
                from: Some("2026-11-01".into()),
                to: None,
            },
            json!({ "from": "2026-11-01" }),
        );
    }

    #[test]
    fn a_counter_keeps_every_digit_up_to_the_largest_allowed() {
        let big = StatsRow {
            input_tokens: MAX_COUNTER,
            ..row()
        };
        let text = serde_json::to_string(&big).unwrap();
        assert!(text.contains("9223372036854775807"));
        assert_eq!(serde_json::from_str::<StatsRow>(&text).unwrap(), big);
        assert_eq!(big.validate(), Ok(()));
    }

    #[test]
    fn days_are_calendar_days() {
        for day in [
            "2026-11-16",
            "2024-02-29",
            "2000-02-29",
            "2026-01-31",
            "0001-01-01",
        ] {
            assert!(is_day(day), "{day}");
        }
        for day in [
            "",
            "2026-11-6",
            "2026-13-01",
            "2026-00-10",
            "2026-11-00",
            "2026-11-31",
            "2026-02-29",
            "1900-02-29",
            "2026/11/16",
            "2026-11-16T00:00:00Z",
            "+026-11-16",
            "２０２６-11-16",
        ] {
            assert!(!is_day(day), "{day}");
        }
    }

    #[test]
    fn a_row_is_checked_before_it_is_sent_or_stored() {
        assert_eq!(row().validate(), Ok(()));
        assert_eq!(row().key(), ("2026-11-16", "claude-opus-5-5", "hangar"));

        let with = |change: fn(&mut StatsRow)| {
            let mut changed = row();
            change(&mut changed);
            changed.validate()
        };
        assert_eq!(
            with(|r| r.day = "yesterday".into()),
            Err(StatsRowError::Day)
        );
        assert_eq!(with(|r| r.model.clear()), Err(StatsRowError::ModelEmpty));
        assert_eq!(
            with(|r| r.model = "m".repeat(MAX_MODEL_CHARS + 1)),
            Err(StatsRowError::ModelTooLong)
        );
        assert_eq!(
            with(|r| r.project = "p".repeat(MAX_PROJECT_CHARS + 1)),
            Err(StatsRowError::ProjectTooLong)
        );
        // Characters, not bytes: a hundred of three bytes each still fit.
        assert_eq!(with(|r| r.project = "格".repeat(MAX_PROJECT_CHARS)), Ok(()));
        assert_eq!(with(|r| r.project.clear()), Ok(()));
        // A path is not a name.
        assert_eq!(
            with(|r| r.project = "C:\\Users\\ada\\hangar".into()),
            Err(StatsRowError::Name)
        );
        assert_eq!(
            with(|r| r.project = "/home/ada/hangar".into()),
            Err(StatsRowError::Name)
        );
        assert_eq!(
            with(|r| r.project = "a\nb".into()),
            Err(StatsRowError::Name)
        );
        assert_eq!(with(|r| r.model = "a\tb".into()), Err(StatsRowError::Name));
        assert_eq!(
            with(|r| r.sessions = MAX_COUNTER + 1),
            Err(StatsRowError::CounterTooLarge)
        );
    }
}
