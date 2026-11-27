//! A pane's header: one 40 px line of status, and the drag handle for the
//! whole pane (`docs/teskad-brand/docs/ecrans.md`, §1).
//!
//! From the left: the state's glyph, the name, the role, the account when the
//! pane runs on its own, the worker's task, the branch; from the right: the
//! alerts, the context gauge, the « Details » button and the actions. What is
//! not on the line is in the Details popover: the agent CLI (with its beta
//! badge and side channel), the shell, the session, and the state in words.
//!
//! What it says about the agent (its activity, whether the mod reports it,
//! the queue, the context gauge, the attention badge, an overlap, the git
//! branch) is read from P4's `panes_logic::Live`; what it says about the
//! shell and the launch is this package's.
//!
//! The agent is named with its « beta » while Hangar labels its support so
//! (`accounts::BETA`); an agent Hangar reaches through a server of the
//! pane's own (Codex, D5) also says whether Hangar is attached to it, and on
//! the floor why not, in the words of the reason its spawn gave.
//!
//! A narrowing pane drops its details in a fixed order, so nothing ever wraps
//! or overlaps, down to eight panes beside both side panels in the smallest
//! window ([`Fit::of`], over the header's content width). The part left of
//! the alerts is clipped: the actions are never pushed off the line.

use hangar_core::pty::FloorReason;
use hangar_i18n::{Key, Translator, args};
use hangar_theme::{AgentState, Theme, style, tokens};
use hangar_ui::badge::{self, Tone};
use hangar_ui::{Icon, button, focus, glyph, icon, menu, popover, tip};
use iced::widget::{Id, Space, column, container, responsive, row, rule, text};
use iced::{Alignment, Background, Border, Element, Length, Padding};

use crate::ctx::Ctx;
use crate::message::Message;
use crate::pane_menu::Folded;
use crate::panes::controller::{PaneRt, Side, folder_of};
use crate::panes::gauge;
use crate::panes::label::fit_end;
use crate::panes::status::{shell_short_name, state_key, ui_glyph};
use crate::panes::{Msg, Picker};
use crate::panes_logic::agents::is_resumable;
use crate::panes_logic::{PaneBranch, short_session_id};
use crate::state::live::PaneStatus;
use crate::state::types::{AGENTS, AgentId, Pane, PinEdge, ShellInfo, Workspace};
use crate::state::{AgentActivity, GlyphState, WsId};

/// The header's height.
pub const HEIGHT: f32 = tokens::scale::size::CONTROL_LG;
/// The header's padding, left and right: what its content box loses.
const PADDING_LEFT: f32 = 12.0;
const PADDING_RIGHT: f32 = 8.0;
pub const PADDING_X: f32 = PADDING_LEFT + PADDING_RIGHT;
/// The gap between two things on the line.
const GAP: f32 = 8.0;
/// The widest the branch's name gets before it is cut.
const BRANCH_MAX: f32 = 112.0;
/// The size of the branch's name, and of the node's.
const MONO_SIZE: f32 = 11.0;
/// The side of the state's glyph.
const STATE_SIDE: f32 = 12.0;
/// The Details button's height.
const DETAILS_HEIGHT: f32 = 22.0;
/// The pin of a pinned pane: a button as small as the header's chips, its
/// glyph the size of the queue's tray.
const PIN_SIDE: f32 = 18.0;
const PIN_ICON: f32 = 11.0;

/// What a header of a given width shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fit {
    /// The account badge of a pane on an account of its own.
    pub account: bool,
    /// The worker's task.
    pub task: bool,
    /// The context gauge's bar beside its figure.
    pub gauge_bar: bool,
    pub branch: bool,
    pub diff: bool,
    /// The word on the Details button, else its chevron alone.
    pub details_word: bool,
    /// The context gauge's figure.
    pub gauge: bool,
    pub commands: bool,
    pub zoom: bool,
    /// The node a worker was sent to.
    pub node: bool,
    /// The delivery chip's word in its short form.
    pub short_states: bool,
    /// Actions 22 px wide instead of 26.
    pub narrow_actions: bool,
    /// The Details button.
    pub details: bool,
    pub queue: bool,
    /// The delivery chip.
    pub delivery: bool,
}

impl Fit {
    /// What a content box `width` logical pixels wide has room for.
    pub fn of(width: f32) -> Fit {
        let at_least = |limit: f32| width > limit;
        Fit {
            account: at_least(500.0),
            task: at_least(500.0),
            gauge_bar: at_least(470.0),
            branch: at_least(430.0),
            diff: at_least(408.0),
            details_word: at_least(360.0),
            gauge: at_least(340.0),
            commands: at_least(330.0),
            zoom: at_least(300.0),
            node: at_least(300.0),
            short_states: !at_least(504.0),
            narrow_actions: !at_least(220.0),
            details: at_least(150.0),
            queue: at_least(144.0),
            delivery: at_least(144.0),
        }
    }

    /// The header buttons this width folded away, which the "…" menu takes
    /// in (`toggleMore`'s `folded`).
    pub fn folded(self) -> Folded {
        Folded {
            commands: !self.commands,
            diff: !self.diff,
            zoom: !self.zoom,
        }
    }
}

/// What the header is drawn with.
pub struct Head<'a> {
    pub ws: &'a Workspace,
    pub pane: &'a Pane,
    pub rt: &'a PaneRt,
    pub focused: bool,
    pub zoomed: bool,
    pub can_close: bool,
    /// Zoom means something: there is more than one pane.
    pub can_zoom: bool,
    pub width: f32,
    /// Its place in the workspace's reading order, from 0.
    pub index: usize,
    pub shell: Option<&'a ShellInfo>,
    pub picker: Option<Picker>,
    /// The Details popover is open.
    pub details: bool,
}

fn wrap(msg: Msg) -> Message {
    Message::Panes(msg)
}

/// The state word's colour, the state's own (`.pane__state--*`).
fn state_tone(glyph: GlyphState) -> AgentState {
    match glyph {
        GlyphState::Working => AgentState::Working,
        GlyphState::Waiting => AgentState::Waiting,
        GlyphState::Yours => AgentState::Yours,
        _ => AgentState::Idle,
    }
}

/// The name's tooltip: the shortcut that jumps to the pane, for the first
/// nine in reading order while `pane.focusN` is bound, else the pane's name
/// (`jumpKeys` of TerminalPane.tsx).
fn name_tip(ctx: Ctx<'_>, pane: &Pane, index: usize) -> String {
    let keys = crate::commands::run::FOCUS
        .get(index)
        .map_or("", |id| ctx.keymap.label(*id));
    if keys.is_empty() {
        ctx.i18n
            .t_with(Key::PaneLabel, &args!("name" => pane.name.as_str()))
            .into_owned()
    } else {
        ctx.i18n
            .t_with(Key::PaneJump, &args!("keys" => keys))
            .into_owned()
    }
}

