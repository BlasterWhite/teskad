//! The « Comptes » page of the settings: the accounts of each agent and what
//! can be done with each (`AccountsSettingsPage.tsx`,
//! `ConnectedAccountsSettings.tsx`), a section per kind in use: Claude
//! Code's, and Codex's (« beta ») once it is installed or has an account
//! (`Live::in_use`).
//!
//! The login itself is the person's, in the browser: a visible pane opens on
//! the account and runs its kind's login there (`login_pane`: `claude auth
//! login` in a Claude profile, `codex login` in a Codex home). Who an account
//! is logged in as (its e-mail, its plan, or « API key ») is read by core
//! from the account's own files; no token is ever read into the app.

use std::collections::HashMap;

use hangar_i18n::{Key, Translator, args};
use hangar_theme::{style, tokens};
use hangar_ui::{Icon, badge, button as ui_button, form, icon, input, text as typography};
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Element, Length, Padding, Task};

use crate::ctx::Ctx;
use crate::message::Message;
use crate::services::Services;
use crate::state::types::{AgentId, AgentKind, WorkspaceFolder};
use crate::state::{AccountTone, Model, Pane, Patch, Settings, StoreAction, Workspace};

use super::account_style::{self, Mark};
use super::model::{
    Account, AccountId, AccountLevels, MAIN_ACCOUNT, names_account, resolve_account,
};
use super::{Msg, agent_label, key, live};

/// What the page itself remembers: the form it has open, the command it
/// waits for, the last failure.
#[derive(Debug, Default)]
pub struct Page {
    /// The account being renamed, and the name as typed so far.
    pub(super) renaming: Option<(AgentKind, AccountId, String)>,
    /// The account whose removal is being asked.
    pub(super) removing: Option<(AgentKind, AccountId)>,
    /// The add form of each kind's section.
    pub(super) drafts: HashMap<AgentKind, Draft>,
    /// A command runs: everything is disabled until it ends.
    pub(super) busy: bool,
    pub(super) error: Option<String>,
}

/// An add form: the name of the account to add, and whether it was sent
/// once (its problem shows from then on).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Draft {
    pub(super) name: String,
    pub(super) touched: bool,
}

impl Page {
    pub(super) fn draft(&self, kind: AgentKind) -> &Draft {
        static EMPTY: Draft = Draft {
            name: String::new(),
            touched: false,
        };
        self.drafts.get(&kind).unwrap_or(&EMPTY)
    }

    fn draft_mut(&mut self, kind: AgentKind) -> &mut Draft {
        self.drafts.entry(kind).or_default()
    }

