//! The inspector's sections (`InspectorPanels.tsx`): session, terminal,
//! context and cost, the board task, the prompt queue and the recent
//! activity. The Files section is `files.rs`.
//!
//! Each is a function of what it shows, built from a [`Scene`] — the pane,
//! the workspace, the clock, the language — so the column is a pure
//! function of the model and a snapshot test can draw it.

use hangar_core::agent::{Capabilities, UsageSource};
use hangar_core::bridge::Queued;
use hangar_i18n::{Key, Locale, Translator, args};
use hangar_theme::tokens::{scale, scale::type_scale, space};
use hangar_theme::{AgentState, Theme, fonts, style};
use hangar_ui::button::{self as buttons, icon_button};
use hangar_ui::glyph::{self, GlyphState};
use hangar_ui::tail_text::tail_text;
use hangar_ui::{Icon, focus, icon, meter, tip};
use iced::widget::text::{self, Text};
use iced::widget::{button, column, container, mouse_area, row, space as filler};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding, mouse, window};

use super::format;
use super::section::{self, Section};
use super::sources::{self, TaskCard};
use super::widgets::{self, kv, stacked};
use super::{Msg, SectionId};
use crate::ctx::Ctx;
use crate::message::Message;
use crate::state::Model;
use crate::state::live::{ContextGauge, PaneEvent, PaneInsight, PaneMcp, PaneMcpMode, RateLimit};
use crate::state::types::{AgentId, Pane, Workspace};
use crate::{board, panes_logic};

/// Everything a section needs besides what it shows.
#[derive(Clone, Copy)]
pub(super) struct Scene<'a> {
    pub ctx: Ctx<'a>,
    pub ws: &'a Workspace,
    pub pane: &'a Pane,
    /// Epoch milliseconds: the clock the sections word elapsed times with.
    pub now: i64,
    /// Section ids folded away (`settings.inspectorCollapsed`).
    pub folded: &'a [String],
    /// The column's width in interface points.
    pub width: f32,
}

impl<'a> Scene<'a> {
    pub fn theme(&self) -> &'a Theme {
        self.ctx.theme
    }

    pub fn tr(&self) -> Translator {
        self.ctx.i18n
    }

    pub fn locale(&self) -> Locale {
        self.ctx.i18n.locale()
    }

    pub fn is_folded(&self, id: SectionId) -> bool {
        let stored = self.folded.iter().any(|folded| folded == id.as_str());
        // An agent's terminal block starts folded: what is stored says it was
        // opened.
        stored != (id == SectionId::AgentTerminal)
    }

    /// The column's own padding at both sides of a section (`--insp-pad`).
    pub const PAD: f32 = 14.0;

    /// A section's frame: its title, a fold that sends `Msg::Toggle`.
    pub fn section(&self, id: SectionId, title: Key) -> Section<'a, Message> {
        section::section(
            self.theme(),
            self.tr().t(title).into_owned(),
            self.is_folded(id),
            Message::Inspector(Msg::Toggle(id)),
            Self::PAD,
        )
    }

    /// The tooltip of a thing, under it.
    pub fn tip(
        &self,
        el: impl Into<Element<'a, Message>>,
        words: impl Into<String>,
    ) -> Element<'a, Message> {
        tip::bottom(self.theme(), el, words)
    }
}

// ---- Small parts ---------------------------------------------------------------------------

/// A text of a size and an ink.
pub(super) fn ink<'a>(content: impl text::IntoFragment<'a>, size: f32, color: Color) -> Text<'a> {
    text::Text::new(content)
        .size(size)
        .style(move |_: &iced::Theme| text::Style { color: Some(color) })
        .wrapping(text::Wrapping::None)
}

/// A line of the quiet colour that says there is nothing (`.insp-empty`).
pub(super) fn empty_note<'a>(
    t: &'a Theme,
    words: impl text::IntoFragment<'a>,
) -> Element<'a, Message> {
    text::Text::new(words)
        .size(type_scale::CAPTION.size)
        .color(t.palette.text_muted)
        .width(Length::Fill)
        .into()
}

