//! P8: « Comptes », the accounts of each agent (docs/native/agents.md §6.1).
//!
//! Ports components/accounts/*, lib/{accounts,accountSwitch}.ts. Read
//! inventory-frontend.md §2.1/13 and crates/hangar-app/PACKAGES.md.
//!
//! Claude Code's accounts are profiles (`CLAUDE_CONFIG_DIR`), Codex's are
//! shadow homes (`CODEX_HOME`); both are listed by core, by kind, and every
//! choice is made per kind (`claude_account`, `codex_account` on the pane,
//! the workspace, the folder and the settings). An account is named by its
//! kind and its id: ids are unique across kinds but for `main`, which each
//! kind has (`key`). Codex's accounts are marked « beta » (`BETA`).
//!
//! Files: `model` (reading a choice, resolving the account of a pane, the
//! accounts panes were launched on), `switch` (restart sets, usage, labels),
//! `usage` (the usage cache on disk), `menu_items` (the lines of the
//! navigator's and the pane's menus), `switcher` (the toolbar button),
//! `badge`, `account_style` (the tone and icon chosen for an account, drawn
//! by every label that names one), `plan` (the plan block with a line per
//! account), `picker` (the workspace dialog's select) and `settings_page`
//! (Settings › Accounts).
//!
//! # What other packages call
//!
//! - the views: [`switcher`], [`pane_badge`], [`badge`], [`menu_items`],
//!   [`workspace_picker`] and [`settings_page`], with the signatures the shell
//!   fixed;
//! - P5, when it launches an agent pane: [`AccountLevels::of_kind`] (pane,
//!   workspace, the workspace's folder, default, for the pane's kind) to know
//!   the account to start it on, then
//!   `m.live.accounts.launched.note(pane, Some(account))`; and
//!   `note(pane, None)` when the pane ends. A pane's usage counts against
//!   the account it was launched on, and "running" in the switcher's
//!   question means "has noted an account". A pane opened to log an account
//!   in types [`login_command`] of its kind.

mod account_style;
mod badge;
mod menu_items;
mod picker;
mod plan;
mod settings_page;
mod switch;
mod switcher;
#[cfg(test)]
mod tests;
mod usage;

use std::time::{SystemTime, UNIX_EPOCH};

use iced::futures::stream;
use iced::{Element, Subscription, Task};

use crate::ctx::Ctx;
use crate::message::{Intent, Message};
use crate::services::{CoreEvent, Services};
use crate::shell::MenuEntry;
use crate::state::live::BridgeEvent;
use crate::state::types::{AgentKind, Pane};
use crate::state::{Model, PaneId, Patch, Settings, StoreAction, Workspace, WsId};
// Which account a pane runs on: hangar-engine's rules, named here as before.
use hangar_engine::accounts as model;

/// What a pane opened to log an account of a kind in types.
pub use hangar_core::accounts::login_command;
#[allow(unused_imports)]
pub use model::{
    Account, AccountId, AccountLevels, AccountSource, ClaudeAccount, DEFAULT_CHOICE, LOGIN_COMMAND,
    Launched, MAIN_ACCOUNT, ResolvedAccount, launch_account_with, read_account_choice,
    resolve_account,
};
pub use switch::{Follow, FollowInput, Job, Turn, follow};
use switch::{
    SwitchPane, SwitchWorkspace, Usage, UsageMeasure, follows_default, merge_usage,
    panes_to_restart_on_default_change, panes_to_restart_on_workspace_change, running_count,
    usage_by_account,
};
pub use switcher::SwitchScope;

/// The kinds whose support is labelled « beta » wherever their accounts are
/// shown, until real users confirm it (docs/native/agents.md §7, decision 5).
pub const BETA: &[AgentKind] = &[AgentKind::Codex];

/// A kind's name where its accounts are listed: « Codex (beta) » while it is
/// in [`BETA`].
pub fn agent_label(t: hangar_i18n::Translator, kind: AgentKind) -> String {
    let label = kind.info().label;
    if BETA.contains(&kind) {
        t.t_with(
            hangar_i18n::Key::AccountsAgentBeta,
            &hangar_i18n::args!("agent" => label),
        )
        .into_owned()
    } else {
        label.to_string()
    }
}