    fn renaming(&self, kind: AgentKind, id: &str) -> Option<&str> {
        self.renaming
            .as_ref()
            .filter(|(of, renamed, _)| *of == kind && renamed == id)
            .map(|(_, _, value)| value.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelProblem {
    NameEmpty,
    NameTaken,
}

/// Why `label` cannot name an account among `accounts` (one kind's: a
/// Claude « Work » and a Codex « Work » can both be), or `None`. `except` is
/// the one being renamed.
pub fn label_problem(
    accounts: &[Account],
    label: &str,
    except: Option<&str>,
) -> Option<LabelProblem> {
    let wanted = label.trim().to_lowercase();
    if wanted.is_empty() {
        return Some(LabelProblem::NameEmpty);
    }
    let taken = accounts.iter().any(|account| {
        Some(account.id.as_str()) != except && account.label.trim().to_lowercase() == wanted
    });
    taken.then_some(LabelProblem::NameTaken)
}

fn problem_text(ctx: Ctx<'_>, problem: LabelProblem) -> String {
    match problem {
        LabelProblem::NameEmpty => ctx.t(Key::AccountsSettingsNameEmpty).into_owned(),
        LabelProblem::NameTaken => ctx.t(Key::AccountsSettingsNameTaken).into_owned(),
    }
}

/// What an account says of its plan: Codex names a login by an API key
/// `apikey`, which reads « API key »; a plan's own name as it is.
pub fn plan_text(t: Translator, plan: &str) -> String {
    if plan == "apikey" {
        t.t(Key::AccountsSettingsApiKey).into_owned()
    } else {
        plan.to_string()
    }
}

// ── the updates ───────────────────────────────────────────────────────

fn failed(t: Translator, detail: &str) -> String {
    t.t_with(Key::AccountsSettingsFailed, &args!("detail" => detail))
        .into_owned()
}

/// A visible pane of `kind`'s agent on account `id`, opened to log it in:
/// the account and the login mark are set before anything spawns the pane,
/// so it starts on that account's folder and types that kind's login
/// (`accounts::login_command`). `false` without an open workspace, or
/// without room.
pub fn login_pane(m: &mut Model, kind: AgentKind, id: &str) -> bool {
    let Some(ws) = m.active_workspace().map(|ws| ws.id.clone()) else {
        return false;
    };
    let account = id.to_string();
    let made = m.apply(StoreAction::AddPane {
        ws,
        agent: Some(AgentId::from(kind)),
        near: None,
        dir: None,
        cwd: None,
        name: None,
        init: Some(Patch::new(move |pane: &mut Pane| {
            *pane.account_mut(kind) = Some(account.clone());
            pane.login = Some(true);
        })),
    });
    made.pane.is_some()
}

/// [`login_pane`], or the page says why not.
fn open_login_pane(page: &mut Page, m: &mut Model, t: Translator, kind: AgentKind, id: &str) {
    if !login_pane(m, kind, id) {
        page.error = Some(t.t(Key::AccountsSettingsNoPane).into_owned());
    }
}

/// What names a removed account, so that it can be cleared: the workspaces
/// with it as their own, the panes with it as theirs, the navigator folders
/// with it for their workspaces, and whether the default was it — each for
/// the account's kind only.
#[derive(Debug, Default, PartialEq, Eq)]
struct Forgotten {
    workspaces: Vec<String>,
    panes: Vec<(String, String)>,
    folders: Vec<String>,
    default: bool,
}

fn who_names(
    workspaces: &[Workspace],
    folders: &[WorkspaceFolder],
    default: Option<&str>,
    kind: AgentKind,
    id: &str,
) -> Forgotten {
    let mut forgotten = Forgotten {
        default: names_account(default, id),
        folders: folders
            .iter()
            .filter(|folder| names_account(folder.account(kind), id))
            .map(|folder| folder.id.clone())
            .collect(),
        ..Forgotten::default()
    };
    // A workspace its node runs names an account of that machine, which
    // has its own: an account removed from this computer is not one of
    // them, whatever its name.
    for ws in workspaces.iter().filter(|ws| !ws.is_hosted()) {
        if names_account(ws.account(kind), id) {
            forgotten.workspaces.push(ws.id.clone());
        }
        for pane in &ws.panes {
            if names_account(pane.account(kind), id) {
                forgotten.panes.push((ws.id.clone(), pane.id.clone()));
            }
        }
    }
    forgotten
}

/// Whatever named a removed account of `kind` falls back to the level above;
/// say so in the file too. A default that named it goes back to main.
fn forget_account(m: &mut Model, kind: AgentKind, id: &str) {
    let forgotten = who_names(
        &m.state().workspaces,
        &m.state().folders,
        m.settings().account(kind),
        kind,
        id,
    );
    // Their workspaces then follow the default, as when the folder names none.
    for folder in forgotten.folders {
        m.apply(StoreAction::UpdateFolder(
            folder,
            Patch::new(move |folder: &mut WorkspaceFolder| *folder.account_mut(kind) = None),
        ));
    }
    for ws in forgotten.workspaces {
        m.apply(StoreAction::UpdateWorkspace(
            ws,
            Patch::new(move |ws: &mut Workspace| *ws.account_mut(kind) = None),
        ));
    }
    for (ws, pane) in forgotten.panes {
        m.apply(StoreAction::UpdatePane {
            ws,
            pane,
            patch: Patch::new(move |pane: &mut Pane| *pane.account_mut(kind) = None),
        });
    }
    if forgotten.default {
        m.apply(StoreAction::UpdateSettings(Patch::new(
            move |settings: &mut Settings| {
                *settings.account_mut(kind) = Some(MAIN_ACCOUNT.to_string());
            },
        )));
    }
    // Its badge goes with it: an account added later under the same id
    // starts plain.
    let style = key(kind, id);
    if m.settings().account_styles.contains_key(&style) {
        m.apply(StoreAction::UpdateSettings(Patch::new(
            move |settings: &mut Settings| {
                settings.account_styles.remove(&style);
            },
        )));
    }
}

/// The three messages that start a command of the backend are the only ones
/// that reach the services; the rest of the page is `step`.
pub fn update(page: &mut Page, m: &mut Model, sv: &Services, msg: Msg) -> Task<Message> {
    match msg {
        Msg::RenameSubmit => match begin_rename(page, m) {
            Some((kind, id, label)) => {
                let host = sv.core().host().clone();
                Task::perform(
                    run_blocking(move || {
                        hangar_core::accounts::account_rename_of(&host, kind, id, label).map(|_| ())
                    }),
                    |result| Message::Accounts(Msg::Renamed(result)),
                )
            }
            None => Task::none(),
        },
        Msg::RemoveConfirm(kind, id) => match begin_remove(page, &id) {
            true => {
                let host = sv.core().host().clone();
                let pty = sv.core().pty().clone();
                let removed = id.clone();
                Task::perform(
                    run_blocking(move || hangar_core::accounts::account_remove(&host, &pty, id)),
                    move |result| Message::Accounts(Msg::Removed(kind, removed.clone(), result)),
                )
            }
            false => Task::none(),
        },
        Msg::AddSubmit(kind) => match begin_add(page, m, kind) {
            Some(label) => {
                let host = sv.core().host().clone();
                Task::perform(
                    run_blocking(move || {
                        hangar_core::accounts::account_add_of(&host, kind, label)
                            .map(|added| added.account.id)
                    }),
                    move |result| Message::Accounts(Msg::Added(kind, result)),
                )
            }
            None => Task::none(),
        },
        msg => step(page, m, msg),
    }
}

/// The rename form was sent: the account and its new name, unless the name
/// is refused. The page then waits for the command.
/// Nothing while a request is in flight: Enter in the field submits whether
/// or not the button is enabled.
pub(super) fn begin_rename(page: &mut Page, m: &Model) -> Option<(AgentKind, AccountId, String)> {
    if page.busy {
        return None;
    }
    let (kind, id, label) = page.renaming.clone()?;
    let accounts = m.live.accounts.of(kind).unwrap_or_default();
    if label_problem(accounts, &label, Some(&id)).is_some() {
        return None;
    }
    page.busy = true;
    page.error = None;
    Some((kind, id, label.trim().to_string()))
}

/// The removal was confirmed. Never for main.
pub(super) fn begin_remove(page: &mut Page, id: &str) -> bool {
    if page.busy || id == MAIN_ACCOUNT {
        return false;
    }
    page.busy = true;
    page.error = None;
    true
}

/// The add form of `kind` was sent: the name, unless it is refused (its
/// problem shows from now on). Nothing while a request is in flight: a
/// quick double Enter would otherwise send the same name twice.
pub(super) fn begin_add(page: &mut Page, m: &Model, kind: AgentKind) -> Option<String> {
    if page.busy {
        return None;
    }
    let draft = page.draft_mut(kind);
    draft.touched = true;
    let name = draft.name.clone();
    let accounts = m.live.accounts.of(kind).unwrap_or_default();
    if label_problem(accounts, &name, None).is_some() {
        return None;
    }
    page.busy = true;
    page.error = None;
    Some(name.trim().to_string())
}

/// Everything the page does that needs no service.
pub(super) fn step(page: &mut Page, m: &mut Model, msg: Msg) -> Task<Message> {
    let t = Translator::new(m.locale());
    match msg {
        Msg::RenameStart(kind, id) => {
            let label = m
                .live
                .accounts
                .of(kind)
                .unwrap_or_default()
                .iter()
                .find(|account| account.id == id)
                .map(|account| account.label.clone())
                .unwrap_or_default();
            page.renaming = Some((kind, id, label));
            Task::none()
        }
        Msg::RenameInput(value) => {
            if let Some((_, _, current)) = &mut page.renaming {
                *current = value;
            }
            Task::none()
        }
        Msg::RenameCancel => {
            page.renaming = None;
            Task::none()
        }
        Msg::Renamed(result) => {
            page.busy = false;
            match result {
                Ok(()) => {
                    page.renaming = None;
                    super::refresh()
                }
                // The error tells; the field stays open.
                Err(err) => {
                    page.error = Some(failed(t, &err));
                    Task::none()
                }
            }
        }

        Msg::RemoveAsk(kind, id) => {
            page.removing = Some((kind, id));
            Task::none()
        }
        Msg::RemoveCancel => {
            page.removing = None;
            Task::none()
        }
        Msg::Removed(kind, id, result) => {
            page.busy = false;
            match result {
                Ok(()) => {
                    forget_account(m, kind, &id);
                    page.removing = None;
                    super::refresh()
                }
                // The error tells; the question stays.
                Err(err) => {
                    page.error = Some(failed(t, &err));
                    Task::none()
                }
            }
        }

        Msg::MakeDefault(kind, id) => {
            m.apply(StoreAction::UpdateSettings(Patch::new(
                move |settings: &mut Settings| *settings.account_mut(kind) = Some(id.clone()),
            )));
            Task::none()
        }
        Msg::SetBadges(on) => {
            m.apply(StoreAction::UpdateSettings(Patch::new(
                move |settings: &mut Settings| settings.account_badges = on,
            )));
            Task::none()
        }
        Msg::SetTone(kind, id, tone) => {
            let style = key(kind, &id);
            m.apply(StoreAction::UpdateSettings(Patch::new(
                move |settings: &mut Settings| settings.set_account_tone(&style, tone),
            )));
            Task::none()
        }
        Msg::SetIcon(kind, id, glyph) => {
            let style = key(kind, &id);
            m.apply(StoreAction::UpdateSettings(Patch::new(
                move |settings: &mut Settings| {
                    settings.set_account_icon(&style, glyph.map(Icon::name));
                },
            )));
            Task::none()
        }
        Msg::LogIn(kind, id) => {
            open_login_pane(page, m, t, kind, &id);
            Task::none()
        }

        Msg::AddInput(kind, value) => {
            page.draft_mut(kind).name = value;
            Task::none()
        }
        Msg::Added(kind, result) => {
            page.busy = false;
            match result {
                Ok(id) => {
                    *page.draft_mut(kind) = Draft::default();
                    open_login_pane(page, m, t, kind, &id);
                    super::refresh()
                }
                // The name stays, to retry.
                Err(err) => {
                    page.error = Some(failed(t, &err));
                    Task::none()
                }
            }
        }
        // Not this page's.
        _ => Task::none(),
    }
}

/// A command of the backend, which reads and writes the accounts' files, off
/// the UI thread.
async fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| format!("the task stopped: {err}"))?
}

// ── the view ──────────────────────────────────────────────────────────

fn press(msg: Msg) -> Message {
    Message::Accounts(msg)
}

pub fn view<'a>(page: &'a Page, ctx: Ctx<'a>) -> Element<'a, Message> {
    let kinds: Vec<(AgentKind, &'a [Account])> = live(ctx).kinds().collect();
    if kinds.is_empty() {
        return Space::new().width(0).height(0).into();
    }
    let single = kinds.len() == 1;
    let total = account_style::count(ctx);
    // Shared by every kind: shown under the last section.
    let mut shared: Vec<Element<'a, Message>> = Vec::new();
    // With a single account there is nothing to tell apart: no badge shows,
    // so none is offered.
    if total >= 2 {
        shared.push(badges_switch(page, ctx));
    }
    // Only worth a word once there is another account to move to.
    let mut ask = (total > 1).then(|| ask_switch(ctx));

    let mut sections = column![].spacing(tokens::space::XXXL);
    let last = kinds.len() - 1;
    for (index, (kind, accounts)) in kinds.into_iter().enumerate() {
        let mut body = column![].spacing(tokens::space::XL);
        if index == 0 {
            if let Some(note) = unsupported_note(ctx) {
                body = body.push(note);
            }
            if let Some(error) = &page.error {
                body = body.push(typography::danger(ctx.theme, error.as_str()));
            }
        }
        body = body.push(rows(page, ctx, kind, accounts));
        // One kind: the page as it always was.
        if single {
            for item in shared.drain(..) {
                body = body.push(item);
            }
        }
        body = body.push(add_form(page, ctx, kind, accounts));
        if index == last {
            for item in shared.drain(..) {
                body = body.push(item);
            }
            if let Some(ask) = ask.take() {
                body = body.push(ask);
            }
        }
        let (title, hint) = if single {
            (
                ctx.t(Key::AccountsSettingsTitle).into_owned(),
                ctx.t(Key::AccountsSettingsIntro).into_owned(),
            )
        } else {
            let agent = agent_label(ctx.i18n, kind);
            (
                ctx.i18n
                    .t_with(
                        Key::AccountsSettingsKindTitle,
                        &args!("agent" => agent.as_str()),
                    )
                    .into_owned(),
                kind_intro(ctx, kind),
            )
        };
        sections = sections.push(form::section(ctx.theme, title).hint(hint).bare(body));
    }
    sections.into()
}

/// What a kind's section says of its accounts, when the page has several.
fn kind_intro(ctx: Ctx<'_>, kind: AgentKind) -> String {
    let var = match kind.info().caps.accounts {
        hangar_core::agent::caps::Accounts::ConfigDir { var } => var,
        hangar_core::agent::caps::Accounts::Single => "",
    };
    ctx.i18n
        .t_with(
            Key::AccountsSettingsKindIntro,
            &args!("agent" => kind.info().label, "var" => var),
        )
        .into_owned()
}

/// The warning of a macOS whose Keychain cannot keep Claude Code's logins
/// apart (Codex's always are: a file per home).
fn unsupported_note<'a>(ctx: Ctx<'a>) -> Option<Element<'a, Message>> {
    let t = ctx.theme;
    let support = live(ctx).support.as_ref().filter(|s| !s.isolated_logins)?;
    let mut note = column![
        typography::body(t, ctx.t(Key::AccountsSettingsUnsupportedTitle))
            .font(hangar_theme::fonts::ui_semibold()),
        typography::hint(t, ctx.t(Key::AccountsSettingsUnsupportedBody)),
    ]
    .spacing(2);
    if let Some(extra) = &support.note {
        note = note.push(typography::hint(t, extra.as_str()));
    }
    Some(
        container(
            row![
                icon(Icon::Alert, tokens::size::ICON).color(t.palette.status.failed),
                note
            ]
            .spacing(tokens::space::LG),
        )
        .padding(tokens::space::XL)
        .width(Length::Fill)
        .style(move |_: &iced::Theme| iced::widget::container::Style {
            background: Some(t.colors.danger_soft.into()),
            border: iced::Border {
                color: t.palette.status.failed,
                width: tokens::BORDER_WIDTH,
                radius: tokens::radius::MD.into(),
            },
            ..iced::widget::container::Style::default()
        })
        .into(),
    )
}