/// The header's surface: `surface`, its top corners following the pane's
/// card; the hairline under it is [`frame`]'s.
fn surface(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.surface)),
        text_color: Some(t.palette.text),
        border: Border {
            radius: iced::border::top(crate::panes::grid::CARD_RADIUS - 1.0),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// A pane's header around its line, a terminal's or a view's: [`HEIGHT`]
/// tall, its last pixel the `border` hairline under it.
pub fn frame<'a>(t: &'a Theme, bar: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    let line = container(bar)
        .padding(Padding {
            top: 0.0,
            right: PADDING_RIGHT,
            bottom: 0.0,
            left: PADDING_LEFT,
        })
        .height(HEIGHT - 1.0)
        .width(Length::Fill)
        .align_y(Alignment::Center)
        .clip(true)
        .style(surface(t));
    column![
        line,
        rule::horizontal(1).style(move |_| rule::Style {
            color: t.palette.border,
            radius: 0.0.into(),
            fill_mode: rule::FillMode::Full,
            snap: true,
        })
    ]
    .into()
}

pub fn view<'a>(ctx: Ctx<'a>, head: Head<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let c = &t.colors;
    let rt = head.rt;
    let pane = head.pane;
    let logic = &ctx.model.live.panes_logic;
    let content_width = head.width - PADDING_X;
    let fit = Fit::of(content_width);

    let state = StateWords::of(ctx, pane, rt);
    let status = state.status;
    let reports = state.reports;
    let insight = logic.insight(&pane.id);

    // Who the pane is, from the left. This part gives way: it is clipped.
    let mut lead = row![].spacing(GAP).align_y(Alignment::Center);
    lead = lead.push(tip::bottom(
        t,
        container(glyph::state(t, ui_glyph(state.glyph), STATE_SIDE - 3.0)).center_x(STATE_SIDE),
        state.title(),
    ));

    let name_tip = name_tip(ctx, pane, head.index);
    let focused = head.focused;
    lead = lead.push(tip::bottom(
        t,
        text(pane.name.as_str())
            .size(tokens::font_size::UI)
            .font(hangar_theme::fonts::ui_semibold())
            .wrapping(text::Wrapping::None)
            .style(move |theme| {
                if focused {
                    style::text::primary(t)(theme)
                } else {
                    style::text::secondary(t)(theme)
                }
            }),
        name_tip,
    ));

    // Pinned to an edge of the grid: the pin says which, and lets it go.
    if let Some(pin) = head.ws.pin_of(&pane.id) {
        lead = lead.push(pin_glyph(ctx, &head.ws.id, &pane.id, pin.edge));
    }

    // A worker sent to a node from this workspace: where it runs.
    if fit.node
        && let Some(node) = crate::nodes::away::pane_node(pane)
    {
        lead = lead.push(node_label(ctx, &node));
    }

    lead = lead.push(crate::orchestration_ui::pane_badge_in(
        ctx,
        &head.ws.id,
        &pane.id,
        content_width,
    ));
    if fit.account {
        lead = lead.push(crate::accounts::pane_badge(ctx, &head.ws.id, &pane.id));
    }

    // The worker's delivery, while Hangar merges, checks, reviews and lands
    // it, then how it went for a few seconds. Its chip takes the branch's
    // room: the branch is the one being delivered, and the header has no
    // width for both beside the worker's badge.
    let now = crate::panes_logic::wall_clock_ms();
    let delivery = pane
        .task_id
        .as_deref()
        .filter(|task| fit.delivery && !task.is_empty())
        .and_then(|task| Some((task, logic.delivery_stage(task, now)?)));
    // A worker's task takes what is left of the line.
    let task = fit
        .task
        .then(|| crate::orchestration_ui::worker_task(ctx, &head.ws.id, &pane.id))
        .flatten();
    let spread = task.is_some();
    if let Some((title, hint)) = task {
        lead = lead.push(task_label(t, title, hint));
    }
    if delivery.is_none()
        && fit.branch
        && let Some(branch) = logic.pane_branch(&folder_of(head.ws, pane))
    {
        lead = lead.push(branch_label(ctx, branch));
    }
    if !spread {
        lead = lead.push(Space::new().width(Length::Fill));
    }

    let mut bar = row![container(lead).width(Length::Fill).clip(true)]
        .spacing(GAP)
        .align_y(Alignment::Center);

    if let Some((task, stage)) = delivery {
        bar = bar.push(crate::chat_view::tasks::chip::delivery_chip(
            ctx,
            &head.ws.cwd,
            task,
            stage,
            now,
            true,
            fit.short_states,
        ));
    }

    // Prompts held for the agent; a click drops them.
    if fit.queue && !insight.queue.is_empty() {
        let n = insight.queue.len();
        let words = ctx.i18n.tn(Key::PaneQueue, n, &[]).into_owned();
        let content = row![
            icon(Icon::Tray, 11.0),
            text(n.to_string())
                .size(tokens::font_size::LABEL)
                .font(hangar_theme::fonts::mono()),
        ]
        .spacing(3)
        .align_y(Alignment::Center);
        bar = bar.push(tip::bottom(
            t,
            iced::widget::button(content)
                .padding(Padding::from([0.0, 6.0]))
                .height(18)
                .style(style::button::toggle(t, true))
                .on_press(wrap(Msg::CancelQueue(pane.id.clone()))),
            words,
        ));
    }

    // Another pane just edited a file this one had edited.
    if let Some(other) = logic.conflict(&pane.id) {
        let words = ctx
            .i18n
            .t_with(Key::PaneConflict, &args!("name" => other))
            .into_owned();
        bar = bar.push(tip::bottom(
            t,
            icon(Icon::Alert, 13.0).color(c.warning),
            words,
        ));
    }

    if missing(ctx, pane.agent) {
        let words = ctx
            .i18n
            .t_with(Key::PaneNotOnPath, &args!("agent" => pane.agent.as_str()))
            .into_owned();
        bar = bar.push(tip::bottom(
            t,
            icon(Icon::Alert, 13.0).color(c.warning),
            words,
        ));
    }
    // On the floor: the line warns, and Details says why.
    if let Some(side @ Side::Floor(_)) = rt.side.as_ref()
        && shows_side(pane.agent)
    {
        let (_, words) = side_words(ctx.i18n, pane.agent, side, state.bridged);
        bar = bar.push(tip::bottom(
            t,
            icon(Icon::Alert, 13.0).color(c.warning),
            words,
        ));
    }
    // An agent pane says the same through its state; a shell keeps the bell
    // for the build that finished while the person was away.
    if logic.needs_attention(&pane.id) && !reports {
        bar = bar.push(tip::bottom(
            t,
            icon(Icon::Bell, 13.0).color(c.state_yours_fill),
            ctx.t(Key::PaneAttention).into_owned(),
        ));
    }
    // A shell that died says how to bring it back, in so many words.
    if status == PaneStatus::Exited {
        bar = bar.push(tip::bottom(
            t,
            iced::widget::button(
                row![
                    icon(Icon::Restart, 11.0),
                    text(ctx.t(Key::PaneRestartNow))
                        .size(tokens::font_size::CAPTION_LG)
                        .font(hangar_theme::fonts::ui_medium()),
                ]
                .spacing(tokens::space::XS)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([0.0, 8.0]))
            .height(20)
            .style(style::button::secondary(t))
            .on_press(wrap(Msg::Restart(head.ws.id.clone(), pane.id.clone()))),
            ctx.t(Key::PaneRestart).into_owned(),
        ));
    }

    if fit.gauge
        && let Some(reading) = &insight.context
    {
        bar = bar.push(gauge::view(ctx, reading, pane.agent, fit.gauge_bar));
    }
    if fit.details {
        bar = bar.push(details(ctx, &head, fit, &state));
    }
    bar = bar.push(actions(ctx, &head, fit));

    frame(t, bar)
}