/// What names an account of `kind` where the kinds share one table (the
/// usage file, the badges' styles): the id of the first kind, whose entries
/// were written before there were others, and `kind:id` for the others. An
/// id is `[a-z0-9-]`, so no key of one kind is another's; `main`, which
/// every kind has, gets one per kind.
pub fn key(kind: AgentKind, id: &str) -> String {
    if kind == AgentKind::default() {
        id.to_string()
    } else {
        format!("{}:{id}", kind.id())
    }
}

/// Whether a running agent of `kind` can move to another of its accounts by
/// restarting on its conversation: each account is a folder of its own
/// named at launch, and its sessions resume by id (§3.2). Whether this pane
/// can now is its own: its turns reported, nothing typed, no command
/// running (`follow`).
pub fn movable_kind(kind: AgentKind) -> bool {
    let caps = &kind.info().caps;
    matches!(
        caps.accounts,
        hangar_core::agent::caps::Accounts::ConfigDir { .. }
    ) && caps.sessions.resume
}

/// The package's UI state; lives in `App.pkg.accounts`.
#[derive(Debug, Default)]
pub struct State {
    switcher: switcher::Menu,
    page: settings_page::Page,
    writes: usage::Writes,
    /// The newest listing applied (`LISTS`): an older read that finishes
    /// last does not overwrite a newer one.
    listed: u64,
    /// The usage file has been read. A save before that would write only
    /// what this run has seen, over the other accounts' figures on disk.
    usage_read: bool,
    /// A save was asked before the file was read: done once it has been.
    save_after_read: bool,
}

/// Numbers every listing asked for, in the order asked.
static LISTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// What logging in on a separate profile does on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Support {
    /// False on a macOS whose Keychain cannot separate logins.
    pub isolated_logins: bool,
    pub note: Option<String>,
}

/// What one reading of the accounts found: each kind's list, main first,
/// and which agents are installed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Listing {
    pub lists: Vec<(AgentKind, Vec<Account>)>,
    pub installed: Vec<AgentKind>,
}

/// The package's shared read-model, which other packages may read; lives in
/// `Model.live.accounts`.
#[derive(Debug, Default)]
pub struct Live {
    /// Claude Code's accounts, main included. `None` until the backend has
    /// answered: the interface then has no accounts to show, as it had none
    /// without a backend.
    pub accounts: Option<Vec<ClaudeAccount>>,
    /// Codex's accounts, main (the person's own Codex home) included; `None`
    /// likewise.
    pub codex: Option<Vec<Account>>,
    /// The agents found installed at the last reading.
    pub installed: Vec<AgentKind>,
    pub support: Option<Support>,
    /// The last usage each account reported, by [`key`].
    pub usage: Usage,
    /// The account each running agent pane was launched on (P5 keeps it).
    pub launched: Launched,
    /// The panes waiting to move to another account, and to which: their
    /// header says so until they do (P5 keeps it, `switch::follow`).
    pub moving: std::collections::HashMap<PaneId, Moving>,
}

impl Live {
    /// The accounts of `kind`, main first; `None` until they are known.
    pub fn of(&self, kind: AgentKind) -> Option<&[Account]> {
        match kind {
            AgentKind::Claude => self.accounts.as_deref(),
            AgentKind::Codex => self.codex.as_deref(),
        }
    }

    fn list_mut(&mut self, kind: AgentKind) -> &mut Option<Vec<Account>> {
        match kind {
            AgentKind::Claude => &mut self.accounts,
            AgentKind::Codex => &mut self.codex,
        }
    }

    /// Whether the person deals with `kind`'s accounts here: Claude Code's
    /// always, as before there were others; another kind's once it is
    /// installed, or one of its accounts is logged in or was added. A
    /// machine without Codex is not shown a Codex section with nothing in
    /// it.
    pub fn in_use(&self, kind: AgentKind) -> bool {
        if kind == AgentKind::default() {
            return self.of(kind).is_some();
        }
        let Some(accounts) = self.of(kind) else {
            return false;
        };
        self.installed.contains(&kind)
            || accounts.len() > 1
            || accounts.iter().any(|account| account.logged_in)
    }