/// A text action: the accent as text, and an arrow after it when it leaves
/// for another surface (`.insp-link`).
pub(super) fn link<'a>(
    t: &'a Theme,
    label: impl Into<String>,
    hint: impl Into<String>,
    arrow: bool,
    msg: Msg,
) -> Element<'a, Message> {
    let mut words = row![
        text::Text::new(label.into())
            .size(type_scale::CAPTION.size)
            .wrapping(text::Wrapping::None)
    ]
    .spacing(space::XS)
    .align_y(Alignment::Center);
    if arrow {
        words = words.push(icon(Icon::ArrowUpRight, 10.0));
    }
    tip::bottom(t, ringed_link(t, words, Message::Inspector(msg)), hint)
}

/// `words` as a link the keyboard can reach and press.
pub(super) fn ringed_link<'a>(
    t: &'a Theme,
    words: impl Into<Element<'a, Message>>,
    msg: Message,
) -> Element<'a, Message> {
    let action = button(words)
        .padding(Padding::from([0.0, 2.0]))
        .style(style::button::link(t))
        .on_press(msg.clone());
    focus::ring(t, action)
        .radius(scale::radius::XS)
        .on_activate(msg)
        .into()
}

/// A value and, at its right end, an action (`.insp-kv__split`).
fn split<'a>(
    value: Element<'a, Message>,
    action: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut line = row![container(value).width(Length::Fill).clip(true)]
        .spacing(space::MD)
        .align_y(Alignment::Center);
    if let Some(action) = action {
        line = line.push(action);
    }
    line.into()
}

/// An id or a name in the mono face, cut at its end when the cell is short
/// (`.insp-mono`).
fn mono_cut<'a>(content: impl Into<String>, color: Color) -> Element<'a, Message> {
    tail_text(content)
        .cut_end()
        .size(type_scale::LABEL.size)
        .font(fonts::mono())
        .color(color)
        .into()
}

/// A path in a cell too narrow for it loses its beginning, not its end: the
/// last folders are the ones that tell two checkouts apart (`TailPath`).
pub(super) fn tail_path<'a>(t: &'a Theme, path: &str) -> Element<'a, Message> {
    tip::bottom(
        t,
        tail_text(path.to_string())
            .size(type_scale::LABEL.size)
            .font(fonts::mono())
            .color(t.palette.text),
        path.to_string(),
    )
}

/// The columns a pane's task can be in are `doing` and `review`: the glyph
/// beside the card's state says whose turn it is.
fn task_glyph(card: &TaskCard) -> GlyphState {
    if card.in_review() {
        GlyphState::Yours
    } else {
        GlyphState::Working
    }
}

// ---- Session and terminal -----------------------------------------------------------------