/// How the pane's state is said: the shell's life until it runs, then the
/// agent's state, or nothing more than a resting ring for a plain shell,
/// whose builds and tests are not an agent at work.
struct StateWords {
    status: PaneStatus,
    /// The agent reports an activity.
    reports: bool,
    /// Its mod does, rather than Hangar guessing.
    bridged: bool,
    glyph: GlyphState,
    label: String,
    /// Where an activity came from.
    source: Option<String>,
}

impl StateWords {
    fn of(ctx: Ctx<'_>, pane: &Pane, rt: &PaneRt) -> StateWords {
        let logic = &ctx.model.live.panes_logic;
        let status = logic.pane_status(&pane.id).unwrap_or(rt.status);
        let published = logic.pane_activity(&pane.id);
        let bridged = published.is_some_and(|p| p.bridged);
        let activity = published.map(|p| p.activity);
        let reports = activity.is_some();
        let source = (status == PaneStatus::Running && reports).then(|| {
            ctx.t(if bridged {
                Key::PaneStateReported
            } else {
                Key::PaneStateGuessed
            })
            .into_owned()
        });
        StateWords {
            status,
            reports,
            bridged,
            glyph: crate::panes_logic::state_glyph_state(ctx.model, &pane.id),
            label: ctx.t(state_key(status, activity)).into_owned(),
            source,
        }
    }

    /// The state in words, and where it came from.
    fn title(&self) -> String {
        match &self.source {
            Some(source) => format!("{} · {source}", self.label),
            None => self.label.clone(),
        }
    }
}

/// The id of the Details button of `pane`'s header.
pub fn details_id(pane: &str) -> Id {
    Id::from(format!("pane-details-{pane}"))
}

/// The look of the Details button: a `border_strong` outline, the accent's
/// while its popover is open.
fn details_style(
    t: &Theme,
    open: bool,
) -> impl Fn(&iced::Theme, iced::widget::button::Status) -> iced::widget::button::Style + '_ {
    use iced::widget::button::{Status, Style};
    move |_, status| {
        let p = &t.palette;
        let background = match status {
            Status::Hovered => Some(Background::Color(p.hover_overlay)),
            Status::Pressed => Some(Background::Color(p.selection_bg)),
            _ => open.then_some(Background::Color(p.selection_bg)),
        };
        Style {
            background,
            text_color: if open { p.text } else { p.text_secondary },
            border: Border {
                color: if open { p.accent } else { p.border_strong },
                width: 1.0,
                radius: tokens::scale::radius::SM.into(),
            },
            ..Style::default()
        }
    }
}

/// « Details ▾ » and the popover under it.
fn details<'a>(
    ctx: Ctx<'a>,
    head: &Head<'a>,
    fit: Fit,
    state: &StateWords,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let pane = head.pane;
    let toggle = wrap(Msg::ToggleDetails(pane.id.clone()));
    let mut content = row![].spacing(4).align_y(Alignment::Center);
    if fit.details_word {
        content = content.push(
            text(ctx.t(Key::PaneDetails))
                .size(MONO_SIZE)
                .font(hangar_theme::fonts::ui_medium())
                .wrapping(text::Wrapping::None),
        );
    }
    content = content.push(icon(Icon::ChevronDown, 10.0));
    let button = iced::widget::button(container(content).center_y(Length::Fill))
        .padding(Padding::from([
            0.0,
            if fit.details_word { 8.0 } else { 4.0 },
        ]))
        .height(DETAILS_HEIGHT)
        .style(details_style(t, head.details))
        .on_press(toggle.clone());
    let button = container(
        focus::ring(t, button)
            .radius(tokens::scale::radius::SM)
            .on_activate(toggle),
    )
    .id(details_id(&pane.id));
    let anchor = tip::bottom(t, button, ctx.t(Key::PaneDetailsTitle).into_owned());
    let content = head.details.then(|| {
        Element::from(
            popover::popover(t, details_content(ctx, head, state))
                .width(popover::WIDTH)
                .padding(tokens::scale::space::S3),
        )
    });
    // Tab walks the popover's controls; no field takes the keyboard from the
    // terminal just because it opened.
    hangar_ui::layer::dropdown(anchor, content)
        .tab_inside()
        .align_end()
        .on_dismiss(wrap(Msg::CloseDetails))
        .into()
}

