//! P7: the « … » menu of a pane header (`PaneMenu.tsx`, and the entries
//! `TerminalPane.tsx` gave it; inventory §2.1/0).
//!
//! What a pane can do that is not worth a button of its own in a header that
//! eight panes share: restart it, resume another session, split it, open a
//! pane of another agent beside it, pin it to its edge of the grid or let it
//! go (`panes::Msg::Pin`); zoom, its saved commands and its changes when the
//! header folded those buttons away; its part in the orchestration; the model
//! its agent runs; then the account it runs on (`accounts::menu_items`).
//!
//! What the pane's agent can do decides which lines are there, never which
//! agent it is (docs/native/agents.md D3): sessions to resume for an agent
//! that resumes one, models for one that takes a model at launch. A line its
//! agent could take but cannot now (an agent not on this computer, a role it
//! could not hold) is greyed with the reason.
//!
//! The menu opens with its first line highlighted, so the arrows work
//! straight away. Every line closes the menu and gives the keyboard back to
//! the pane's terminal before it acts, so whatever it opens takes the
//! keyboard after.

use iced::{Element, Subscription, Task};

use hangar_i18n::{Key, args};
use hangar_ui::icon::Icon;
use hangar_ui::{button, layer, menu};

use crate::commands::CommandId;
use crate::ctx::Ctx;
use crate::message::{Intent, Message};
use crate::navigator::menu::set_off;
use crate::orchestration_ui::actions::{PinRefusal, pin_refusal};
use crate::orchestration_ui::models;
use crate::palette::sources::workspace_of;
use crate::services::{CoreEvent, Services};
use crate::shell::MenuEntry;
use crate::state::roles::RoleRefusal;
use crate::state::types::{AgentKind, Pane, PaneRole, Workspace};
use crate::state::{Model, PaneId, Patch, SplitDir, StoreAction, WsId};

/// The menu's width bounds (`.pane-menu`).
const MIN_WIDTH: f32 = 232.0;
const MAX_WIDTH: f32 = 320.0;

/// Which of the header's buttons the pane header folded away for want of
/// room: their actions join the menu then. The pane header knows; it passes
/// this to [`view_folded`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Folded {
    pub commands: bool,
    pub diff: bool,
    pub zoom: bool,
}

/// The package's UI state; lives in `App.pkg.pane_menu`.
#[derive(Debug, Default)]
pub struct State {
    /// The pane whose menu is open, and what its header had folded then.
    open: Option<(PaneId, Folded)>,
}

#[derive(Debug, Default)]
pub struct Live;

/// The package's messages, routed by the shell as `Message::PaneMenu(Msg)`.
#[derive(Debug, Clone)]
pub enum Msg {
    /// The « … » button: opens the pane's menu, or closes it.
    Toggle(PaneId, Folded),
    /// Escape, Tab, a press elsewhere.
    Close,
    /// A line: the menu closes, the terminal gets the keyboard back, then
    /// what the line does.
    Pick(PaneId, Box<Message>),
    /// « New Codex pane »: a pane of `kind`'s agent beside `near`, which
    /// takes the keyboard.
    NewPane {
        ws: WsId,
        near: PaneId,
        kind: AgentKind,
    },
}

fn send(msg: Msg) -> Message {
    Message::PaneMenu(msg)
}

pub fn update(st: &mut State, m: &mut Model, sv: &Services, msg: Msg) -> Task<Message> {
    let _ = sv;
    step(st, m, msg)
}