pub(super) fn session<'a>(
    s: &Scene<'a>,
    insight: &'a PaneInsight,
    folder: &'a str,
) -> Section<'a, Message> {
    let (t, tr) = (s.theme(), s.tr());
    let c = &t.palette;
    let agent = s.pane.agent.info();
    let kind = s.pane.agent.kind();
    let ws = s.ws.id.clone();
    let mut rows: Vec<Element<'a, Message>> = Vec::new();

    // The session it resumes, for an agent that resumes one (`sessions`).
    if kind.is_some_and(|kind| kind.info().caps.sessions.resume) {
        let id: Element<'a, Message> = match &s.pane.session_id {
            Some(id) => s.tip(mono_cut(id.clone(), c.text), id.clone()),
            None => ink(
                tr.t(Key::InspectorResumeNew),
                type_scale::CAPTION.size,
                c.text_muted,
            )
            .width(Length::Fill)
            .into(),
        };
        let change = link(
            t,
            tr.t(Key::InspectorResumeChange),
            tr.t(Key::InspectorResumeChangeHint),
            false,
            Msg::OpenSessions(ws.clone(), s.pane.id.clone()),
        );
        rows.push(kv(t, tr.t(Key::InspectorResume), split(id, Some(change))));
    }

    if let Some(since) = insight.session_since.map(sources::at_ms) {
        let words = format::since(since, s.now, s.locale()).text(tr);
        let shown = ink(words, type_scale::CAPTION.size, c.text).width(Length::Shrink);
        rows.push(kv(
            t,
            tr.t(Key::InspectorSince),
            s.tip(shown, format::full_date(since, s.locale())),
        ));
    }

    if let Some(model) = insight.model.as_deref().filter(|model| !model.is_empty()) {
        rows.push(kv(
            t,
            tr.t(Key::InspectorModel),
            s.tip(mono_cut(model.to_string(), c.text), model.to_string()),
        ));
    }

    // The MCP servers its agent was started with, as its launch recorded
    // them (`PaneMcp`); a shell has none.
    if kind.is_some()
        && let Some(servers) = insight.mcp.as_ref().map(mcp_servers)
    {
        let label = args!("agent" => agent.label);
        let value: Element<'a, Message> = match servers {
            Some(list) => s.tip(mono_cut(list.clone(), c.text), list),
            None => ink(
                tr.t_with(Key::InspectorMcpInherit, &label).into_owned(),
                type_scale::CAPTION.size,
                c.text,
            )
            .width(Length::Fill)
            .into(),
        };
        let change = link(
            t,
            tr.t(Key::InspectorMcpChange),
            tr.t_with(Key::InspectorMcpChangeHint, &label),
            false,
            Msg::OpenMcpServers(ws.clone()),
        );
        rows.push(kv(t, tr.t(Key::InspectorMcp), split(value, Some(change))));
    }

    rows.push(kv(t, tr.t(Key::InspectorFolder), tail_path(t, folder)));
    let mut body = column![stacked(0.0, rows)].spacing(space::MD);
    // What the sessions picker's first row does, one press away.
    if kind.is_some_and(|kind| kind.info().caps.sessions.resume) {
        let fresh = buttons::secondary(t, tr.t(Key::SessionsNew).into_owned())
            .icon(Icon::Plus)
            .tip(tr.t(Key::SessionsNewHint).into_owned())
            .on_press(Message::Inspector(Msg::NewSession(ws, s.pane.id.clone())));
        body = body.push(Element::from(fresh));
    }
    s.section(SectionId::Session, Key::InspectorSectionSession)
        .body(body)
}

/// The servers a custom launch named, joined; `None` when the pane inherits
/// its agent's own (`PaneMcp`).
pub(super) fn mcp_servers(mcp: &PaneMcp) -> Option<String> {
    match &mcp.mode {
        PaneMcpMode::Inherit => None,
        PaneMcpMode::Custom { servers } => Some(servers.join(", ")),
    }
}

pub(super) fn terminal<'a>(
    s: &Scene<'a>,
    id: SectionId,
    shell: Option<&str>,
    folder: &'a str,
) -> Section<'a, Message> {
    let (t, tr) = (s.theme(), s.tr());
    let mut rows = Vec::new();
    if let Some(shell) = shell {
        rows.push(kv(
            t,
            tr.t(Key::InspectorShell),
            ink(shell.to_string(), type_scale::CAPTION.size, t.palette.text).width(Length::Fill),
        ));
    }
    rows.push(kv(t, tr.t(Key::InspectorFolder), tail_path(t, folder)));
    let mut section = s.section(id, Key::InspectorSectionTerminal);
    // The shell is named on the title row, so a folded block still says it.
    if let Some(shell) = shell {
        section = section.meta(shell);
    }
    section.body(stacked(0.0, rows))
}

// ---- Context and cost ---------------------------------------------------------------------