/// What the line does not say: the agent CLI, the shell, the session, and
/// the state in words.
fn details_content<'a>(ctx: Ctx<'a>, head: &Head<'a>, state: &StateWords) -> Element<'a, Message> {
    let t = ctx.theme;
    let pane = head.pane;
    let said = |key: Key| ctx.t(key).into_owned();
    let mut lines = column![
        text(said(Key::PaneDetailsTitle))
            .size(MONO_SIZE)
            .style(style::text::muted(t))
    ]
    .spacing(tokens::space::MD);

    let mut agent = row![].spacing(tokens::space::SM).align_y(Alignment::Center);
    if let Some(beta) = beta_badge(ctx, pane.agent) {
        agent = agent.push(beta);
    }
    agent = agent.push(agent_picker(ctx, head));
    lines = lines.push(popover::row(t, said(Key::PaneDetailsAgent), agent));
    if let Some(side) = side_chip(ctx, pane.agent, head.rt.side.as_ref(), state.bridged) {
        lines = lines.push(row![Space::new().width(Length::Fill), side]);
    }
    lines = lines.push(popover::row(
        t,
        said(Key::PaneShell),
        shell_picker(ctx, head),
    ));
    if is_resumable(pane.agent) {
        lines = lines.push(popover::row(
            t,
            said(Key::PaneDetailsSession),
            session_value(ctx, head),
        ));
    }
    let mut value = row![
        glyph::state(t, ui_glyph(state.glyph), 8.0),
        text(state.label.clone())
            .size(tokens::font_size::LABEL)
            .style(style::text::state(t, state_tone(state.glyph))),
    ]
    .spacing(tokens::space::SM)
    .align_y(Alignment::Center);
    if let Some(source) = &state.source {
        value = value.push(tip::over(t, icon(Icon::Info, 12.0), source.clone()));
    }
    lines
        .push(popover::row(t, said(Key::PaneDetailsState), value))
        .into()
}

/// A worker's task, cut to the room it has, whole in its tip.
fn task_label<'a>(t: &'a Theme, title: &'a str, hint: String) -> Element<'a, Message> {
    let words = responsive(move |bounds| {
        let font = hangar_theme::fonts::ui();
        container(
            text(fit_end(
                title,
                bounds.width,
                tokens::font_size::CAPTION,
                font,
            ))
            .size(tokens::font_size::CAPTION)
            .font(font)
            .wrapping(text::Wrapping::None)
            .style(style::text::secondary(t)),
        )
        .center_y(Length::Fill)
        .into()
    });
    tip::bottom(
        t,
        container(words).width(Length::Fill).height(18).clip(true),
        hint,
    )
}

/// The id of the pin a pinned pane's header shows (`pin_glyph`).
pub fn pin_glyph_id(pane: &str) -> Id {
    Id::from(format!("pane-pin-{pane}"))
}

/// The pin beside a pinned pane's name, a terminal's or a view's: always
/// there, pressed in, its tip saying which edge it keeps to and that a
/// click lets it go, which is what the click does.
pub fn pin_glyph<'a>(ctx: Ctx<'a>, ws: &WsId, pane: &str, edge: PinEdge) -> Element<'a, Message> {
    let words = ctx.t(match edge {
        PinEdge::Left => Key::PanePinnedLeft,
        PinEdge::Right => Key::PanePinnedRight,
        PinEdge::Top => Key::PanePinnedTop,
        PinEdge::Bottom => Key::PanePinnedBottom,
    });
    container(
        button::toggle_button(ctx.theme, Icon::Pin, true)
            .side(PIN_SIDE)
            .icon_size(PIN_ICON)
            .tip(words.into_owned())
            .on_press(wrap(Msg::Pin(ws.clone(), pane.to_owned(), false))),
    )
    .id(pin_glyph_id(pane))
    .into()
}

/// The machine a pane of a local workspace runs on: its icon and its
/// label, in the look of its link's state.
fn node_label<'a>(ctx: Ctx<'a>, node: &str) -> Element<'a, Message> {
    use crate::nodes::presence;
    let t = ctx.theme;
    let label = crate::nodes::label(ctx.model, node);
    let look = presence::look_of(ctx.model, node, ctx.now);
    let words = presence::tip_of(ctx.model, ctx.i18n, node, ctx.now);
    let content = row![
        icon(look.icon, 11.0).color(look.tone.color(t)),
        text(fit_end(
            &label,
            BRANCH_MAX,
            MONO_SIZE,
            hangar_theme::fonts::ui()
        ))
        .size(MONO_SIZE)
        .wrapping(text::Wrapping::None)
        .style(style::text::muted(t)),
    ]
    .spacing(3)
    .align_y(Alignment::Center);
    tip::bottom(t, content, words)
}

/// The git branch of the pane's folder; a detached head shows its short sha,
/// marked (`PaneBranchLabel`).
fn branch_label<'a>(ctx: Ctx<'a>, branch: &'a PaneBranch) -> Element<'a, Message> {
    let t = ctx.theme;
    let (label, words, detached) = match branch {
        PaneBranch::Branch {
            name,
            linked_worktree,
        } => (
            name,
            ctx.i18n.t_with(
                if *linked_worktree {
                    Key::PaneBranchBranchWorktree
                } else {
                    Key::PaneBranchBranch
                },
                &args!("branch" => name.as_str()),
            ),
            false,
        ),
        PaneBranch::Detached { head } => (
            head,
            ctx.i18n
                .t_with(Key::PaneBranchDetached, &args!("head" => head.as_str())),
            true,
        ),
    };
    let mono = hangar_theme::fonts::mono();
    let mut content = row![
        icon(Icon::Branch, 12.0).color(t.palette.text_muted),
        text(fit_end(label, BRANCH_MAX, MONO_SIZE, mono))
            .size(MONO_SIZE)
            .font(mono)
            .wrapping(text::Wrapping::None)
            .style(style::text::muted(t)),
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    if detached {
        content = content.push(
            text(ctx.t(Key::PaneBranchDetachedMark))
                .size(MONO_SIZE)
                .font(iced::Font {
                    style: iced::font::Style::Italic,
                    ..hangar_theme::fonts::ui()
                })
                .style(style::text::muted(t)),
        );
    }
    tip::bottom(t, content, words.into_owned())
}

/// The ⚡, diff, zoom, "…" and close buttons, on every pane.
fn actions<'a>(ctx: Ctx<'a>, head: &Head<'a>, fit: Fit) -> Element<'a, Message> {
    let t = ctx.theme;
    let pane = head.pane;
    let ws = &head.ws.id;
    let side = action_side(fit.narrow_actions);
    let mut group = row![].spacing(1).align_y(Alignment::Center);
    if fit.commands {
        group = group.push(crate::saved_commands::menu_view(
            &ctx.pkg.saved_commands,
            ctx,
            &pane.id,
        ));
    }
    if fit.diff {
        group = group.push(
            button::icon_button(t, Icon::Diff)
                .side(side)
                .tip(ctx.t(Key::DiffOpen).into_owned())
                .on_press(wrap(Msg::Diff(ws.clone(), pane.id.clone()))),
        );
    }
    if fit.zoom && head.can_zoom {
        let words = ctx
            .t(if head.zoomed {
                Key::ZoomRestore
            } else {
                Key::PaneZoomIn
            })
            .into_owned();
        group = group.push(
            button::toggle_button(
                t,
                if head.zoomed {
                    Icon::Collapse
                } else {
                    Icon::Expand
                },
                head.zoomed,
            )
            .side(side)
            .tip(words)
            .on_press(wrap(Msg::ToggleZoom(ws.clone(), pane.id.clone()))),
        );
    }
    group = group.push(crate::pane_menu::view_folded(
        &ctx.pkg.pane_menu,
        ctx,
        &pane.id,
        fit.folded(),
    ));
    let close_words = ctx
        .t(if head.can_close {
            Key::PaneClose
        } else {
            Key::PaneCloseLast
        })
        .into_owned();
    group = group.push(
        button::icon_button(t, Icon::Close)
            .side(side)
            .danger()
            .tip(close_words)
            .on_press_maybe(
                head.can_close
                    .then(|| wrap(Msg::Close(ws.clone(), pane.id.clone()))),
            ),
    );
    group.into()
}

/// The side of a header's action button, a terminal's or a view's.
pub fn action_side(narrow: bool) -> f32 {
    if narrow {
        22.0
    } else {
        button::Side::Xs.pixels()
    }
}

/// A picker that reads as text: its label and a chevron, a menu under it.
fn picker_button<'a>(
    ctx: Ctx<'a>,
    label: String,
    chevron: bool,
    mono: bool,
    on_press: Msg,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let mut content = row![
        text(label)
            .size(if mono {
                tokens::font_size::LABEL
            } else {
                tokens::font_size::UI
            })
            .font(if mono {
                hangar_theme::fonts::mono()
            } else {
                hangar_theme::fonts::ui()
            })
            .wrapping(text::Wrapping::None)
    ]
    .spacing(3)
    .align_y(Alignment::Center);
    if chevron {
        content = content.push(icon(Icon::ChevronDown, 10.0));
    }
    let message = wrap(on_press);
    let button = iced::widget::button(container(content).center_y(Length::Fill))
        .padding(Padding::from([0.0, 4.0]))
        .height(20)
        .style(style::button::ghost(t))
        .on_press(message.clone());
    focus::ring(t, button)
        .radius(tokens::scale::radius::SM)
        .on_activate(message)
        .into()
}