/// Everything a message does, which needs nothing from the services.
fn step(st: &mut State, m: &mut Model, msg: Msg) -> Task<Message> {
    match msg {
        Msg::Toggle(pane, folded) => {
            let again = st.open.as_ref().is_some_and(|(open, _)| open == &pane);
            st.open = if again {
                None
            } else {
                Some((pane.clone(), folded))
            };
            // An agent that lists its own models is asked for them as its
            // menu opens, for the model lines.
            let kind = workspace_of(m, &pane)
                .and_then(|ws| ws.panes.iter().find(|entry| entry.id == pane))
                .and_then(|entry| entry.agent.kind());
            match kind {
                Some(kind) if !again => models::ask(m, kind),
                _ => Task::none(),
            }
        }
        Msg::Close => {
            let Some((pane, _)) = st.open.take() else {
                return Task::none();
            };
            focus_terminal(m, pane)
        }
        Msg::Pick(pane, action) => {
            st.open = None;
            focus_terminal(m, pane).chain(Task::done(*action))
        }
        Msg::NewPane { ws, near, kind } => {
            let made = m
                .apply(StoreAction::AddPane {
                    ws: ws.clone(),
                    agent: Some(kind.into()),
                    near: Some(near),
                    dir: None,
                    cwd: None,
                    name: None,
                    init: None,
                })
                .pane;
            match made {
                Some(pane) => Task::done(Message::Intent(Intent::FocusPane {
                    ws,
                    pane,
                    terminal: true,
                })),
                None => Task::none(),
            }
        }
    }
}

/// Typing is what comes after a menu, whatever it did.
fn focus_terminal(m: &Model, pane: PaneId) -> Task<Message> {
    match workspace_of(m, &pane) {
        Some(ws) => Task::done(Message::Intent(Intent::FocusPane {
            ws: ws.id.clone(),
            pane,
            terminal: true,
        })),
        None => Task::none(),
    }
}

pub fn subscription(st: &State, m: &Model) -> Subscription<Message> {
    let _ = (st, m);
    Subscription::none()
}

pub fn on_intent(
    st: &mut State,
    m: &mut Model,
    sv: &Services,
    intent: &Intent,
) -> Option<Task<Message>> {
    let _ = (m, sv);
    if let Intent::ClosePopovers(_) = intent {
        st.open = None;
    }
    None
}

pub fn on_event(
    st: &mut State,
    m: &mut Model,
    sv: &Services,
    event: &CoreEvent,
) -> Option<Task<Message>> {
    let _ = (st, m, sv, event);
    None
}

/// What a pane is to the orchestration of its workspace, for the last group
/// of its menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// It is the workspace's orchestrator: the menu offers to let go, and its
    /// settings.
    Orchestrator,
    /// It may become the orchestrator: the menu offers to pin it.
    Free,
    /// Its agent (named) cannot hold the role: the line is there, greyed,
    /// with why.
    Unable(&'static str),
}

/// The pane's part, by the orchestration panels' own rule (`pinRefusal`: one
/// orchestrator per workspace, an agent that can hold the role, never a
/// worker, never the manager, never in the HQ). `None` for a pane the menu
/// says nothing about: a shell, or a pane the other rules leave out.
pub fn part_in_orchestration(ws: &Workspace, pane: &Pane) -> Option<Part> {
    if pane.role == Some(PaneRole::Orchestrator) {
        return Some(Part::Orchestrator);
    }
    match pin_refusal(ws, &pane.id) {
        None => Some(Part::Free),
        Some(PinRefusal::Agent(RoleRefusal::Unable { label })) => Some(Part::Unable(label)),
        Some(_) => None,
    }
}

/// A line that closes the menu, gives the terminal the keyboard back, then
/// sends `action`.
fn line(
    ctx: Ctx<'_>,
    pane: &PaneId,
    label: Key,
    icon: Icon,
    keys: Option<CommandId>,
    action: Message,
) -> menu::Entry<Message> {
    let mut item = menu::item(ctx.t(label))
        .icon(icon)
        .on_select(send(Msg::Pick(pane.clone(), Box::new(action))));
    if let Some(binding) = keys.and_then(|id| ctx.keymap.first_binding(id)) {
        item = item.shortcut(binding);
    }
    item.into()
}

/// The line that pins `pane` of `ws` to the edge of the grid it lies along,
/// or lets it go when it is `pinned` (`panes::Msg::Pin`).
fn pin_line(ctx: Ctx<'_>, ws: &WsId, pane: &PaneId, pinned: bool) -> menu::Entry<Message> {
    let label = if pinned { Key::PaneUnpin } else { Key::PanePin };
    let action = Message::Panes(crate::panes::Msg::Pin(ws.clone(), pane.clone(), !pinned));
    line(
        ctx,
        pane,
        label,
        Icon::Pin,
        Some(CommandId::PanePin),
        action,
    )
}

