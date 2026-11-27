//! P6: Status bar (`StatusBar.tsx`).
//!
//! The bar along the bottom of the workspace: the broadcast field while the
//! terminals are on screen, the folder and how its agents are doing otherwise,
//! and on the right what is true of the whole workspace (its git branch, the
//! dictation keys, how many panes).
//!
//! The git branch is read again every 20 seconds while the window has the
//! focus, and as soon as the workspace or the focus changes: a branch changed
//! in another program is the likeliest thing to have moved meanwhile. A window
//! nobody is looking at asks nothing at all: one git process every 20 seconds
//! for a bar nobody can see is not nothing.
//!
//! Every window has a bar of its own, about its own workspace: the line typed
//! in one is not the other's, and a broadcast reaches the panes of the
//! workspace its window draws, never the main window's active one.

pub mod tail;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Duration;

use iced::widget::{Id, Space, container, responsive, row, text, text_input};
use iced::{Alignment, Element, Event, Length, Padding, Subscription, Task, window};

use hangar_core::git::RepoInfo;
use hangar_i18n::{Key, args};
use hangar_theme::tokens::{self, font_size, space};
use hangar_theme::{fonts, style};
use hangar_ui::{Icon, button, icon, keycap, keys, tip};

use crate::commands::CommandId;
use crate::ctx::Ctx;
use crate::message::{Intent, Message};
use crate::navigator::{reads, skin};
use crate::services::{CoreEvent, Services};
use crate::shell::{Focus, Region, View};
use crate::state::{AgentActivity, Model, WsId};

/// The bar's height under the terminals, where it holds the broadcast field
/// (`ecrans.md`, « Barre d'envoi groupé »); elsewhere it keeps
/// [`tokens::size::STATUS_BAR_HEIGHT`].
pub const BROADCAST_HEIGHT: f32 = 52.0;
/// The broadcast field's height.
const FIELD_HEIGHT: f32 = 34.0;

/// Whether the window's bar holds the broadcast field: over the terminals.
/// The chat view has its own input: the field would type into terminals that
/// are not on screen.
fn broadcasts(ctx: Ctx<'_>, ws: &crate::state::Workspace) -> bool {
    ctx.view() == View::Terminals && !crate::chat_view::shown(ctx.model, ws)
}

/// How tall the window's bar is.
pub fn height(ctx: Ctx<'_>) -> f32 {
    if ctx.workspace().is_some_and(|ws| broadcasts(ctx, ws)) {
        BROADCAST_HEIGHT
    } else {
        tokens::size::STATUS_BAR_HEIGHT
    }
}

/// Often enough to follow an agent switching branches, rarely enough to cost
/// nothing.
const GIT_REFRESH: Duration = Duration::from_secs(20);

/// The broadcast field of a window, which the `view.broadcast` shortcut
/// focuses: one per window, so that focusing it in one never lands in another.
fn input_id(window: window::Id) -> Id {
    Id::from(format!("hangar-status-broadcast-{window:?}"))
}

/// What the bar shows of the repository the workspace folder is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// `None` when the head is detached.
    pub branch: Option<String>,
    /// Short sha.
    pub head: Option<String>,
    pub root: Option<String>,
    pub is_linked_worktree: bool,
}

impl Repo {
    /// The repository, or `None` when the folder is in none, or git is not
    /// installed.
    pub fn of(info: RepoInfo) -> Option<Repo> {
        (info.installed && info.is_repo).then_some(Repo {
            branch: info.branch,
            head: info.head,
            root: info.root,
            is_linked_worktree: info.is_linked_worktree,
        })
    }
}

/// The package's UI state; lives in `App.pkg.status_bar`.
#[derive(Debug, Default)]
pub struct State {
    /// What git said of each folder a bar was asked about, so an answer for
    /// the workspace shown before is not taken for this one's, and two
    /// windows on two folders each read their own. `None`: no repository.
    repos: HashMap<String, Option<Repo>>,
    /// The windows that lost the system's focus. Git is only asked for a
    /// window that has it; one that never said it lost it counts as having
    /// it, as every window does at start.
    unfocused: HashSet<window::Id>,
    /// What each window's broadcast field holds. Not persisted, and not shared
    /// between windows: each one's App had its own (`useState("")`). This is
    /// the only copy: a send carries the text in `Intent::Broadcast`, and
    /// nothing else (the palette's `view.broadcast`, the phone's ops) reads
    /// the field. A window that closes takes its line with it.
    broadcast: HashMap<window::Id, String>,
}