fn agent_picker<'a>(ctx: Ctx<'a>, head: &Head<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let pane = head.pane;
    let anchor = picker_button(
        ctx,
        pane.agent.info().label.to_owned(),
        true,
        false,
        Msg::OpenPicker(pane.id.clone(), Picker::Agent),
    );
    let open = head.picker == Some(Picker::Agent);
    let entries = open.then(|| {
        AGENTS
            .iter()
            .map(|agent| {
                let mut label = agent.label.to_owned();
                if missing(ctx, agent.id) {
                    label.push_str(&ctx.t(Key::PaneMissing));
                }
                menu::item(label)
                    .checked(agent.id == pane.agent)
                    .on_select(wrap(Msg::SetAgent(
                        head.ws.id.clone(),
                        pane.id.clone(),
                        agent.id,
                    )))
                    .into()
            })
            .collect()
    });
    tip::bottom(
        t,
        menu::dropdown(t, anchor, entries).on_dismiss(wrap(Msg::ClosePicker)),
        ctx.t(Key::PaneAgent).into_owned(),
    )
}

/// Whether `agent` is an agent this machine was found not to have.
fn missing(ctx: Ctx<'_>, agent: AgentId) -> bool {
    agent
        .kind()
        .is_some_and(|kind| !crate::shell::setup::installed(ctx.model, kind))
}

/// The « beta » beside the name of an agent whose support Hangar labels so
/// until real users confirm it (`accounts::BETA`, docs/native/agents.md §7,
/// decision 5); `None` for any other, and for a shell.
fn beta_badge<'a>(ctx: Ctx<'a>, agent: AgentId) -> Option<Element<'a, Message>> {
    let kind = agent
        .kind()
        .filter(|kind| crate::accounts::BETA.contains(kind))?;
    let words = ctx
        .i18n
        .t_with(Key::PaneAgentBetaHint, &args!("agent" => kind.info().label))
        .into_owned();
    Some(tip::bottom(
        ctx.theme,
        badge::badge(ctx.theme, ctx.t(Key::PaneAgentBeta), Tone::Accent),
        words,
    ))
}

/// Whether the header says how `agent`'s side channel stands: for a kind
/// Hangar reaches through a server of the pane's own (`headless.app_server`,
/// Codex, D5), which can be missing for a reason the person can act on. A
/// Claude Code's mod says what it has to through the state's tip, reported
/// or guessed, as it always did.
pub fn shows_side(agent: AgentId) -> bool {
    agent
        .kind()
        .is_some_and(|kind| kind.info().caps.headless.app_server)
}

/// What the side channel's chip says, and its tip: attached, and whether its
/// client has heard from the agent yet (`bridged`); or not, and why.
pub fn side_words(tr: Translator, agent: AgentId, side: &Side, bridged: bool) -> (String, String) {
    let label = agent.kind().map_or("", |kind| kind.info().label);
    match side {
        Side::Attached => (
            tr.t(Key::PaneAttached).into_owned(),
            tr.t_with(
                if bridged {
                    Key::PaneAttachedHint
                } else {
                    Key::PaneAttachedQuietHint
                },
                &args!("agent" => label),
            )
            .into_owned(),
        ),
        Side::Off => (
            tr.t(Key::PaneUnattached).into_owned(),
            tr.t_with(
                Key::PaneUnattachedOffHint,
                &args!(
                    "setting" => tr.t(Key::SettingsCodexAttach),
                    "agent" => label,
                ),
            )
            .into_owned(),
        ),
        Side::Floor(reason) => (
            tr.t(Key::PaneUnattached).into_owned(),
            tr.t_with(
                Key::PaneUnattachedHint,
                &args!(
                    "agent" => label,
                    "reason" => floor_reason(tr, reason).as_str(),
                ),
            )
            .into_owned(),
        ),
    }
}

