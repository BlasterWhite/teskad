//! The conversation, drawn: a header with the orchestrator and what it does,
//! the messages newest at the bottom (the scroll anchored there), and the
//! composer, or in its place the card of the question or permission it waits
//! on.
//!
//! Laid out as a chat reads: the person's messages in bubbles on the right,
//! the agent's words open on the page, its calls quiet lines of the work log —
//! an icon for what they do, what they did, folded in runs in a block of
//! their own, Teskad's messages in an inset that carries its mark, and, while
//! it works, a line that says so at the bottom.
//!
//! Keyboard: every line that opens, every choice and every setting is in a
//! focus ring, so Tab reaches it and Space or Enter presses it.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use hangar_core::chat::{Answer, Body, CallResult, CallState, ChatItem, Question, Sender};
use hangar_core::git::{ChangedFile, Changes};
use hangar_i18n::{Key, args};
use hangar_theme::{AgentState, Theme, fonts, style, tokens};
use hangar_ui::glyph::GlyphState;
use hangar_ui::{Icon, button as ui_button, empty, focus, icon};
use iced::keyboard::{self, key::Named};
use iced::widget::text_editor::{Binding, KeyPress, Status};
use iced::widget::{
    Space, Stack, button, column, container, row, rule, scrollable, text, text_editor, text_input,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding};

use crate::ctx::Ctx;
use crate::message::Message;
use crate::mobile::chat_live::PLAN_TOOL;
use crate::mobile::contract::{ChatPending, ChatReason};
use crate::orchestration_ui::actions::can_create_orchestrator;
use crate::state::roles::orchestrator_of;
use crate::state::{AgentActivity, Pane, Workspace};

use super::calls::{self, Kind, Piece};
use super::dashed::Dashed;
use super::hangar;
use super::settled::settled;
use super::thread::{Outgoing, Sent};
use super::{Blocked, Changed, Chat, Msg, State, blocked, msg, plan_key, rich, titles};
use crate::orchestration::events::DeliveryOutcome;
use crate::orchestration::heard::{Heard, Range};

/// The widest the column of messages gets: lines longer than this read badly.
const COLUMN_WIDTH: f32 = 760.0;
/// How wide a bubble of the person's gets, at most.
const BUBBLE_WIDTH: f32 = 520.0;
/// The line heights of the mock-up: a bubble's 14 px on 21.
const BUBBLE_LINE: f32 = 1.5;
/// The header over the conversation.
const HEADER_HEIGHT: f32 = 44.0;
/// A block of calls' header line.
const BLOCK_HEAD_HEIGHT: f32 = 34.0;
/// The halo around a card that waits on the person.
const HALO: f32 = 3.0;
const HALO_ALPHA: f32 = 0.16;
/// The side of the round Send and Stop buttons, and of a setting's height.
const ROUND: f32 = 30.0;
const CHIP_HEIGHT: f32 = 26.0;
/// A state's glyph.
const STATE_GLYPH: f32 = 12.0;
/// The lines of a call's detail or result shown expanded, at most.
const DETAIL_LINES: usize = 40;
/// A call's icon, and how far its words sit from the row's edge.
const CALL_ICON: f32 = 13.0;
const CALL_INDENT: f32 = tokens::space::SM + CALL_ICON + tokens::space::MD;

/// `content`, a button when there is something to press, in the keyboard's
/// focus ring: Space and Enter press it too.
fn ringed<'a>(
    t: &'a Theme,
    content: button::Button<'a, Message>,
    press: Option<Message>,
    radius: f32,
) -> Element<'a, Message> {
    focus::ring(t, content.on_press_maybe(press.clone()))
        .radius(radius)
        .on_activate_maybe(press)
        .into()
}

/// The input's widget id.
pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("chat-view-conversation-input")
}

pub fn view<'a>(st: &'a State, ctx: Ctx<'a>, ws: &'a Workspace) -> Element<'a, Message> {
    let t = ctx.theme;
    let Some(pane) = orchestrator_of(ws) else {
        return no_orchestrator(ctx, ws);
    };
    let chat = st.chat(&ws.id).filter(|chat| chat.pane == pane.id);
    let Some(chat) = chat.filter(|chat| chat.loaded) else {
        return loading(t, ctx.t(Key::ChatConvLoading).into_owned());
    };
    let body = column![
        header(ctx, pane),
        rule::horizontal(tokens::BORDER_WIDTH).style(style::rule::sep(t)),
        messages(ctx, ws, pane, chat),
        composer(ctx, ws, pane, chat),
    ]
    .height(Length::Fill);
    container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::container::plain(t))
        .into()
}

/// The column, centred, no wider than reads well.
fn centred<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(container(content).max_width(COLUMN_WIDTH))
        .width(Length::Fill)
        .center_x(Length::Fill)
        .into()
}

/// Before a first read: the shape of the conversation to come, still, and
/// the arc and the words that say it loads.
fn loading<'a>(t: &'a Theme, words: String) -> Element<'a, Message> {
    let bar = |width: f32, height: f32| empty::skeleton(t, Length::Fixed(width), height);
    let skeleton = column![
        row![Space::new().width(Length::Fill), bar(280.0, 40.0)],
        column![bar(520.0, 14.0), bar(470.0, 14.0), bar(320.0, 14.0)].spacing(tokens::space::MD),
        bar(440.0, BLOCK_HEAD_HEIGHT),
        row![
            hangar_ui::glyph::state(t, GlyphState::Working, STATE_GLYPH),
            hangar_ui::text::muted(t, words),
        ]
        .spacing(tokens::space::MD)
        .align_y(Alignment::Center),
    ]
    .spacing(tokens::space::XXL);
    container(centred(container(skeleton).padding(Padding {
        top: 0.0,
        right: tokens::space::XXXL,
        bottom: tokens::space::XXXL,
        left: tokens::space::XXXL,
    })))
    .height(Length::Fill)
    .align_bottom(Length::Fill)
    .style(style::container::plain(t))
    .into()
}

fn no_orchestrator<'a>(ctx: Ctx<'a>, ws: &'a Workspace) -> Element<'a, Message> {
    let t = ctx.theme;
    // The agent « Start the orchestrator » would start (D11, D20).
    let agent = crate::orchestration_ui::agents::of(ctx.model, ws)
        .orchestrator
        .info()
        .label;
    let state = empty::empty(t, Icon::Orchestrator, ctx.t(Key::ChatConvEmptyTitle)).description(
        ctx.i18n
            .t_with(Key::ChatConvEmptyBody, &args!("agent" => agent))
            .into_owned(),
    );
    if can_create_orchestrator(ws) {
        state
            .action(
                ui_button::primary(t, ctx.t(Key::ChatConvEmptyStart))
                    .icon(Icon::Play)
                    .on_press(msg(Msg::Start(ws.id.clone()))),
            )
            .into()
    } else {
        state.description(ctx.t(Key::ChatConvEmptyCannot)).into()
    }
}

/// `color` at `alpha`.
fn tint(color: Color, alpha: f32) -> Color {
    Color {
        a: color.a * alpha,
        ..color
    }
}

/// The orchestrator, how it is doing, and on the right the model it runs and
/// how full its context is.
fn header<'a>(ctx: Ctx<'a>, pane: &'a Pane) -> Element<'a, Message> {
    let t = ctx.theme;
    let logic = &ctx.model.live.panes_logic;
    let activity = logic.activity_or_idle(&pane.id);
    let label = match activity {
        AgentActivity::Working => Key::PaneStateWorking,
        AgentActivity::Waiting => Key::PaneStateWaiting,
        AgentActivity::Yours => Key::PaneStateYoursShort,
        AgentActivity::Idle => Key::PaneStateIdle,
    };
    let state = crate::state::live::agent_state(activity);
    let pill = container(
        row![
            hangar_ui::glyph::state(t, GlyphState::from(state), 7.0),
            text(ctx.t(label))
                .size(tokens::scale::type_scale::CAPTION.size)
                .style(style::text::state(t, state)),
        ]
        .spacing(tokens::space::SM)
        .align_y(Alignment::Center),
    )
    .padding([tokens::space::XXS, tokens::space::MD])
    .style(move |_| container::Style {
        border: Border {
            color: t.palette.border,
            width: tokens::BORDER_WIDTH,
            radius: tokens::radius::FULL.into(),
        },
        ..container::Style::default()
    });
    let mut line = row![
        icon(Icon::Orchestrator, tokens::size::ICON_LG).color(t.palette.accent_text),
        text(pane.name.as_str())
            .size(15.0)
            .font(fonts::display())
            .style(style::text::primary(t)),
        pill,
        Space::new().width(Length::Fill),
    ]
    .spacing(tokens::space::LG)
    .align_y(Alignment::Center);
    let insight = logic.insight(&pane.id);
    let mut meta: Vec<String> = Vec::new();
    if let Some(model) = insight.model.as_deref().filter(|m| !m.trim().is_empty()) {
        meta.push(crate::stats::model::model_label(model));
    }
    if let Some(context) = &insight.context {
        let pct = context.pct.round().clamp(0.0, 100.0) as u32;
        meta.push(
            ctx.i18n
                .t_with(Key::ChatConvContext, &args!("pct" => pct.to_string()))
                .into_owned(),
        );
    }
    if !meta.is_empty() {
        line = line.push(
            text(meta.join(" · "))
                .size(tokens::scale::type_scale::LABEL.size)
                .font(fonts::mono())
                .style(style::text::muted(t)),
        );
    }
    container(line)
        .padding([0.0, tokens::space::XXL])
        .width(Length::Fill)
        .height(HEADER_HEIGHT)
        .align_y(Alignment::Center)
        .into()
}