impl State {
    /// A repository read in; the same branch, head and root keep the object
    /// that was there. Returns whether anything changed.
    fn take_repo(&mut self, cwd: &str, repo: Option<Repo>) -> bool {
        if self.repos.get(cwd) == Some(&repo) {
            return false;
        }
        self.repos.insert(cwd.to_string(), repo);
        true
    }

    /// What the window's broadcast field holds.
    pub fn broadcast_text(&self, window: window::Id) -> &str {
        self.broadcast.get(&window).map_or("", String::as_str)
    }

    /// A window closed: its line and its focus are nobody's any more.
    fn forget(&mut self, window: window::Id) {
        self.unfocused.remove(&window);
        self.broadcast.remove(&window);
    }

    fn set_broadcast_text(&mut self, window: window::Id, text: String) {
        if text.is_empty() {
            self.broadcast.remove(&window);
        } else {
            self.broadcast.insert(window, text);
        }
    }
}

/// The package's shared read-model, which other packages may read; lives in
/// `Model.live.status_bar`.
#[derive(Debug, Default)]
pub struct Live;

/// The package's messages, routed by the shell as `Message::StatusBar(Msg)`.
///
/// A message names the window it came from, and a send the workspace that
/// window drew: when it is handled, the focused window or the main window's
/// active workspace may be others.
#[derive(Debug, Clone)]
pub enum Msg {
    /// The broadcast field of a window was typed in.
    Input(window::Id, String),
    /// Enter sends; Alt+Enter, and the *When free* button, hold the line until
    /// each agent is done.
    Send {
        window: window::Id,
        ws: WsId,
        when_free: bool,
    },
    /// Time to ask git about this folder.
    Poll(String),
    Repo(String, Option<Repo>),
    /// A window gained or lost the system's focus.
    Focus(window::Id, bool),
    /// A window closed: what it held is let go.
    Closed(window::Id),
}

/// What a send comes to: the line the window holds, for the workspace that
/// window drew when the button was pressed, which must still exist. The field
/// empties; a line with nothing in it, or for a workspace that is gone, sends
/// nothing and leaves the field as it is.
fn send(
    st: &mut State,
    m: &Model,
    window: window::Id,
    ws: WsId,
    when_free: bool,
) -> Option<Intent> {
    let text = st.broadcast_text(window).trim().to_string();
    if text.is_empty() || m.workspace(&ws).is_none() {
        return None;
    }
    st.set_broadcast_text(window, String::new());
    Some(Intent::Broadcast {
        ws,
        text,
        when_free,
    })
}

pub fn update(st: &mut State, m: &mut Model, _sv: &Services, msg: Msg) -> Task<Message> {
    step(st, m, msg)
}

/// What a message does; nothing of it needs the services.
pub(crate) fn step(st: &mut State, m: &mut Model, msg: Msg) -> Task<Message> {
    match msg {
        Msg::Input(window, value) => {
            st.set_broadcast_text(window, value);
            Task::none()
        }
        Msg::Send {
            window,
            ws,
            when_free,
        } => match send(st, m, window, ws, when_free) {
            Some(intent) => Task::done(Message::Intent(intent)),
            None => Task::none(),
        },
        Msg::Poll(cwd) => {
            Task::perform(hangar_core::git::git_repo_info(cwd.clone()), move |info| {
                Message::StatusBar(Msg::Repo(cwd.clone(), Repo::of(info)))
            })
        }
        Msg::Repo(cwd, repo) => {
            st.take_repo(&cwd, repo);
            Task::none()
        }
        Msg::Focus(window, on) => {
            if on {
                st.unfocused.remove(&window);
            } else {
                st.unfocused.insert(window);
            }
            Task::none()
        }
        Msg::Closed(window) => {
            st.forget(window);
            Task::none()
        }
    }
}

/// The git beat of the workspace of each window that has the focus (an
/// immediate read, then every 20 seconds), and the windows' focus. The beat is
/// a subscription of its own for each folder, so a change of workspace
/// restarts it, which is the read the TS did when `cwd` changed. Two windows
/// on one folder share a beat.
pub fn subscription(st: &State, m: &Model) -> Subscription<Message> {
    let focus = iced::event::listen_with(|event, _status, window| match event {
        Event::Window(window::Event::Focused) => Some(Message::StatusBar(Msg::Focus(window, true))),
        Event::Window(window::Event::Unfocused) => {
            Some(Message::StatusBar(Msg::Focus(window, false)))
        }
        Event::Window(window::Event::Closed) => Some(Message::StatusBar(Msg::Closed(window))),
        _ => None,
    });
    Subscription::batch(
        std::iter::once(focus).chain(
            beats(st, m)
                .into_iter()
                .map(|cwd| Subscription::run_with(cwd, beat)),
        ),
    )
}