    /// The kinds in use (`in_use`), in `AgentKind::ALL`'s order, with their
    /// accounts.
    pub fn kinds(&self) -> impl Iterator<Item = (AgentKind, &[Account])> {
        AgentKind::ALL
            .into_iter()
            .filter(|kind| self.in_use(*kind))
            .filter_map(|kind| Some((kind, self.of(kind)?)))
    }

    /// The kind of `pane`'s agent, when an account applies to it: an agent
    /// pane, not one opened to log in.
    pub fn kind_of(pane: &Pane) -> Option<AgentKind> {
        pane.agent.kind().filter(|_| pane.login != Some(true))
    }
}

/// A pane's move to another account, as its header shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moving {
    pub to: AccountId,
    /// What holds it back, which its header's tooltip says.
    pub hold: Hold,
}

impl Moving {
    /// `to` is not logged in: the pane stays where it is until it is.
    pub fn blocked(&self) -> bool {
        self.hold == Hold::NotLoggedIn
    }
}

/// What holds a pane's move to another account back (`switch::follow`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hold {
    /// The account is not logged in: the pane stays.
    NotLoggedIn,
    /// Its agent is inside a turn: at work, or waiting on the person.
    Turn,
    /// Its turns are not reported: the hangar-bridge mod is not loaded, or
    /// has said nothing yet.
    Unreported,
    /// Something typed in its prompt and not sent.
    Draft,
    /// Whether a command of its own still runs is being asked.
    Checking,
    /// A command its agent started still runs: what it is, shortened.
    Command(String),
    /// Its processes could not be read, so whether one runs is not known.
    Unreadable,
}

/// The package's messages, routed by the shell as `Message::Accounts(Msg)`.
#[derive(Debug, Clone)]
pub enum Msg {
    /// The app has started: read the accounts and the usage.
    Started,
    /// A listing answered, numbered as it was asked (`LISTS`).
    Listed(u64, Listing),
    SupportRead(Support),
    UsageRead(Usage),
    UsageSaved,

    // The toolbar's switcher.
    ToggleMenu,
    CloseMenu,
    Choose {
        ws: WsId,
        kind: AgentKind,
        id: AccountId,
        scope: SwitchScope,
    },
    /// The answer to « restart the running agents now? ».
    Decide(bool),
    /// Back from the question to the list.
    Back,
    /// Settings › Accounts: ask before moving running agents, or let each
    /// move by itself once it has nothing to lose (`follow`).
    AskSwitch(bool),

    // Choosing an account for something, from a menu or a picker: each
    // choice is of one kind's accounts.
    SetWorkspace {
        ws: WsId,
        kind: AgentKind,
        id: Option<AccountId>,
        restart: bool,
    },
    SetDefault {
        kind: AgentKind,
        id: AccountId,
        restart: bool,
    },
    /// The pane's own account, of its agent's kind.
    SetPane {
        ws: WsId,
        pane: PaneId,
        id: Option<AccountId>,
    },
    /// The account of a navigator folder's workspaces that chose none of
    /// their own; `None`: none, they follow the default.
    SetFolder {
        folder: String,
        kind: AgentKind,
        id: Option<AccountId>,
    },

    // Settings › Accounts: an account is its kind and its id (`main` is
    // every kind's).
    RenameStart(AgentKind, AccountId),
    RenameInput(String),
    RenameSubmit,
    RenameCancel,
    RemoveAsk(AgentKind, AccountId),
    RemoveConfirm(AgentKind, AccountId),
    RemoveCancel,
    MakeDefault(AgentKind, AccountId),
    /// Coloured badges turned on or off for every account.
    SetBadges(bool),
    /// The tone of an account's badge; `None` takes it back to plain.
    SetTone(AgentKind, AccountId, Option<crate::state::AccountTone>),
    /// The icon of an account's badge; `None` takes it off.
    SetIcon(AgentKind, AccountId, Option<hangar_ui::Icon>),
    LogIn(AgentKind, AccountId),
    /// The name typed in the add form of a kind's section.
    AddInput(AgentKind, String),
    AddSubmit(AgentKind),
    /// `account_add_of` ended: the new account's id, or why not.
    Added(AgentKind, Result<AccountId, String>),
    Renamed(Result<(), String>),
    Removed(AgentKind, AccountId, Result<(), String>),
}