/// The menu's lines for `pane`, with what the header folded.
pub fn entries(ctx: Ctx<'_>, pane: &PaneId, folded: Folded) -> Vec<menu::Entry<Message>> {
    let Some(ws) = workspace_of(ctx.model, pane) else {
        return Vec::new();
    };
    let Some(entry) = ws.panes.iter().find(|entry| &entry.id == pane) else {
        return Vec::new();
    };
    let intent = |intent| Message::Intent(intent);
    let ws_id = ws.id.clone();
    let mut entries = vec![line(
        ctx,
        pane,
        Key::PaneRestart,
        Icon::Restart,
        Some(CommandId::PaneRestart),
        intent(Intent::ReplacePane {
            ws: ws_id.clone(),
            pane: pane.clone(),
            patch: Patch::none(),
        }),
    )];
    // Sessions to resume for an agent that resumes one, where its kind keeps
    // them (the session picker lists them by kind).
    let kind = entry.agent.kind();
    if kind.is_some_and(|kind| kind.info().caps.sessions.resume) {
        entries.push(line(
            ctx,
            pane,
            Key::CmdPaneSessions,
            Icon::Clock,
            Some(CommandId::PaneSessions),
            intent(Intent::OpenSessions {
                ws: ws_id.clone(),
                pane: pane.clone(),
            }),
        ));
    }
    entries.push(menu::separator());
    for (label, icon, keys, dir) in [
        (
            Key::CmdPaneSplitRight,
            Icon::SplitRight,
            CommandId::PaneSplitRight,
            SplitDir::Row,
        ),
        (
            Key::CmdPaneSplitDown,
            Icon::SplitDown,
            CommandId::PaneSplitDown,
            SplitDir::Col,
        ),
    ] {
        entries.push(line(
            ctx,
            pane,
            label,
            icon,
            Some(keys),
            intent(Intent::SplitPane {
                ws: ws_id.clone(),
                near: Some(pane.clone()),
                dir: Some(dir),
            }),
        ));
    }
    // The splits open a pane of the same agent; one of each other agent
    // opens beside it from here.
    for other in AgentKind::ALL {
        if kind != Some(other) {
            entries.push(new_pane_line(ctx, ws, pane, other));
        }
    }
    // With the splits, what else shapes the grid: the pane kept on its edge,
    // at its size, whatever opens or closes beside it; or let go again.
    entries.push(pin_line(ctx, &ws_id, pane, ws.pin_of(pane).is_some()));

    // Nothing to zoom out of, or onto, with a single pane.
    let zoomable = folded.zoom && ws.panes.len() > 1;
    if zoomable {
        let ui = &ctx.model.ui;
        let zoomed = ui.zoom_by_workspace.get(&ws.id).copied().unwrap_or(false)
            && ui.focus_by_workspace.get(&ws.id) == Some(pane);
        let (label, icon) = if zoomed {
            (Key::ZoomRestore, Icon::Collapse)
        } else {
            (Key::PaneMenuZoom, Icon::Expand)
        };
        // The zoom is the focused pane's, and every line focuses its pane
        // first: zooming from this menu zooms onto this pane.
        let action = intent(Intent::SetZoom {
            ws: ws_id.clone(),
            on: !zoomed,
        });
        entries.push(menu::separator());
        entries.push(line(
            ctx,
            pane,
            label,
            icon,
            Some(CommandId::PaneZoom),
            action,
        ));
    }
    if folded.commands {
        if !zoomable {
            entries.push(menu::separator());
        }
        let open = Message::SavedCommands(crate::saved_commands::Msg::OpenMenu {
            pane: pane.clone(),
            from_more: true,
        });
        entries.push(line(ctx, pane, Key::CommandsMenu, Icon::Bolt, None, open));
    }
    if folded.diff {
        if !zoomable && !folded.commands {
            entries.push(menu::separator());
        }
        entries.push(line(
            ctx,
            pane,
            Key::PaneMenuDiff,
            Icon::Diff,
            None,
            intent(Intent::OpenDiff {
                ws: ws_id.clone(),
                pane: Some(pane.clone()),
            }),
        ));
    }

    // Last, a group of its own: what the pane is to the workspace rather
    // than what it does.
    let part = part_in_orchestration(ws, entry);
    if part == Some(Part::Orchestrator) {
        entries.push(menu::separator());
        entries.push(line(
            ctx,
            pane,
            Key::OrchUnpin,
            Icon::Orchestrator,
            Some(CommandId::PaneUnpinOrchestrator),
            intent(Intent::PinOrchestrator {
                ws: ws_id.clone(),
                pane: pane.clone(),
                pinned: false,
            }),
        ));
        entries.push(line(
            ctx,
            pane,
            Key::OrchSettingsMenu,
            Icon::Sliders,
            Some(CommandId::WorkspaceOrchestration),
            intent(Intent::OpenOrchestration(ws_id.clone())),
        ));
    } else if part == Some(Part::Free) {
        entries.push(menu::separator());
        entries.push(line(
            ctx,
            pane,
            Key::OrchPin,
            Icon::Orchestrator,
            Some(CommandId::PanePinOrchestrator),
            intent(Intent::PinOrchestrator {
                ws: ws_id.clone(),
                pane: pane.clone(),
                pinned: true,
            }),
        ));
    } else if let Some(Part::Unable(agent)) = part {
        entries.push(menu::separator());
        let reason = ctx
            .i18n
            .t_with(Key::PaneMenuPinUnable, &args!("agent" => agent));
        entries.push(
            menu::item(ctx.t(Key::OrchPin))
                .icon(Icon::Orchestrator)
                .disabled_because(reason.into_owned())
                .into(),
        );
    }

    // The model its agent runs, for an agent that takes one at launch.
    if let Some(kind) = kind.filter(|kind| kind.info().caps.launch.model) {
        entries.push(menu::separator());
        entries.extend(model_lines(ctx, &ws_id, entry, kind));
    }

    // The account of the pane's agent, which says nothing for a shell. The
    // accounts package starts its block with its own separator and heading,
    // so none is added here: a second one would draw twice.
    entries.extend(
        set_off(crate::accounts::menu_items(ctx, &ws.id, Some(pane)))
            .into_iter()
            .map(|entry| account_line(pane, entry)),
    );
    entries
}

