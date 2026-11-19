//! The agents' setup: what the window shows when no coding agent is on this
//! machine, at start and whenever a workspace is about to be made, and what
//! it shows when opened to install the other one (docs/native/agents.md
//! §6.3, D17).
//!
//! Hangar runs Claude Code and Codex, and either one is enough. The sheet has
//! a card for each, Codex's labelled beta: whether it is installed (and which
//! version), whether it is signed in, one click that installs it with its
//! maker's official installer, its output shown as it runs ([`install`]), and
//! « Sign in », which opens it in a pane where it asks for the account the
//! first time it starts.
//!
//! The sheet stands in the way only while neither agent is installed: no
//! close button, deaf to Escape. The one way past it without installing is
//! « Use Hangar with shells only », kept in the settings (`shellsOnly`) and
//! undone there; with it, the new-workspace dialog falls back to a shell as it
//! always did. With an agent installed it is not shown at start, and when it
//! is opened it closes like any dialog, still offering the other agent.
//!
//! On NixOS the `claude` Anthropic's installer drops does not start
//! ([`nixos`]): there Claude Code's card says it comes from nixpkgs, shows the
//! lines that install it to copy, and « Check again » looks for it once they
//! are run. The installer is still there, behind « Use Anthropic's installer
//! anyway ». Codex's installer drops a static (musl) binary on Linux, which
//! starts on NixOS like anywhere else: its card is the same everywhere.
//!
//! The sheet is an overlay of the shell's (`Overlay::AgentSetup`). Its state
//! is `Packages::setup`; its messages ride in `shell::Action::Setup`.

pub mod install;
pub mod nixos;
mod view;

#[cfg(test)]
mod tests;

use hangar_core::agent::AgentKind;
use hangar_core::install::agent::Detected;
use iced::Task;

use crate::message::{Intent, Message};
use crate::shell::Overlay;
use crate::state::types::AgentId;
use crate::state::{Model, Patch, StoreAction};

pub use view::view;

/// The setup's state.
#[derive(Debug)]
pub struct State {
    /// Whether the detection at start has answered: before it does,
    /// `ui.claude_installed` and `ui.codex_installed` are `false` for every
    /// machine.
    pub detected: bool,
    /// This machine runs NixOS ([`nixos::here`], looked at once, with the
    /// state): Claude Code's card offers nixpkgs rather than the installer.
    pub nixos: bool,
    /// On NixOS, « Use Anthropic's installer anyway » was chosen: Claude
    /// Code's card is the installer's, as everywhere else.
    pub installer_anyway: bool,
    pub claude: Card,
    pub codex: Card,
    /// Whether Codex's main account is signed in, as the sheet's last look
    /// read its `auth.json`; `None` before a look answers. Claude Code's is
    /// the accounts package's, which watches it ([`signed_in`]).
    pub codex_signed_in: Option<bool>,
    /// The sheet stood in the way of a new workspace: the dialog opens once
    /// it is done.
    pub then_create: bool,
    /// The last install, look or check started: each card and the sheet keep
    /// the number of their own, so what an earlier one still sends is
    /// ignored.
    run: u64,
    /// The look the sheet started as it opened.
    look: u64,
}

impl Default for State {
    /// The state at start, with this machine looked at for NixOS.
    fn default() -> Self {
        State {
            detected: false,
            nixos: nixos::here(),
            installer_anyway: false,
            claude: Card::default(),
            codex: Card::default(),
            codex_signed_in: None,
            then_create: false,
            run: 0,
            look: 0,
        }
    }
}

impl State {
    /// Whether `kind`'s card offers it from nixpkgs: on NixOS, for an agent
    /// whose installer's program does not start there, unless the installer
    /// was chosen anyway.
    pub fn offers_nixpkgs(&self, kind: AgentKind) -> bool {
        self.nixos && nixos::from_nixpkgs(kind) && !self.installer_anyway
    }

    pub fn card(&self, kind: AgentKind) -> &Card {
        match kind {
            AgentKind::Claude => &self.claude,
            AgentKind::Codex => &self.codex,
        }
    }

    fn card_mut(&mut self, kind: AgentKind) -> &mut Card {
        match kind {
            AgentKind::Claude => &mut self.claude,
            AgentKind::Codex => &mut self.codex,
        }
    }

    /// A number no install, look or check had yet.
    fn next_run(&mut self) -> u64 {
        self.run += 1;
        self.run
    }
}