/// An account's label for a line written outside the views (a pane's note):
/// its id when it is not listed.
pub fn account_label(accounts: &[ClaudeAccount], id: &str) -> String {
    switch::account_name(accounts, Some(id), id)
}

fn live<'a>(ctx: Ctx<'a>) -> &'a Live {
    &ctx.model.live.accounts
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

pub fn update(st: &mut State, m: &mut Model, sv: &Services, msg: Msg) -> Task<Message> {
    match msg {
        Msg::Started => Task::batch([refresh(), read_support(), read_usage(sv)]),
        Msg::Listed(seq, listing) => {
            listed(st, m, seq, listing);
            Task::none()
        }
        Msg::SupportRead(support) => {
            m.live.accounts.support = Some(support);
            Task::none()
        }
        Msg::UsageRead(read) => {
            // What arrived while the file was being read is newer than it.
            if usage_arrived(st, m, read) {
                return save_usage(st, m, sv);
            }
            Task::none()
        }
        Msg::UsageSaved => {
            if st.writes.ended() {
                write_usage(m, sv)
            } else {
                Task::none()
            }
        }

        menu @ (Msg::ToggleMenu
        | Msg::CloseMenu
        | Msg::Choose { .. }
        | Msg::Decide(_)
        | Msg::Back) => switcher_step(st, m, menu),

        Msg::AskSwitch(on) => set_ask_switch(m, on),
        Msg::SetWorkspace {
            ws,
            kind,
            id,
            restart,
        } => set_workspace_account(m, &ws, kind, id, restart),
        Msg::SetDefault { kind, id, restart } => set_default_account(m, kind, id, restart),
        Msg::SetPane { ws, pane, id } => set_pane_account(m, &ws, &pane, id),
        Msg::SetFolder { folder, kind, id } => {
            set_folder_account(m, folder, kind, id);
            Task::none()
        }

        page @ (Msg::RenameStart(..)
        | Msg::RenameInput(_)
        | Msg::RenameSubmit
        | Msg::RenameCancel
        | Msg::RemoveAsk(..)
        | Msg::RemoveConfirm(..)
        | Msg::RemoveCancel
        | Msg::MakeDefault(..)
        | Msg::SetBadges(_)
        | Msg::SetTone(..)
        | Msg::SetIcon(..)
        | Msg::LogIn(..)
        | Msg::AddInput(..)
        | Msg::AddSubmit(_)
        | Msg::Added(..)
        | Msg::Renamed(_)
        | Msg::Removed(..)) => settings_page::update(&mut st.page, m, sv, page),
    }
}

fn set_ask_switch(m: &mut Model, on: bool) -> Task<Message> {
    m.apply(StoreAction::UpdateSettings(Patch::new(
        move |settings: &mut Settings| settings.account_switch_ask = on,
    )));
    Task::none()
}

/// The switcher's popover: the list, and the question asked before a switch
/// that restarts running agents. It needs no service, only the model.
fn switcher_step(st: &mut State, m: &mut Model, msg: Msg) -> Task<Message> {
    match msg {
        Msg::ToggleMenu => {
            st.switcher = match st.switcher {
                switcher::Menu::Closed => switcher::Menu::Open,
                _ => switcher::Menu::Closed,
            };
            Task::none()
        }
        Msg::CloseMenu => {
            st.switcher = switcher::Menu::Closed;
            Task::none()
        }
        Msg::Choose {
            ws,
            kind,
            id,
            scope,
        } => {
            // Unasked, each pane follows on its own, once it has nothing to
            // lose (`follow`, run by the panes).
            let running = if m.settings().account_switch_ask {
                running_in(m, &ws, kind, scope)
            } else {
                0
            };
            if running == 0 {
                st.switcher = switcher::Menu::Closed;
                return apply_switch(m, &ws, kind, id, scope, false);
            }
            st.switcher = switcher::Menu::Ask {
                ws,
                kind,
                id,
                scope,
            };
            Task::none()
        }
        Msg::Decide(restart) => {
            let switcher::Menu::Ask {
                ws,
                kind,
                id,
                scope,
            } = std::mem::replace(&mut st.switcher, switcher::Menu::Closed)
            else {
                return Task::none();
            };
            apply_switch(m, &ws, kind, id, scope, restart)
        }
        Msg::Back => {
            st.switcher = switcher::Menu::Open;
            Task::none()
        }
        _ => Task::none(),
    }
}