/// The folders to ask git about: those of the workspaces the windows with the
/// system's focus draw, each once.
fn beats(st: &State, m: &Model) -> BTreeSet<String> {
    m.live
        .windows
        .keys()
        .filter(|window| !st.unfocused.contains(window))
        .filter_map(|window| m.window_workspace(*window))
        .map(|ws| ws.cwd.clone())
        .collect()
}

// `Subscription::run_with` hands the builder a reference to its data, which
// is the `String` the subscription is identified by.
#[allow(clippy::ptr_arg)]
fn beat(cwd: &String) -> impl iced::futures::Stream<Item = Message> + use<> {
    let cwd = cwd.clone();
    iced::futures::stream::unfold(true, move |first| {
        let cwd = cwd.clone();
        async move {
            if !first {
                tokio::time::sleep(GIT_REFRESH).await;
            }
            Some((Message::StatusBar(Msg::Poll(cwd)), false))
        }
    })
}

/// F6 reaches the broadcast field of the window the person is in, which is
/// only there over the terminals.
pub fn focus_entry(_st: &State, m: &Model, _region: Region) -> Option<Task<Message>> {
    let window = m.focused_window()?;
    let chat = m
        .window_workspace(window)
        .is_some_and(|ws| crate::chat_view::shown(m, ws));
    (m.view_in(window) == View::Terminals && !chat)
        .then(|| iced::widget::operation::focus(input_id(window)))
}

/// A dictation lands in the broadcast field when that is where the keyboard
/// is, joined to what is already typed.
pub fn on_intent(
    st: &mut State,
    m: &mut Model,
    _sv: &Services,
    intent: &Intent,
) -> Option<Task<Message>> {
    let Intent::InsertTranscript(words) = intent else {
        return None;
    };
    let window = m.focused_window()?;
    if m.ui.focus != Some(Focus::Region(Region::Status)) || m.view_in(window) != View::Terminals {
        return None;
    }
    let typed = st.broadcast_text(window);
    let joined = if typed.is_empty() {
        words.clone()
    } else {
        format!("{typed} {words}")
    };
    st.set_broadcast_text(window, joined);
    Some(Task::none())
}

/// Nothing in the core's events is the bar's: which panes can hold a line
/// until their agent is free is the pane logic's (`bridged_count`).
pub fn on_event(
    _st: &mut State,
    _m: &mut Model,
    _sv: &Services,
    _event: &CoreEvent,
) -> Option<Task<Message>> {
    None
}

/// Region `status`: the broadcast input and the bar's readouts over the
/// terminals; the folder, the agents' news and the readouts elsewhere.
pub fn view<'a>(st: &'a State, ctx: Ctx<'a>) -> Element<'a, Message> {
    // The workspace this window draws: a workspace window's own, never the
    // main window's active one.
    let Some(ws) = ctx.workspace() else {
        return Space::new().into();
    };
    if !broadcasts(ctx, ws) {
        return info(st, ctx, ws);
    }
    let t = ctx.theme;

    let mut items = row![].spacing(2).align_y(Alignment::Center);
    // Under the broadcast field the count is words alone, as in the mockup.
    for item in [
        branch(st, ctx, ws),
        dictation(ctx),
        Some(panes(ctx, ws, false)),
    ]
    .into_iter()
    .flatten()
    {
        items = items.push(item.element);
    }

    let right = row![
        container(Space::new())
            .width(1)
            .height(18)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(t.palette.border)),
                ..container::Style::default()
            }),
        items,
    ]
    .spacing(space::SM)
    .align_y(Alignment::Center);

    row![broadcast(st, ctx, ws), right]
        .spacing(space::LG)
        .padding(Padding {
            top: 0.0,
            right: space::MD,
            bottom: 0.0,
            left: space::LG,
        })
        .height(Length::Fill)
        .align_y(Alignment::Center)
        .into()
}

/// The size of the bar's words: mono, quiet.
const WORDS: f32 = font_size::MICRO;
/// A readout's padding on each side, the side of its icon and the gap after
/// it.
const PAD: f32 = 7.0;
const ICON: f32 = 13.0;
const GAP: f32 = 5.0;
/// The bar's own padding and the gap between the folder and the readouts.
const EDGE: f32 = 8.0;
/// Under this room the folder says too little to be worth showing.
const FOLDER_ROOM: f32 = 96.0;

