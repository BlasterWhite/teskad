//! The plan's rate-limit windows, as the last session to report saw them
//! (`NavigatorPlan.tsx`): one account behind every pane, so one block for all
//! of them. Nothing shows until a pane's side channel has reported: the
//! teskad-bridge mod in a Claude Code, Hangar attached to a Codex — either
//! agent's windows read the same (`five_hour`, `seven_day`, a gateway's).
//! Whose they are is said by the agents that report them (`usage.limits`),
//! never by a name written here; with several agents' accounts the per-account
//! block (`accounts::plan`) takes over.

use iced::widget::{column, container, row, text, tooltip};
use iced::{Alignment, Element, Length};

use hangar_core::agent::UsageSource;
use hangar_i18n::{Key, Translator, args};
use hangar_theme::tokens::{font_size, scale, scale::type_scale, space};
use hangar_theme::{Theme, fonts, style};
use hangar_ui::meter;
use hangar_ui::tail_text::tail_text;

use super::skin;

use crate::ctx::Ctx;
use crate::inspector::format::{format_percent, reset_when, resets_at_ms};
use crate::message::Message;
use crate::panes_logic::wall_clock_ms;
use crate::state::Model;
use crate::state::live::{PlanLimits, RateLimit};
use crate::state::types::AgentKind;

/// A window's usage as the meter draws it: whole percent, held between 0 and
/// 100.
pub fn meter_pct(percent_used: f64) -> u8 {
    percent_used.round().clamp(0.0, 100.0) as u8
}

/// The percent a window's meter shows at `now`: none once its reset has
/// passed, the plan having started the window over since the figure was
/// reported.
pub fn shown_pct(limit: &RateLimit, now: i64) -> u8 {
    match resets_at_ms(limit) {
        Some(reset) if reset <= now => 0,
        _ => meter_pct(limit.percent_used),
    }
}

/// The name of a window: `five_hour` and `seven_day` are the plan's own, any
/// other is the gateway's, spelled with spaces (`label`).
pub fn window_label(tr: Translator, kind: &str) -> String {
    match kind {
        "five_hour" => tr.t(Key::SidebarPlanFiveHour).into_owned(),
        "seven_day" => tr.t(Key::SidebarPlanWeek).into_owned(),
        other => other.replace('_', " "),
    }
}

/// When the plan next frees up something: the soonest reset still to come
/// (in ms since the epoch), which is what a pane about to hit its limit is
/// waiting for.
pub fn next_reset(limits: &[RateLimit], now: i64) -> Option<i64> {
    limits
        .iter()
        .filter_map(resets_at_ms)
        .filter(|at| *at > now)
        .min()
}

/// A reset as the end of a sentence, in the language of `tr`
/// (`resetWhen`; the inspector says it the same way).
pub fn reset_text(tr: Translator, at: i64, now: i64) -> String {
    reset_when(at, now, tr.locale()).text(tr)
}

/// The agents whose sessions report the plan's windows (`usage.limits`),
/// among the panes open, each once, in the order the panes come: whom the
/// block's figures can be from.
pub fn reporters(m: &Model) -> Vec<AgentKind> {
    let mut kinds: Vec<AgentKind> = Vec::new();
    let panes = m.state().workspaces.iter().flat_map(|ws| ws.panes.iter());
    for kind in panes.filter_map(|pane| pane.agent.kind()) {
        if kind.info().caps.usage.limits != UsageSource::None && !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    kinds
}

/// The tooltip: what the block is, then one line per window that says when it
/// resets. The figures are whichever session reported last: named after its
/// agent when only one agent among `reporters` could have, said plainly
/// otherwise.
pub fn hint(tr: Translator, reporters: &[AgentKind], limits: &[RateLimit], now: i64) -> String {
    let head = match reporters {
        [one] => tr.t_with(Key::SidebarPlanHint, &args!("agent" => one.info().label)),
        _ => tr.t(Key::SidebarPlanHintAny),
    };
    let mut lines = vec![head.into_owned()];
    for limit in limits {
        let Some(at) = resets_at_ms(limit) else {
            continue;
        };
        lines.push(
            tr.t_with(
                Key::SidebarPlanResets,
                &args!(
                    "window" => window_label(tr, &limit.kind),
                    "reset" => reset_text(tr, at, now)
                ),
            )
            .into_owned(),
        );
    }
    lines.join("\n")
}

/// The report of a session, if it says anything: an empty one keeps the last
/// (`setPlanLimits`).
pub fn take(limits: &[RateLimit]) -> Option<PlanLimits> {
    (!limits.is_empty()).then(|| PlanLimits {
        limits: limits.to_vec(),
        // Wall-clock milliseconds, as the shared `PlanLimits` keeps every moment
        // (state/live.rs): the inspector shows them as clock times.
        at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as u64),
    })
}

pub fn view<'a>(plan: Option<&'a PlanLimits>, ctx: Ctx<'a>) -> Option<Element<'a, Message>> {
    let plan = plan?;
    let t = ctx.theme;
    let tr = ctx.i18n;
    let now = i64::try_from(wall_clock_ms()).unwrap_or_default();

    let entry = entry(
        t,
        tr,
        tr.t(Key::SidebarPlan).into_owned(),
        &plan.limits,
        now,
    );
    let words = hint(tr, &reporters(ctx.model), &plan.limits, now);
    Some(card(t, vec![entry], words))
}

/// One account's part of the card: its name and its next reset, then a gauge
/// per window, or that there are no figures yet. The per-account block draws
/// the same parts.
pub fn entry<'a>(
    t: &'a Theme,
    tr: Translator,
    title: String,
    limits: &[RateLimit],
    now: i64,
) -> Element<'a, Message> {
    let mut head = row![
        tail_text(title)
            .readable()
            .cut_end()
            .size(type_scale::CAPTION.size)
            .font(fonts::ui_semibold())
            .color(t.palette.text),
    ]
    .spacing(space::MD)
    .align_y(Alignment::Center);
    if let Some(next) = next_reset(limits, now) {
        head = head.push(
            text(reset_text(tr, next, now))
                .size(type_scale::LABEL.size)
                .color(t.palette.text_muted)
                .wrapping(text::Wrapping::None),
        );
    }
    let mut lines = column![head].spacing(space::MD);
    for limit in limits {
        let pct = shown_pct(limit, now);
        lines = lines.push(meter::plan_row_with(
            t,
            window_label(tr, &limit.kind),
            format_percent(f64::from(pct), tr.locale()),
            pct,
        ));
    }
    if limits.is_empty() {
        lines = lines.push(
            text(tr.t(Key::SidebarPlanNoFigures).into_owned())
                .size(type_scale::LABEL.size)
                .color(t.palette.text_muted),
        );
    }
    lines.into()
}

/// The « Plan » card at the foot of the list, with its tooltip.
pub fn card<'a>(
    t: &'a Theme,
    entries: Vec<Element<'a, Message>>,
    hint: String,
) -> Element<'a, Message> {
    let card = container(column(entries).spacing(space::MD))
        .width(Length::Fill)
        .padding(scale::space::S3)
        .style(skin::card(t));
    container(tooltip(
        card,
        container(text(hint).size(font_size::CAPTION_LG))
            .padding([space::XS - 1.0, space::MD - 1.0])
            .style(style::container::tooltip(t)),
        tooltip::Position::Top,
    ))
    .padding(space::LG)
    .into()
}

#[cfg(test)]
mod tests;