/// Keeps a listing unless a newer one is already shown.
fn listed(st: &mut State, m: &mut Model, seq: u64, listing: Listing) {
    if seq > st.listed {
        st.listed = seq;
        let live = &mut m.live.accounts;
        for (kind, accounts) in listing.lists {
            *live.list_mut(kind) = Some(accounts);
        }
        live.installed = listing.installed;
    }
}

/// The usage file was read: merged under what arrived meanwhile, which is
/// newer. Returns whether a save held back until now is due.
fn usage_arrived(st: &mut State, m: &mut Model, read: Usage) -> bool {
    let fresh = std::mem::take(&mut m.live.accounts.usage);
    m.live.accounts.usage = merge_usage(&read, &fresh);
    st.usage_read = true;
    std::mem::take(&mut st.save_after_read)
}

/// Whether figures may be written now; if not (the file is not read yet),
/// the save is remembered for when it is.
fn may_save(st: &mut State) -> bool {
    if !st.usage_read {
        st.save_after_read = true;
    }
    st.usage_read
}

/// The accounts of every kind, read again: at start and after every
/// `accounts:changed`. They are small files on disk, and whether an agent is
/// installed is a look on PATH, so off the UI thread.
fn refresh() -> Task<Message> {
    let seq = LISTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Task::perform(
        async {
            tokio::task::spawn_blocking(|| Listing {
                lists: AgentKind::ALL
                    .into_iter()
                    .map(|kind| (kind, hangar_core::accounts::accounts_of(kind)))
                    .collect(),
                installed: AgentKind::ALL
                    .into_iter()
                    .filter(|kind| hangar_core::agent::detect::installed(*kind))
                    .collect(),
            })
            .await
            .ok()
        },
        move |read| match read {
            Some(listing) => Message::Accounts(Msg::Listed(seq, listing)),
            None => Message::Noop,
        },
    )
}

/// What the machine can do with them, read once (on macOS it asks the
/// Keychain).
fn read_support() -> Task<Message> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(|| {
                let support = hangar_core::accounts::accounts_support();
                Support {
                    isolated_logins: support.isolated_logins,
                    note: support.note,
                }
            })
            .await
            .ok()
        },
        |read| match read {
            Some(support) => Message::Accounts(Msg::SupportRead(support)),
            None => Message::Noop,
        },
    )
}

fn read_usage(sv: &Services) -> Task<Message> {
    let Some(file) = usage::path(sv.core().host()) else {
        // No file to read, so nothing on disk to overwrite either.
        return Task::done(Message::Accounts(Msg::UsageRead(Usage::default())));
    };
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || usage::load(&file))
                .await
                .unwrap_or_default()
        },
        |read| Message::Accounts(Msg::UsageRead(read)),
    )
}

/// Writes the figures, unless a write is running: it then writes again, with
/// the latest, when it ends.
fn save_usage(st: &mut State, m: &Model, sv: &Services) -> Task<Message> {
    if !may_save(st) {
        return Task::none();
    }
    if st.writes.wanted() {
        write_usage(m, sv)
    } else {
        Task::none()
    }
}

fn write_usage(m: &Model, sv: &Services) -> Task<Message> {
    let Some(file) = usage::path(sv.core().host()) else {
        return Task::done(Message::Accounts(Msg::UsageSaved));
    };
    let figures = m.live.accounts.usage.clone();
    Task::perform(
        async move {
            let saved = tokio::task::spawn_blocking(move || usage::save(&file, &figures)).await;
            if let Ok(Err(err)) = saved {
                eprintln!("accounts: could not save the usage figures: {err}");
            }
        },
        |()| Message::Accounts(Msg::UsageSaved),
    )
}