/// Why a pane that was to run attached runs the floor, in the person's
/// language: what `FloorReason` says in English for the log.
pub fn floor_reason(tr: Translator, reason: &FloorReason) -> String {
    let detail = |reason: String, detail: &str| {
        if detail.trim().is_empty() {
            return reason;
        }
        tr.t_with(
            Key::PaneFloorDetail,
            &args!("reason" => reason.as_str(), "detail" => detail.trim()),
        )
        .into_owned()
    };
    match reason {
        FloorReason::Unsupported => tr.t(Key::PaneFloorUnsupported).into_owned(),
        FloorReason::RuntimeFolder { detail: what } => {
            detail(tr.t(Key::PaneFloorFolder).into_owned(), what.as_str())
        }
        FloorReason::Untypable => tr.t(Key::PaneFloorUntypable).into_owned(),
        FloorReason::NotStarted {
            program,
            detail: what,
        } => detail(
            tr.t_with(
                Key::PaneFloorNotStarted,
                &args!("program" => program.as_str()),
            )
            .into_owned(),
            what.as_str(),
        ),
        FloorReason::Exited { code, said } => {
            let mut reason = tr.t(Key::PaneFloorExited).into_owned();
            if let Some(code) = code {
                reason = tr
                    .t_with(
                        Key::PaneFloorExitCode,
                        &args!("reason" => reason.as_str(), "code" => code.to_string()),
                    )
                    .into_owned();
            }
            detail(reason, said.as_str())
        }
        FloorReason::Timeout { seconds } => tr
            .t_with(
                Key::PaneFloorTimeout,
                &args!("seconds" => seconds.to_string()),
            )
            .into_owned(),
    }
}

/// The chip that says whether Hangar is attached to the pane's agent, for
/// an agent [`shows_side`] is true of, once its spawn said (`side`).
fn side_chip<'a>(
    ctx: Ctx<'a>,
    agent: AgentId,
    side: Option<&Side>,
    bridged: bool,
) -> Option<Element<'a, Message>> {
    let side = side.filter(|_| shows_side(agent))?;
    let t = ctx.theme;
    let (label, words) = side_words(ctx.i18n, agent, side, bridged);
    let (tone, lead) = match side {
        Side::Attached => (Tone::Neutral, Some(Icon::Link)),
        // The person's choice: nothing to warn of.
        Side::Off => (Tone::Neutral, None),
        Side::Floor(_) => (Tone::Warning, Some(Icon::Alert)),
    };
    Some(tip::bottom(
        t,
        badge::named(t, label, tone).icon(lead),
        words,
    ))
}

fn shell_picker<'a>(ctx: Ctx<'a>, head: &Head<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let pane = head.pane;
    let name = shell_short_name(head.shell).unwrap_or_else(|| ctx.t(Key::PaneShell).into_owned());
    let anchor = picker_button(
        ctx,
        name,
        false,
        true,
        Msg::OpenPicker(pane.id.clone(), Picker::Shell),
    );
    let open = head.picker == Some(Picker::Shell);
    let entries = open.then(|| {
        let inherited_name = head
            .shell
            .map_or_else(|| ctx.t(Key::PaneShell).into_owned(), |s| s.label.clone());
        let mut entries = vec![
            menu::item(
                ctx.i18n
                    .t_with(
                        Key::PaneInherited,
                        &args!("name" => inherited_name.as_str()),
                    )
                    .into_owned(),
            )
            .checked(pane.shell_id.is_none())
            .on_select(wrap(Msg::SetShell(
                head.ws.id.clone(),
                pane.id.clone(),
                None,
            )))
            .into(),
        ];
        entries.extend(ctx.model.ui.shells.iter().map(|shell| {
            menu::item(shell.label.clone())
                .checked(pane.shell_id.as_deref() == Some(shell.id.as_str()))
                .on_select(wrap(Msg::SetShell(
                    head.ws.id.clone(),
                    pane.id.clone(),
                    Some(shell.id.clone()),
                )))
                .into()
        }));
        entries
    });
    let words = match head.shell {
        Some(shell) => ctx
            .i18n
            .t_with(
                Key::PaneShellTitle,
                &args!("program" => shell.program.as_str()),
            )
            .into_owned(),
        None => ctx.t(Key::PaneShell).into_owned(),
    };
    tip::bottom(
        t,
        menu::dropdown(t, anchor, entries).on_dismiss(wrap(Msg::ClosePicker)),
        words,
    )
}

/// The session a resumable agent comes back on, or "new": a press opens the
/// sessions picker. A session's id can be copied.
fn session_value<'a>(ctx: Ctx<'a>, head: &Head<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let pane = head.pane;
    let (words, label) = match pane.session_id.as_deref() {
        Some(id) => (
            ctx.i18n
                .t_with(Key::PaneResuming, &args!("id" => id))
                .into_owned(),
            format!("{}…", short_session_id(id)),
        ),
        None => (
            ctx.t(Key::PaneNoSession).into_owned(),
            ctx.t(Key::PaneNewSession).into_owned(),
        ),
    };
    let pick = picker_button(
        ctx,
        label,
        false,
        true,
        Msg::Sessions(head.ws.id.clone(), pane.id.clone()),
    );
    let mut value = row![tip::over(t, pick, words)]
        .spacing(tokens::space::XS)
        .align_y(Alignment::Center);
    if let Some(id) = &pane.session_id {
        value = value.push(
            button::icon_button(t, Icon::Copy)
                .side(PIN_SIDE + 4.0)
                .icon_size(12.0)
                .tip(ctx.t(Key::PaneCopySession).into_owned())
                .on_press(wrap(Msg::CopySession(id.clone()))),
        );
    }
    value.into()
}

/// The activity's word for a pane out of sight (the zoom bar's chips).
pub fn activity_word(ctx: &Ctx<'_>, activity: AgentActivity) -> String {
    ctx.t(crate::panes::status::activity_key(activity))
        .into_owned()
}

#[cfg(test)]
mod tests {
    use hangar_i18n::Locale;

    use super::*;
    use crate::app::Packages;
    use crate::navigator::testing::{ctx, model_of, pane, simulate, workspace};
    use crate::state::Model;
    use crate::state::types::AppState;

    const WS: &str = "head-ws";

    /// A Codex pane and a Claude Code pane, mounted, both agents installed.
    fn two_agents() -> (Model, Packages) {
        let mut m = model_of(AppState {
            workspaces: vec![workspace(
                WS,
                "Shop",
                None,
                vec![
                    pane("cx", "Iris", AgentId::Codex),
                    pane("cl", "Ava", AgentId::Claude),
                ],
            )],
            active_workspace_id: Some(WS.into()),
            ..AppState::default()
        });
        m.ui.claude_installed = true;
        m.ui.codex_installed = true;
        let mut pkg = Packages::default();
        pkg.panes
            .mount_for_test(m.workspace(WS).expect("the workspace"));
        (m, pkg)
    }

    /// What `pane` says how it was started, as its spawn would.
    fn started(pkg: &mut Packages, pane: &str, side: Side) {
        let rt = pkg.panes.panes.get_mut(pane).expect("mounted");
        rt.status = PaneStatus::Running;
        rt.side = Some(side);
    }