/// What the agent's context holds and what its session costs, and the plan's
/// windows (`limits`, as they stand for its agent). Each reads what the
/// agent can tell (`caps`, its kind's levels): the plan for one whose usage
/// reports its windows, the ring for one that measures its context. What it
/// cannot tell is left out rather than shown empty.
pub(super) fn context<'a>(
    s: &Scene<'a>,
    insight: &'a PaneInsight,
    agent_label: &'a str,
    limits: Option<&[RateLimit]>,
    caps: Option<&Capabilities>,
) -> Section<'a, Message> {
    let (t, tr) = (s.theme(), s.tr());
    let show_plan = caps.is_some_and(|caps| caps.usage.limits != UsageSource::None);
    let measures = caps.is_some_and(|caps| caps.usage.context != UsageSource::None);
    let five_hour = limits
        .filter(|_| show_plan)
        .and_then(|limits| format::plan_usage(limits, "five_hour"));
    let week = limits
        .filter(|_| show_plan)
        .and_then(|limits| format::plan_usage(limits, "seven_day"));
    let has_plan = five_hour.is_some() || week.is_some();
    // Cost and plan get their tiles once either is known; an agent that
    // reports neither gets no row of dashes.
    let stats = insight.cost_usd.is_some() || has_plan;

    let mut body: Vec<Element<'a, Message>> = Vec::new();
    match &insight.context {
        Some(gauge) => body.push(gauge_row(s, gauge)),
        None if measures => body.push(empty_note(t, tr.t(Key::InspectorContextEmpty))),
        None => {}
    }

    if stats {
        let cost = cost_tile(s, insight, agent_label);
        let mut tiles: Vec<Element<'a, Message>> = vec![cost];
        if show_plan {
            tiles.push(plan_tile(s, five_hour, week));
        }
        // Side by side while both fit (two tiles of 118 and the gap), stacked
        // in a narrow column.
        let room = s.width - 2.0 * Scene::PAD;
        body.push(if tiles.len() == 2 && room >= 2.0 * 118.0 + 8.0 {
            let plan = tiles.pop().unwrap_or_else(|| filler::vertical().into());
            let cost = tiles.pop().unwrap_or_else(|| filler::vertical().into());
            row![
                container(cost).width(Length::FillPortion(1)),
                container(plan).width(Length::FillPortion(1)),
            ]
            .spacing(space::MD)
            .into()
        } else {
            column(tiles).spacing(space::MD).into()
        });
    }

    s.section(SectionId::Context, Key::InspectorSectionContext)
        .body(column(body).spacing(space::LG).width(Length::Fill))
}

/// The ring and, beside it, how much of the window is used and what that
/// means (`.insp-ctx`).
fn gauge_row<'a>(s: &Scene<'a>, gauge: &'a ContextGauge) -> Element<'a, Message> {
    let (t, tr) = (s.theme(), s.tr());
    let c = &t.palette;
    let pct = gauge.pct.round() as i64;
    let used = format::spaced_unit(&panes_logic::format_tokens(gauge.used_tokens), s.locale());
    let window = format::spaced_unit(&panes_logic::format_tokens(gauge.window), s.locale());
    let hint = if gauge.pct >= 90.0 {
        Key::InspectorContextHintSoon
    } else {
        Key::InspectorContextHint
    };
    let line = type_scale::CAPTION.relative_line_height();
    let info = column![
        text::Text::new(
            tr.t_with(Key::InspectorContextUsed, &args!("pct" => pct))
                .into_owned()
        )
        .size(type_scale::CAPTION.size)
        .line_height(line)
        .font(fonts::ui_semibold())
        .style(style::text::primary(t)),
        text::Text::new(
            tr.t_with(
                Key::InspectorContextTokens,
                &args!("used" => used.as_str(), "window" => window.as_str()),
            )
            .into_owned()
        )
        .size(type_scale::LABEL.size)
        .font(fonts::mono())
        .color(c.text_secondary),
        text::Text::new(tr.t(hint).into_owned())
            .size(type_scale::CAPTION.size)
            .line_height(line)
            .color(c.text_muted)
            .width(Length::Fill),
    ]
    .spacing(space::XXS)
    .width(Length::Fill);
    row![widgets::ring(t, gauge.pct), info]
        .spacing(space::XL)
        .align_y(Alignment::Center)
        .into()
}