/// The rows of one kind's accounts, in a card.
fn rows<'a>(
    page: &'a Page,
    ctx: Ctx<'a>,
    kind: AgentKind,
    accounts: &'a [Account],
) -> Element<'a, Message> {
    let default = ctx.model.settings().account(kind);
    let default_id = resolve_account(accounts, default, None, None).id;
    let rows: Vec<Element<'a, Message>> = accounts
        .iter()
        .map(|account| account_row(page, ctx, kind, accounts, account, &default_id))
        .collect();
    form::group(ctx.theme, rows)
}

/// Ask before moving running agents, for every kind.
fn ask_switch<'a>(ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let ask = ctx.model.settings().account_switch_ask;
    form::group(
        t,
        vec![
            form::row(
                t,
                ctx.t(Key::AccountsSettingsAskSwitch),
                input::switch(t, ask).on_toggle(|on| Message::Accounts(Msg::AskSwitch(on))),
            )
            .hint(ctx.t(Key::AccountsSettingsAskSwitchHint))
            .into(),
        ],
    )
}

fn account_row<'a>(
    page: &'a Page,
    ctx: Ctx<'a>,
    kind: AgentKind,
    accounts: &'a [Account],
    account: &'a Account,
    default_id: &str,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let is_default = account.id == default_id;
    let users: Vec<&str> = ctx
        .model
        .state()
        .workspaces
        .iter()
        .filter(|ws| {
            AccountLevels::of_kind(ctx.model.state(), ws, None, kind)
                .resolve(accounts)
                .id
                == account.id
        })
        .map(|ws| ws.name.as_str())
        .collect();

    let mut head =
        row![typography::body(t, account.label.as_str()).font(hangar_theme::fonts::ui_semibold())]
            .spacing(tokens::space::MD)
            .align_y(Alignment::Center);
    if is_default {
        head = head.push(badge::tag(t, ctx.t(Key::AccountsSettingsDefault)));
    }
    head = head.push(if account.logged_in {
        badge::badge(
            t,
            ctx.t(Key::AccountsSettingsLoggedIn),
            badge::Tone::Success,
        )
    } else {
        badge::badge(
            t,
            ctx.t(Key::AccountsSettingsLoggedOut),
            badge::Tone::Neutral,
        )
    });

    let mut lines = column![head].spacing(tokens::space::SM);
    // Who it is: its e-mail and its plan, never a token.
    let identity: Vec<String> = [
        account.email.clone(),
        account
            .plan
            .as_deref()
            .map(|plan| plan_text(ctx.i18n, plan)),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !identity.is_empty() {
        lines = lines.push(typography::hint(t, identity.join(" · ")));
    }
    lines = lines.push(typography::hint(
        t,
        if users.is_empty() {
            ctx.t(Key::AccountsSettingsUsedByNone).into_owned()
        } else {
            ctx.i18n
                .t_with(
                    Key::AccountsSettingsUsedBy,
                    &args!("names" => users.join(", ")),
                )
                .into_owned()
        },
    ));

    if account_style::shown(ctx) {
        lines = lines.push(badge_editor(page, ctx, kind, account));
    }

    let controls: Element<'a, Message> = if let Some(value) = page.renaming(kind, &account.id) {
        rename_form(page, ctx, accounts, account, value)
    } else if page.removing.as_ref() == Some(&(kind, account.id.clone())) {
        remove_confirm(page, ctx, kind, account)
    } else {
        actions(page, ctx, kind, account, is_default)
    };
    lines = lines.push(controls);
    container(lines)
        .padding(Padding::from([tokens::space::XL, tokens::space::XL]))
        .width(Length::Fill)
        .into()
}

