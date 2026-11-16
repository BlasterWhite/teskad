//! The statistics of every machine of the Hangar account, merged for the
//! « Toutes les machines » view of the Statistics page.
//!
//! The rows come from the account server as `StatsResponse`: totals per
//! device, day, model and project name. Here they are only added up, by day,
//! machine, project and model, within a period. Tokens count as the local page
//! counts them: input, output, cache writes and cache reads.

use std::collections::BTreeMap;

use hangar_core::cloud::{Device, DeviceStatsRow, StatsResponse};

/// What a slice of rows adds up to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub tokens: u64,
    pub active_seconds: u64,
    pub cost_micros: u64,
}

impl Totals {
    fn add(&mut self, row: &DeviceStatsRow) {
        let r = &row.row;
        self.tokens = self.tokens.saturating_add(
            r.input_tokens
                .saturating_add(r.output_tokens)
                .saturating_add(r.cache_read_tokens)
                .saturating_add(r.cache_write_tokens),
        );
        self.active_seconds = self.active_seconds.saturating_add(r.active_seconds);
        self.cost_micros = self.cost_micros.saturating_add(r.cost_usd_micros);
    }

    /// The cost in dollars.
    pub fn cost_usd(&self) -> f64 {
        self.cost_micros as f64 / 1e6
    }
}

/// What the account's machines did together, as the page shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Merged {
    pub total: Totals,
    /// Newest day first.
    pub by_day: Vec<(String, Totals)>,
    /// Largest cost first, then by name: one entry per machine, `this` marks
    /// the PC the page runs on.
    pub by_machine: Vec<MachineTotals>,
    /// Largest cost first. An empty name is a project with none of its own.
    pub by_project: Vec<(String, Totals)>,
    pub by_model: Vec<(String, Totals)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineTotals {
    pub id: String,
    pub name: String,
    pub os: String,
    pub this: bool,
    pub signed_out: bool,
    pub totals: Totals,
}

/// The rows of `from` (a `YYYY-MM-DD` day, included) on, added up. `this` is
/// the server's id for this PC's device. A machine with rows only in a
/// period that is gone still appears, with zeros, so the breakdown names every
/// machine of the account.
pub fn merge(answer: &StatsResponse, from: Option<&str>, this: Option<&str>) -> Merged {
    let mut merged = Merged::default();
    let mut days: BTreeMap<&str, Totals> = BTreeMap::new();
    let mut machines: BTreeMap<&str, Totals> = BTreeMap::new();
    let mut projects: BTreeMap<&str, Totals> = BTreeMap::new();
    let mut models: BTreeMap<&str, Totals> = BTreeMap::new();

    for row in &answer.rows {
        // Days are `YYYY-MM-DD`: text order is calendar order.
        if from.is_some_and(|from| row.row.day.as_str() < from) {
            continue;
        }
        merged.total.add(row);
        days.entry(&row.row.day).or_default().add(row);
        machines.entry(&row.device_id).or_default().add(row);
        projects.entry(&row.row.project).or_default().add(row);
        models.entry(&row.row.model).or_default().add(row);
    }

    merged.by_day = days
        .into_iter()
        .rev()
        .map(|(day, totals)| (day.to_string(), totals))
        .collect();
    merged.by_project = ranked(projects);
    merged.by_model = ranked(models);

    let mut by_machine: Vec<MachineTotals> = answer
        .devices
        .iter()
        .map(|device| machine(device, machines.get(device.id.as_str()), this))
        .collect();
    by_machine.sort_by(|a, b| {
        b.totals
            .cost_micros
            .cmp(&a.totals.cost_micros)
            .then_with(|| a.name.cmp(&b.name))
    });
    merged.by_machine = by_machine;
    merged
}

fn machine(device: &Device, totals: Option<&Totals>, this: Option<&str>) -> MachineTotals {
    MachineTotals {
        id: device.id.clone(),
        name: device.name.clone(),
        os: device.os.clone(),
        this: this == Some(device.id.as_str()),
        signed_out: device.revoked_at.is_some(),
        totals: totals.copied().unwrap_or_default(),
    }
}