fn tile<'a>(s: &Scene<'a>, label: String, body: Element<'a, Message>) -> Element<'a, Message> {
    let t = s.theme();
    let label = ink(label, type_scale::LABEL.size, t.palette.text_muted);
    container(column![label, body].spacing(3))
        .padding(Padding::from([space::MD, space::LG]))
        .width(Length::Fill)
        .style(widgets::tile(t))
        .into()
}

/// The "—" a tile shows when the agent does not report the figure, with the
/// reason as its tooltip (`.insp-stat__value--none`).
fn unknown<'a>(s: &Scene<'a>, why: String) -> Element<'a, Message> {
    let t = s.theme();
    s.tip(
        ink("—", type_scale::SUBTITLE.size, t.palette.text_muted).font(fonts::ui_medium()),
        why,
    )
}

fn cost_tile<'a>(
    s: &Scene<'a>,
    insight: &'a PaneInsight,
    agent_label: &str,
) -> Element<'a, Message> {
    let (t, tr) = (s.theme(), s.tr());
    let value: Element<'a, Message> = match insight.cost_usd {
        Some(usd) => {
            let mut line = row![
                ink(
                    format::format_cost(usd, s.locale()),
                    type_scale::SUBTITLE.size,
                    t.palette.text
                )
                .font(fonts::display())
            ]
            .spacing(space::SM)
            .align_y(Alignment::End);
            if let Some(since) = insight.session_since.map(sources::at_ms) {
                let over = board::format::elapsed_label(s.now - since, tr);
                line = line.push(ink(
                    tr.t_with(Key::InspectorCostOver, &args!("duration" => over.as_str()))
                        .into_owned(),
                    type_scale::LABEL.size,
                    t.palette.text_muted,
                ));
            }
            line.into()
        }
        None => unknown(
            s,
            tr.t_with(Key::InspectorCostUnknown, &args!("agent" => agent_label))
                .into_owned(),
        ),
    };
    tile(s, tr.t(Key::InspectorCost).into_owned(), value)
}

fn plan_tile<'a>(
    s: &Scene<'a>,
    five_hour: Option<RateLimit>,
    week: Option<RateLimit>,
) -> Element<'a, Message> {
    let tr = s.tr();
    let body: Element<'a, Message> = if five_hour.is_some() || week.is_some() {
        let meters = [
            (
                Key::InspectorPlanFiveHour,
                Key::InspectorPlanWindowFiveHour,
                five_hour,
            ),
            (Key::InspectorPlanWeek, Key::InspectorPlanWindowWeek, week),
        ]
        .into_iter()
        .filter_map(|(label, window, limit)| Some(plan_meter(s, label, window, limit?)));
        column(meters).spacing(2).into()
    } else {
        unknown(s, tr.t(Key::InspectorPlanUnknown).into_owned())
    };
    tile(s, plan_name(tr, s.pane), body)
}

/// The name of the plan whose windows a pane's agent spends: its maker's
/// (`inspector.plan.<kind>`: Claude's for Claude Code, ChatGPT's for Codex),
/// else plainly « Plan » for an agent no name was written for.
pub(super) fn plan_name(tr: Translator, pane: &Pane) -> String {
    let key = pane
        .agent
        .kind()
        .and_then(|kind| Key::from_id(&format!("inspector.plan.{}", kind.id())))
        .unwrap_or(Key::InspectorPlan);
    tr.t(key).into_owned()
}

/// One plan window: its name, a bar and the percentage used (`.insp-meter`),
/// and when it resets in the tooltip.
fn plan_meter<'a>(
    s: &Scene<'a>,
    label: Key,
    window: Key,
    limit: RateLimit,
) -> Element<'a, Message> {
    let (t, tr) = (s.theme(), s.tr());
    let pct = limit.percent_used;
    let window = tr.t(window);
    let hint = match format::resets_at_ms(&limit) {
        Some(at) => {
            let reset = format::reset_when(at, s.now, s.locale()).text(tr);
            tr.t_with(
                Key::InspectorPlanHintResets,
                &args!("window" => &*window, "pct" => pct as i64, "reset" => reset.as_str()),
            )
            .into_owned()
        }
        None => tr
            .t_with(
                Key::InspectorPlanHint,
                &args!("window" => &*window, "pct" => pct as i64),
            )
            .into_owned(),
    };
    // Cut, not rounded, to the whole percent: the warning starts at 70, not
    // at 69.5.
    let line = meter::plan_row_with(
        t,
        tr.t(label).into_owned(),
        format::format_percent(pct, s.locale()),
        pct.clamp(0.0, 100.0) as u8,
    );
    s.tip(line, hint)
}