/// Coloured badges, off by default: on, each account row offers a tone and
/// an icon; off, the badges are the plain labels they always were and the
/// choices made stay kept for when they are turned on again.
fn badges_switch<'a>(page: &'a Page, ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let mut switch = hangar_ui::input::switch(t, ctx.model.settings().account_badges)
        .label(ctx.t(Key::AccountsSettingsBadges));
    if !page.busy {
        switch = switch.on_toggle(|on| press(Msg::SetBadges(on)));
    }
    column![
        Element::from(switch),
        typography::hint(t, ctx.t(Key::AccountsSettingsBadgesHint)),
    ]
    .spacing(tokens::space::XS)
    .into()
}

/// How the account is marked: a choice of tone, each shown as the badge it
/// makes, then a choice of icon, each named in its tooltip. What is chosen
/// shows at once everywhere the account is named.
fn badge_editor<'a>(
    page: &'a Page,
    ctx: Ctx<'a>,
    kind: AgentKind,
    account: &'a Account,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let current = account_style::of(ctx, kind, &account.id);
    let idle = !page.busy;

    let mut tones = row![].spacing(tokens::space::XS).align_y(Alignment::Center);
    for tone in std::iter::once(None).chain(AccountTone::ALL.into_iter().map(Some)) {
        let name = match tone {
            None => Key::AccountsSettingsToneNone,
            Some(AccountTone::Accent) => Key::AccountsSettingsToneAccent,
            Some(AccountTone::Success) => Key::AccountsSettingsToneSuccess,
            Some(AccountTone::Warning) => Key::AccountsSettingsToneWarning,
            Some(AccountTone::Danger) => Key::AccountsSettingsToneDanger,
        };
        let preview = account_style::label(
            t,
            Mark { tone, ..current },
            account.label.clone(),
            tokens::font_size::MICRO,
            Some(t.palette.text_secondary),
        );
        let choice = iced::widget::button(preview)
            .padding(Padding::from([tokens::space::XXS, tokens::space::XS]))
            .style(style::button::toggle(t, tone == current.tone))
            .on_press_maybe(idle.then(|| press(Msg::SetTone(kind, account.id.clone(), tone))));
        tones = tones.push(hangar_ui::tip::bottom(t, choice, ctx.t(name)));
    }

    let mut icons = row![]
        .spacing(tokens::space::XXS)
        .align_y(Alignment::Center);
    let none = iced::widget::button(
        iced::widget::text(ctx.t(Key::AccountsSettingsIconNone)).size(tokens::font_size::LABEL),
    )
    .padding(Padding::from([tokens::space::XXS, tokens::space::SM]))
    .style(style::button::toggle(t, current.icon.is_none()))
    .on_press_maybe(idle.then(|| press(Msg::SetIcon(kind, account.id.clone(), None))));
    icons = icons.push(none);
    for (glyph, name) in account_style::ICONS {
        icons = icons.push(
            ui_button::toggle_button(t, glyph, current.icon == Some(glyph))
                .small()
                .tip(ctx.t(name))
                .on_press_maybe(
                    idle.then(|| press(Msg::SetIcon(kind, account.id.clone(), Some(glyph)))),
                ),
        );
    }

    column![
        typography::hint(t, ctx.t(Key::AccountsSettingsBadge)),
        tones.wrap(),
        icons.wrap(),
    ]
    .spacing(tokens::space::SM)
    .into()
}