fn ranked(map: BTreeMap<&str, Totals>) -> Vec<(String, Totals)> {
    let mut list: Vec<(String, Totals)> = map
        .into_iter()
        .map(|(name, totals)| (name.to_string(), totals))
        .collect();
    list.sort_by(|a, b| {
        b.1.cost_micros
            .cmp(&a.1.cost_micros)
            .then_with(|| b.1.tokens.cmp(&a.1.tokens))
            .then_with(|| a.0.cmp(&b.0))
    });
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use hangar_core::cloud::StatsRow;

    fn device(id: &str, name: &str, revoked: bool) -> Device {
        Device {
            id: id.into(),
            name: name.into(),
            os: "windows".into(),
            created_at: 1,
            last_seen_at: 2,
            revoked_at: revoked.then_some(3),
        }
    }

    fn row(
        device: &str,
        day: &str,
        model: &str,
        project: &str,
        out: u64,
        cost: u64,
    ) -> DeviceStatsRow {
        DeviceStatsRow {
            device_id: device.into(),
            row: StatsRow {
                day: day.into(),
                model: model.into(),
                project: project.into(),
                input_tokens: 10,
                output_tokens: out,
                cache_read_tokens: 100,
                cache_write_tokens: 1,
                cost_usd_micros: cost,
                active_seconds: 60,
                sessions: 1,
            },
        }
    }

    fn answer() -> StatsResponse {
        StatsResponse {
            devices: vec![
                device("d1", "Atelier", false),
                device("d2", "Portable", false),
                device("d3", "Ancien", true),
            ],
            rows: vec![
                row("d1", "2026-11-15", "opus", "hangar", 5, 2_000_000),
                row("d1", "2026-11-16", "opus", "hangar", 5, 1_000_000),
                row("d2", "2026-11-16", "sonnet", "hangar", 7, 500_000),
                row("d2", "2026-11-16", "sonnet", "blog", 9, 250_000),
            ],
        }
    }

    #[test]
    fn rows_of_every_machine_add_up_by_day_project_model_and_machine() {
        let merged = merge(&answer(), None, Some("d1"));
        // 10 + out + 100 + 1 tokens each (out: 5, 5, 7, 9).
        assert_eq!(merged.total.tokens, 116 + 116 + 118 + 120);
        assert_eq!(merged.total.cost_micros, 3_750_000);
        assert_eq!(merged.total.active_seconds, 240);
        assert_eq!(merged.by_day[0].0, "2026-11-16");
        assert_eq!(merged.by_day[0].1.cost_micros, 1_750_000);
        assert_eq!(merged.by_day[1].0, "2026-11-15");
        assert_eq!(merged.by_project[0].0, "hangar");
        assert_eq!(merged.by_project[1].0, "blog");
        assert_eq!(merged.by_model[0].0, "opus");
    }

    #[test]
    fn the_breakdown_names_every_machine_and_marks_this_pc() {
        let merged = merge(&answer(), None, Some("d2"));
        let names: Vec<_> = merged.by_machine.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Atelier", "Portable", "Ancien"]);
        assert!(merged.by_machine[1].this);
        assert!(!merged.by_machine[0].this);
        assert!(merged.by_machine[2].signed_out);
        assert_eq!(merged.by_machine[2].totals, Totals::default());
    }

    #[test]
    fn a_period_leaves_out_the_days_before_it() {
        let merged = merge(&answer(), Some("2026-11-16"), None);
        assert_eq!(merged.by_day.len(), 1);
        assert_eq!(merged.total.cost_micros, 1_750_000);
        // Atelier still listed, with its one day.
        assert_eq!(merged.by_machine[0].totals.cost_micros, 1_000_000);
    }

    #[test]
    fn no_rows_is_an_empty_merge() {
        let merged = merge(
            &StatsResponse {
                devices: vec![],
                rows: vec![],
            },
            None,
            None,
        );
        assert_eq!(merged, Merged::default());
    }
}