// ---- The board task -----------------------------------------------------------------------

/// The priority as a signal: how many of the three bars are lit, and the
/// word (`BoardPriority`).
///
/// The board draws the same signal on its cards, privately; the level and the
/// word come from `board::format`.
fn priority<'a>(s: &Scene<'a>, level_of: u8) -> Element<'a, Message> {
    let (t, tr) = (s.theme(), s.tr());
    let c = &t.palette;
    let level = board::format::priority_level(f64::from(level_of)).bars();
    let word = tr.t(board::format::priority_key(f64::from(level_of)));
    let bar = |lit: bool, height: f32| -> Element<'a, Message> {
        let color = if lit {
            c.text_secondary
        } else {
            c.border_strong
        };
        container(filler::horizontal())
            .width(2.5)
            .height(height)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(Background::Color(color)),
                border: Border {
                    radius: 1.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into()
    };
    let bars = row![bar(true, 4.0), bar(level >= 2, 6.5), bar(level >= 3, 9.0)]
        .spacing(1.5)
        .align_y(Alignment::End);
    let ink_of = if level == 3 {
        c.text_secondary
    } else {
        c.text_muted
    };
    let line = row![
        bars,
        ink(word.to_string(), type_scale::LABEL.size, ink_of).font(fonts::ui_medium())
    ]
    .spacing(5)
    .align_y(Alignment::Center);
    s.tip(
        line,
        tr.t_with(
            Key::BoardPriorityTitle,
            &args!("n" => i64::from(level_of), "level" => &*word),
        )
        .into_owned(),
    )
}

pub(super) fn task<'a>(s: &Scene<'a>, card: Option<&TaskCard>) -> Section<'a, Message> {
    let (t, tr) = (s.theme(), s.tr());
    let c = &t.palette;
    let open = Msg::OpenBoard;
    let mut section = s.section(SectionId::Task, Key::InspectorSectionTask);
    let Some(card) = card else {
        return section.body(empty_note(t, tr.t(Key::InspectorTaskNone).into_owned()));
    };
    section = section.action(link(
        t,
        tr.t(Key::InspectorTaskOpen),
        tr.t(Key::InspectorTaskOpenHint),
        true,
        open.clone(),
    ));

    let id = format!("#{}", board::format::short_task_id(&card.id));
    let top = row![
        s.tip(
            ink(id, type_scale::LABEL.size, c.text_muted).font(fonts::mono()),
            tr.t_with(Key::InspectorTaskId, &args!("id" => card.id.as_str()))
                .into_owned()
        ),
        filler::horizontal(),
        priority(s, card.priority),
    ]
    .spacing(space::MD)
    .align_y(Alignment::Center);

    let title = container(
        text::Text::new(card.title.clone())
            .size(type_scale::SMALL.size)
            .font(fonts::ui_medium())
            .style(style::text::primary(t)),
    )
    // Two lines, then cut.
    .max_height(2.0 * type_scale::SMALL.size * 1.35 + 1.0)
    .clip(true);

    let doing = card.doing_since.filter(|_| !card.in_review());
    let state: Element<'a, Message> = if card.in_review() {
        ink(
            tr.t(Key::InspectorTaskReview).into_owned(),
            type_scale::CAPTION.size,
            c.text_secondary,
        )
        .into()
    } else if let Some(since) = doing {
        let elapsed = board::format::elapsed_label(s.now - since, tr);
        let words = tr
            .t_with(
                Key::InspectorTaskDoing,
                &args!("duration" => elapsed.as_str()),
            )
            .into_owned();
        let date = format::full_date(since, s.locale());
        s.tip(
            ink(words, type_scale::CAPTION.size, c.text_secondary),
            tr.t_with(Key::BoardDoingSince, &args!("date" => date.as_str()))
                .into_owned(),
        )
    } else {
        ink(
            tr.t(Key::InspectorTaskDoingNow).into_owned(),
            type_scale::CAPTION.size,
            c.text_secondary,
        )
        .into()
    };
    let mut foot = row![glyph::state(t, task_glyph(card), 7.0), state,]
        .spacing(space::SM)
        .align_y(Alignment::Center);
    if card.comments > 0 {
        foot = foot.push(filler::horizontal());
        foot = foot.push(
            s.tip(
                row![
                    icon(Icon::Bubble, 11.0).color(c.text_muted),
                    ink(
                        card.comments.to_string(),
                        type_scale::LABEL.size,
                        c.text_muted
                    )
                    .font(fonts::mono()),
                ]
                .spacing(space::XS)
                .align_y(Alignment::Center),
                tr.tn(Key::InspectorTaskComments, card.comments, &[])
                    .into_owned(),
            ),
        );
    }

    let card_body = container(column![top, title, foot].spacing(5))
        .padding(Padding::from([9.0, 10.0]))
        .width(Length::Fill)
        .style(widgets::tile(t));
    // The whole card answers a click; the "Open" link beside the title is
    // the same action for people who aim at text.
    section.body(
        mouse_area(card_body)
            .on_press(Message::Inspector(open))
            .interaction(mouse::Interaction::Pointer),
    )
}