fn actions<'a>(
    page: &'a Page,
    ctx: Ctx<'a>,
    kind: AgentKind,
    account: &'a Account,
    is_default: bool,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let idle = (!page.busy).then_some(());
    let mut actions = row![].spacing(tokens::space::MD).align_y(Alignment::Center);
    if !account.logged_in {
        actions = actions.push(
            ui_button::primary(t, ctx.t(Key::AccountsSettingsLogIn))
                .small()
                .on_press_maybe(idle.map(|()| press(Msg::LogIn(kind, account.id.clone())))),
        );
    }
    if !is_default {
        actions = actions.push(
            ui_button::secondary(t, ctx.t(Key::AccountsSettingsSetDefault))
                .small()
                .on_press_maybe(idle.map(|()| press(Msg::MakeDefault(kind, account.id.clone())))),
        );
    }
    actions = actions.push(
        ui_button::ghost(t, ctx.t(Key::AccountsSettingsRename))
            .small()
            .on_press_maybe(idle.map(|()| press(Msg::RenameStart(kind, account.id.clone())))),
    );
    if account.is_main {
        actions = actions.push(typography::hint(
            t,
            ctx.i18n.t_with(
                Key::AccountsSettingsMainFixed,
                &args!("agent" => kind.info().label),
            ),
        ));
    } else {
        actions = actions.push(
            ui_button::ghost(t, ctx.t(Key::AccountsSettingsRemove))
                .small()
                .on_press_maybe(idle.map(|()| press(Msg::RemoveAsk(kind, account.id.clone())))),
        );
    }
    actions.into()
}