fn messages<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    pane: &'a Pane,
    chat: &'a Chat,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let mut list = column![].spacing(tokens::space::XXL).width(Length::Fill);
    if chat.reading_older {
        list = list.push(centre_line(hangar_ui::text::hint(
            t,
            ctx.t(Key::ChatConvOlderLoading),
        )));
    } else if chat.thread.older {
        list = list.push(centre_line(
            ui_button::ghost(t, ctx.t(Key::ChatConvOlder))
                .small()
                .on_press(msg(Msg::LoadOlder(ws.id.clone()))),
        ));
    }
    let tasks = ctx.model.live.board.tasks(&ws.cwd);
    let task_title = move |id: &str| {
        tasks?
            .iter()
            .find(|task| task.id == id)
            .map(|task| task.title.clone())
    };
    let pending_question = match &chat.thread.pending {
        Some(ChatPending::Question { tool_use_id, .. }) => Some(tool_use_id.as_str()),
        _ => None,
    };
    // The question the pane waits on is the card below, not an item.
    let items = chat.thread.items.iter().filter(|item| {
        !matches!(&item.body, Body::Question { tool_use_id, .. }
            if Some(tool_use_id.as_str()) == pending_question)
    });
    for piece in calls::pieces(items) {
        list = list.push(match piece {
            Piece::One(item) => item_view(ctx, ws, chat, item, &task_title),
            Piece::Run(run) => run_view(ctx, ws, chat, &run, &task_title),
        });
    }
    for out in &chat.thread.outgoing {
        list = list.push(outgoing(ctx, pane, out));
    }
    if chat.thread.items.is_empty()
        && chat.thread.outgoing.is_empty()
        && let Some(reason) = chat.thread.reason
    {
        let key = match reason {
            ChatReason::NotClaude => Key::ChatConvReasonNotClaude,
            ChatReason::NoSession => Key::ChatConvReasonNoSession,
            ChatReason::NoTranscript => Key::ChatConvReasonNoTranscript,
        };
        list = list.push(notice(
            t,
            ctx.i18n
                .t_with(key, &args!("name" => pane.name.as_str()))
                .into_owned(),
        ));
    }
    if let Some(error) = &chat.error {
        list = list.push(
            hangar_ui::text::danger(
                t,
                ctx.i18n
                    .t_with(Key::ChatConvReadFailed, &args!("error" => error.as_str()))
                    .into_owned(),
            )
            .size(tokens::font_size::CAPTION),
        );
    }
    let ws_id = ws.id.clone();
    let content = centred(container(list).padding(Padding {
        top: tokens::scale::space::S5,
        right: tokens::space::XXXL,
        bottom: tokens::space::XL,
        left: tokens::space::XXXL,
    }));
    // The working line moves on its own clock: outside the laid-out messages.
    let mut page = column![settled(layout_key(ctx, pane, chat, tasks), content)];
    if let Some(working) = working(ctx, pane, chat) {
        page = page.push(centred(container(working).width(Length::Fill).padding(
            Padding {
                top: 0.0,
                right: tokens::space::XXXL,
                bottom: tokens::space::XL,
                left: tokens::space::XXXL,
            },
        )));
    }
    scrollable(page)
        .anchor_bottom()
        .on_scroll(move |viewport| msg(Msg::Scrolled(ws_id.clone(), viewport)))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::scrollable::default(t))
        .into()
}

/// What the messages' words come from: the chat's own state, and the
/// names, titles and language they are written with, and the theme (a
/// Markdown span's colour is set as it is laid out). Laid out again only
/// when it changes.
fn layout_key(
    ctx: Ctx<'_>,
    pane: &Pane,
    chat: &Chat,
    tasks: Option<&[crate::board::model::Task]>,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    chat.revision.hash(&mut hasher);
    ctx.model.locale().hash(&mut hasher);
    ctx.theme.id.hash(&mut hasher);
    pane.name.hash(&mut hasher);
    for task in tasks.unwrap_or_default() {
        task.id.hash(&mut hasher);
        task.title.hash(&mut hasher);
    }
    hasher.finish()
}

/// `ms` as a short span of time: 8s, 2m 05s, 1h 04m.
pub(super) fn span(ms: u64) -> String {
    let seconds = ms / 1000;
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60),
    }
}

/// While the orchestrator works and nothing waits on the person: a line that
/// says so, with how long its turn has run.
fn working<'a>(ctx: Ctx<'a>, pane: &'a Pane, chat: &'a Chat) -> Option<Element<'a, Message>> {
    let t = ctx.theme;
    let logic = &ctx.model.live.panes_logic;
    if logic.activity_or_idle(&pane.id) != AgentActivity::Working || chat.thread.pending.is_some() {
        return None;
    }
    let since = logic.insight(&pane.id).turn_started_at;
    let words = match since {
        Some(since) => ctx
            .i18n
            .t_with(
                Key::ChatConvWorkingFor,
                &args!("time" => span(crate::control::types::now_ms().saturating_sub(since))),
            )
            .into_owned(),
        None => ctx.t(Key::ChatConvWorking).into_owned(),
    };
    Some(
        row![
            hangar_ui::glyph::state(t, GlyphState::Working, 7.0),
            text(words)
                .size(tokens::font_size::CAPTION)
                .style(style::text::state(t, AgentState::Working)),
        ]
        .spacing(tokens::space::MD)
        .align_y(Alignment::Center)
        .padding([0.0, tokens::space::SM])
        .into(),
    )
}

fn centre_line<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content).center_x(Length::Fill).into()
}

/// A line across the conversation with words in its middle: compacted,
/// interrupted, resumed.
fn notice<'a>(t: &'a Theme, words: String) -> Element<'a, Message> {
    let line = || {
        container(rule::horizontal(tokens::BORDER_WIDTH).style(style::rule::sep(t)))
            .width(Length::Fill)
    };
    row![
        line(),
        text(words)
            .size(tokens::font_size::CAPTION)
            .style(style::text::subtle(t))
            .align_x(iced::alignment::Horizontal::Center),
        line(),
    ]
    .spacing(tokens::space::LG)
    .align_y(Alignment::Center)
    .into()
}

fn item_view<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    chat: &'a Chat,
    item: &'a ChatItem,
    task_title: &dyn Fn(&str) -> Option<String>,
) -> Element<'a, Message> {
    let t = ctx.theme;
    match &item.body {
        Body::User {
            text: words,
            from: Sender::Person,
            images,
        } => {
            let mut bubble = column![
                text(words.as_str())
                    .size(tokens::font_size::BODY)
                    .line_height(BUBBLE_LINE)
                    .style(style::text::primary(t))
            ]
            .spacing(tokens::space::XS);
            if *images > 0 {
                bubble = bubble.push(
                    text(ctx.i18n.tn(Key::ChatConvImages, *images, &[]).into_owned())
                        .size(tokens::font_size::CAPTION)
                        .style(style::text::muted(t)),
                );
            }
            mine(t, bubble, false)
        }
        Body::User {
            text: words,
            from: Sender::Hangar,
            ..
        } => teskad(ctx, item.at, hangar_message(ctx, ws, chat, item, words)),
        Body::Assistant { .. } => {
            let Some((_, content)) = chat.prose.get(&item.id) else {
                return Space::new().into();
            };
            container(rich::view(t, content, &chat.code))
                .width(Length::Fill)
                .into()
        }
        Body::Action { .. } => framed(t, action_item(ctx, ws, chat, item, task_title)),
        Body::Question {
            questions, answer, ..
        } => asked(ctx, questions, answer.as_deref()),
        Body::Notice { text: words } => notice(t, words.clone()),
        // A Codex image: shown by T26 (agents.md §5.3).
        Body::Image { .. } => Space::new().into(),
    }
}

/// The icon of a call: what kind of thing it does.
fn call_icon(tool: &str) -> Icon {
    match calls::kind(tool) {
        Kind::Command => Icon::Terminal,
        Kind::Edit => Icon::Pencil,
        Kind::Read => Icon::Folder,
        Kind::Search => Icon::Search,
        Kind::Task => Icon::Board,
        Kind::Memory => Icon::Bookmark,
        Kind::Message => Icon::Bubble,
        Kind::Other if super::titles::own_tool(tool).is_some() => Icon::Orchestrator,
        Kind::Other if tool.starts_with("mcp__") => Icon::Plug,
        Kind::Other => Icon::Bolt,
    }
}

/// A run of calls, folded into one quiet line saying what they did; the
/// calls, each its own line as alone, once pressed.
fn run_view<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    chat: &'a Chat,
    run: &[&'a ChatItem],
    task_title: &dyn Fn(&str) -> Option<String>,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let Some(first) = run.first() else {
        return Space::new().into();
    };
    let key = Piece::key(first);
    let open = chat.expanded.contains(&key);
    let tally = calls::tally(run);
    let state = if tally.running > 0 {
        GlyphState::Working
    } else if tally.failed > 0 {
        GlyphState::Failed
    } else {
        GlyphState::Completed
    };
    // The icon of what the run does, when it does one thing only.
    let kind = match tally.kinds.as_slice() {
        [_] => match &first.body {
            Body::Action { tool, .. } => call_icon(tool),
            _ => Icon::Bolt,
        },
        _ => Icon::Terminal,
    };
    let mut line = row![
        chevron(t, Some(open)),
        icon(kind, CALL_ICON).color(t.palette.text_secondary),
        text(calls::summary(ctx.i18n, &tally))
            .size(tokens::font_size::CAPTION)
            .font(fonts::ui_medium())
            .style(style::text::primary(t)),
    ]
    .spacing(tokens::space::MD)
    .align_y(Alignment::Center);
    if tally.failed > 0 {
        line = line.push(
            text(format!(
                "· {}",
                ctx.i18n.tn(Key::ChatConvCallsFailed, tally.failed, &[])
            ))
            .size(tokens::font_size::CAPTION)
            .color(t.inks.warning()),
        );
    }
    if tally.running > 0 {
        line = line.push(
            text(format!(
                "· {}",
                ctx.i18n.tn(Key::ChatConvCallsRunning, tally.running, &[])
            ))
            .size(tokens::font_size::CAPTION)
            .color(t.palette.status.running),
        );
    }
    let line = line
        .push(Space::new().width(Length::Fill))
        .push(hangar_ui::glyph::state(t, state, STATE_GLYPH));
    let head = block_head(t, line, Some(msg(Msg::Toggle(ws.id.clone(), key))));
    if !open {
        return framed(t, head);
    }
    let mut list = column![].spacing(tokens::space::XXS);
    for item in run {
        list = list.push(action_item(ctx, ws, chat, item, task_title));
    }
    framed(
        t,
        column![
            head,
            container(list).padding(Padding {
                top: 0.0,
                right: tokens::space::XS,
                bottom: tokens::space::SM,
                left: tokens::space::XL,
            })
        ],
    )
}