/// « New Codex pane (beta) »: a pane of `kind`'s agent beside `pane`, greyed
/// with why when that agent is not on this computer.
fn new_pane_line(
    ctx: Ctx<'_>,
    ws: &Workspace,
    pane: &PaneId,
    kind: AgentKind,
) -> menu::Entry<Message> {
    let agent = kind.info().label;
    let mut label = ctx
        .i18n
        .t_with(Key::PaneMenuNewAgentPane, &args!("agent" => agent))
        .into_owned();
    if crate::accounts::BETA.contains(&kind) {
        label = ctx
            .i18n
            .t_with(Key::AccountsAgentBeta, &args!("agent" => label))
            .into_owned();
    }
    let item = menu::item(label).icon(Icon::Plus);
    // A workspace on a node runs the node's agents, which this computer
    // cannot tell here: the node says so if it lacks one.
    if ws.node.is_none() && !crate::shell::setup::installed(ctx.model, kind) {
        let reason = ctx
            .i18n
            .t_with(Key::PaneMenuNotInstalled, &args!("agent" => agent));
        return item.disabled_because(reason.into_owned()).into();
    }
    let action = send(Msg::NewPane {
        ws: ws.id.clone(),
        near: pane.clone(),
        kind,
    });
    item.on_select(send(Msg::Pick(pane.clone(), Box::new(action))))
        .into()
}