pub fn subscription(_st: &State, _m: &Model) -> Subscription<Message> {
    Subscription::run(started)
}

/// A stream that says once that the app has started: this package has no
/// other hook that runs before the first window is drawn.
fn started() -> impl iced::futures::Stream<Item = Message> {
    stream::once(async { Message::Accounts(Msg::Started) })
}

/// Every intent, after the shell's own handling of it: `Some` when this
/// package acted on it.
pub fn on_intent(
    st: &mut State,
    _m: &mut Model,
    _sv: &Services,
    intent: &Intent,
) -> Option<Task<Message>> {
    // A popover goes when something else asks for it (a dialog opens).
    if matches!(intent, Intent::ClosePopovers(_)) {
        st.switcher = switcher::Menu::Closed;
    }
    None
}

/// Every core event, after the shell's own handling of it.
pub fn on_event(
    st: &mut State,
    m: &mut Model,
    sv: &Services,
    event: &CoreEvent,
) -> Option<Task<Message>> {
    match event {
        CoreEvent::AccountsChanged => Some(refresh()),
        // A session counts for the account its pane was launched on.
        CoreEvent::Bridge { pane, event } => {
            record_usage(m, pane, event, now_ms()).then(|| save_usage(st, m, sv))
        }
        _ => None,
    }
}

/// A report of a pane's side channel (the Claude mod's, or Hangar's client
/// of an attached Codex), kept when it carries the plan's limits and the
/// pane runs on an account: under that account of the pane's kind (`key`).
/// Returns whether the figures changed.
fn record_usage(m: &mut Model, pane: &PaneId, event: &serde_json::Value, at: u64) -> bool {
    let Some(BridgeEvent::Usage { rate_limits, .. }) = BridgeEvent::parse(event) else {
        return false;
    };
    if rate_limits.is_empty() {
        return false;
    }
    let Some(kind) = m.find_pane(pane).and_then(|(_, pane)| pane.agent.kind()) else {
        return false;
    };
    let Some(account) = m
        .live
        .accounts
        .launched
        .account(pane)
        .map(|id| key(kind, id))
    else {
        return false;
    };
    let fresh = usage_by_account(
        &[UsageMeasure {
            pane_id: pane.clone(),
            at,
            rate_limits,
        }],
        |_| Some(account.clone()),
    );
    m.live.accounts.usage = merge_usage(&m.live.accounts.usage, &fresh);
    true
}

// ── switching ─────────────────────────────────────────────────────────

/// A workspace as the switching logic reads it for `kind`'s accounts, with
/// the account it reads from its folder; `running` = an agent is up in the
/// pane, on an account.
fn switch_workspace(m: &Model, workspace: &Workspace, kind: AgentKind) -> SwitchWorkspace {
    let live = &m.live.accounts;
    let levels = AccountLevels::of_kind(m.state(), workspace, None, kind);
    SwitchWorkspace {
        id: workspace.id.clone(),
        kind,
        account: levels.workspace,
        folder_account: levels.folder,
        panes: workspace
            .panes
            .iter()
            .map(|pane| SwitchPane {
                id: pane.id.clone(),
                // A pane opened to log in runs no agent: nothing to restart.
                kind: Live::kind_of(pane),
                account: pane.account(kind).map(str::to_string),
                // A pane on a node runs on the node's login: no account of
                // this computer's to restart it on.
                running: live.launched.is_running(&pane.id)
                    && crate::nodes::away::node_of(workspace, pane).is_none(),
            })
            .collect(),
    }
}

fn switch_workspaces(m: &Model, kind: AgentKind) -> Vec<SwitchWorkspace> {
    m.state()
        .workspaces
        .iter()
        .map(|workspace| switch_workspace(m, workspace, kind))
        .collect()
}

/// The panes of `workspace` a change of its account restarts, running ones
/// only: a stopped pane has no conversation to resume.
fn running_ids(panes: Vec<&SwitchPane>) -> Vec<PaneId> {
    panes
        .into_iter()
        .filter(|pane| pane.running)
        .map(|pane| pane.id.clone())
        .collect()
}