/// The block calls are shown in: a framed surface, folded or not.
fn framed<'a>(t: &'a Theme, content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(t.palette.surface)),
            border: Border {
                color: t.palette.border,
                width: tokens::BORDER_WIDTH,
                radius: tokens::scale::radius::MD.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// A block's header line, a button when there is something to open.
fn block_head<'a>(
    t: &'a Theme,
    line: impl Into<Element<'a, Message>>,
    press: Option<Message>,
) -> Element<'a, Message> {
    let head = button(
        container(line)
            .height(Length::Shrink)
            .padding([tokens::space::SM, 0.0]),
    )
    .padding(Padding::from([0.0, tokens::space::XL]))
    .width(Length::Fill)
    .style(style::button::row(t, false));
    ringed(t, head, press, tokens::scale::radius::MD)
}

/// The chevron of a line that opens, or its room on one that does not.
fn chevron<'a>(t: &'a Theme, open: Option<bool>) -> Element<'a, Message> {
    let mark: Element<'a, Message> = match open {
        Some(open) => icon(
            if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            11.0,
        )
        .color(t.palette.text_muted)
        .into(),
        None => Space::new().into(),
    };
    container(mark).width(CALL_ICON).center_x(CALL_ICON).into()
}

/// A bubble of the person's, on the right.
fn mine<'a>(
    t: &'a Theme,
    content: impl Into<Element<'a, Message>>,
    faded: bool,
) -> Element<'a, Message> {
    let bubble = container(content)
        .padding([tokens::space::LG, tokens::space::XL + tokens::space::XXS])
        .max_width(BUBBLE_WIDTH)
        .style(move |_| container::Style {
            background: Some(Background::Color(tint(
                t.palette.surface_raised,
                if faded { 0.6 } else { 1.0 },
            ))),
            border: Border {
                color: t.palette.border,
                width: tokens::BORDER_WIDTH,
                radius: tokens::scale::radius::LG.into(),
            },
            text_color: Some(t.palette.text),
            ..container::Style::default()
        });
    row![Space::new().width(Length::Fill), bubble].into()
}

/// A message of Teskad's, never taken for the person's: an inset of its own,
/// under its mark and the Fork, with when it was written.
fn teskad<'a>(ctx: Ctx<'a>, at: i64, content: Element<'a, Message>) -> Element<'a, Message> {
    let t = ctx.theme;
    let mut head = row![
        icon(Icon::Orchestrator, tokens::size::ICON).color(t.palette.accent_text),
        text(format!("[{}]", hangar_i18n::APP_NAME))
            .size(tokens::scale::type_scale::LABEL.size)
            .font(fonts::mono())
            .color(t.palette.accent_text),
        Space::new().width(Length::Fill),
    ]
    .spacing(tokens::space::MD)
    .align_y(Alignment::Center);
    if at > 0 {
        head = head.push(
            text(crate::inspector::format::clock_time(at))
                .size(tokens::scale::type_scale::LABEL.size)
                .font(fonts::mono())
                .style(style::text::muted(t)),
        );
    }
    container(
        column![container(head).padding([0.0, tokens::space::SM]), content]
            .spacing(tokens::space::SM),
    )
    .padding([tokens::space::LG, tokens::space::SM])
    .width(Length::Fill)
    .style(move |_| container::Style {
        background: Some(Background::Color(t.palette.selection_bg)),
        border: Border {
            color: t.palette.border_strong,
            width: tokens::BORDER_WIDTH,
            radius: tokens::scale::radius::MD.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// How long the first line of a message gets in its one line.
const NOTE_WORDS: usize = 140;

/// A line of the workspace's news in the person's words: its icon and its
/// colour, by what it says.
fn heard_line(ctx: Ctx<'_>, heard: &Heard) -> (Icon, Color, String) {
    let t = ctx.theme;
    let inks = &t.inks;
    let say = |key: Key, args: &[(&str, &str)]| {
        let list: Vec<(&str, hangar_i18n::Arg)> = args
            .iter()
            .map(|(name, value)| (*name, hangar_i18n::Arg::from(*value)))
            .collect();
        ctx.i18n.t_with(key, &list).into_owned()
    };
    let column = |id: &str| {
        crate::board::model::Column::from_id(id)
            .map(|column| ctx.t(column.label_key()).into_owned())
            .unwrap_or_else(|| id.to_string())
    };
    match heard {
        Heard::Delivery {
            task,
            worker,
            outcome,
            ..
        } => {
            let (glyph, colour, key) = match outcome {
                DeliveryOutcome::Landed => (
                    Icon::Check,
                    t.palette.status.completed,
                    Key::ChatConvHeardLanded,
                ),
                DeliveryOutcome::Done => (
                    Icon::Check,
                    t.palette.status.completed,
                    Key::ChatConvHeardDone,
                ),
                DeliveryOutcome::Ready => {
                    (Icon::Merge, t.palette.accent_text, Key::ChatConvHeardReady)
                }
                DeliveryOutcome::Conflict => {
                    (Icon::Alert, inks.warning(), Key::ChatConvHeardConflict)
                }
                DeliveryOutcome::CheckFailed => {
                    (Icon::Alert, inks.warning(), Key::ChatConvHeardCheckFailed)
                }
                DeliveryOutcome::ChangesRequested => (
                    Icon::Alert,
                    inks.warning(),
                    Key::ChatConvHeardChangesRequested,
                ),
                DeliveryOutcome::Dirty => (Icon::Alert, inks.warning(), Key::ChatConvHeardDirty),
                DeliveryOutcome::Stale => (Icon::Alert, inks.warning(), Key::ChatConvHeardStale),
                DeliveryOutcome::Error => (
                    Icon::Alert,
                    t.palette.status.failed,
                    Key::ChatConvHeardFailed,
                ),
            };
            (
                glyph,
                colour,
                say(key, &[("worker", worker.as_str()), ("task", task.as_str())]),
            )
        }
        Heard::NeedsYou { task, reason } => (
            Icon::Bell,
            inks.waiting.text,
            say(
                Key::ChatConvHeardNeedsYou,
                &[("task", task.as_str()), ("reason", reason.as_str())],
            ),
        ),
        Heard::Blocked { task, by, reason } => {
            let words = match by {
                Some(who) => say(
                    Key::ChatConvHeardBlocked,
                    &[
                        ("who", who.as_str()),
                        ("task", task.as_str()),
                        ("reason", reason.as_str()),
                    ],
                ),
                None => say(
                    Key::ChatConvHeardBlockedByYou,
                    &[("task", task.as_str()), ("reason", reason.as_str())],
                ),
            };
            (Icon::Lock, inks.warning(), words)
        }
        Heard::Unblocked { task } => (
            Icon::Lock,
            t.palette.text_muted,
            say(Key::ChatConvHeardUnblocked, &[("task", task.as_str())]),
        ),
        Heard::RecheckDue { task } => (
            Icon::Clock,
            inks.warning(),
            say(Key::ChatConvHeardRecheck, &[("task", task.as_str())]),
        ),
        Heard::Commented { task, by, text } => {
            let words = match by {
                Some(who) => say(
                    Key::ChatConvHeardCommented,
                    &[
                        ("who", who.as_str()),
                        ("task", task.as_str()),
                        ("text", text.as_str()),
                    ],
                ),
                None => say(
                    Key::ChatConvHeardCommentedByYou,
                    &[("task", task.as_str()), ("text", text.as_str())],
                ),
            };
            (Icon::Bubble, t.palette.text_muted, words)
        }
        Heard::Moved { task, from, to } => (
            Icon::ArrowRight,
            t.palette.text_muted,
            say(
                Key::ChatConvHeardMoved,
                &[
                    ("task", task.as_str()),
                    ("from", &column(from)),
                    ("to", &column(to)),
                ],
            ),
        ),
        Heard::Started { worker, task } => {
            let words = match task {
                Some(task) => say(
                    Key::ChatConvHeardStartedOn,
                    &[("worker", worker.as_str()), ("task", task.as_str())],
                ),
                None => say(Key::ChatConvHeardStarted, &[("worker", worker.as_str())]),
            };
            (Icon::Play, t.palette.status.running, words)
        }
        Heard::Stopped { worker, .. } => (
            Icon::Stop,
            t.palette.text_muted,
            say(Key::ChatConvHeardStopped, &[("worker", worker.as_str())]),
        ),
        Heard::Exited { worker, .. } => (
            Icon::Close,
            t.palette.text_muted,
            say(Key::ChatConvHeardExited, &[("worker", worker.as_str())]),
        ),
        Heard::Waiting {
            worker, minutes, ..
        } => (
            Icon::Bell,
            inks.waiting.text,
            say(
                Key::ChatConvHeardWaiting,
                &[
                    ("worker", worker.as_str()),
                    ("minutes", &minutes.to_string()),
                ],
            ),
        ),
        Heard::Idle { worker, .. } => (
            Icon::Pause,
            inks.warning(),
            say(Key::ChatConvHeardIdle, &[("worker", worker.as_str())]),
        ),
    }
}

/// One line of what Hangar said: an icon, the words, and when there is more
/// to read, the chevron that opens it.
fn note_line<'a>(
    t: &'a Theme,
    glyph: Icon,
    colour: Color,
    words: String,
    toggle: Option<(bool, Message)>,
) -> Element<'a, Message> {
    let mut line = row![
        container(icon(glyph, CALL_ICON).color(colour))
            .width(CALL_ICON)
            .center_x(CALL_ICON),
        text(words)
            .size(tokens::font_size::CAPTION)
            .width(Length::Fill)
            .style(style::text::secondary(t)),
    ]
    .spacing(tokens::space::MD)
    .align_y(Alignment::Center);
    let Some((open, press)) = toggle else {
        return container(line)
            .padding([tokens::space::XS, tokens::space::SM])
            .into();
    };
    line = line.push(
        icon(
            if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            11.0,
        )
        .color(t.palette.text_muted),
    );
    let line = button(line)
        .padding([tokens::space::XS, tokens::space::SM])
        .width(Length::Fill)
        .style(style::button::row(t, false));
    ringed(t, line, Some(press), tokens::radius::SM)
}

/// The most files of a delivery listed under its line; the rest are in the
/// "Changes" overlay.
const DELIVERY_FILES: usize = 8;

/// The +/− of a list of files, as the "Changes" overlay counts them.
fn line_counts(files: &[ChangedFile]) -> (u32, u32) {
    files.iter().fold((0, 0), |(added, deleted), file| {
        (
            added + file.additions.unwrap_or(0),
            deleted + file.deletions.unwrap_or(0),
        )
    })
}

/// "+12 −3", each side in its colour; a side that is nothing is left out.
fn counts<'a>(t: &'a Theme, added: u32, deleted: u32) -> Element<'a, Message> {
    let mut both = row![].spacing(tokens::space::SM);
    if added > 0 {
        both = both.push(
            text(format!("+{added}"))
                .size(tokens::font_size::CAPTION)
                .font(fonts::mono())
                .style(style::text::success(t)),
        );
    }
    if deleted > 0 {
        both = both.push(
            text(format!("−{deleted}"))
                .size(tokens::font_size::CAPTION)
                .font(fonts::mono())
                .style(style::text::danger(t)),
        );
    }
    both.into()
}