/// One agent's card.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Card {
    pub step: Step,
    /// The installer's output so far, its last [`install::KEPT_LINES`].
    pub output: Vec<String>,
    /// « Check again » found nothing: the card says so until the next look.
    pub still_missing: bool,
    /// Where the agent is and which version, as the sheet's last look found
    /// it; `None` when it found nothing, or before a look answers.
    pub found: Option<Detected>,
    /// The install or check of this card under way, or the last one.
    run: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Step {
    /// Nothing under way: the card offers the install, or says the agent is
    /// installed and offers to sign in.
    #[default]
    Idle,
    /// The installer is running.
    Installing,
    /// « Check again »: the PATH asked again, after an install from nixpkgs.
    Checking,
    /// It ran and is done: why it did not work.
    Failed(Failure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The installer exited with this code.
    Exit(i32),
    /// A signal ended it.
    Stopped,
    /// It could not be started, for this reason.
    Start(String),
    /// It said it worked, and the agent is still nowhere to be found.
    NotFound,
}

/// What a look at the agents found: where each is and which version, and
/// whether Codex's main account is signed in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Look {
    pub claude: Option<Detected>,
    pub codex: Option<Detected>,
    pub codex_signed_in: bool,
}

impl Look {
    pub fn found(&self, kind: AgentKind) -> Option<&Detected> {
        match kind {
            AgentKind::Claude => self.claude.as_ref(),
            AgentKind::Codex => self.codex.as_ref(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// « Install … », or « Try again ».
    Install(AgentKind),
    /// A line the installer of `kind`'s run `.1` printed.
    Output(AgentKind, u64, String),
    /// The installer of `kind`'s run `.1` ended.
    Exited(AgentKind, u64, install::Exit),
    /// After `kind`'s install, or « Check again »: the PATH asked again, and
    /// the agents looked at.
    Found(AgentKind, u64, Look),
    /// The look the sheet started as it opened.
    Looked(u64, Look),
    /// « Check again », on NixOS: the agent installed from nixpkgs looked
    /// for.
    Recheck(AgentKind),
    /// « Use Anthropic's installer anyway », on NixOS.
    InstallerAnyway,
    /// « Install from nixpkgs instead »: back from the installer, on NixOS.
    Nixpkgs,
    /// A line to copy, on the clipboard.
    Copy(&'static str),
    /// « Use Hangar with shells only ».
    ShellsOnly,
    /// « Sign in »: the agent in a pane, where it asks for the account.
    SignIn(AgentKind),
    /// « Done », or the close button: the sheet closes.
    Done,
    /// The install guide of `kind`, in the browser.
    OpenDocs(AgentKind),
}

fn msg(msg: Msg) -> Message {
    Message::Shell(crate::shell::Action::Setup(msg))
}

/// Whether `kind` is installed, as the window last found it: at start, then
/// by each of the sheet's looks.
pub fn installed(m: &Model, kind: AgentKind) -> bool {
    match kind {
        AgentKind::Claude => m.ui.claude_installed,
        AgentKind::Codex => m.ui.codex_installed,
    }
}

fn set_installed(m: &mut Model, kind: AgentKind, installed: bool) {
    match kind {
        AgentKind::Claude => m.ui.claude_installed = installed,
        AgentKind::Codex => m.ui.codex_installed = installed,
    }
}

/// Whether Hangar has an agent to run: either one is enough.
pub fn any_installed(m: &Model) -> bool {
    AgentKind::ALL.into_iter().any(|kind| installed(m, kind))
}

/// Whether the setup stands in the way: the agents were looked for and
/// neither was found, and the person did not choose to go without them.
pub fn needed(st: &State, m: &Model) -> bool {
    st.detected && !any_installed(m) && !crate::settings::shells_only(m.settings())
}

/// Whether the setup is the focused window's top overlay.
pub fn is_open(m: &Model) -> bool {
    m.ui.overlays.last() == Some(&Overlay::AgentSetup)
}

/// Whether the setup closes: once an agent is installed, or Hangar is set to
/// shells only, and not while an installer runs or « Check again » looks (its
/// end would come to a sheet closed). Until then it has no close button, and
/// Escape and the veil leave it.
pub fn closable(st: &State, m: &Model) -> bool {
    (any_installed(m) || crate::settings::shells_only(m.settings())) && !busy(st)
}

/// Escape, while the setup is the top overlay: what its close button does,
/// when it has one, so that it closes the one way that also opens the dialog
/// it stood in front of.
pub fn escape(st: &State, m: &Model) -> Task<Message> {
    if closable(st, m) {
        Task::done(msg(Msg::Done))
    } else {
        Task::none()
    }
}

/// At start, once both the state and the detection are in: the sheet, when
/// it is needed and not up already.
pub fn offer_at_start(st: &State, m: &Model) -> Option<Intent> {
    (m.live.hydrated && needed(st, m) && !m.ui.overlays.contains(&Overlay::AgentSetup))
        .then_some(Intent::OpenOverlay(Overlay::AgentSetup))
}

/// Every intent passes here before the shell and the packages hear it: a new
/// workspace asked for while the setup is needed opens the setup instead,
/// and the dialog once it is done. The setup opening, from there or from
/// anywhere else, looks at the agents again (versions, Codex's login): the
/// task that does is returned beside the intent.
pub fn redirect(st: &mut State, m: &Model, intent: Intent) -> (Intent, Task<Message>) {
    let intent = match intent {
        Intent::OpenOverlay(Overlay::WorkspaceDialog) if needed(st, m) => {
            st.then_create = true;
            Intent::OpenOverlay(Overlay::AgentSetup)
        }
        other => other,
    };
    let opening = matches!(intent, Intent::OpenOverlay(Overlay::AgentSetup))
        && !m.ui.overlays.contains(&Overlay::AgentSetup);
    let task = if opening {
        let run = st.next_run();
        st.look = run;
        Task::perform(look(), move |found| msg(Msg::Looked(run, found)))
    } else {
        Task::none()
    };
    (intent, task)
}

/// Whether `kind`'s main account is signed in. Claude Code's as the accounts
/// package last read it (its watcher reads the files every few seconds, so a
/// login made in a pane shows here by itself); Codex's as the sheet's last
/// look read it. `None` while not known.
pub fn signed_in(st: &State, m: &Model, kind: AgentKind) -> Option<bool> {
    match kind {
        AgentKind::Claude => m.live.accounts.accounts.as_ref().map(|accounts| {
            accounts
                .iter()
                .any(|account| account.is_main && account.logged_in)
        }),
        AgentKind::Codex => st.codex_signed_in,
    }
}

pub fn update(st: &mut State, m: &mut Model, message: Msg) -> Task<Message> {
    match message {
        Msg::Install(kind) => {
            if busy(st) {
                return Task::none();
            }
            let run = st.next_run();
            let card = st.card_mut(kind);
            card.run = run;
            card.step = Step::Installing;
            card.still_missing = false;
            card.output.clear();
            Task::run(install::run(install::chosen(kind), kind, run), |m| m)
        }
        Msg::Output(kind, run, line) => {
            let card = st.card_mut(kind);
            if card.run == run && card.step == Step::Installing {
                card.output.push(line);
                let over = card.output.len().saturating_sub(install::KEPT_LINES);
                card.output.drain(..over);
            }
            Task::none()
        }
        Msg::Exited(kind, run, exit) => {
            let card = st.card_mut(kind);
            if card.run != run || card.step != Step::Installing {
                return Task::none();
            }
            match exit {
                // It says it worked: look again, through a PATH asked again
                // of the login shell, which the installer may just have
                // taught where the agent is.
                Ok(Some(0)) => {
                    Task::perform(redetect(), move |found| msg(Msg::Found(kind, run, found)))
                }
                Ok(Some(code)) => {
                    card.step = Step::Failed(Failure::Exit(code));
                    Task::none()
                }
                Ok(None) => {
                    card.step = Step::Failed(Failure::Stopped);
                    Task::none()
                }
                Err(error) => {
                    card.step = Step::Failed(Failure::Start(error));
                    Task::none()
                }
            }
        }
        Msg::Found(kind, run, look) => {
            let card = st.card(kind);
            if card.run != run || !matches!(card.step, Step::Installing | Step::Checking) {
                return Task::none();
            }
            let checked = card.step == Step::Checking;
            let found = look.found(kind).is_some();
            apply(st, m, look);
            let card = st.card_mut(kind);
            card.still_missing = checked && !found;
            card.step = if found || checked {
                Step::Idle
            } else {
                Step::Failed(Failure::NotFound)
            };
            Task::none()
        }
        Msg::Looked(run, look) => {
            if run == st.look {
                apply(st, m, look);
            }
            Task::none()
        }
        Msg::Recheck(kind) => {
            if busy(st) {
                return Task::none();
            }
            let run = st.next_run();
            let card = st.card_mut(kind);
            card.run = run;
            card.step = Step::Checking;
            Task::perform(redetect(), move |found| msg(Msg::Found(kind, run, found)))
        }
        Msg::InstallerAnyway => {
            choose_installer(st, true);
            Task::none()
        }
        Msg::Nixpkgs => {
            choose_installer(st, false);
            Task::none()
        }
        Msg::Copy(line) => crate::services::clipboard::write(line.to_owned()),
        Msg::ShellsOnly => {
            m.apply(StoreAction::UpdateSettings(Patch::new(|s| {
                crate::settings::set_shells_only(s, true)
            })));
            finish(st)
        }
        Msg::Done => {
            if !closable(st, m) {
                return Task::none();
            }
            finish(st)
        }
        Msg::SignIn(kind) => {
            // The pane goes in the workspace on screen, when it is one of this
            // machine; else, or when a new workspace was on its way, that
            // workspace's own agent pane is where the login happens.
            let here = m
                .active_workspace()
                .filter(|ws| ws.node.is_none())
                .map(|ws| ws.id.clone());
            match here {
                Some(ws) if !st.then_create => {
                    let created = m
                        .apply(StoreAction::AddPane {
                            ws: ws.clone(),
                            agent: Some(AgentId::from(kind)),
                            near: None,
                            dir: None,
                            cwd: None,
                            name: None,
                            init: None,
                        })
                        .pane;
                    let close = finish(st);
                    match created {
                        Some(pane) => close.chain(Task::done(Message::Intent(Intent::FocusPane {
                            ws,
                            pane,
                            terminal: true,
                        }))),
                        None => close,
                    }
                }
                _ => {
                    st.then_create = true;
                    finish(st)
                }
            }
        }
        Msg::OpenDocs(kind) => {
            if let Err(error) = hangar_core::opener::open_url(install::docs_url(kind)) {
                eprintln!("the install guide could not be opened: {error}");
            }
            Task::none()
        }
    }
}

/// What a look found, kept: each card's agent, whether each is installed
/// (the window's own flags, which the panes and the dialog read too), and
/// Codex's login.
fn apply(st: &mut State, m: &mut Model, look: Look) {
    for kind in AgentKind::ALL {
        set_installed(m, kind, look.found(kind).is_some());
    }
    st.codex_signed_in = Some(look.codex_signed_in);
    st.claude.found = look.claude;
    st.codex.found = look.codex;
}

/// Whether an installer or a look for an agent is under way: the sheet waits
/// for it, and starts no other.
fn busy(st: &State) -> bool {
    [&st.claude, &st.codex]
        .iter()
        .any(|card| matches!(card.step, Step::Installing | Step::Checking))
}

/// On NixOS, Claude Code's card as the installer's (`true`) or nixpkgs'
/// (`false`), from its start; not while an installer or a look runs.
fn choose_installer(st: &mut State, installer: bool) {
    if busy(st) {
        return;
    }
    st.installer_anyway = installer;
    for kind in AgentKind::ALL
        .into_iter()
        .filter(|kind| nixos::from_nixpkgs(*kind))
    {
        let card = st.card_mut(kind);
        card.still_missing = false;
        card.step = Step::Idle;
        card.output.clear();
    }
}

/// The sheet closes, and the new-workspace dialog opens when the sheet stood
/// in its way.
fn finish(st: &mut State) -> Task<Message> {
    // In turn, not batched: the sheet is closed before the dialog opens.
    closing(st).into_iter().fold(Task::none(), |done, intent| {
        done.chain(Task::done(Message::Intent(intent)))
    })
}

/// What [`finish`] sends, in order, the sheet's state back to its start.
fn closing(st: &mut State) -> Vec<Intent> {
    let then_create = std::mem::take(&mut st.then_create);
    for card in [&mut st.claude, &mut st.codex] {
        card.step = Step::Idle;
        card.output.clear();
        card.still_missing = false;
    }
    st.installer_anyway = false;
    let mut intents = vec![Intent::CloseOverlay];
    if then_create {
        intents.push(Intent::OpenOverlay(Overlay::WorkspaceDialog));
    }
    intents
}

/// Both agents looked at (where, which version: `--version`, which a first
/// run of a Node shim can make slow, so the two at once), and Codex's main
/// login read.
async fn look() -> Look {
    use hangar_core::agent::detect;
    let (claude, codex) = tokio::join!(
        detect::detect(AgentKind::Claude),
        detect::detect(AgentKind::Codex)
    );
    let codex_signed_in = tokio::task::spawn_blocking(|| {
        hangar_core::accounts::accounts_of(AgentKind::Codex)
            .iter()
            .any(|account| account.is_main && account.logged_in)
    })
    .await
    .unwrap_or(false);
    Look {
        claude: claude.found,
        codex: codex.found,
        codex_signed_in,
    }
}

/// The PATH repaired again (`path_env::refresh`), then the agents looked at:
/// after an installer, and on « Check again ».
async fn redetect() -> Look {
    // The login shell is asked afresh: off the async threads.
    let _ = tokio::task::spawn_blocking(hangar_core::path_env::refresh).await;
    look().await
}