fn rename_form<'a>(
    page: &'a Page,
    ctx: Ctx<'a>,
    accounts: &'a [Account],
    account: &'a Account,
    value: &'a str,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let problem = label_problem(accounts, value, Some(&account.id));
    let field = input::text_field(t, value)
        .on_input(|value| press(Msg::RenameInput(value)))
        .on_submit(press(Msg::RenameSubmit))
        .error(problem.map(|problem| problem_text(ctx, problem)));
    let save = ui_button::primary(t, ctx.t(Key::AccountsSettingsSave))
        .small()
        .on_press_maybe((!page.busy && problem.is_none()).then(|| press(Msg::RenameSubmit)));
    let cancel = ui_button::ghost(t, ctx.t(Key::AccountsSettingsCancel))
        .small()
        .on_press(press(Msg::RenameCancel));
    column![field, row![save, cancel].spacing(tokens::space::MD),]
        .spacing(tokens::space::MD)
        .into()
}

fn remove_confirm<'a>(
    page: &'a Page,
    ctx: Ctx<'a>,
    kind: AgentKind,
    account: &'a Account,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let ask = ctx
        .i18n
        .t_with(
            Key::AccountsSettingsRemoveAsk,
            &args!("name" => account.label.as_str()),
        )
        .into_owned();
    container(
        column![
            typography::body(t, ask).font(hangar_theme::fonts::ui_semibold()),
            typography::hint(t, ctx.t(Key::AccountsSettingsRemoveBody)),
            row![
                ui_button::danger(t, ctx.t(Key::AccountsSettingsRemoveConfirm))
                    .small()
                    .on_press_maybe(
                        (!page.busy).then(|| press(Msg::RemoveConfirm(kind, account.id.clone())))
                    ),
                ui_button::ghost(t, ctx.t(Key::AccountsSettingsCancel))
                    .small()
                    .on_press(press(Msg::RemoveCancel)),
            ]
            .spacing(tokens::space::MD),
        ]
        .spacing(tokens::space::MD),
    )
    .padding(tokens::space::XL)
    .width(Length::Fill)
    .style(style::container::well(t))
    .into()
}