/// A delivery that went onto a branch: its line, with how many files it
/// brought and their +/−, unfolding into the files, each of which opens
/// the "Changes" overlay on it.
fn delivery_note<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    chat: &'a Chat,
    (glyph, colour, words): (Icon, Color, String),
    task: String,
    range: Range,
    changes: Arc<Changes>,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let key = super::changes_key(&range);
    let open = chat.expanded.contains(&key);
    let (added, deleted) = line_counts(&changes.files);
    let head = button(
        row![
            container(icon(glyph, CALL_ICON).color(colour))
                .width(CALL_ICON)
                .center_x(CALL_ICON),
            text(words)
                .size(tokens::font_size::CAPTION)
                .width(Length::Fill)
                .style(style::text::secondary(t)),
            text(
                ctx.i18n
                    .tn(Key::DiffFiles, changes.files.len(), &[])
                    .into_owned()
            )
            .size(tokens::font_size::CAPTION)
            .style(style::text::muted(t)),
            counts(t, added, deleted),
            icon(
                if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                11.0,
            )
            .color(t.palette.text_muted),
        ]
        .spacing(tokens::space::MD)
        .align_y(Alignment::Center),
    )
    .padding([tokens::space::XS, tokens::space::SM])
    .width(Length::Fill)
    .style(style::button::row(t, false));
    let head = ringed(
        t,
        head,
        Some(msg(Msg::Toggle(ws.id.clone(), key))),
        tokens::radius::SM,
    );
    if !open {
        return head;
    }
    let open_at = |file: Option<String>| {
        msg(Msg::OpenChanges {
            ws: ws.id.clone(),
            range: range.clone(),
            task: task.clone(),
            file,
        })
    };
    let mut files = column![].spacing(tokens::space::XXS);
    for file in changes.files.iter().take(DELIVERY_FILES) {
        let inks = &t.inks;
        let (mark, tone) = match file.status.as_str() {
            "A" | "C" => ("A", t.palette.status.completed),
            "D" => ("D", t.palette.status.failed),
            "R" => ("R", t.palette.accent_text),
            _ => ("M", inks.warning()),
        };
        let shown = match &file.old_path {
            Some(old) if file.status == "R" => format!("{old} → {}", file.path),
            _ => file.path.clone(),
        };
        let line = button(
            row![
                container(
                    text(mark)
                        .size(tokens::font_size::MICRO)
                        .font(fonts::mono())
                        .color(tone)
                )
                .width(CALL_ICON)
                .center_x(CALL_ICON),
                text(shown)
                    .size(tokens::font_size::CAPTION)
                    .font(fonts::mono())
                    .width(Length::Fill)
                    .style(style::text::secondary(t)),
                counts(t, file.additions.unwrap_or(0), file.deletions.unwrap_or(0)),
            ]
            .spacing(tokens::space::MD)
            .align_y(Alignment::Center),
        )
        .padding([tokens::space::XXS, tokens::space::SM])
        .width(Length::Fill)
        .style(style::button::row(t, false));
        files = files.push(ringed(
            t,
            line,
            Some(open_at(Some(file.path.clone()))),
            tokens::radius::SM,
        ));
    }
    let rest = changes.files.len().saturating_sub(DELIVERY_FILES);
    let all = if rest > 0 {
        ctx.i18n
            .tn(Key::ChatConvChangesMore, rest, &[])
            .into_owned()
    } else {
        ctx.t(Key::ChatConvChangesAll).into_owned()
    };
    let all = button(
        text(all)
            .size(tokens::font_size::CAPTION)
            .style(style::text::accent(t)),
    )
    .padding([tokens::space::XXS, tokens::space::SM])
    .style(style::button::row(t, false));
    files = files.push(
        container(ringed(t, all, Some(open_at(None)), tokens::radius::SM)).padding(Padding {
            left: CALL_ICON + tokens::space::MD,
            ..Padding::ZERO
        }),
    );
    column![
        head,
        container(files).padding(Padding {
            left: CALL_ICON + tokens::space::MD,
            bottom: tokens::space::SM,
            ..Padding::ZERO
        }),
    ]
    .into()
}

/// A message Hangar put in the orchestrator's conversation, compact: a line
/// per thing that happened in the workspace, in the person's words; or one
/// line for a message from another agent, the state of the work handed to a
/// conversation that starts, or anything else, the whole of it a click away.
fn hangar_message<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    chat: &'a Chat,
    item: &'a ChatItem,
    words: &'a str,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let open = chat.expanded.contains(&item.id);
    let toggle = || (open, msg(Msg::Toggle(ws.id.clone(), item.id.clone())));
    let (head, whole): (Element<'a, Message>, Option<String>) = match hangar::note(words) {
        hangar::Note::News(lines) => {
            let mut list = column![].spacing(tokens::space::XXS);
            for line in &lines {
                let (glyph, colour, words) = match line {
                    hangar::Line::Heard(heard) => heard_line(ctx, heard),
                    hangar::Line::Said(words) => {
                        (Icon::Broadcast, t.palette.text_muted, words.clone())
                    }
                };
                let delivered = match line {
                    hangar::Line::Heard(Heard::Delivery {
                        task,
                        changes: Some(range),
                        ..
                    }) => match chat.changes.get(range) {
                        Some(Changed::Read(changes)) if !changes.files.is_empty() => {
                            Some((task.clone(), range.clone(), changes.clone()))
                        }
                        _ => None,
                    },
                    _ => None,
                };
                list = list.push(match delivered {
                    Some((task, range, changes)) => {
                        delivery_note(ctx, ws, chat, (glyph, colour, words), task, range, changes)
                    }
                    None => note_line(t, glyph, colour, words, None),
                });
            }
            return list.into();
        }
        hangar::Note::Agent { from, text: said } => {
            let line = hangar::first_line(&said, NOTE_WORDS);
            let more = said.trim() != line;
            let words = ctx
                .i18n
                .t_with(
                    Key::ChatConvHeardMessage,
                    &args!("from" => from.as_str(), "text" => line.as_str()),
                )
                .into_owned();
            (
                note_line(
                    t,
                    Icon::Bubble,
                    t.palette.text_muted,
                    words,
                    more.then(toggle),
                ),
                more.then_some(said),
            )
        }
        hangar::Note::Briefing(state) => {
            let name = orchestrator_of(ws).map_or("", |pane| pane.name.as_str());
            let words = ctx
                .i18n
                .t_with(Key::ChatConvHeardBriefing, &args!("name" => name))
                .into_owned();
            let more = !state.trim().is_empty();
            (
                note_line(
                    t,
                    Icon::Board,
                    t.palette.text_muted,
                    words,
                    more.then(toggle),
                ),
                more.then_some(state),
            )
        }
        hangar::Note::Other(said) => {
            let line = hangar::first_line(&said, NOTE_WORDS);
            let more = said.trim() != line;
            (
                note_line(
                    t,
                    Icon::Broadcast,
                    t.palette.text_muted,
                    line,
                    more.then(toggle),
                ),
                more.then_some(said),
            )
        }
    };
    match whole.filter(|_| open) {
        Some(whole) => column![
            head,
            container(
                text(whole)
                    .size(tokens::font_size::CAPTION)
                    .line_height(tokens::line_height::BODY)
                    .style(style::text::muted(t)),
            )
            .padding(Padding {
                top: 0.0,
                right: tokens::space::SM,
                bottom: tokens::space::SM,
                left: CALL_INDENT,
            })
        ]
        .into(),
        None => head,
    }
}