    /// The header of `pane`, focused, as wide as a pane alone in the window.
    fn header_of<'a>(m: &'a Model, pkg: &'a Packages, pane: &str) -> Element<'a, Message> {
        let ws = m.workspace(WS).expect("the workspace");
        let entry = ws.panes.iter().find(|p| p.id == pane).expect("the pane");
        view(
            ctx(m, pkg),
            Head {
                ws,
                pane: entry,
                rt: &pkg.panes.panes[pane],
                focused: true,
                zoomed: false,
                can_close: true,
                can_zoom: true,
                width: 1000.0,
                index: 0,
                shell: None,
                picker: None,
                details: false,
            },
        )
    }

    /// Whether the line of `pane`'s header says `words`.
    fn says(m: &Model, pkg: &Packages, pane: &str, words: &str) -> bool {
        simulate(header_of(m, pkg, pane)).find(words).is_ok()
    }

    /// What the Details popover of `pane` holds, laid out on its own.
    fn details_of<'a>(m: &'a Model, pkg: &'a Packages, pane: &str) -> Element<'a, Message> {
        let ws = m.workspace(WS).expect("the workspace");
        let entry = ws.panes.iter().find(|p| p.id == pane).expect("the pane");
        let rt = &pkg.panes.panes[pane];
        let head = Head {
            ws,
            pane: entry,
            rt,
            focused: true,
            zoomed: false,
            can_close: true,
            can_zoom: true,
            width: 1000.0,
            index: 0,
            shell: None,
            picker: None,
            details: true,
        };
        let state = StateWords::of(ctx(m, pkg), entry, rt);
        iced::widget::container(details_content(ctx(m, pkg), &head, &state))
            .width(hangar_ui::popover::WIDTH)
            .into()
    }

    fn details_say(m: &Model, pkg: &Packages, pane: &str, words: &str) -> bool {
        simulate(details_of(m, pkg, pane)).find(words).is_ok()
    }

    /// A Codex pane's Details name its agent with its « beta » and say
    /// Hangar is attached to it; a Claude Code pane's have neither. None of
    /// it is on the header's one line.
    #[test]
    fn a_codex_panes_details_name_its_agent_in_beta_and_say_hangar_is_attached() {
        let (m, mut pkg) = two_agents();
        started(&mut pkg, "cx", Side::Attached);
        started(&mut pkg, "cl", Side::Attached);
        assert!(details_say(&m, &pkg, "cx", "Codex"));
        assert!(details_say(&m, &pkg, "cx", "beta"));
        assert!(details_say(&m, &pkg, "cx", "attached"));
        assert!(details_say(&m, &pkg, "cl", "Claude Code"));
        for word in ["beta", "attached", "not attached"] {
            assert!(!details_say(&m, &pkg, "cl", word), "{word}");
        }
        for word in ["Codex", "beta", "attached", "Shell"] {
            assert!(!says(&m, &pkg, "cx", word), "{word}");
        }
        assert!(says(&m, &pkg, "cx", "Iris"));
        assert!(says(&m, &pkg, "cx", "Details"));
    }

    /// On the floor, the chip says it is not attached; before its spawn
    /// has said, it says nothing either way.
    #[test]
    fn a_codex_pane_on_the_floor_says_it_is_not_attached() {
        let (m, mut pkg) = two_agents();
        assert!(!details_say(&m, &pkg, "cx", "attached"));
        assert!(!details_say(&m, &pkg, "cx", "not attached"));
        started(
            &mut pkg,
            "cx",
            Side::Floor(FloorReason::Timeout { seconds: 10 }),
        );
        assert!(details_say(&m, &pkg, "cx", "not attached"));
        assert!(!details_say(&m, &pkg, "cx", "attached"));
        started(&mut pkg, "cx", Side::Off);
        assert!(details_say(&m, &pkg, "cx", "not attached"));
        // A Claude Code without the mod keeps its Details as they were.
        started(&mut pkg, "cl", Side::Off);
        assert!(!details_say(&m, &pkg, "cl", "not attached"));
    }

    /// The Details popover holds what the line does not: the agent CLI, the
    /// shell, the session, which a press resumes another of, and the state.
    #[test]
    fn the_details_hold_the_agent_the_shell_the_session_and_the_state() {
        let (m, pkg) = two_agents();
        for word in ["Session details", "Agent CLI", "Shell", "Session", "State"] {
            assert!(details_say(&m, &pkg, "cl", word), "{word}");
        }
        let mut ui = simulate(details_of(&m, &pkg, "cl"));
        ui.click("new").expect("no session yet");
        ui.click("Claude Code").expect("the agent's picker");
        let sent: Vec<Message> = ui.into_messages().collect();
        assert!(
            matches!(
                sent.as_slice(),
                [
                    Message::Panes(Msg::Sessions(_, a)),
                    Message::Panes(Msg::OpenPicker(b, Picker::Agent)),
                ] if a == "cl" && b == "cl"
            ),
            "{sent:?}"
        );
    }

    /// The agent picker opens over the Details popover: picking a line there
    /// sends the choice, and the press is not one « outside » the popover,
    /// which would close it first.
    #[test]
    fn a_line_picked_in_the_agent_menu_over_the_details_does_not_dismiss_them() {
        let (m, pkg) = two_agents();
        let ws = m.workspace(WS).expect("the workspace");
        let entry = ws.panes.iter().find(|p| p.id == "cl").expect("the pane");
        let element = view(
            ctx(&m, &pkg),
            Head {
                ws,
                pane: entry,
                rt: &pkg.panes.panes["cl"],
                focused: true,
                zoomed: false,
                can_close: true,
                can_zoom: true,
                width: 600.0,
                index: 0,
                shell: None,
                picker: Some(Picker::Agent),
                details: true,
            },
        );
        let mut ui = simulate(element);
        // The simulator's selectors do not look into overlays: the menu's
        // first line, « Shell », by where it hangs, under the picker at the
        // right of the popover, itself under the Details button.
        let button = ui.find(details_id("cl")).expect("the button").bounds();
        let line = iced::Point::new(
            button.x + button.width - 62.0,
            button.y + button.height + 82.0,
        );
        ui.point_at(line);
        ui.simulate([iced::Event::Mouse(iced::mouse::Event::CursorMoved {
            position: line,
        })]);
        ui.simulate(iced_test::simulator::click());
        let sent: Vec<Message> = ui.into_messages().collect();
        assert!(
            sent.iter()
                .any(|msg| matches!(msg, Message::Panes(Msg::SetAgent(_, pane, AgentId::Shell)) if pane == "cl")),
            "{sent:?}"
        );
        assert!(
            !sent
                .iter()
                .any(|msg| matches!(msg, Message::Panes(Msg::CloseDetails))),
            "{sent:?}"
        );
    }