/// How many running agents a switch of `kind`'s account in this scope would
/// restart.
fn running_in(m: &Model, ws: &WsId, kind: AgentKind, scope: SwitchScope) -> usize {
    match scope {
        SwitchScope::Workspace => m
            .workspace(ws)
            .map(|workspace| switch_workspace(m, workspace, kind))
            .map_or(0, |workspace| {
                running_count(&panes_to_restart_on_workspace_change(&workspace))
            }),
        SwitchScope::Default => running_count(&panes_to_restart_on_default_change(
            &switch_workspaces(m, kind),
        )),
    }
}

fn apply_switch(
    m: &mut Model,
    ws: &WsId,
    kind: AgentKind,
    id: AccountId,
    scope: SwitchScope,
    restart: bool,
) -> Task<Message> {
    match scope {
        SwitchScope::Workspace => set_workspace_account(m, ws, kind, Some(id), restart),
        SwitchScope::Default => set_default_account(m, kind, id, restart),
    }
}

/// The agents restart on their new account with their conversation, which
/// the panes resume whatever `auto_resume` says, as for a move they make by
/// themselves (`panes::Msg::MoveAccounts`). Asked of the person when
/// `account_switch_ask` is on; when it is off, the panes move by themselves
/// instead (`follow`).
fn restart_task(ws: &WsId, panes: Vec<PaneId>) -> Task<Message> {
    if panes.is_empty() {
        return Task::none();
    }
    Task::done(Message::Panes(crate::panes::Msg::MoveAccounts(
        ws.clone(),
        panes,
    )))
}

fn set_workspace_account(
    m: &mut Model,
    ws: &WsId,
    kind: AgentKind,
    id: Option<AccountId>,
    restart: bool,
) -> Task<Message> {
    let Some(workspace) = m.workspace(ws) else {
        return Task::none();
    };
    // Which panes follow is decided before the change; ids survive it, and
    // the respawn re-resolves the account at launch.
    let switch = switch_workspace(m, workspace, kind);
    let panes = running_ids(panes_to_restart_on_workspace_change(&switch));
    m.apply(StoreAction::UpdateWorkspace(
        ws.clone(),
        Patch::new(move |workspace: &mut Workspace| *workspace.account_mut(kind) = id.clone()),
    ));
    if restart {
        restart_task(ws, panes)
    } else {
        Task::none()
    }
}

fn set_default_account(
    m: &mut Model,
    kind: AgentKind,
    id: AccountId,
    restart: bool,
) -> Task<Message> {
    let workspaces = switch_workspaces(m, kind);
    let affected: Vec<(WsId, Vec<PaneId>)> = workspaces
        .iter()
        .filter(|workspace| follows_default(workspace))
        .map(|workspace| {
            (
                workspace.id.clone(),
                running_ids(panes_to_restart_on_workspace_change(workspace)),
            )
        })
        .collect();
    m.apply(StoreAction::UpdateSettings(Patch::new(
        move |settings: &mut Settings| *settings.account_mut(kind) = Some(id.clone()),
    )));
    if !restart {
        return Task::none();
    }
    Task::batch(
        affected
            .into_iter()
            .map(|(ws, panes)| restart_task(&ws, panes)),
    )
}

/// The account of `kind` of a folder's workspaces, those that chose none of
/// their own (read at each launch, never copied). Nothing restarts here, as
/// for a pick in a workspace's menu: their agents move on their next launch,
/// or by themselves between turns when the person does not want to be asked
/// (`follow`).
fn set_folder_account(m: &mut Model, folder: String, kind: AgentKind, id: Option<AccountId>) {
    m.apply(StoreAction::UpdateFolder(
        folder,
        Patch::new(move |folder: &mut crate::state::types::WorkspaceFolder| {
            *folder.account_mut(kind) = id.clone();
        }),
    ));
}