struct Action<'b> {
    tool: &'b str,
    title: &'b str,
    detail: Option<&'b str>,
    result: Option<&'b CallResult>,
    state: CallState,
}

/// The name of a tool as a chip shows it: an MCP tool's own name.
fn tool_name(tool: &str) -> &str {
    tool.strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
        .map_or(tool, |(_, name)| name)
}

/// Lines `at_most` of `words`, the rest said to be there.
fn clipped(words: &str, at_most: usize) -> String {
    let count = words.lines().count();
    if count <= at_most {
        return words.to_string();
    }
    let head: Vec<&str> = words.lines().take(at_most).collect();
    format!("{}\n…", head.join("\n"))
}

/// The line of call `item`.
fn action_item<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    chat: &'a Chat,
    item: &'a ChatItem,
    task_title: &dyn Fn(&str) -> Option<String>,
) -> Element<'a, Message> {
    let Body::Action {
        tool,
        title,
        detail,
        result,
        state,
    } = &item.body
    else {
        return Space::new().into();
    };
    action(
        ctx,
        ws,
        chat,
        item,
        Action {
            tool,
            title: &titles::title(ctx.i18n, tool, title, detail.as_deref(), task_title),
            detail: detail.as_deref(),
            result: result.as_ref(),
            state: *state,
        },
    )
}

/// A call, a line of the work log: what kind of thing it does, what it did,
/// how it went; its detail below once pressed, the command and then what it
/// printed.
fn action<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    chat: &'a Chat,
    item: &'a ChatItem,
    call: Action<'_>,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let (state, colour, hint) = match call.state {
        CallState::Running => (
            GlyphState::Working,
            t.palette.status.running,
            Some(ctx.t(Key::ChatConvActionRunning)),
        ),
        CallState::Done => (GlyphState::Completed, t.palette.text_secondary, None),
        CallState::Error => (
            GlyphState::Failed,
            t.palette.status.failed,
            Some(ctx.t(Key::ChatConvActionFailed)),
        ),
    };
    let more = call.detail.is_some() || call.result.is_some_and(|r| !r.text.trim().is_empty());
    let open = more && chat.expanded.contains(&item.id);
    let mut line = row![
        chevron(t, more.then_some(open)),
        icon(call_icon(call.tool), CALL_ICON).color(colour),
        container(
            text(call.title.to_string())
                .size(tokens::font_size::CAPTION)
                .font(fonts::ui_medium())
                .style(style::text::primary(t)),
        )
        .width(Length::Fill),
    ]
    .spacing(tokens::space::MD)
    .align_y(Alignment::Center);
    if let Some(hint) = hint {
        line = line.push(text(hint).size(tokens::font_size::MICRO).color(colour));
    }
    let line = line.push(hangar_ui::glyph::state(t, state, STATE_GLYPH));
    let head = block_head(
        t,
        line,
        more.then(|| msg(Msg::Toggle(ws.id.clone(), item.id.clone()))),
    );
    if !open {
        return head;
    }
    let mono = |words: String| {
        text(words)
            .size(tokens::font_size::LABEL)
            .line_height(tokens::line_height::BODY)
            .font(fonts::mono())
            .width(Length::Fill)
    };
    let mut detail = column![].spacing(tokens::space::SM);
    if let Some(words) = call.detail {
        detail = detail.push(
            mono(clipped(&titles::detail(call.tool, words), DETAIL_LINES))
                .style(style::text::primary(t)),
        );
    }
    if let Some(result) = call.result.filter(|r| !r.text.trim().is_empty()) {
        detail = detail.push(
            text(ctx.t(Key::ChatConvActionResult))
                .size(tokens::font_size::MICRO)
                .font(fonts::ui_semibold())
                .style(style::text::muted(t)),
        );
        detail =
            detail.push(mono(clipped(&result.text, DETAIL_LINES)).style(style::text::muted(t)));
        if !result.ok {
            detail = detail.push(
                text(ctx.t(Key::ChatConvActionFailed))
                    .size(tokens::font_size::MICRO)
                    .style(style::text::danger(t)),
            );
        }
    }
    column![
        head,
        container(detail).padding(Padding {
            top: 0.0,
            right: tokens::space::XL,
            bottom: tokens::space::LG,
            left: tokens::space::XL + CALL_ICON + tokens::space::MD,
        })
    ]
    .into()
}

fn mono_block<'a>(t: &'a Theme, words: String) -> Element<'a, Message> {
    container(
        text(words)
            .size(tokens::font_size::LABEL)
            .font(fonts::mono())
            .style(style::text::primary(t)),
    )
    .padding([tokens::space::MD, tokens::space::LG])
    .width(Length::Fill)
    .style(move |_| container::Style {
        background: Some(Background::Color(t.palette.surface_sunken)),
        border: Border {
            color: t.palette.border,
            width: tokens::BORDER_WIDTH,
            radius: tokens::radius::MD.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// A question asked and answered, or left without an answer.
fn asked<'a>(
    ctx: Ctx<'a>,
    questions: &'a [Question],
    answers: Option<&'a [Answer]>,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let mut stack = column![].spacing(tokens::space::XS);
    for (at, question) in questions.iter().enumerate() {
        stack = stack.push(
            text(
                ctx.i18n
                    .t_with(
                        Key::ChatConvQuestionAsked,
                        &args!("question" => question.question.as_str()),
                    )
                    .into_owned(),
            )
            .size(tokens::font_size::UI)
            .style(style::text::secondary(t)),
        );
        let answer = answers.and_then(|answers| answers.get(at));
        let words: Vec<String> = answer
            .map(|answer| {
                answer
                    .selected
                    .iter()
                    .cloned()
                    .chain(answer.other.clone())
                    .collect()
            })
            .unwrap_or_default();
        let line = if words.is_empty() {
            text(ctx.t(Key::ChatConvQuestionNoAnswer))
                .size(tokens::font_size::CAPTION)
                .style(style::text::subtle(t))
        } else {
            text(format!("→ {}", words.join(", ")))
                .size(tokens::font_size::CAPTION)
                .style(style::text::accent(t))
        };
        stack = stack.push(line);
    }
    row![
        container(icon(Icon::Bubble, CALL_ICON).color(t.palette.text_muted))
            .width(CALL_ICON)
            .center_x(CALL_ICON),
        container(stack).width(Length::Fill),
    ]
    .spacing(tokens::space::MD)
    .padding([tokens::space::XS, tokens::space::SM])
    .into()
}

/// A message sent from here the transcript does not have yet.
fn outgoing<'a>(ctx: Ctx<'a>, pane: &'a Pane, out: &'a Outgoing) -> Element<'a, Message> {
    let t = ctx.theme;
    let state = match &out.state {
        Sent::Sending => Some(
            text(ctx.t(Key::ChatConvSending))
                .size(tokens::font_size::MICRO)
                .style(style::text::muted(t)),
        ),
        Sent::Typed => None,
        Sent::Queued => Some(
            text(
                ctx.i18n
                    .t_with(
                        Key::ChatConvQueued,
                        &args!("name" => pane.name.as_str(), "agent" => pane.agent.info().label),
                    )
                    .into_owned(),
            )
            .size(tokens::font_size::MICRO)
            .style(style::text::muted(t)),
        ),
        Sent::Failed(error) => Some(
            text(
                ctx.i18n
                    .t_with(Key::ChatConvNotSent, &args!("error" => error.as_str()))
                    .into_owned(),
            )
            .size(tokens::font_size::MICRO)
            .style(style::text::danger(t)),
        ),
    };
    let mut bubble = column![
        text(out.text.as_str())
            .size(tokens::font_size::BODY)
            .line_height(tokens::line_height::BODY)
            .style(style::text::primary(t))
    ]
    .spacing(tokens::space::XS);
    if let Some(state) = state {
        bubble = bubble.push(state);
    }
    mine(t, bubble, !matches!(out.state, Sent::Typed))
}

/// The tallest the body of a card gets: past it, it scrolls, and its
/// title and buttons stay in sight.
const CARD_BODY_HEIGHT: f32 = 340.0;

/// A plan is read before it is approved: its card gets more room.
const PLAN_BODY_HEIGHT: f32 = 480.0;

/// A card's parts: its title row, what it shows (scrolled when tall) and
/// its buttons (always shown).
struct CardParts<'a> {
    title: Element<'a, Message>,
    body: Element<'a, Message>,
    actions: Element<'a, Message>,
    /// The body's scroll, per question shown: a new one starts at its top.
    scroll: iced::widget::Id,
}

