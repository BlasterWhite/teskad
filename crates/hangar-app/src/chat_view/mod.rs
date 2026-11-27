//! The chat view of a workspace (experiment `chat-view`): in place of its
//! grid, the conversation on the left, the panes that are not terminals (the
//! tools area: previews, the 3D view, kit tabs) beside it when it has some,
//! and its tasks in a column on the right.
//!
//! ```text
//! conversation │ tools (view panes, when any) │ tasks
//! ```
//!
//! It is per workspace and reversible: `Workspace.chat_view`, set with
//! `StoreAction::SetChatView`, and only while the experiment is on, so
//! turning the experiment off shows every grid again without touching the
//! file. Nothing restarts either way: the terminals live in the panes'
//! state, not in the grid, and go on running hidden (`panes::on_screen`
//! keeps them off screen: no keyboard, no resume ahead of the others). The
//! view panes are drawn by the panes' own widget (`panes::view_tools`), so
//! the previews and the kits do not know where they are shown.
//!
//! The conversation and the tasks are their own modules, each with its
//! State and Msg routed through this package; the conversation also hears
//! the core's events and has a tick of its own (`conversation`).

pub mod conversation;
pub mod tasks;

use hangar_theme::{style, tokens};
use iced::widget::{container, row, rule};
use iced::{Element, Length, Subscription, Task};

use crate::ctx::Ctx;
use crate::message::{Intent, Message};
use crate::services::{CoreEvent, Services};
use crate::settings::experiments;
use crate::state::{Model, StoreAction, Workspace, WsId};

/// The tasks panel's width, fixed: the conversation and the tools share the
/// rest.
pub const TASKS_WIDTH: f32 = 300.0;

/// The tasks fold to their icon under this width of the main area: a window
/// of 1180 px with the navigator shown.
const TASKS_FOLD_BELOW: f32 = 1180.0 - tokens::scale::size::SIDEBAR;

/// The least the conversation is given while the main area has that much.
const CONVERSATION_MIN_WIDTH: f32 = 480.0;

/// How the room left of the tasks is shared when there are tools: the
/// conversation's part, then the tools'.
const CONVERSATION_PORTION: u16 = 3;
const TOOLS_PORTION: u16 = 2;

#[derive(Debug, Default)]
pub struct State {
    pub conversation: conversation::State,
    pub tasks: tasks::State,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// The workspace shown as the chat view (`true`) or as its grid: the
    /// toolbar's switch, the palette's entry.
    SetShown(WsId, bool),
    Conversation(conversation::Msg),
    Tasks(tasks::Msg),
}

/// The experiment is on: the switch is offered.
pub fn available(m: &Model) -> bool {
    experiments::enabled(m.settings(), experiments::CHAT_VIEW)
}

/// Workspace `ws` is shown as the chat view rather than as its grid: the
/// person asked for it, and the experiment is on.
pub fn shown(m: &Model, ws: &Workspace) -> bool {
    ws.chat_view == Some(true) && available(m)
}

pub fn update(st: &mut State, m: &mut Model, sv: &Services, msg: Msg) -> Task<Message> {
    match msg {
        Msg::SetShown(ws, on) => {
            m.apply(StoreAction::SetChatView { ws: ws.clone(), on });
            if on {
                conversation::on_shown(&mut st.conversation, m, &ws)
            } else {
                Task::none()
            }
        }
        Msg::Conversation(msg) => conversation::update(&mut st.conversation, m, sv, msg),
        Msg::Tasks(msg) => tasks::update(&mut st.tasks, m, msg),
    }
}

pub fn subscription(st: &State, m: &Model) -> Subscription<Message> {
    Subscription::batch([
        conversation::subscription(&st.conversation, m),
        tasks::subscription(&st.tasks, m),
    ])
}

pub fn on_intent(
    _st: &mut State,
    _m: &mut Model,
    _sv: &Services,
    _intent: &Intent,
) -> Option<Task<Message>> {
    None
}

pub fn on_event(
    st: &mut State,
    m: &mut Model,
    _sv: &Services,
    event: &CoreEvent,
) -> Option<Task<Message>> {
    let tasks: Vec<Task<Message>> = [
        conversation::on_event(&mut st.conversation, m, event),
        tasks::on_event(&mut st.tasks, m, event),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!tasks.is_empty()).then(|| Task::batch(tasks))
}

/// The chat view of `ws`, filling the main area.
pub fn view<'a>(ctx: Ctx<'a>, ws: &'a Workspace) -> Element<'a, Message> {
    let st = &ctx.pkg.chat_view;
    let narrow = ctx.metrics.body_width < TASKS_FOLD_BELOW;
    let tasks_width = if st.tasks.folded(narrow) {
        tasks::RAIL_WIDTH
    } else {
        TASKS_WIDTH
    };
    let left = (ctx.metrics.body_width - tasks_width).max(0.0);
    let shared =
        left * f32::from(CONVERSATION_PORTION) / f32::from(CONVERSATION_PORTION + TOOLS_PORTION);
    // Its part of the room, never under its least: the tools take the rest.
    let least = shared < CONVERSATION_MIN_WIDTH;
    let conversation_width = shared.max(CONVERSATION_MIN_WIDTH.min(left));
    let tools = crate::panes::view_tools(ctx, ws, left - conversation_width);

    let conversation = container(conversation::view(&st.conversation, ctx, ws))
        .width(match (&tools, least) {
            (Some(_), true) => Length::Fixed(conversation_width),
            (Some(_), false) => Length::FillPortion(CONVERSATION_PORTION),
            (None, _) => Length::Fill,
        })
        .height(Length::Fill);
    let mut regions = row![conversation].height(Length::Fill);
    if let Some(tools) = tools {
        regions = regions.push(sep(ctx)).push(
            container(tools)
                .width(if least {
                    Length::Fill
                } else {
                    Length::FillPortion(TOOLS_PORTION)
                })
                .height(Length::Fill),
        );
    }
    regions
        .push(sep(ctx))
        .push(
            container(tasks::view(&st.tasks, ctx, ws, narrow))
                .width(tasks_width)
                .height(Length::Fill)
                .style(style::container::plain(ctx.theme)),
        )
        .into()
}

fn sep<'a>(ctx: Ctx<'a>) -> Element<'a, Message> {
    rule::vertical(tokens::BORDER_WIDTH)
        .style(style::rule::sep(ctx.theme))
        .into()
}

#[cfg(test)]
mod tests;