fn add_form<'a>(
    page: &'a Page,
    ctx: Ctx<'a>,
    kind: AgentKind,
    accounts: &'a [Account],
) -> Element<'a, Message> {
    let t = ctx.theme;
    let draft = page.draft(kind);
    let problem = (draft.touched)
        .then(|| label_problem(accounts, &draft.name, None))
        .flatten();
    let placeholder = static_text(ctx.t(Key::AccountsSettingsAddPlaceholder));
    let field = input::text_field(t, &draft.name)
        .placeholder(placeholder)
        .on_input(move |value| press(Msg::AddInput(kind, value)))
        .on_submit(press(Msg::AddSubmit(kind)));
    let submit = ui_button::primary(t, ctx.t(Key::AccountsSettingsAdd))
        .on_press_maybe((!page.busy).then(|| press(Msg::AddSubmit(kind))));
    // The command the login pane runs, as it will be typed there.
    let note = match problem {
        Some(problem) => typography::danger(t, problem_text(ctx, problem)),
        None => typography::hint(
            t,
            ctx.i18n.t_with(
                Key::AccountsSettingsAddHint,
                &args!("command" => super::login_command(kind)),
            ),
        ),
    };
    form::group(
        t,
        vec![
            container(
                column![
                    typography::body(t, ctx.t(Key::AccountsSettingsAddTitle))
                        .font(hangar_theme::fonts::ui_semibold()),
                    row![field, submit]
                        .spacing(tokens::space::MD)
                        .align_y(Alignment::Center),
                    note,
                ]
                .spacing(tokens::space::MD),
            )
            .padding(tokens::space::XL)
            .width(Length::Fill)
            .into(),
        ],
    )
}