/// The card of what the pane waits on the person for, in the composer's
/// place, framed in the accent with its halo.
fn card<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    pane: &'a Pane,
    chat: &'a Chat,
) -> Option<Element<'a, Message>> {
    let t = ctx.theme;
    let parts = match chat.thread.pending.as_ref()? {
        ChatPending::Question { questions, .. } => question_card(ctx, ws, pane, chat, questions),
        ChatPending::Permission {
            tool,
            plan: Some(_),
            prompt,
            ..
        } if tool == PLAN_TOOL => plan_card(ctx, ws, pane, chat, prompt),
        ChatPending::Permission {
            tool,
            title,
            detail,
            ..
        } => permission_card(ctx, ws, pane, chat, tool, title, detail.as_deref()),
    };
    let tall = matches!(
        chat.thread.pending,
        Some(ChatPending::Permission { ref tool, plan: Some(_), .. }) if tool == PLAN_TOOL
    );
    let body = container(
        scrollable(container(parts.body).padding(Padding {
            right: tokens::space::LG,
            ..Padding::ZERO
        }))
        .id(parts.scroll)
        .height(Length::Shrink)
        .style(style::scrollable::default(t)),
    )
    .max_height(if tall {
        PLAN_BODY_HEIGHT
    } else {
        CARD_BODY_HEIGHT
    });
    let mut stack = column![parts.title, body].spacing(tokens::space::LG);
    // What happened to the last answer, right above the buttons: never out
    // of sight.
    if chat.deciding {
        stack = stack.push(hangar_ui::text::hint(t, ctx.t(Key::ChatConvBusy)));
    }
    if let Some(refusal) = &chat.refusal {
        stack = stack.push(
            hangar_ui::text::danger(
                t,
                ctx.i18n
                    .t_with(Key::ChatConvRefused, &args!("error" => refusal.as_str()))
                    .into_owned(),
            )
            .size(tokens::font_size::CAPTION),
        );
    }
    stack = stack.push(parts.actions);
    let inside = container(stack)
        .padding([tokens::space::XL, tokens::space::XL + tokens::space::XXS])
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(t.palette.surface)),
            border: Border {
                color: t.palette.accent,
                width: tokens::BORDER_WIDTH,
                radius: tokens::scale::radius::MD.into(),
            },
            ..container::Style::default()
        });
    Some(
        container(inside)
            .padding(HALO)
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(tint(t.palette.accent, HALO_ALPHA))),
                border: Border {
                    radius: (tokens::scale::radius::MD + HALO).into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into(),
    )
}

/// A card's title: what it is about, and at the end the mark of what waits.
fn card_title<'a>(t: &'a Theme, glyph: Icon, words: String) -> Element<'a, Message> {
    row![
        icon(glyph, tokens::size::ICON_LG).color(t.palette.accent_text),
        text(words)
            .size(tokens::font_size::UI)
            .font(fonts::ui_semibold())
            .width(Length::Fill)
            .style(style::text::primary(t)),
        hangar_ui::glyph::state(t, GlyphState::Waiting, 8.0),
    ]
    .spacing(tokens::space::MD)
    .align_y(Alignment::Center)
    .into()
}

/// A question's header, as a chip.
fn header_chip<'a>(t: &'a Theme, header: &'a str) -> Element<'a, Message> {
    container(
        text(header)
            .size(tokens::font_size::MICRO)
            .style(style::text::muted(t)),
    )
    .padding([0.0, tokens::space::SM])
    .style(style::container::chip(t))
    .into()
}

/// An option of a question, chosen or not.
fn choice<'a>(
    t: &'a Theme,
    label: String,
    description: Option<&'a str>,
    multi: bool,
    on: bool,
    press: Option<Message>,
) -> Element<'a, Message> {
    let mark = match (multi, on) {
        (_, true) => Icon::Check,
        (true, false) => Icon::Minus,
        (false, false) => Icon::ChevronRight,
    };
    let mut words = column![
        text(label)
            .size(tokens::font_size::UI)
            .style(style::text::primary(t))
    ]
    .width(Length::Fill);
    if let Some(description) = description.filter(|d| !d.trim().is_empty()) {
        words = words.push(
            text(description)
                .size(tokens::font_size::CAPTION)
                .style(style::text::muted(t)),
        );
    }
    let line = button(
        row![
            icon(mark, 12.0).color(if on {
                t.palette.accent_text
            } else {
                t.palette.text_muted
            }),
            words
        ]
        .spacing(tokens::space::MD)
        .align_y(Alignment::Center),
    )
    .padding([tokens::space::MD, tokens::space::LG])
    .width(Length::Fill)
    .style(style::button::toggle(t, on));
    ringed(t, line, press, tokens::radius::MD)
}

/// The question card: one question at a time, as Claude Code asks them —
/// its header and words, its options and "Other", the step it is (1/3),
/// Back and Next, and Send on the last one. Nothing of the answers given
/// before: Back goes to them.
fn question_card<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    pane: &'a Pane,
    chat: &'a Chat,
    questions: &'a [Question],
) -> CardParts<'a> {
    let t = ctx.theme;
    let form = &chat.form;
    let free = !chat.deciding;
    let count = questions.len();
    let at = form.step();
    let last = at + 1 >= count;
    let mut title = row![
        container(card_title(
            t,
            Icon::Bubble,
            ctx.i18n
                .t_with(
                    Key::ChatConvQuestionTitle,
                    &args!("name" => pane.name.as_str())
                )
                .into_owned(),
        ))
        .width(Length::Fill)
    ]
    .align_y(Alignment::Center);
    if count > 1 {
        title = title.push(
            text(format!("{}/{count}", at + 1))
                .size(tokens::font_size::CAPTION)
                .font(fonts::mono())
                .style(style::text::muted(t)),
        );
    }
    let mut block = column![].spacing(tokens::space::SM);
    if let Some(question) = questions.get(at) {
        if !question.header.trim().is_empty() {
            block = block.push(header_chip(t, question.header.as_str()));
        }
        block = block.push(
            text(question.question.as_str())
                .size(tokens::font_size::BODY)
                .width(Length::Fill)
                .style(style::text::primary(t)),
        );
        if question.multi_select {
            block = block.push(hangar_ui::text::hint(t, ctx.t(Key::ChatConvQuestionMulti)));
        }
        let picked = form.picked.get(at).map(Vec::as_slice).unwrap_or(&[]);
        for (option_at, option) in question.options.iter().enumerate() {
            block = block.push(choice(
                t,
                option.label.clone(),
                Some(option.description.as_str()),
                question.multi_select,
                picked.contains(&option_at),
                free.then(|| {
                    msg(Msg::Pick {
                        ws: ws.id.clone(),
                        question: at,
                        option: option_at,
                    })
                }),
            ));
        }
        let other_on = form.other_on.get(at).copied().unwrap_or(false);
        block = block.push(choice(
            t,
            ctx.t(Key::ChatConvQuestionOther).into_owned(),
            None,
            question.multi_select,
            other_on,
            free.then(|| {
                msg(Msg::Other {
                    ws: ws.id.clone(),
                    question: at,
                })
            }),
        ));
        if other_on {
            let words = form.other.get(at).map(String::as_str).unwrap_or("");
            let ws_id = ws.id.clone();
            let field = text_input(&ctx.t(Key::ChatConvQuestionOtherPlaceholder), words)
                .size(tokens::font_size::UI)
                .padding([tokens::space::SM, tokens::space::MD])
                .style(style::text_input::field(t))
                .on_input_maybe(free.then_some(move |text| {
                    msg(Msg::OtherText {
                        ws: ws_id.clone(),
                        question: at,
                        text,
                    })
                }));
            block = block.push(crate::keymap::guard(ctx, field));
        }
    }
    let answered = form.answered(questions, at);
    let mut actions = row![].spacing(tokens::space::MD).align_y(Alignment::Center);
    if at > 0 {
        actions = actions.push(
            ui_button::secondary(t, ctx.t(Key::ChatConvQuestionBack)).on_press_maybe(free.then(
                || {
                    msg(Msg::Step {
                        ws: ws.id.clone(),
                        to: at - 1,
                    })
                },
            )),
        );
    }
    // Why the button waits, said rather than a button that does nothing.
    let waits: Element<'a, Message> = if answered {
        Space::new().width(Length::Fill).into()
    } else {
        container(hangar_ui::text::hint(
            t,
            ctx.t(Key::ChatConvQuestionNeedsAnswer),
        ))
        .width(Length::Fill)
        .into()
    };
    actions = actions.push(waits);
    actions = actions.push(if last {
        let ready = form.answers(questions).is_some();
        ui_button::primary(t, ctx.t(Key::ChatConvQuestionSubmit))
            .icon(Icon::Send)
            .on_press_maybe((ready && free).then(|| msg(Msg::SubmitAnswer(ws.id.clone()))))
    } else {
        ui_button::primary(t, ctx.t(Key::ChatConvQuestionNext))
            .icon_after(Icon::ChevronRight)
            .on_press_maybe((answered && free).then(|| {
                msg(Msg::Step {
                    ws: ws.id.clone(),
                    to: at + 1,
                })
            }))
    });
    CardParts {
        title: title.into(),
        body: block.into(),
        actions: actions.into(),
        scroll: iced::widget::Id::from(format!("chat-view-question-{}-{at}", form.tool_use_id)),
    }
}