    /// A press on « Details » asks for the popover.
    #[test]
    fn the_details_button_toggles_the_popover() {
        let (m, pkg) = two_agents();
        let mut ui = simulate(header_of(&m, &pkg, "cl"));
        ui.click("Details").expect("the button");
        let sent: Vec<Message> = ui.into_messages().collect();
        assert!(
            matches!(
                sent.as_slice(),
                [Message::Panes(Msg::ToggleDetails(pane))] if pane == "cl"
            ),
            "{sent:?}"
        );
    }

    /// Each reason a pane runs the floor is said in the person's language,
    /// with what its spawn knew of it; the tip says the reason and how
    /// Hangar reads the agent meanwhile.
    #[test]
    fn the_floors_reason_is_said_in_english_and_in_french() {
        let reasons = [
            FloorReason::Unsupported,
            FloorReason::RuntimeFolder {
                detail: "Permission denied".into(),
            },
            FloorReason::Untypable,
            FloorReason::NotStarted {
                program: "codex".into(),
                detail: "No such file or directory".into(),
            },
            FloorReason::Exited {
                code: Some(2),
                said: "error: unexpected argument '--listen'".into(),
            },
            FloorReason::Exited {
                code: None,
                said: String::new(),
            },
            FloorReason::Timeout { seconds: 10 },
        ];
        let en = Translator::new(Locale::En);
        let fr = Translator::new(Locale::Fr);
        for reason in &reasons {
            let (english, french) = (floor_reason(en, reason), floor_reason(fr, reason));
            assert!(!english.is_empty() && !french.is_empty(), "{reason:?}");
            assert_ne!(english, french, "{reason:?}");
            let (label, tip) = side_words(en, AgentId::Codex, &Side::Floor(reason.clone()), false);
            assert_eq!(label, "not attached");
            assert!(tip.contains(&english) && tip.contains("Codex"), "{tip}");
        }
        assert_eq!(
            floor_reason(en, &reasons[4]),
            "its server stopped before it listened (exit 2): error: unexpected argument '--listen'"
        );
        assert_eq!(
            floor_reason(en, &reasons[6]),
            "its server did not listen within 10\u{a0}s"
        );
        assert!(floor_reason(fr, &reasons[3]).contains("«\u{202f}codex app-server\u{202f}»"));
        assert!(floor_reason(fr, &reasons[1]).ends_with("Permission denied"));

        // Attached, the tip says whether its client has heard from it yet;
        // off, it names the setting.
        let (label, heard) = side_words(en, AgentId::Codex, &Side::Attached, true);
        let (_, quiet) = side_words(en, AgentId::Codex, &Side::Attached, false);
        assert_eq!(label, "attached");
        assert_ne!(heard, quiet);
        let (_, off) = side_words(fr, AgentId::Codex, &Side::Off, false);
        assert!(off.contains(&*fr.t(Key::SettingsCodexAttach)), "{off}");
        assert_eq!(fr.t(Key::PaneAgentBeta), "bêta");
    }

    /// The side channel is said for an agent Hangar reaches through a server
    /// of the pane's own, by its capabilities: not for a Claude Code, nor a
    /// shell.
    #[test]
    fn only_an_agent_with_a_server_of_its_own_says_how_it_is_attached() {
        assert!(shows_side(AgentId::Codex));
        assert!(!shows_side(AgentId::Claude));
        assert!(!shows_side(AgentId::Shell));
    }

    #[test]
    fn the_name_tells_the_key_that_jumps_to_the_first_nine_panes() {
        let m = crate::state::Model::default();
        let pkg = crate::app::Packages::default();
        let ctx = crate::navigator::testing::ctx(&m, &pkg);
        let pane = Pane {
            name: "Ava".into(),
            ..Pane::default()
        };
        let keys = ctx.keymap.label(crate::commands::CommandId::PaneFocus1);
        assert!(!keys.is_empty(), "pane.focus1 is bound by default");
        assert_eq!(name_tip(ctx, &pane, 0), format!("Focus with {keys}"));
        let ninth = ctx.keymap.label(crate::commands::CommandId::PaneFocus9);
        assert_eq!(name_tip(ctx, &pane, 8), format!("Focus with {ninth}"));
        // The tenth has no key of its own: its name.
        assert_eq!(name_tip(ctx, &pane, 9), "Pane Ava");
    }

    #[test]
    fn a_wide_header_shows_everything() {
        let fit = Fit::of(800.0);
        assert!(fit.account && fit.task && fit.branch && fit.gauge && fit.gauge_bar);
        assert!(fit.details && fit.details_word && fit.diff && fit.commands && fit.zoom);
        assert!(!fit.short_states && !fit.narrow_actions);
    }

    /// A pane of the mockups, half of a 1440 window, keeps the whole line.
    #[test]
    fn a_pane_of_the_mockup_keeps_the_gauge_the_branch_and_the_details_word() {
        let fit = Fit::of(583.0 - PADDING_X);
        assert!(fit.task && fit.branch && fit.gauge_bar && fit.details_word);
    }

    #[test]
    fn what_gives_way_goes_in_order() {
        // The task and the account first, then the gauge's bar, the branch.
        let fit = Fit::of(490.0);
        assert!(!fit.task && !fit.account && fit.gauge_bar && fit.branch);
        let fit = Fit::of(420.0);
        assert!(!fit.gauge_bar && !fit.branch && fit.diff && fit.gauge);
        // The diff button, the Details' word, the gauge, then the commands.
        let fit = Fit::of(335.0);
        assert!(!fit.diff && !fit.details_word && !fit.gauge && fit.commands);
        let fit = Fit::of(310.0);
        assert!(!fit.commands && fit.zoom);
        let fit = Fit::of(200.0);
        assert!(!fit.zoom && fit.details && fit.narrow_actions);
        let fit = Fit::of(140.0);
        assert!(!fit.details && !fit.queue && !fit.delivery);
    }

    #[test]
    fn what_the_header_folds_the_menu_takes() {
        assert_eq!(Fit::of(800.0).folded(), Folded::default());
        assert_eq!(
            Fit::of(360.0).folded(),
            Folded {
                commands: false,
                diff: true,
                zoom: false
            }
        );
        assert_eq!(
            Fit::of(240.0).folded(),
            Folded {
                commands: true,
                diff: true,
                zoom: true
            }
        );
    }
}