/// One readout of the right-hand side, and how wide it is drawn.
struct Item<'a> {
    element: Element<'a, Message>,
    width: f32,
}

/// How wide `words` are with an icon before them, in a readout.
fn item_width(words: &str) -> f32 {
    2.0 * PAD + ICON + GAP + tail::path_width(words, WORDS)
}

fn mono<'a>(words: String) -> iced::widget::Text<'a> {
    text(words)
        .size(WORDS)
        .font(fonts::mono())
        .wrapping(text::Wrapping::None)
}

/// The branch the workspace folder is on, if it is in a repository.
fn branch<'a>(st: &State, ctx: Ctx<'a>, ws: &'a crate::state::Workspace) -> Option<Item<'a>> {
    let t = ctx.theme;
    let tr = ctx.i18n;
    let repo = st.repos.get(&ws.cwd)?.as_ref()?;
    let root = repo.root.as_deref().unwrap_or(&ws.cwd);
    let hint = match &repo.branch {
        Some(branch) => tr.t_with(
            Key::StatusBranch,
            &args!("branch" => branch.as_str(), "root" => root),
        ),
        None => tr.t_with(
            Key::StatusDetached,
            &args!("head" => repo.head.as_deref().unwrap_or("?"), "root" => root),
        ),
    };
    let name = repo
        .branch
        .as_deref()
        .or(repo.head.as_deref())
        .unwrap_or("")
        .to_string();
    let mut width = item_width(&name);
    let mut item = row![icon(Icon::Branch, ICON), mono(name)]
        .spacing(GAP)
        .align_y(Alignment::Center);
    if repo.is_linked_worktree {
        let word = tr.t(Key::StatusWorktree).into_owned();
        width += GAP + 8.0 + tail::path_width(&word, WORDS);
        item = item.push(
            container(mono(word))
                .padding([0.0, 4.0])
                .style(style::container::tag(t)),
        );
    }
    Some(Item {
        element: tip::over(t, readout(t, item), hint.into_owned()),
        width,
    })
}

/// The keys that dictate into the focused pane, while the voice is on.
fn dictation<'a>(ctx: Ctx<'a>) -> Option<Item<'a>> {
    let t = ctx.theme;
    if !ctx.model.settings().voice_enabled {
        return None;
    }
    let dictate = reads::binding(&ctx, CommandId::VoiceDictate);
    if dictate.is_empty() {
        return None;
    }
    let shown = hangar_ui::shortcut::format_binding(&dictate);
    let item = row![icon(Icon::Mic, ICON), mono(shown.clone())]
        .spacing(GAP)
        .align_y(Alignment::Center);
    Some(Item {
        width: item_width(&shown),
        element: tip::over(
            t,
            readout(t, item),
            ctx.i18n
                .t_with(Key::StatusDictate, &args!("keys" => shown.as_str()))
                .into_owned(),
        ),
    })
}

/// How many panes the workspace has, with their icon when `marked`.
fn panes<'a>(ctx: Ctx<'a>, ws: &'a crate::state::Workspace, marked: bool) -> Item<'a> {
    let t = ctx.theme;
    let words = ctx
        .i18n
        .tn(Key::StatusPanes, ws.panes.len(), &[])
        .into_owned();
    let width = item_width(&words);
    let mut line = row![].spacing(GAP).align_y(Alignment::Center);
    if marked {
        line = line.push(icon(Icon::Layout4, ICON));
    }
    line = line.push(mono(words));
    Item {
        element: tip::over(
            t,
            readout(t, line),
            ctx.t(Key::StatusPanesHint).into_owned(),
        ),
        width,
    }
}

/// A readout of the right-hand side: padded, muted.
fn readout<'a>(
    t: &'a hangar_theme::Theme,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .height(24)
        .padding([0.0, 7.0])
        .align_y(Alignment::Center)
        .style(move |_: &iced::Theme| container::Style {
            text_color: Some(t.palette.text_muted),
            ..container::Style::default()
        })
        .into()
}