/// The plan the pane presents, read as Claude's messages are, then
/// approved, or sent back to keep planning with what to change in it.
fn plan_card<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    pane: &'a Pane,
    chat: &'a Chat,
    prompt: &'a str,
) -> CardParts<'a> {
    let t = ctx.theme;
    let free = !chat.deciding;
    let body: Element<'a, Message> = match chat.prose.get(&plan_key(prompt)) {
        Some((_, content)) => rich::view(t, content, &chat.code),
        None => Space::new().into(),
    };
    let ws_id = ws.id.clone();
    let feedback = text_input(&ctx.t(Key::ChatConvPlanFeedback), &chat.plan_feedback)
        .size(tokens::font_size::UI)
        .padding([tokens::space::SM, tokens::space::MD])
        .width(Length::Fill)
        .style(style::text_input::field(t))
        .on_input_maybe(free.then_some(move |words| msg(Msg::PlanFeedback(ws_id.clone(), words))))
        .on_submit_maybe(free.then(|| msg(Msg::Plan(ws.id.clone(), false))));
    let actions = column![
        crate::keymap::guard(ctx, feedback),
        row![
            ui_button::primary(t, ctx.t(Key::ChatConvPlanApprove))
                .icon(Icon::Check)
                .size(ui_button::Size::Large)
                .on_press_maybe(free.then(|| msg(Msg::Plan(ws.id.clone(), true)))),
            ui_button::secondary(t, ctx.t(Key::ChatConvPlanKeep))
                .size(ui_button::Size::Large)
                .on_press_maybe(free.then(|| msg(Msg::Plan(ws.id.clone(), false)))),
        ]
        .spacing(tokens::space::MD)
        .align_y(Alignment::Center),
    ]
    .spacing(tokens::space::MD);
    CardParts {
        title: card_title(
            t,
            Icon::Flag,
            ctx.i18n
                .t_with(Key::ChatConvPlanTitle, &args!("name" => pane.name.as_str()))
                .into_owned(),
        ),
        body,
        actions: actions.into(),
        scroll: iced::widget::Id::from(format!("chat-view-plan-{prompt}")),
    }
}

fn permission_card<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    pane: &'a Pane,
    chat: &'a Chat,
    tool: &'a str,
    title: &'a str,
    detail: Option<&'a str>,
) -> CardParts<'a> {
    let t = ctx.theme;
    let free = !chat.deciding;
    let tasks = ctx.model.live.board.tasks(&ws.cwd);
    let task_title = move |id: &str| {
        tasks?
            .iter()
            .find(|task| task.id == id)
            .map(|task| task.title.clone())
    };
    let what = titles::title(ctx.i18n, tool, title, detail, &task_title);
    let mut body = column![
        row![
            container(icon(call_icon(tool), CALL_ICON).color(t.palette.text_muted))
                .width(CALL_ICON)
                .center_x(CALL_ICON),
            text(what)
                .size(tokens::font_size::UI_LG)
                .width(Length::Fill)
                .style(style::text::secondary(t)),
        ]
        .spacing(tokens::space::MD)
        .align_y(Alignment::Center),
    ]
    .spacing(tokens::space::MD);
    if let Some(detail) = detail {
        body = body.push(mono_block(
            t,
            clipped(&titles::detail(tool, detail), DETAIL_LINES),
        ));
    }
    let actions = row![
        ui_button::primary(t, ctx.t(Key::ChatConvPermissionAllow))
            .size(ui_button::Size::Large)
            .on_press_maybe(free.then(|| msg(Msg::Permit(ws.id.clone(), true)))),
        ui_button::text_button(t, ctx.t(Key::ChatConvPermissionDeny))
            .size(ui_button::Size::Large)
            .on_press_maybe(free.then(|| msg(Msg::Permit(ws.id.clone(), false)))),
    ]
    .spacing(tokens::space::MD)
    .align_y(Alignment::Center);
    CardParts {
        title: card_title(
            t,
            Icon::Lock,
            ctx.i18n
                .t_with(
                    Key::ChatConvPermissionTitle,
                    &args!("name" => pane.name.as_str(), "tool" => tool_name(tool)),
                )
                .into_owned(),
        ),
        body: body.into(),
        actions: actions.into(),
        scroll: iced::widget::Id::new("chat-view-permission"),
    }
}

/// The words that say why the input is disabled.
pub fn blocked_words(ctx: Ctx<'_>, pane: &Pane, why: Blocked) -> String {
    let name = args!("name" => pane.name.as_str());
    match why {
        Blocked::Loading => ctx.t(Key::ChatConvDisabledLoading).into_owned(),
        Blocked::NoConversation => ctx
            .i18n
            .t_with(Key::ChatConvDisabledNotClaude, &name)
            .into_owned(),
        Blocked::Answering => ctx
            .i18n
            .t_with(Key::ChatConvDisabledAnswering, &name)
            .into_owned(),
        Blocked::Resuming => ctx
            .i18n
            .t_with(
                Key::ChatConvDisabledResuming,
                &args!("name" => pane.name.as_str(), "agent" => pane.agent.info().label),
            )
            .into_owned(),
        Blocked::Question => ctx.t(Key::ChatConvDisabledQuestion).into_owned(),
        Blocked::Permission => ctx.t(Key::ChatConvDisabledPermission).into_owned(),
    }
}

/// Enter sends, Shift+Enter breaks the line; everything else as the editor
/// does it.
fn keys(ws: String) -> impl Fn(KeyPress) -> Option<Binding<Message>> {
    move |press: KeyPress| {
        let enter = matches!(press.key, keyboard::Key::Named(Named::Enter));
        if enter && matches!(press.status, Status::Focused { .. }) && !press.modifiers.shift() {
            return Some(Binding::Custom(msg(Msg::Send(ws.clone()))));
        }
        Binding::from_key_press(press)
    }
}

/// The editor inside the composer: no field of its own, the composer is.
fn bare_editor(t: &Theme) -> impl Fn(&iced::Theme, Status) -> text_editor::Style + '_ {
    move |_, status| {
        let disabled = matches!(status, Status::Disabled);
        let fade = |color: Color| tint(color, if disabled { 0.6 } else { 1.0 });
        text_editor::Style {
            background: Background::Color(Color::TRANSPARENT),
            border: Border::default(),
            placeholder: fade(t.palette.text_muted),
            value: fade(t.palette.text),
            selection: t.palette.selection_bg,
        }
    }
}

/// What a round button shows: an icon, or Stop's square.
enum Glyph {
    Icon(Icon),
    Square,
}

/// A round button with just a glyph: Send, Stop.
fn round<'a>(
    t: &'a Theme,
    glyph: Glyph,
    primary: bool,
    press: Option<Message>,
) -> Element<'a, Message> {
    let on = press.is_some();
    let colour = match (primary, on) {
        (true, true) => t.palette.on_accent,
        (true, false) => tint(t.palette.on_accent, 0.7),
        (false, true) => t.palette.text,
        (false, false) => t.palette.text_muted,
    };
    let mark: Element<'a, Message> = match glyph {
        Glyph::Icon(glyph) => icon(glyph, 14.0).color(colour).into(),
        Glyph::Square => container(Space::new())
            .width(10.0)
            .height(10.0)
            .style(move |_| container::Style {
                background: Some(Background::Color(colour)),
                border: Border {
                    radius: 2.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into(),
    };
    let round = button(container(mark).center_x(ROUND).center_y(ROUND))
        .padding(0)
        .style(move |theme, status| {
            let mut look = if primary {
                style::button::primary(t)(theme, status)
            } else {
                style::button::secondary(t)(theme, status)
            };
            look.border.radius = tokens::radius::FULL.into();
            if primary && !on {
                look.background = Some(Background::Color(tint(t.palette.accent, 0.35)));
            }
            look
        });
    ringed(t, round, press, tokens::radius::FULL)
}

/// The composer: the input, and under it what to know before sending, Stop
/// while the orchestrator works, Send; or, while a question or a permission
/// waits, its card instead.
/// A small button of the input's settings: on, or not.
fn chip<'a>(
    t: &'a Theme,
    words: String,
    open: Option<bool>,
    on: bool,
    press: Option<Message>,
) -> Element<'a, Message> {
    let mut inside = row![text(words).size(tokens::scale::type_scale::CAPTION.size)]
        .spacing(tokens::space::SM)
        .height(Length::Fill)
        .align_y(Alignment::Center);
    if let Some(open) = open {
        inside = inside.push(
            icon(
                if open {
                    Icon::ChevronUp
                } else {
                    Icon::ChevronDown
                },
                10.0,
            )
            .color(t.palette.text_muted),
        );
    }
    let chip = button(inside)
        .padding([0.0, tokens::space::LG])
        .height(CHIP_HEIGHT)
        .style(move |theme, status| {
            let mut look = style::button::toggle(t, on)(theme, status);
            look.border.radius = tokens::radius::SM.into();
            if !on {
                look.border.color = t.palette.border;
                look.border.width = tokens::BORDER_WIDTH;
            }
            look
        });
    ringed(t, chip, press, tokens::radius::SM)
}

/// The words of an effort level.
fn effort_words(ctx: Ctx<'_>, level: &str) -> String {
    let key = match level {
        "low" => Key::ChatConvTuneLow,
        "medium" => Key::ChatConvTuneMedium,
        "high" => Key::ChatConvTuneHigh,
        "xhigh" => Key::ChatConvTuneXhigh,
        "max" => Key::ChatConvTuneMax,
        other => return other.to_string(),
    };
    ctx.t(key).into_owned()
}

/// The session's model, its effort and plan mode, by the input: each opens
/// its choices above it (Plan is one click), and a choice is the command
/// that sets it, run through the session's queue.
fn settings_chips<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    pane: &'a Pane,
    chat: &'a Chat,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let insight = ctx.model.live.panes_logic.insight(&pane.id);
    let picker = |which: super::Picker| msg(Msg::Picker(ws.id.clone(), Some(which)));
    let model = insight
        .model
        .as_deref()
        .map(crate::stats::model::model_label)
        .unwrap_or_else(|| ctx.t(Key::ChatConvTuneModel).into_owned());
    let effort = match insight.effort.as_deref() {
        Some(level) => ctx
            .i18n
            .t_with(
                Key::ChatConvTuneEffort,
                &args!("level" => effort_words(ctx, level)),
            )
            .into_owned(),
        None => ctx.t(Key::ChatConvTuneEffortUnknown).into_owned(),
    };
    let planning = insight.permission_mode.as_deref() == Some("plan");
    let model_open = chat.picker == Some(super::Picker::Model);
    let effort_open = chat.picker == Some(super::Picker::Effort);
    let plan_hint = if planning {
        Key::ChatConvTunePlanOnHint
    } else {
        Key::ChatConvTunePlanHint
    };
    row![
        hangar_ui::tip::over(
            t,
            chip(
                t,
                model,
                Some(model_open),
                model_open,
                Some(picker(super::Picker::Model))
            ),
            ctx.t(Key::ChatConvTuneModelHint).into_owned(),
        ),
        chip(
            t,
            effort,
            Some(effort_open),
            effort_open,
            Some(picker(super::Picker::Effort))
        ),
        hangar_ui::tip::over(
            t,
            chip(
                t,
                ctx.t(Key::ChatConvTunePlan).into_owned(),
                None,
                planning,
                (!planning).then(|| msg(Msg::Tune(ws.id.clone(), "/plan".into()))),
            ),
            ctx.t(plan_hint).into_owned(),
        ),
    ]
    .spacing(tokens::space::SM)
    .align_y(Alignment::Center)
    .into()
}