// ---- The prompt queue ---------------------------------------------------------------------

pub(super) fn queue<'a>(s: &Scene<'a>, queue: &'a [Queued]) -> Section<'a, Message> {
    let (t, tr) = (s.theme(), s.tr());
    let c = &t.palette;
    let mut section = s.section(SectionId::Queue, Key::InspectorSectionQueue);
    if !queue.is_empty() {
        section = section.meta(queue.len().to_string());
    }
    if queue.is_empty() {
        return section.body(empty_note(t, tr.t(Key::InspectorQueueEmpty).into_owned()));
    }
    let lines = queue.iter().enumerate().map(|(at, entry)| {
        let mut line = row![
            container(
                ink((at + 1).to_string(), type_scale::LABEL.size, c.text_muted).font(fonts::mono())
            )
            .width(Length::Shrink),
            container(
                tail_text(entry.text.clone())
                    .cut_end()
                    .size(type_scale::CAPTION.size)
                    .color(c.text)
            )
            .width(Length::Fill),
        ]
        .spacing(space::MD)
        .align_y(Alignment::Center);
        if let Some(by) = panes_logic::queued_by(entry.from.as_deref()) {
            line = line.push(
                s.tip(
                    ink(by.clone(), type_scale::LABEL.size, c.text_muted),
                    tr.t_with(Key::InspectorQueueFrom, &args!("name" => by.as_str()))
                        .into_owned(),
                ),
            );
        }
        line = line.push(
            icon_button(t, Icon::Close)
                .small()
                .side(20.0)
                .icon_size(10.0)
                .tip(tr.t(Key::InspectorQueueCancel).into_owned())
                .on_press(Message::Inspector(Msg::CancelQueued(
                    s.pane.id.clone(),
                    entry.id.clone(),
                ))),
        );
        let item: Element<'a, Message> = container(line)
            .padding(Padding {
                top: 2.0,
                right: 3.0,
                bottom: 2.0,
                left: 8.0,
            })
            .width(Length::Fill)
            .style(widgets::tile(t))
            .into();
        s.tip(item, entry.text.clone())
    });
    section.body(column(lines).spacing(4).width(Length::Fill))
}

// ---- Recent activity ----------------------------------------------------------------------

/// How many events the section keeps: a glance back, not a log.
const ACTIVITY_SHOWN: usize = 8;