/// The broadcast field and its two buttons.
fn broadcast<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    ws: &'a crate::state::Workspace,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let tr = ctx.i18n;
    let value = st.broadcast_text(ctx.window);
    let empty = value.trim().is_empty();
    let send = |when_free: bool| {
        Message::StatusBar(Msg::Send {
            window: ctx.window,
            ws: ws.id.clone(),
            when_free,
        })
    };
    let placeholder = tr
        .t_with(Key::BroadcastPlaceholder, &args!("n" => ws.panes.len()))
        .into_owned();
    let hint = reads::binding(&ctx, CommandId::ViewBroadcast);

    let window = ctx.window;
    let input = text_input("", value)
        .id(input_id(window))
        .size(font_size::UI)
        .padding(0)
        .width(Length::Fill)
        .on_input(move |value| Message::StatusBar(Msg::Input(window, value)))
        .on_submit(send(false))
        .style(style::text_input::bare(t));
    // Alt+Enter holds the line until each agent is done. The key is spent here
    // so the field's own Enter does not also send it at once.
    let held = send(true);
    let input = keys::on_field_key(input, move |key, modifiers| match key {
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter) if modifiers.alt() => {
            Some(held.clone())
        }
        _ => None,
    });
    // The keymap goes first: Ctrl+Shift+Enter typed in the field splits the
    // pane behind it, and must not also send the line. While the field has
    // the keyboard the status bar does, as after F6: no terminal takes what
    // is typed here, and a click in a pane takes the keyboard back.
    let input = crate::keymap::guard_in(ctx, Region::Status, input);
    // The placeholder is drawn by hand, in the secondary grey the stylesheet
    // gave it, since the field's own is a step fainter. The field keeps its
    // place under it whether it shows or not: drawn without the stack once
    // something was typed, the field was a new widget to iced, whose own
    // state (the keyboard with it) started again after the first character.
    let placeholder: Element<'a, Message> = if value.is_empty() {
        container(
            text(placeholder)
                .size(font_size::UI)
                .style(style::text::secondary(t)),
        )
        .align_y(Alignment::Center)
        .height(Length::Fill)
        .into()
    } else {
        Space::new().into()
    };
    let shown = iced::widget::stack![input, placeholder];

    // The broadcast mark inside the field at the left, its shortcut as key
    // caps at the right.
    let mut field = row![
        icon(Icon::Broadcast, 16.0).color(t.palette.text_muted),
        shown
    ]
    .align_y(Alignment::Center)
    .spacing(space::LG);
    if !hint.is_empty() {
        field = field.push(tip::over(
            t,
            Element::from(keycap::binding(t, hint)),
            tr.t(Key::CmdViewBroadcast).into_owned(),
        ));
    }
    let field = container(field)
        .height(FIELD_HEIGHT)
        .padding([0.0, space::LG])
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .style(field_style(t));

    // The panes whose Claude Code loaded the mod, which can hold a line until
    // it is free (`useBridgedCount`).
    let bridged = ctx
        .model
        .live
        .panes_logic
        .bridged_count(ws.panes.iter().map(|pane| &pane.id));
    let when_free = button::secondary(t, tr.t(Key::BroadcastWhenFree).into_owned())
        .icon(Icon::Clock)
        .on_press_maybe((!empty).then(|| send(true)))
        .tip(
            tr.t_with(Key::BroadcastWhenFreeHint, &args!("n" => bridged))
                .into_owned(),
        );
    let now = button::primary(t, tr.t(Key::BroadcastSend).into_owned())
        .icon(Icon::Send)
        .on_press_maybe((!empty).then(|| send(false)))
        .tip(tr.t(Key::StatusSendHint).into_owned());

    row![field, when_free, now,]
        .spacing(space::SM)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
}

/// The broadcast field's surface: `surface`, a `border` hairline, corners
/// of 8.
fn field_style(t: &hangar_theme::Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(iced::Background::Color(t.palette.surface)),
        text_color: Some(t.palette.text),
        border: iced::Border {
            color: t.palette.border,
            width: 1.0,
            radius: tokens::scale::radius::MD.into(),
        },
        ..container::Style::default()
    }
}