/// A dictionary string that takes no argument: always borrowed from the
/// dictionary, which a field's placeholder needs to be.
fn static_text(text: std::borrow::Cow<'static, str>) -> &'static str {
    match text {
        std::borrow::Cow::Borrowed(text) => text,
        std::borrow::Cow::Owned(_) => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, label: &str) -> Account {
        Account {
            id: id.into(),
            label: label.into(),
            is_main: id == "main",
            logged_in: true,
            email: None,
            plan: None,
        }
    }

    fn accounts() -> Vec<Account> {
        vec![account("main", "Principal"), account("perso", "Perso")]
    }

    #[test]
    fn a_name_is_refused_when_empty_or_when_another_account_has_it() {
        let all = accounts();
        assert_eq!(label_problem(&all, "", None), Some(LabelProblem::NameEmpty));
        assert_eq!(
            label_problem(&all, "  ", None),
            Some(LabelProblem::NameEmpty)
        );
        assert_eq!(label_problem(&all, "Travail", None), None);
        assert_eq!(
            label_problem(&all, " perso ", None),
            Some(LabelProblem::NameTaken)
        );
        assert_eq!(
            label_problem(&all, "PRINCIPAL", None),
            Some(LabelProblem::NameTaken)
        );
    }

    #[test]
    fn the_account_being_renamed_may_keep_its_own_name() {
        let all = accounts();
        assert_eq!(label_problem(&all, "Perso", Some("perso")), None);
        assert_eq!(
            label_problem(&all, "Principal", Some("perso")),
            Some(LabelProblem::NameTaken)
        );
    }

    #[test]
    fn a_failure_is_shown_with_its_detail_in_the_language() {
        let en = failed(Translator::new(hangar_i18n::Locale::En), "disk full");
        assert_eq!(en, "Could not do that: disk full");
        let fr = failed(Translator::new(hangar_i18n::Locale::Fr), "disque plein");
        assert!(fr.starts_with("Impossible\u{202f}: "), "{fr}");
        assert!(fr.ends_with("disque plein"));
    }

    #[test]
    fn a_removed_account_is_forgotten_by_whatever_named_it() {
        let workspace = |id: &str, own: Option<&str>, panes: &[(&str, Option<&str>)]| Workspace {
            id: id.into(),
            claude_account: own.map(str::to_string),
            panes: panes
                .iter()
                .map(|(pane, account)| Pane {
                    id: (*pane).into(),
                    claude_account: account.map(str::to_string),
                    ..Pane::default()
                })
                .collect(),
            ..Workspace::default()
        };
        let all = [
            workspace("w1", Some("perso"), &[("a", None), ("b", Some(" perso "))]),
            workspace("w2", None, &[("c", Some("pro")), ("d", Some("perso"))]),
            workspace("w3", Some("pro"), &[]),
        ];
        let folder = |id: &str, account: Option<&str>| WorkspaceFolder {
            id: id.into(),
            name: id.into(),
            claude_account: account.map(str::to_string),
            ..WorkspaceFolder::default()
        };
        let folders = [
            folder("games", Some(" perso ")),
            folder("work", Some("pro")),
            folder("plain", None),
        ];
        assert_eq!(
            who_names(&all, &folders, Some("perso"), AgentKind::Claude, "perso"),
            Forgotten {
                workspaces: vec!["w1".into()],
                panes: vec![("w1".into(), "b".into()), ("w2".into(), "d".into())],
                folders: vec!["games".into()],
                default: true,
            }
        );
        assert_eq!(
            who_names(&all, &folders, Some("main"), AgentKind::Claude, "pro"),
            Forgotten {
                workspaces: vec!["w3".into()],
                panes: vec![("w2".into(), "c".into())],
                folders: vec!["work".into()],
                default: false,
            }
        );
        assert_eq!(
            who_names(&all, &folders, None, AgentKind::Claude, "gone"),
            Forgotten::default()
        );

        // A workspace its node runs names that machine's accounts: nothing
        // of it is touched, and its node is told nothing.
        let mut mirror = workspace("w4", Some("perso"), &[("e", Some("perso"))]);
        mirror.node = Some("n1".into());
        mirror.hosted = Some(true);
        assert_eq!(
            who_names(&[mirror], &[], None, AgentKind::Claude, "perso"),
            Forgotten::default()
        );
    }

    /// A Codex account removed is forgotten by the Codex choices only: a
    /// Claude choice of the same id (only `main` can be both) is another
    /// account's.
    #[test]
    fn a_removed_codex_account_is_forgotten_by_the_codex_choices_only() {
        let ws = Workspace {
            id: "w".into(),
            claude_account: Some("team".into()),
            codex_account: Some("team".into()),
            panes: vec![
                Pane {
                    id: "a".into(),
                    codex_account: Some("team".into()),
                    ..Pane::default()
                },
                Pane {
                    id: "b".into(),
                    claude_account: Some("team".into()),
                    ..Pane::default()
                },
            ],
            ..Workspace::default()
        };
        let folders = [WorkspaceFolder {
            id: "f".into(),
            claude_account: Some("team".into()),
            codex_account: Some(" team ".into()),
            ..WorkspaceFolder::default()
        }];
        assert_eq!(
            who_names(&[ws], &folders, None, AgentKind::Codex, "team"),
            Forgotten {
                workspaces: vec!["w".into()],
                panes: vec![("w".into(), "a".into())],
                folders: vec!["f".into()],
                default: false,
            }
        );
    }

    #[test]
    fn an_api_key_login_reads_as_such_and_a_plan_as_it_is() {
        let en = Translator::new(hangar_i18n::Locale::En);
        assert_eq!(plan_text(en, "apikey"), "API key");
        assert_eq!(plan_text(en, "plus"), "plus");
    }
}