/// The mark of an event kind (`.insp-event__dot--*`): the state glyph of
/// what happened, drawn at rest since it is in the past. Anything else is a
/// quiet dot.
fn dot<'a>(t: &Theme, kind: &str) -> Element<'a, Message> {
    let shown = match kind {
        "turn-start" => GlyphState::Working,
        "permission" => GlyphState::Waiting,
        "turn-end" => GlyphState::Yours,
        "dropped" => GlyphState::Failed,
        "exit" => GlyphState::Exited,
        _ => GlyphState::Yours,
    };
    let mark = glyph::state(t, shown, 8.0).still();
    if matches!(
        kind,
        "turn-start" | "permission" | "turn-end" | "dropped" | "exit"
    ) {
        mark.into()
    } else {
        mark.tint(t.palette.text_muted).into()
    }
}

pub(super) fn activity<'a>(
    s: &Scene<'a>,
    events: &'a [PaneEvent],
    nothing: String,
) -> Section<'a, Message> {
    let (t, tr) = (s.theme(), s.tr());
    let c = &t.palette;
    let section = s.section(SectionId::Activity, Key::InspectorSectionActivity);
    if events.is_empty() {
        return section.body(empty_note(t, nothing));
    }
    let lines = events.iter().take(ACTIVITY_SHOWN).map(|event| {
        let at = sources::at_ms(event.at);
        let time = s.tip(
            container(
                ink(format::clock_time(at), type_scale::LABEL.size, c.text_muted)
                    .font(fonts::mono()),
            )
            .width(34.0),
            format::full_date(at, s.locale()),
        );
        // The dot sits on the first line, whatever the text wraps to.
        let marker = container(dot(t, event.detail.kind())).padding(Padding {
            top: 3.0,
            ..Padding::ZERO
        });
        // At most two lines, then cut.
        let words = container(
            text::Text::new(panes_logic::pane_event_text(&event.detail, tr))
                .size(type_scale::CAPTION.size)
                .color(t.palette.text_secondary)
                .line_height(type_scale::CAPTION.relative_line_height())
                .width(Length::Fill),
        )
        .max_height(2.0 * type_scale::CAPTION.line_height)
        .clip(true)
        .width(Length::Fill);
        row![time, marker, words]
            .spacing(space::MD)
            .align_y(Alignment::Start)
            .into()
    });
    section.body(column(lines).spacing(space::SM).width(Length::Fill))
}

// ---- The clock ----------------------------------------------------------------------------

/// Whether something on screen in window `w` counts time, so that the
/// half-minute clock is worth running: a start time or a cost "over" it, or a
/// task in `doing`, in a section that is not folded.
pub(super) fn counts_time(m: &Model, w: window::Id) -> bool {
    if !super::shown_in(m, w) {
        return false;
    }
    let Some(ws) = m.window_workspace(w) else {
        return false;
    };
    let Some((pane, _)) = sources::focused_pane(m, ws) else {
        return false;
    };
    let folded = |id: SectionId| {
        m.settings()
            .inspector_collapsed
            .iter()
            .any(|folded| folded == id.as_str())
    };
    let insight = sources::insight(m, &pane.id);
    let agent = pane.agent != AgentId::Shell;
    let session = agent && !folded(SectionId::Session) && insight.session_since.is_some();
    let cost = agent
        && !folded(SectionId::Context)
        && insight.cost_usd.is_some()
        && insight.session_since.is_some();
    let doing = !folded(SectionId::Task)
        && sources::task_for_pane(m, ws, &pane.id)
            .is_some_and(|card| !card.in_review() && card.doing_since.is_some());
    session || cost || doing
}

/// The state word and colour of a header, from the pane's activity.
pub(super) fn state_word(tr: Translator, state: AgentState) -> String {
    tr.t(match state {
        AgentState::Working => Key::PaneStateWorking,
        AgentState::Waiting => Key::PaneStateWaiting,
        AgentState::Yours => Key::PaneStateYours,
        AgentState::Idle => Key::PaneStateIdle,
    })
    .into_owned()
}
