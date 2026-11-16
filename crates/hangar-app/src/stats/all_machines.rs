//! « Toutes les machines »: the statistics of every machine of the Hangar
//! account, as the sync last read them (hangar_account::Pulled): the merged
//! totals, then a table per machine, per day, per project and per model.
//!
//! The rows are totals per machine, day, model and project name: there are no
//! sessions or turns to chart here, so the page is tables. « Ce PC » is the
//! page as it always was.

use hangar_i18n::Key;
use hangar_theme::tokens::space;
use hangar_ui::text as typography;
use iced::widget::{Space, column, row, scrollable};
use iced::{Element, Length, Padding};

use super::format::{format_integer, format_usd};
use super::model::Period;
use super::tables::{FIRST_COLUMN, NUMBER_COLUMN, cell, frame};
use super::ui::Ui;
use super::view::card;
use crate::ctx::Ctx;
use crate::hangar_account::{Merged, Pulled, Totals, merge};
use crate::message::Message;

/// The first day a period takes in, `YYYY-MM-DD` on this machine's calendar,
/// or `None` for all of the history.
pub fn period_start(period: Period, today: chrono::NaiveDate) -> Option<String> {
    let days = match period {
        Period::D7 => 7,
        Period::D30 => 30,
        Period::D90 => 90,
        Period::All => return None,
    };
    Some((today - chrono::Duration::days(days - 1)).to_string())
}

/// The rows of the account for `period`, added up.
pub fn merged_for(pulled: &Pulled, period: Period) -> Merged {
    let from = period_start(period, chrono::Local::now().date_naive());
    merge(&pulled.answer, from.as_deref(), pulled.this.as_deref())
}

fn seconds_text(ui: Ui, seconds: u64) -> String {
    super::format::format_duration(seconds as f64 * 1000.0, ui.tr)
}

fn figures<'a>(ui: Ui, totals: &Totals) -> [String; 3] {
    let locale = ui.locale();
    [
        format_integer(totals.tokens as f64, locale),
        seconds_text(ui, totals.active_seconds),
        format_usd(totals.cost_usd(), locale, false),
    ]
}

/// A table: its first column (`first`), then tokens, active time and cost, a
/// line per entry and a closing total.
fn table<'a>(
    ui: Ui,
    first: Key,
    entries: Vec<(String, Totals)>,
    total: &Totals,
) -> Element<'a, Message> {
    let line = |name: String, totals: &Totals, head: bool| -> Element<'a, Message> {
        let [tokens, active, cost] = if head {
            [
                ui.s(Key::StatsAllTokens),
                ui.s(Key::StatsAllActive),
                ui.s(Key::StatsAllCost),
            ]
        } else {
            figures(ui, totals)
        };
        row![
            cell(ui, name, FIRST_COLUMN, true, head),
            cell(ui, tokens, NUMBER_COLUMN, false, head),
            cell(ui, active, NUMBER_COLUMN, false, head),
            cell(ui, cost, NUMBER_COLUMN, false, head),
        ]
        .into()
    };
    let mut rows = vec![line(ui.s(first), &Totals::default(), true)];
    for (name, totals) in entries {
        rows.push(line(name, &totals, false));
    }
    rows.push(line(ui.s(Key::StatsTotal), total, false));
    frame(ui, rows, false)
}

pub fn view<'a>(ctx: Ctx<'a>, ui: Ui, pulled: &Pulled, period: Period) -> Element<'a, Message> {
    let merged = merged_for(pulled, period);
    let none = ui.s(Key::StatsAllNoProject);

    if merged.by_day.is_empty() {
        return scrollable(
            iced::widget::container(typography::muted(ui.t, ui.s(Key::StatsAllEmpty)))
                .padding(space::XXL),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    }

    let machines = merged
        .by_machine
        .iter()
        .map(|machine| {
            let name = if machine.this {
                format!("{} · {}", machine.name, ctx.t(Key::HangarAccountThisPc))
            } else if machine.signed_out {
                format!(
                    "{} · {}",
                    machine.name,
                    ctx.t(Key::HangarAccountMachineSignedOut)
                )
            } else {
                machine.name.clone()
            };
            (name, machine.totals)
        })
        .collect();
    let projects = merged
        .by_project
        .iter()
        .map(|(name, totals)| {
            let shown = if name.is_empty() {
                none.clone()
            } else {
                name.clone()
            };
            (shown, *totals)
        })
        .collect();

    let page = column![
        card(
            ui,
            ui.s(Key::StatsAllByMachine),
            None,
            None,
            table(ui, Key::StatsAllMachine, machines, &merged.total),
        ),
        card(
            ui,
            ui.s(Key::StatsAllByDay),
            None,
            None,
            table(ui, Key::StatsAllDay, merged.by_day.clone(), &merged.total),
        ),
        card(
            ui,
            ui.s(Key::StatsAllByProject),
            None,
            None,
            table(ui, Key::StatsAllProject, projects, &merged.total),
        ),
        card(
            ui,
            ui.s(Key::StatsAllByModel),
            None,
            None,
            table(
                ui,
                Key::StatsAllModel,
                merged.by_model.clone(),
                &merged.total
            ),
        ),
        Space::new().height(space::XL),
    ]
    .spacing(14)
    .max_width(1240);

    scrollable(
        iced::widget::container(page)
            .padding(Padding::from([space::XL, space::XXL]))
            .center_x(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn a_period_starts_the_right_number_of_days_back() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        assert_eq!(
            period_start(Period::D7, today).as_deref(),
            Some("2026-11-10")
        );
        assert_eq!(
            period_start(Period::D30, today).as_deref(),
            Some("2026-10-18")
        );
        assert_eq!(period_start(Period::All, today), None);
    }
}