/// « Model · Codex »: the models the pane's agent is offered
/// (`orchestration_ui::models`), its own default first and the one it runs
/// ticked. Another one restarts it with that model, on its conversation.
fn model_lines(
    ctx: Ctx<'_>,
    ws: &WsId,
    entry: &Pane,
    kind: AgentKind,
) -> Vec<menu::Entry<Message>> {
    let agent = kind.info().label;
    let mut lines = vec![menu::heading(
        ctx.i18n
            .t_with(Key::PaneMenuModel, &args!("agent" => agent))
            .into_owned(),
    )];
    let current = entry
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty());
    let offered = models::offered(ctx.model, kind);
    // None is the agent's own default.
    let mut names: Vec<Option<String>> = vec![None];
    names.extend(offered.iter().flatten().cloned().map(Some));
    if let Some(current) = current
        && !names.iter().any(|name| name.as_deref() == Some(current))
    {
        names.push(Some(current.to_owned()));
    }
    for name in names {
        let ticked = name.as_deref() == current;
        let item = match &name {
            Some(model) => menu::item(model.clone()).mono(),
            None => menu::item(
                ctx.i18n
                    .t_with(Key::PaneMenuModelDefault, &args!("agent" => agent))
                    .into_owned(),
            ),
        }
        .checked(ticked);
        // The one it runs is kept: picking it only closes the menu.
        let action = if ticked {
            send(Msg::Close)
        } else {
            let respawn = Message::Intent(Intent::ReplacePane {
                ws: ws.clone(),
                pane: entry.id.clone(),
                patch: Patch::new(move |pane: &mut Pane| pane.model = name.clone()),
            });
            send(Msg::Pick(entry.id.clone(), Box::new(respawn)))
        };
        lines.push(item.on_select(action).into());
    }
    if offered.is_none() && models::asking(ctx.model, kind) {
        let asking = ctx
            .i18n
            .t_with(Key::OrchModelsAsking, &args!("agent" => agent));
        lines.push(menu::item(asking.into_owned()).disabled().into());
    }
    lines
}

/// One of the accounts package's lines, in this menu: picking it closes the
/// menu too (`onDone`).
fn account_line(pane: &PaneId, entry: MenuEntry) -> menu::Entry<Message> {
    match entry {
        MenuEntry::Item {
            label,
            icon,
            danger,
            checked,
            disabled,
            on_select,
        } => {
            let mut item = menu::item(label);
            if let Some(icon) = icon.and_then(Icon::from_name) {
                item = item.icon(icon);
            }
            if danger {
                item = item.danger();
            }
            if let Some(checked) = checked {
                item = item.checked(checked);
            }
            match disabled {
                Some(reason) if reason.is_empty() => item.disabled().into(),
                Some(reason) => item.disabled_because(reason).into(),
                None => item
                    .on_select(send(Msg::Pick(pane.clone(), Box::new(on_select))))
                    .into(),
            }
        }
        MenuEntry::Heading(label) => menu::heading(label),
        MenuEntry::Separator => menu::separator(),
    }
}

/// The « … » button and its menu, for a header that folded `folded` away.
pub fn view_folded<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    pane: &'a PaneId,
    folded: Folded,
) -> Element<'a, Message> {
    let open = st
        .open
        .as_ref()
        .filter(|(open, _)| open == pane)
        .map(|(_, folded)| *folded);
    let commands = crate::saved_commands::open_from_more(&ctx.pkg.saved_commands, pane);
    let anchor = button::icon_button(ctx.theme, Icon::More)
        .tip(ctx.t(Key::PaneMore))
        .toggled(open.is_some() || commands)
        .sized(button::Side::Xs)
        .on_press(send(Msg::Toggle(pane.clone(), folded)));
    // The saved commands hang off this button when the header folded the ⚡
    // one away.
    if commands {
        return crate::saved_commands::menu_under(&ctx.pkg.saved_commands, ctx, anchor, pane);
    }
    let content = open.map(|folded| {
        menu::menu(ctx.theme, entries(ctx, pane, folded))
            .width_between(MIN_WIDTH, MAX_WIDTH)
            .from_keyboard(true)
            .into()
    });
    layer::dropdown(anchor, content)
        .align_end()
        .on_dismiss(send(Msg::Close))
        .into()
}

#[cfg(test)]
mod tests;