/// The agents' news: what needs the person first, as the navigator's row
/// orders it. It leads to the first pane in that state, in reading order;
/// with no agent at work there is nowhere to go.
fn news<'a>(ctx: Ctx<'a>, ws: &'a crate::state::Workspace) -> Item<'a> {
    let t = ctx.theme;
    let tr = ctx.i18n;
    let counts = reads::counts(ctx.model, ws);
    let found = [
        (
            AgentActivity::Waiting,
            counts.waiting,
            Key::SidebarActivityWaiting,
        ),
        (
            AgentActivity::Yours,
            counts.yours,
            Key::SidebarActivityYours,
        ),
        (
            AgentActivity::Working,
            counts.working,
            Key::SidebarActivityWorking,
        ),
    ]
    .into_iter()
    .find(|(_, count, _)| *count > 0);
    let Some((kind, count, key)) = found else {
        let words = tr.t(Key::ToolbarActivityQuiet).into_owned();
        return Item {
            width: 2.0 * PAD + tail::path_width(&words, WORDS),
            element: readout(t, mono(words)),
        };
    };
    let words = tr.tn(key, count, &[]).into_owned();
    let width = item_width(&words);
    let line = row![
        hangar_ui::glyph::state(t, reads::activity_glyph(kind), 8.0),
        mono(words).style(skin::state_text(t, kind)),
    ]
    .spacing(GAP + 1.0)
    .align_y(Alignment::Center);
    let first = reads::agents_in(ctx.model, &[ws], kind)
        .first()
        .map(|(_, pane)| (pane.id.clone(), pane.name.clone()));
    let Some((pane, name)) = first else {
        return Item {
            element: readout(t, line),
            width,
        };
    };
    let go = Message::Intent(Intent::FocusPane {
        ws: ws.id.clone(),
        pane,
        terminal: true,
    });
    let face = iced::widget::button(container(line).center_y(Length::Fill))
        .height(24)
        .padding([0.0, PAD])
        .style(style::button::ghost(t))
        .on_press(go.clone());
    let face = hangar_ui::focus::ring(t, face)
        .radius(tokens::scale::radius::SM)
        .on_activate(go);
    Item {
        element: tip::over(
            t,
            face,
            tr.t_with(Key::BoardFocusAgent, &args!("name" => name.as_str()))
                .into_owned(),
        ),
        width,
    }
}

/// What a bar `room` wide has room for besides the agents' news, which always
/// stays: the panes, then the branch, then the dictation keys, then the
/// folder, which is the first to go (`ecrans.md`, « Place qui manque »).
/// Each is `true` when it is shown, in that order.
fn kept(room: f32, news: f32, rest: [Option<f32>; 3]) -> ([bool; 3], bool) {
    let mut used = 2.0 * EDGE + news;
    let mut shown = [false; 3];
    for (at, width) in rest.into_iter().enumerate() {
        let Some(width) = width else {
            continue;
        };
        if used + width > room {
            return (shown, false);
        }
        used += width;
        shown[at] = true;
    }
    (shown, used + EDGE + FOLDER_ROOM <= room)
}

/// The bar of every view but the terminals: the folder, the agents' news, the
/// branch and the panes.
fn info<'a>(st: &'a State, ctx: Ctx<'a>, ws: &'a crate::state::Workspace) -> Element<'a, Message> {
    responsive(move |bounds| {
        let t = ctx.theme;
        let news = news(ctx, ws);
        let panes = panes(ctx, ws, true);
        let branch = branch(st, ctx, ws);
        let dictation = dictation(ctx);
        let ([panes_fit, branch_fits, dictation_fits], folder_fits) = kept(
            bounds.width,
            news.width,
            [
                Some(panes.width),
                branch.as_ref().map(|item| item.width),
                dictation.as_ref().map(|item| item.width),
            ],
        );

        let mut line = row![].align_y(Alignment::Center);
        if folder_fits {
            let cwd = ws.cwd.as_str();
            let path = responsive(move |bounds| {
                let shown = tail::fit(cwd, bounds.width, |s| tail::path_width(s, WORDS));
                mono(shown)
                    .style(move |_: &iced::Theme| text::Style {
                        color: Some(t.palette.text_muted),
                    })
                    .into()
            });
            let path = tip::over(
                t,
                container(path).width(Length::Fill).height(15),
                cwd.to_string(),
            );
            line = line.push(
                row![icon(Icon::Folder, ICON).color(t.palette.text_muted), path]
                    .spacing(space::SM)
                    .align_y(Alignment::Center)
                    .padding(Padding {
                        left: PAD,
                        right: PAD,
                        ..Padding::ZERO
                    })
                    .width(Length::Fill),
            );
        } else {
            line = line.push(Space::new().width(Length::Fill));
        }
        line = line.push(news.element);
        if let Some(item) = branch.filter(|_| branch_fits) {
            line = line.push(item.element);
        }
        if let Some(item) = dictation.filter(|_| dictation_fits) {
            line = line.push(item.element);
        }
        if panes_fit {
            line = line.push(panes.element);
        }
        container(line)
            .padding([0.0, EDGE])
            .height(Length::Fill)
            .align_y(Alignment::Center)
            .into()
    })
    .into()
}

#[cfg(test)]
mod interaction;
#[cfg(test)]
mod tests;