/// The person's pick in the pane's menu, an account of its agent's kind: a
/// running agent restarts on it, conversation kept — at once when they asked
/// to be asked (`account_switch_ask`), otherwise once it has nothing to lose
/// (`follow`, run by the panes).
fn set_pane_account(
    m: &mut Model,
    ws: &WsId,
    pane: &PaneId,
    id: Option<AccountId>,
) -> Task<Message> {
    let Some(kind) = m
        .find_pane(pane)
        .filter(|(found, _)| found.id == *ws)
        .and_then(|(_, pane)| Live::kind_of(pane))
    else {
        return Task::none();
    };
    // Unasked, the pane moves on its own once it has nothing to lose.
    let running = m.settings().account_switch_ask && m.live.accounts.launched.is_running(pane);
    m.apply(StoreAction::UpdatePane {
        ws: ws.clone(),
        pane: pane.clone(),
        patch: Patch::new(move |pane: &mut crate::state::Pane| {
            *pane.account_mut(kind) = id.clone();
        }),
    });
    if running {
        restart_task(ws, vec![pane.clone()])
    } else {
        Task::none()
    }
}

// ── the views other packages embed ────────────────────────────────────

/// The toolbar's account switcher (embedded by the toolbar).
pub fn switcher<'a>(st: &'a State, ctx: Ctx<'a>) -> Element<'a, Message> {
    switcher::view(st, ctx)
}

/// The navigator's plan block with a line per account, when there are
/// several and some have reported; `None` leaves the plain block.
pub fn plan_block<'a>(ctx: Ctx<'a>) -> Option<Element<'a, Message>> {
    plan::view(ctx)
}

/// A pane header's account badge.
pub fn pane_badge<'a>(ctx: Ctx<'a>, ws: &'a WsId, pane: &'a PaneId) -> Element<'a, Message> {
    badge::pane(ctx, ws, pane)
}

/// A navigator row's account badge.
pub fn badge<'a>(ctx: Ctx<'a>, ws: &'a WsId) -> Element<'a, Message> {
    badge::workspace(ctx, ws)
}

/// The account entries of the navigator's and the pane's menus.
pub fn menu_items<'a>(ctx: Ctx<'a>, ws: &'a WsId, pane: Option<&'a PaneId>) -> Vec<MenuEntry> {
    menu_items::menu_entries(ctx, ws, pane)
}

/// The account entries of a navigator folder's menu: the account the
/// workspaces made in it, or moved into it, start on.
pub fn folder_menu_items<'a>(ctx: Ctx<'a>, folder: &'a str) -> Vec<MenuEntry> {
    menu_items::folder_entries(ctx, folder)
}

/// The workspace dialog's account picker, among `kind`'s accounts: those of
/// the agent the workspace's orchestrator runs. `value` is the account of
/// `kind` the workspace being made has chosen for itself (`None`: none, it
/// follows its folder's, else the default; `DEFAULT_CHOICE`: the default
/// over the folder's), `folder` the account of `kind` its folder names, if
/// any.
pub fn workspace_picker<'a>(
    ctx: Ctx<'a>,
    kind: AgentKind,
    value: Option<&'a str>,
    folder: Option<&'a str>,
) -> Element<'a, Message> {
    picker::view(ctx, kind, value, folder, |id| {
        Message::WorkspaceDialog(crate::workspace_dialog::Msg::Account(id))
    })
}

/// What the workspace dialog calls its account picker for `kind`'s
/// accounts: « Claude account » for Claude Code's, as before there were
/// others; « Codex (beta) account » for another kind's.
pub fn picker_label(t: hangar_i18n::Translator, kind: AgentKind) -> String {
    if kind == AgentKind::default() {
        return t.t(hangar_i18n::Key::AccountsPickerLabel).into_owned();
    }
    let agent = agent_label(t, kind);
    t.t_with(
        hangar_i18n::Key::AccountsMenuTitle,
        &hangar_i18n::args!("agent" => agent.as_str()),
    )
    .into_owned()
}

/// Settings › Accounts (embedded by the settings).
pub fn settings_page<'a>(st: &'a State, ctx: Ctx<'a>) -> Element<'a, Message> {
    settings_page::view(&st.page, ctx)
}

/// Whether there is any account of `kind` to choose from (the dialog shows
/// its picker then).
pub fn has_any_of(ctx: Ctx<'_>, kind: AgentKind) -> bool {
    live(ctx)
        .of(kind)
        .is_some_and(|accounts| !accounts.is_empty())
}