/// The choices of the setting opened by the input, the current one on.
fn choices<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    pane: &'a Pane,
    picker: super::Picker,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let insight = ctx.model.live.panes_logic.insight(&pane.id);
    let tune = |command: String| Some(msg(Msg::Tune(ws.id.clone(), command)));
    let options: Vec<Element<'a, Message>> = match picker {
        super::Picker::Model => super::MODELS
            .iter()
            .map(|(alias, name)| {
                let on = insight
                    .model
                    .as_deref()
                    .is_some_and(|model| model.contains(alias));
                chip(
                    t,
                    (*name).to_string(),
                    None,
                    on,
                    tune(format!("/model {alias}")),
                )
            })
            .collect(),
        super::Picker::Effort => super::EFFORTS
            .iter()
            .map(|level| {
                let on = insight.effort.as_deref() == Some(*level);
                chip(
                    t,
                    effort_words(ctx, level),
                    None,
                    on,
                    tune(format!("/effort {level}")),
                )
            })
            .collect(),
    };
    container(
        iced::widget::Row::with_children(options)
            .spacing(tokens::space::XS)
            .align_y(Alignment::Center),
    )
    .padding(Padding {
        top: tokens::space::MD,
        right: tokens::space::MD,
        bottom: 0.0,
        left: tokens::space::XL,
    })
    .into()
}

/// The longest a prompt waiting in the queue is shown, on one line.
const QUEUED_WORDS: usize = 120;

/// What the person sent while the orchestrator works, waiting for its turn
/// to end, in a dashed frame over the input: each can be taken back, or taken back
/// into the input to be changed; "Send now" stops the turn, and the first
/// one goes at once.
fn queue<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    pane: &'a Pane,
    chat: &'a Chat,
    works: bool,
) -> Option<Element<'a, Message>> {
    let t = ctx.theme;
    let waiting: Vec<_> = super::waiting(ctx.model, chat).collect();
    if waiting.is_empty() {
        return None;
    }
    let title = ctx.i18n.t_with(
        Key::ChatConvQueueTitle,
        &args!("name" => pane.name.as_str()),
    );
    let mut head = row![
        text(format!("{} · {}", title.to_uppercase(), waiting.len()))
            .size(tokens::scale::type_scale::LABEL.size)
            .font(fonts::mono())
            .width(Length::Fill)
            .style(style::text::muted(t)),
    ]
    .spacing(tokens::space::MD)
    .align_y(Alignment::Center);
    if works {
        head = head.push(hangar_ui::tip::over(
            t,
            ui_button::ghost(t, ctx.t(Key::ChatConvQueueSendNow).into_owned())
                .on_press_maybe((!chat.stopping).then(|| msg(Msg::Stop(ws.id.clone())))),
            ctx.t(Key::ChatConvQueueSendNowHint).into_owned(),
        ));
    }
    let mut list = column![head].spacing(tokens::space::XS);
    for queued in waiting {
        let id = queued.id.clone();
        list = list.push(
            row![
                icon(Icon::Clock, CALL_ICON).color(t.palette.text_muted),
                text(hangar::first_line(&queued.text, QUEUED_WORDS))
                    .size(tokens::font_size::UI)
                    .width(Length::Fill)
                    .style(style::text::secondary(t)),
                ui_button::icon_button(t, Icon::Pencil)
                    .side(24.0)
                    .tip(ctx.t(Key::ChatConvQueueEdit).into_owned())
                    .on_press(msg(Msg::Unqueue {
                        ws: ws.id.clone(),
                        id: id.clone(),
                        edit: true,
                    })),
                ui_button::icon_button(t, Icon::Close)
                    .side(24.0)
                    .tip(ctx.t(Key::ChatConvQueueRemove).into_owned())
                    .on_press(msg(Msg::Unqueue {
                        ws: ws.id.clone(),
                        id,
                        edit: false,
                    })),
            ]
            .spacing(tokens::space::MD)
            .align_y(Alignment::Center),
        );
    }
    let frame = iced::widget::canvas(Dashed {
        colour: t.palette.border_strong,
        radius: tokens::scale::radius::MD,
    })
    .width(Length::Fill)
    .height(Length::Fill);
    Some(
        Stack::new()
            .push(
                container(list)
                    .padding([tokens::space::MD, tokens::space::LG])
                    .width(Length::Fill),
            )
            .push(frame)
            .into(),
    )
}

fn composer<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    pane: &'a Pane,
    chat: &'a Chat,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let why = blocked(chat, pane);
    let placeholder = ctx
        .i18n
        .t_with(
            Key::ChatConvPlaceholder,
            &args!("name" => pane.name.as_str()),
        )
        .into_owned();
    let mut editor = text_editor(&chat.draft)
        .id(input_id())
        .placeholder(placeholder)
        .size(tokens::font_size::BODY)
        .padding(Padding {
            top: tokens::space::XL,
            right: tokens::space::XL,
            bottom: tokens::space::XS,
            left: tokens::space::XL,
        })
        .min_height(48.0)
        .max_height(220.0)
        .style(bare_editor(t));
    if why.is_none() {
        let ws_id = ws.id.clone();
        editor = editor
            .on_action(move |action| msg(Msg::Edit(ws_id.clone(), action)))
            .key_binding(keys(ws.id.clone()));
    }
    let can_send = why.is_none() && !chat.draft.text().trim().is_empty();
    let under = match (&chat.stop_error, why) {
        (Some(error), _) => text(
            ctx.i18n
                .t_with(Key::ChatConvStopFailed, &args!("error" => error.as_str()))
                .into_owned(),
        )
        .size(tokens::font_size::CAPTION)
        .style(style::text::danger(t)),
        (None, Some(why)) => text(blocked_words(ctx, pane, why))
            .size(tokens::font_size::CAPTION)
            .style(style::text::warning(t)),
        (None, None) if super::queueing(ctx.model, pane) => text(
            ctx.i18n
                .t_with(Key::ChatConvQueueHint, &args!("name" => pane.name.as_str()))
                .into_owned(),
        )
        .size(tokens::font_size::CAPTION)
        .style(style::text::subtle(t)),
        (None, None) => text(ctx.t(Key::ChatConvInputHint))
            .size(tokens::font_size::CAPTION)
            .style(style::text::subtle(t)),
    };
    let mut footer = row![].spacing(tokens::space::MD).align_y(Alignment::Center);
    let tunable = super::tunable(ctx.model, pane);
    if tunable {
        footer = footer.push(settings_chips(ctx, ws, pane, chat));
    }
    footer = footer.push(container(under).width(Length::Fill));
    let works = ctx.model.live.panes_logic.activity_or_idle(&pane.id) == AgentActivity::Working;
    if works && chat.thread.pending.is_none() {
        let stop = round(
            t,
            Glyph::Square,
            false,
            (!chat.stopping).then(|| msg(Msg::Stop(ws.id.clone()))),
        );
        footer = footer.push(hangar_ui::tip::over(
            t,
            stop,
            ctx.t(Key::ChatConvStopHint).into_owned(),
        ));
    }
    footer = footer.push(round(
        t,
        Glyph::Icon(Icon::Send),
        true,
        can_send.then(|| msg(Msg::Send(ws.id.clone()))),
    ));
    // What waits on the person takes the input's place: nothing can be sent
    // until it is answered, and the card says what to do.
    let mut stack = column![].spacing(tokens::space::MD);
    match card(ctx, ws, pane, chat) {
        Some(card) => stack = stack.push(card),
        None => {
            if let Some(queue) = queue(ctx, ws, pane, chat, works) {
                stack = stack.push(queue);
            }
            let mut parts = column![];
            if let Some(choices) = chat
                .picker
                .filter(|_| tunable)
                .map(|picker| choices(ctx, ws, pane, picker))
            {
                parts = parts.push(choices);
            }
            let parts =
                parts
                    .push(crate::keymap::guard(ctx, editor))
                    .push(container(footer).padding(Padding {
                        top: 0.0,
                        right: tokens::space::XL,
                        bottom: tokens::space::LG,
                        left: tokens::space::XL,
                    }));
            stack =
                stack.push(
                    container(parts)
                        .width(Length::Fill)
                        .style(move |_| container::Style {
                            background: Some(Background::Color(t.palette.surface)),
                            border: Border {
                                color: t.palette.border_strong,
                                width: tokens::BORDER_WIDTH,
                                radius: tokens::scale::radius::LG.into(),
                            },
                            ..container::Style::default()
                        }),
                );
        }
    }
    centred(container(stack).padding(Padding {
        top: tokens::space::MD,
        right: tokens::space::XXXL,
        bottom: tokens::space::XL + tokens::space::XXS,
        left: tokens::space::XXXL,
    }))
}
