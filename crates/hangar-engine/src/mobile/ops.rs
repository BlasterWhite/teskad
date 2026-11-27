//! The phone's operations on the panes and the projects (lib/remoteControl.ts):
//! what waits for the person, a pane's screen, typing into it as the person,
//! a permission answered, the manager told, a project woken, the history.
//!
//! The person is the one asking, from their phone, so what is sent is typed
//! as them, through P18's `pane_send` and `pane_read` run as the person:
//! agents' pane control being turned off does not stop it.
//!
//! A permission the pane's side channel holds open — Claude Code's
//! teskad-bridge mod, Hangar's client of a Codex pane's server — is answered
//! through it, as the decision itself (`held`). Otherwise the keys of the
//! agent's own dialog. Claude Code's permission prompt takes one key: Enter
//! accepts the highlighted "1. Yes", Escape is "No, and tell Claude what to
//! do differently"; the mod reports what the call would do, never which
//! choices the prompt offers, so those two keys are all that is sent.
//! Codex's approval dialog is read on the screen first, and answered only
//! when the screen shows it as Codex draws it, the cursor on the option the
//! person chose (answer.rs).
//!
//! What a pane's agent can take follows its capabilities, never its name
//! (docs/native/agents.md D3); only whether its side channel is turned on is
//! a setting of each kind's (`side_channel`).

use serde_json::{Map, Value, json};

use hangar_core::agent::AgentKind;
use hangar_i18n::{Key, Locale, t};

use crate::control::port::{ModelCmd, Port, activity_of, is_asleep, is_open};
use crate::control::run_as_person;
use crate::control::types::same;
use crate::state::roles::{manager_of, orchestrator_of};
use crate::state::{AgentActivity, AgentId, AppState, Pane, Settings, Snapshot, Workspace};

use super::answer::{self, Answering, Dialogs, PaneIo};
use super::contract::{
    ChatPending, NamedActivity, REMOTE_KEYS, RemoteAnswered, RemoteHistory, RemoteManager,
    RemotePane, RemoteProject, RemoteScreen, RemoteSent, RemoteState, RemoteWaiting, RemoteWoken,
    RemoteWorkspace, WaitingKind,
};
use super::desk::pending_now;
use super::engine::Mobile;
use super::held::{self, Permit};
use crate::orchestration::manager_feed::{ManagerEvent, ManagerJournalEntry};

const SCREEN_DEFAULT: f64 = 80.0;
pub const NO_MANAGER: &str = "Open the project manager on the PC";
/// A closed or asleep workspace gets this long to start a pane's shell for a
/// send: within the 10 s the mobile server gives the window, which is within
/// the 12 s the phone gives the server, past which the phone tries again.
pub const OPEN_WAIT_MS: u64 = 8_000;
const OPEN_POLL_MS: u64 = 250;
/// Past this, an agent woken from the phone whose side channel never said
/// hello is taken to run without it.
pub const RESUME_MS: u64 = 120_000;
/// How long a dialog gets to go down after the no that closes it, before
/// the words said with the no are typed at the prompt it leaves.
const WORDS_AFTER_MS: u64 = 300;

/// A string argument, trimmed; empty when it is anything else.
fn text_arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .map(|text| text.trim().to_string())
        .unwrap_or_default()
}

/// A pane by id or by name, wherever it is (`findPane`).
pub fn find_pane(state: &AppState, wanted: Option<&Value>) -> Result<(Workspace, Pane), String> {
    let wanted = wanted
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if wanted.is_empty() {
        return Err("Name the pane, by id or by name.".into());
    }
    let found = |hit: &dyn Fn(&Pane) -> bool| {
        state.workspaces.iter().find_map(|ws| {
            ws.panes
                .iter()
                .find(|pane| hit(pane))
                .map(|pane| (ws.clone(), pane.clone()))
        })
    };
    found(&|pane| pane.id == wanted)
        .or_else(|| found(&|pane| same(&pane.name, wanted)))
        .ok_or_else(|| format!("No pane \"{wanted}\" in Hangar."))
}

/// A project by id or by name (`findWorkspace`).
pub fn find_workspace(state: &AppState, wanted: Option<&Value>) -> Result<Workspace, String> {
    let wanted = wanted
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if wanted.is_empty() {
        return Err("Name the project, by id or by name.".into());
    }
    state
        .workspaces
        .iter()
        .find(|ws| ws.id == wanted)
        .or_else(|| state.workspaces.iter().find(|ws| same(&ws.name, wanted)))
        .cloned()
        .ok_or_else(|| format!("No project \"{wanted}\" in Hangar."))
}

fn activity(snapshot: &Snapshot, pane: &str) -> Option<AgentActivity> {
    activity_of(snapshot, &pane.to_string()).map(|entry| entry.activity)
}

fn bridged(snapshot: &Snapshot, pane: &str) -> bool {
    activity_of(snapshot, &pane.to_string()).is_some_and(|entry| entry.bridged)
}

/// The workspace as it stands now, or as it was given.
fn current(snapshot: &Snapshot, ws: &Workspace) -> Workspace {
    snapshot
        .state
        .workspaces
        .iter()
        .find(|entry| entry.id == ws.id)
        .cloned()
        .unwrap_or_else(|| ws.clone())
}

fn open(snapshot: &Snapshot, ws: &Workspace) -> bool {
    is_open(snapshot, &current(snapshot, ws))
}

/* ── remote_state ─────────────────────────────────────────────── */

/// Since when something waits: the journal's time when it has one, else the
/// first time it was seen (`since`).
fn since(mobile: &Mobile, key: &str, journaled: Option<u64>, at: u64) -> u64 {
    let mut book = mobile.book();
    match journaled {
        Some(journaled) => {
            book.first_seen.insert(key.to_string(), journaled);
            journaled
        }
        None => *book.first_seen.entry(key.to_string()).or_insert(at),
    }
}

/// The latest time the manager's journal recorded an event that matches.
fn journaled_at(
    journal: &[ManagerJournalEntry],
    hit: impl Fn(&ManagerEvent) -> bool,
) -> Option<u64> {
    journal
        .iter()
        .filter(|entry| hit(&entry.event))
        .map(|entry| entry.at)
        .max()
}

/// `remote_state {}`: every project, the manager, and what waits for the
/// person, oldest first. Each phone asks every 2 s.
///
/// The summaries come from `Sources::projects`, which the app keeps for a few
/// seconds (live.rs): what moves faster than that — whether a project is
/// open or asleep, what its agents are doing — is read from the snapshot of
/// now rather than from the summary.
pub async fn remote_state(port: &Port, mobile: &Mobile) -> RemoteState {
    let at = port.now_ms();
    let summaries = mobile.sources().projects().await;
    let journal = mobile.sources().manager_journal();
    let snapshot = port.snapshot();
    let state = &snapshot.state;

    let mut waiting = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for ws in &state.workspaces {
        for pane in &ws.panes {
            if pane.agent == AgentId::Shell
                || activity(&snapshot, &pane.id) != Some(AgentActivity::Waiting)
            {
                continue;
            }
            let key = format!("pane:{}", pane.id);
            let journaled = journaled_at(
                &journal,
                |event| matches!(event, ManagerEvent::PaneWaiting { pane_id, .. } if *pane_id == pane.id),
            );
            waiting.push(RemoteWaiting {
                kind: WaitingKind::Pane,
                workspace_id: ws.id.clone(),
                workspace: ws.name.clone(),
                pane_id: Some(pane.id.clone()),
                pane: Some(pane.name.clone()),
                task_id: None,
                title: None,
                since: since(mobile, &key, journaled, at),
            });
            seen.insert(key);
        }
    }
    for project in &summaries {
        for (kind, name, tasks) in [
            (WaitingKind::NeedsYou, "needsYou", &project.needs_you),
            (
                WaitingKind::ReadyToLand,
                "readyToLand",
                &project.ready_to_land,
            ),
        ] {
            for task in tasks {
                let key = format!("{name}:{}:{}", project.id, task.id);
                let journaled = journaled_at(&journal, |event| match event {
                    ManagerEvent::TaskNeedsYou { task_id, .. } => {
                        kind == WaitingKind::NeedsYou && *task_id == task.id
                    }
                    ManagerEvent::ReadyToLand { task_id, .. } => {
                        kind == WaitingKind::ReadyToLand && *task_id == task.id
                    }
                    _ => false,
                });
                waiting.push(RemoteWaiting {
                    kind,
                    workspace_id: project.id.clone(),
                    workspace: project.name.clone(),
                    pane_id: None,
                    pane: None,
                    task_id: Some(task.id.clone()),
                    title: Some(task.title.clone()),
                    since: since(mobile, &key, journaled, at),
                });
                seen.insert(key);
            }
        }
    }
    {
        // What is kept of panes closed since goes with them (MOB-16).
        let panes: std::collections::HashSet<&str> = state
            .workspaces
            .iter()
            .flat_map(|ws| ws.panes.iter().map(|pane| pane.id.as_str()))
            .collect();
        let mut book = mobile.book();
        book.first_seen.retain(|key, _| seen.contains(key));
        book.resuming
            .retain(|pane, _| panes.contains(pane.as_str()));
        book.chat.keep_only(&panes);
    }
    waiting.sort_by_key(|entry| entry.since);

    let manager = manager_of(state).map(|(_, pane)| RemoteManager {
        pane_id: pane.id.clone(),
        name: pane.name.clone(),
        activity: activity(&snapshot, &pane.id),
    });
    let projects = summaries
        .iter()
        .map(|project| {
            let ws = state.workspaces.iter().find(|ws| ws.id == project.id);
            RemoteProject {
                id: project.id.clone(),
                name: project.name.clone(),
                open: ws.map_or(project.open, |ws| is_open(&snapshot, ws)),
                asleep: ws.map_or(project.asleep, |ws| is_asleep(&snapshot, &ws.id)),
                git: project.git,
                orchestrator: match ws {
                    Some(ws) => orchestrator_of(ws).map(|pane| NamedActivity {
                        name: pane.name.clone(),
                        activity: activity(&snapshot, &pane.id),
                    }),
                    None => project.orchestrator.as_ref().map(|pane| NamedActivity {
                        name: pane.name.clone(),
                        activity: pane.activity,
                    }),
                },
                panes: ws
                    .map(|ws| ws.panes.as_slice())
                    .unwrap_or_default()
                    .iter()
                    .map(|pane| RemotePane {
                        id: pane.id.clone(),
                        name: pane.name.clone(),
                        agent: pane.agent.as_str().into(),
                        role: pane.role,
                        activity: activity(&snapshot, &pane.id),
                    })
                    .collect(),
                board: project.board,
                plan_status: project.plan_status.clone(),
                needs_you: project.needs_you.clone(),
                ready_to_land: project.ready_to_land.clone(),
            }
        })
        .collect();
    RemoteState {
        at,
        manager,
        waiting,
        projects,
    }
}

/* ── remote_screen ────────────────────────────────────────────── */

/// `remote_screen {pane, lines?}`: the pane's last lines; empty while its
/// workspace sleeps or is closed, which is not the pane being gone. While it
/// waits on a permission the PC can name, `prompt` says which, for the
/// phone's answer to name it back (MOB-02).
pub async fn remote_screen(
    port: &Port,
    mobile: &Mobile,
    args: &Value,
) -> Result<RemoteScreen, String> {
    let snapshot = port.snapshot();
    let (ws, pane) = find_pane(&snapshot.state, args.get("pane"))?;
    let lines = args
        .get("lines")
        .and_then(Value::as_f64)
        .unwrap_or(SCREEN_DEFAULT);
    let read = run_as_person(
        "pane_read",
        json!({ "workspace": ws.id, "pane": pane.id, "lines": lines }),
        port,
    )
    .await;
    let text = match read {
        Ok(read) => read["text"].as_str().unwrap_or("").to_string(),
        Err(error) if port.terminal(&pane.id).is_some() => return Err(error),
        Err(_) => String::new(),
    };
    let prompt = if activity(&snapshot, &pane.id) == Some(AgentActivity::Waiting) {
        match pending_now(port, mobile, &ws, &pane).await {
            Ok(Some(ChatPending::Permission { prompt, .. })) => Some(prompt),
            _ => None,
        }
    } else {
        None
    };
    Ok(RemoteScreen {
        prompt,
        pane_id: pane.id.clone(),
        pane: pane.name.clone(),
        workspace: ws.name.clone(),
        agent: pane.agent.as_str().into(),
        activity: activity(&snapshot, &pane.id),
        text,
    })
}

/* ── Typing as the person ─────────────────────────────────────── */

/// A pane's side channel as the settings have it: what drains the queue
/// Hangar keeps for the pane and holds its agent's asks open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideChannelSetting {
    pub on: bool,
    /// What is off when it is, as a refusal says it.
    pub off: &'static str,
}

/// The side channel of the pane's agent: the teskad-bridge mod for Claude
/// Code (Settings › Agents), Hangar attached to the pane's own Codex server
/// (« Attach Hangar to Codex », D5), which delivers the queue too (D9). Whether
/// it is on is a setting kept once per kind, read here as `state::types` reads
/// its other fields kept per kind; the rest of this file reads only this.
/// None for a shell.
pub fn side_channel(settings: &Settings, pane: &Pane) -> Option<SideChannelSetting> {
    Some(match pane.agent.kind()? {
        AgentKind::Claude => SideChannelSetting {
            on: settings.claude_bridge,
            off: "The teskad-bridge mod is off",
        },
        AgentKind::Codex => SideChannelSetting {
            on: settings.codex_attach,
            off: "Hangar is not attached to Codex (« Attach Hangar to Codex » is off)",
        },
    })
}

/// The queue Hangar keeps for the pane drains: its agent's side channel is
/// on.
fn queues(port: &Port, pane: &Pane) -> bool {
    side_channel(&port.snapshot().state.settings, pane).is_some_and(|channel| channel.on)
}

/// Wakes a workspace asleep, or opens one closed, as a click on the PC does:
/// its terminals mount, each agent resuming its session, and it counts as
/// used, so hibernation leaves it alone for its whole delay.
pub async fn wake(port: &Port, mobile: &Mobile, ws: &Workspace) {
    let snapshot = port.snapshot();
    let starting = is_asleep(&snapshot, &ws.id) || !open(&snapshot, ws);
    let _ = port.apply(ModelCmd::OpenWorkspace(ws.id.clone())).await;
    let _ = port.apply(ModelCmd::Touch(ws.id.clone())).await;
    if !starting {
        return;
    }
    let at = port.now_ms();
    let mut book = mobile.book();
    for pane in &ws.panes {
        let says_hello =
            side_channel(&snapshot.state.settings, pane).is_some_and(|channel| channel.on);
        if says_hello && !book.resuming.contains_key(&pane.id) && !bridged(&snapshot, &pane.id) {
            book.resuming.insert(pane.id.clone(), at);
        }
    }
}

/// An agent pane not up yet: no terminal — its workspace asleep or closed —
/// or one woken from the phone whose side channel has not said hello.
pub fn resumes(port: &Port, mobile: &Mobile, pane: &Pane) -> bool {
    if pane.agent.kind().is_none() {
        return false;
    }
    if port.terminal(&pane.id).is_none() {
        return true;
    }
    let mut book = mobile.book();
    let Some(since) = book.resuming.get(&pane.id).copied() else {
        return false;
    };
    if bridged(&port.snapshot(), &pane.id) || port.now_ms().saturating_sub(since) >= RESUME_MS {
        book.resuming.remove(&pane.id);
        return false;
    }
    true
}

/// Nothing was sent to an agent not up yet: why, said before `wake`
/// changes it.
fn not_up(port: &Port, ws: &Workspace, pane: &Pane) -> String {
    let snapshot = port.snapshot();
    let why = if is_asleep(&snapshot, &ws.id) {
        format!(
            "\"{}\" was asleep: it is waking up now, its agents resuming their sessions.",
            ws.name
        )
    } else if !open(&snapshot, ws) {
        format!("\"{}\" was not open: its panes are starting now.", ws.name)
    } else {
        format!("\"{}\" is resuming its session.", pane.name)
    };
    format!("{why} Nothing was sent; send it again in a few seconds.")
}

/// A pane whose terminal or shell did not start in time: nothing was typed.
fn not_started(pane: &Pane) -> String {
    format!(
        "\"{}\" has not started yet: nothing was sent; send it again in a few seconds.",
        pane.name
    )
}

/// What the person sent an agent not up yet, put in the pane's queue, which
/// the session takes once it is ready, as their own words; the workspace is
/// woken for it. Without text to queue, or without the side channel to take
/// it, nothing is sent (`hold`).
pub async fn hold(
    port: &Port,
    mobile: &Mobile,
    ws: &Workspace,
    pane: &Pane,
    text: Option<&str>,
) -> Result<(), String> {
    let Some(text) = text.filter(|_| queues(port, pane)) else {
        let error = not_up(port, ws, pane);
        wake(port, mobile, ws).await;
        return Err(error);
    };
    port.enqueue(&pane.id, text, None).await?;
    wake(port, mobile, ws).await;
    Ok(())
}

/// The person's words for a pane whose agent works and runs its side
/// channel: into its queue, which the session takes when the turn ends,
/// rather than typed into the turn. What waits there can be taken back
/// (`unqueue_pane`).
pub async fn queue_pane(port: &Port, pane: &Pane, text: &str) -> Result<RemoteSent, String> {
    if text.trim().is_empty() {
        return Err("Nothing to send: give text.".into());
    }
    let channel = side_channel(&port.snapshot().state.settings, pane);
    if let Some(off) = channel
        .filter(|channel| !channel.on)
        .map(|channel| channel.off)
    {
        return Err(format!(
            "{off}: nothing can wait for \"{}\". Nothing was sent.",
            pane.name
        ));
    }
    if channel.is_none() {
        return Err(format!(
            "\"{}\" runs no agent: nothing can wait for it. Nothing was sent.",
            pane.name
        ));
    }
    port.enqueue(&pane.id, text, None).await?;
    Ok(RemoteSent::QUEUED)
}

/// Takes back what waits in the pane's queue, by its id.
pub async fn unqueue_pane(port: &Port, pane: &Pane, id: &str) -> Result<(), String> {
    port.unqueue(&pane.id, id).await
}

/// What `type` sends.
#[derive(Debug, Clone, Default)]
pub struct Input {
    pub text: Option<String>,
    pub keys: Vec<String>,
    /// `Some(false)`: the text is pasted without Enter.
    pub submit: Option<bool>,
}

/// Typed as the person: pane_send's path, without when_idle (`type`).
///
/// A Claude Code pane that is not up — its workspace asleep or closed, or
/// resuming after the phone woke it — has a shell first and Claude Code
/// seconds later: typed then, the text runs in the shell, or reaches no
/// process at all. It is held in the queue instead (`hold`); keys, or text
/// left unsubmitted, cannot wait there. Any other pane is typed into once
/// its terminal and its shell are up.
pub async fn type_into(
    port: &Port,
    mobile: &Mobile,
    ws: &Workspace,
    pane: &Pane,
    input: Input,
) -> Result<RemoteSent, String> {
    // Keys typed while an answer is being typed into the same dialog would
    // land in the middle of it (MOB-18).
    if mobile.book().answering.contains(&pane.id) {
        return Err(format!(
            "An answer is being typed into \"{}\": wait for it. Nothing was sent.",
            pane.name
        ));
    }
    if resumes(port, mobile, pane) {
        let text = input.text.as_deref().unwrap_or("");
        let holdable =
            !text.trim().is_empty() && input.keys.is_empty() && input.submit != Some(false);
        hold(port, mobile, ws, pane, holdable.then_some(text)).await?;
        return Ok(RemoteSent::QUEUED);
    }
    if port.terminal(&pane.id).is_none() {
        wake(port, mobile, ws).await;
        // A terminal mounts before its shell is spawned: what it takes until
        // then reaches no process.
        let mut waited = 0;
        while waited < OPEN_WAIT_MS {
            port.sleep(OPEN_POLL_MS).await;
            if port.terminal(&pane.id).is_some() && port.alive(&pane.id).await {
                break;
            }
            waited += OPEN_POLL_MS;
        }
        // Still no shell: what is typed would reach no process, and the
        // phone be told it was sent (MOB-17).
        if port.terminal(&pane.id).is_none() || !port.alive(&pane.id).await {
            return Err(not_started(pane));
        }
    }
    let mut args = Map::new();
    args.insert("workspace".into(), json!(ws.id));
    args.insert("pane".into(), json!(pane.id));
    if let Some(text) = &input.text {
        args.insert("text".into(), json!(text));
    }
    args.insert("keys".into(), json!(input.keys));
    if let Some(submit) = input.submit {
        args.insert("submit".into(), json!(submit));
    }
    run_as_person("pane_send", Value::Object(args), port).await?;
    Ok(RemoteSent::SENT)
}

/// How a delivery's words reach its worker (remoteDelivery's `TellWorker`).
pub trait Tell: Send + Sync {
    fn tell<'a>(
        &'a self,
        port: &'a Port,
        mobile: &'a Mobile,
        ws: &'a Workspace,
        pane: &'a Pane,
        text: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>>;
}

/// The person's words for a worker sent back from the phone: through the
/// queue, which its agent's session takes once it is ready, its project
/// woken for it; typed as `remote_send` does where no side channel runs
/// (`toWorker`).
pub struct ToWorker;

impl Tell for ToWorker {
    fn tell<'a>(
        &'a self,
        port: &'a Port,
        mobile: &'a Mobile,
        ws: &'a Workspace,
        pane: &'a Pane,
        text: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            if !queues(port, pane) {
                let input = Input {
                    text: Some(text.to_string()),
                    ..Input::default()
                };
                return type_into(port, mobile, ws, pane, input).await.map(|_| ());
            }
            port.enqueue(&pane.id, text, None).await?;
            wake(port, mobile, ws).await;
            Ok(())
        })
    }
}

/* ── The operations ───────────────────────────────────────────── */

/// `remote_send {pane, text?, keys?, submit?}`.
pub async fn remote_send(port: &Port, mobile: &Mobile, args: &Value) -> Result<RemoteSent, String> {
    let (ws, pane) = find_pane(&port.snapshot().state, args.get("pane"))?;
    let keys: Vec<String> = args
        .get("keys")
        .and_then(Value::as_array)
        .map(|keys| {
            keys.iter()
                .map(|key| match key {
                    Value::String(name) => name.to_lowercase(),
                    other => other.to_string().to_lowercase(),
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(bad) = keys.iter().find(|key| !REMOTE_KEYS.contains(&key.as_str())) {
        return Err(format!(
            "Unknown key \"{bad}\". One of: {}.",
            REMOTE_KEYS.join(", ")
        ));
    }
    let input = Input {
        text: args.get("text").and_then(Value::as_str).map(str::to_string),
        keys,
        submit: (args.get("submit") == Some(&Value::Bool(false))).then_some(false),
    };
    type_into(port, mobile, &ws, &pane, input).await
}

/// `remote_wake {workspace}`: wakes a project asleep; one awake is left as
/// it is.
pub async fn remote_wake(
    port: &Port,
    mobile: &Mobile,
    args: &Value,
) -> Result<RemoteWoken, String> {
    let snapshot = port.snapshot();
    let ws = find_workspace(&snapshot.state, args.get("workspace"))?;
    if !is_asleep(&snapshot, &ws.id) {
        return Ok(RemoteWoken { waking: false });
    }
    wake(port, mobile, &ws).await;
    Ok(RemoteWoken { waking: true })
}

/// Refuses, with nothing typed, unless the prompt open now is the one the
/// phone showed (`prompt`, from `remote_chat` or `remote_screen`): Enter
/// approves whatever is open, and the phone's look at the pane is a second or
/// more old — the person may have answered at the PC, the agent moved on to
/// its next call (MOB-02). A question is not a permission: Enter would pick
/// its highlighted option.
///
/// A pane the PC can name no prompt of (a session without a transcript or a
/// side channel: Codex's plain TUI reports none) is answered as asked, as
/// before: the phone has nothing to name either, and showed only its screen.
/// Codex's dialog is then read on the screen before any key (answer.rs).
async fn shown_prompt(
    port: &Port,
    mobile: &Mobile,
    ws: &Workspace,
    pane: &Pane,
    shown: Option<&str>,
) -> Result<Option<ChatPending>, String> {
    let changed = || {
        format!(
            "What \"{}\" asks has changed since the phone showed it: look again. Nothing was sent.",
            pane.name
        )
    };
    let pending = pending_now(port, mobile, ws, pane).await?;
    let as_shown = matches!(&pending, Some(ChatPending::Permission { prompt, .. })
        if shown == Some(prompt.as_str()));
    match pending {
        Some(ChatPending::Question { .. }) => Err(format!(
            "\"{}\" waits on a question, not a permission: answer it in the chat. Nothing was sent.",
            pane.name
        )),
        Some(permission @ ChatPending::Permission { .. }) if as_shown => Ok(Some(permission)),
        Some(ChatPending::Permission { .. }) => Err(changed()),
        None if shown.is_some() => Err(changed()),
        None => Ok(None),
    }
}

/// `remote_permission {pane, allow, forSession?, message?, prompt?}`: the
/// permission a pane waits on, answered on the prompt the phone showed —
/// allowed, for the rest of the session with `forSession`, or denied with
/// the person's words in `message` (`permit_as`).
pub async fn remote_permission(
    port: &Port,
    mobile: &Mobile,
    args: &Value,
) -> Result<RemoteAnswered, String> {
    let Some(allow) = args.get("allow").and_then(Value::as_bool) else {
        return Err("Say whether to allow: \"allow\" is true or false.".into());
    };
    let session = match args.get("forSession") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(session)) => *session,
        Some(_) => return Err("\"forSession\" is true or false.".into()),
    };
    let words = match args.get("message") {
        None | Some(Value::Null) => None,
        Some(Value::String(words)) => Some(words.as_str()),
        Some(_) => return Err("\"message\" is the person's words, as text.".into()),
    };
    let permit = permit_of(allow, session, words)?;
    let (ws, pane) = find_pane(&port.snapshot().state, args.get("pane"))?;
    let shown = args.get("prompt").and_then(Value::as_str);
    permit_as(port, mobile, &ws, &pane, permit, shown).await
}

/// The phone's answer to a permission, checked: words go with a no, the
/// session with a yes.
pub fn permit_of(allow: bool, session: bool, words: Option<&str>) -> Result<Permit, String> {
    let words = words
        .map(str::trim)
        .filter(|words| !words.is_empty())
        .map(str::to_string);
    match (allow, session, words) {
        (true, session, None) => Ok(Permit::Allow { session }),
        (true, _, Some(_)) => {
            Err("Words go with a no: \"message\" is what to do instead of what was denied.".into())
        }
        (false, true, _) => Err(
            "Only an allow is for the session: \"forSession\" goes with \"allow\": true.".into(),
        ),
        (false, false, words) => Ok(Permit::Deny { words }),
    }
}

/// Enter (`allow`) or Escape to a pane that waits, on the prompt shown
/// (`shown`, `ChatPending::Permission`'s `prompt`; see `shown_prompt`).
pub async fn permit_pane(
    port: &Port,
    mobile: &Mobile,
    ws: &Workspace,
    pane: &Pane,
    allow: bool,
    shown: Option<&str>,
) -> Result<RemoteAnswered, String> {
    permit_as(port, mobile, ws, pane, Permit::of(allow), shown).await
}

/// The person's answer to the permission a pane waits on, on the prompt
/// shown: through the ask its side channel holds, for either agent; else
/// with the keys of its agent's own dialog. A no stops the turn, as the
/// dialogs' "No, and tell … what to do differently" does, and the words said
/// with it then reach the session as the person's next message — through
/// its queue, which the side channel delivers once the turn is over, or
/// typed at the prompt the dialog leaves.
pub async fn permit_as(
    port: &Port,
    mobile: &Mobile,
    ws: &Workspace,
    pane: &Pane,
    permit: Permit,
    shown: Option<&str>,
) -> Result<RemoteAnswered, String> {
    let decision = held::permit_decision(&permit);
    let way = answer_dialog(port, mobile, ws, pane, shown, decision, &permit).await?;
    if let Permit::Deny { words: Some(words) } = &permit {
        say_after(port, mobile, ws, pane, words, way).await?;
    }
    Ok(RemoteAnswered {
        answered: true,
        queued: None,
    })
}

/// The plan a pane's ExitPlanMode dialog shows, on the prompt shown:
/// approved, or sent back to keep planning with what to change in it
/// (`feedback`). Through the ask its mod holds, the words reach the model;
/// with keys, Enter approves and Escape keeps planning, without them.
pub async fn plan_pane(
    port: &Port,
    mobile: &Mobile,
    ws: &Workspace,
    pane: &Pane,
    approve: bool,
    feedback: Option<&str>,
    shown: Option<&str>,
) -> Result<RemoteAnswered, String> {
    let decision = held::plan_decision(approve, feedback);
    answer_dialog(
        port,
        mobile,
        ws,
        pane,
        shown,
        decision,
        &Permit::of(approve),
    )
    .await?;
    Ok(RemoteAnswered {
        answered: true,
        queued: None,
    })
}

/// How a dialog was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Way {
    /// Through the ask its side channel held.
    Held,
    /// With keys typed into it.
    Keys,
}

/// Answers the dialog a pane waits on, when it is still the one shown:
/// `decision` through the ask its side channel holds open for it, else the
/// keys of `keys` in the agent's own dialog.
async fn answer_dialog(
    port: &Port,
    mobile: &Mobile,
    ws: &Workspace,
    pane: &Pane,
    shown: Option<&str>,
    decision: Value,
    keys: &Permit,
) -> Result<Way, String> {
    let snapshot = port.snapshot();
    if activity(&snapshot, &pane.id) != Some(AgentActivity::Waiting) {
        // A sleep stops the agents: no dialog outlives it, and Enter or
        // Escape typed while it resumes would reach the shell.
        if is_asleep(&snapshot, &ws.id) {
            wake(port, mobile, ws).await;
            return Err(format!(
                "\"{}\" was asleep, so \"{}\" no longer waits for a permission. It is waking up now; nothing was sent.",
                ws.name, pane.name
            ));
        }
        return Err(format!(
            "\"{}\" is not waiting for a permission: nothing was sent.",
            pane.name
        ));
    }
    let pending = shown_prompt(port, mobile, ws, pane, shown).await?;
    // The ask the pane's side channel holds open for this dialog takes the
    // answer itself: no key to land anywhere else.
    if let Some(ChatPending::Permission { tool, .. }) = &pending {
        let input = mobile.book().chat.permission_input(&pane.id, tool);
        let ask = held::permission(ws, pane, tool, input.as_ref());
        if let Some(ask) = ask
            && held::give(port, pane, &ask, decision).await
        {
            return Ok(Way::Held);
        }
    }
    if Dialogs::of(pane) == Some(Dialogs::Codex) {
        press_codex(port, mobile, ws, pane, keys).await?;
        return Ok(Way::Keys);
    }
    let key = if keys.allows() { "enter" } else { "escape" };
    let input = Input {
        keys: vec![key.into()],
        ..Input::default()
    };
    type_into(port, mobile, ws, pane, input).await?;
    Ok(Way::Keys)
}

/// Codex's approval dialog answered with its own keys, once the screen shows
/// it as Codex draws it (answer.rs): the cursor moved to the option the
/// person chose, then Enter; Escape for a no. Nothing typed into a pane
/// another answer is being typed into (MOB-18), nor into one not up yet.
async fn press_codex(
    port: &Port,
    mobile: &Mobile,
    ws: &Workspace,
    pane: &Pane,
    permit: &Permit,
) -> Result<(), String> {
    if resumes(port, mobile, pane) {
        // No dialog outlives a sleep, and keys cannot wait in the queue.
        return hold(port, mobile, ws, pane, None).await;
    }
    if port.terminal(&pane.id).is_none() {
        return Err(answer::CODEX_NOT_ON_SCREEN.into());
    }
    let Some(_answering) = Answering::start(mobile, &pane.id) else {
        return Err(format!(
            "An answer is being typed into \"{}\": wait for it. Nothing was sent.",
            pane.name
        ));
    };
    let io = PaneIo::new(port, ws, pane);
    answer::press_codex_approval(&io, permit).await
}

/// The words said with a no, sent to the session as the person's next
/// message once the dialog is down: into the pane's queue when its side
/// channel took the no (it delivers them once the turn is over), else typed
/// at the prompt the dialog leaves — Codex's checked to be gone first.
async fn say_after(
    port: &Port,
    mobile: &Mobile,
    ws: &Workspace,
    pane: &Pane,
    words: &str,
    way: Way,
) -> Result<(), String> {
    if way == Way::Held {
        return port.enqueue(&pane.id, words, None).await;
    }
    if Dialogs::of(pane) == Some(Dialogs::Codex) {
        let io = PaneIo::new(port, ws, pane);
        answer::codex_approval_gone(&io).await?;
    } else {
        port.sleep(WORDS_AFTER_MS).await;
    }
    let input = Input {
        text: Some(words.to_string()),
        ..Input::default()
    };
    type_into(port, mobile, ws, pane, input).await.map(|_| ())
}

/// `remote_manager {text}`: typed into the project manager's pane.
pub async fn remote_manager(
    port: &Port,
    mobile: &Mobile,
    args: &Value,
) -> Result<RemoteSent, String> {
    let text = args.get("text").and_then(Value::as_str).unwrap_or("");
    if text.trim().is_empty() {
        return Err("Nothing to send: give text.".into());
    }
    let snapshot = port.snapshot();
    let Some((ws, pane)) = manager_of(&snapshot.state) else {
        return Err(NO_MANAGER.into());
    };
    let input = Input {
        text: Some(text.to_string()),
        ..Input::default()
    };
    type_into(port, mobile, ws, pane, input).await
}

/// A line written without words (Hangar stopping is written by core itself)
/// gets them here, in the PC's language (`textOf`).
pub fn text_of(entry: &hangar_core::mobile_data::ActivityEntry, locale: Locale) -> String {
    if !entry.text.is_empty() {
        return entry.text.clone();
    }
    match entry.kind.as_str() {
        "appStarted" => t(locale, Key::ActivityAppStarted).into_owned(),
        "appStopped" => t(locale, Key::ActivityAppStopped).into_owned(),
        other => other.to_string(),
    }
}

/// `remote_history {since?}`: what happened, from the log kept on disk
/// (activity_log.rs), newest first. The journals in memory forget with every
/// restart; the disk does not.
pub async fn remote_history(
    port: &Port,
    mobile: &Mobile,
    args: &Value,
) -> Result<RemoteHistory, String> {
    let from = args
        .get("since")
        .and_then(Value::as_f64)
        .filter(|since| *since > 0.0)
        .unwrap_or(0.0);
    let locale = port
        .snapshot()
        .state
        .settings
        .locale
        .unwrap_or_else(Locale::system);
    let mut entries: Vec<_> = mobile
        .sources()
        .history(from as u64)
        .await?
        .into_iter()
        .filter(|entry| entry.at as f64 > from)
        .map(|mut entry| {
            entry.text = text_of(&entry, locale);
            entry
        })
        .collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.at));
    Ok(RemoteHistory { entries })
}

/// A folder as the Windows way compares it: either separator, any case, no
/// trailing one.
fn folder_key(path: &str) -> String {
    let mut key = String::new();
    let mut separator = false;
    for c in path.trim().chars() {
        if c == '/' || c == '\\' {
            if !separator {
                key.push('/');
            }
            separator = true;
        } else {
            key.push(c);
            separator = false;
        }
    }
    key.trim_end_matches('/').to_lowercase()
}

/// `remote_workspace {workspace? | folder?}`: a workspace by id, or by folder.
/// Not one of the phone's calls (core's `CALLS` leaves it out): the mobile
/// server asks it itself, for the folder an upload lands in.
pub fn remote_workspace(port: &Port, args: &Value) -> Result<RemoteWorkspace, String> {
    let id = text_arg(args, "workspace");
    let folder = text_arg(args, "folder");
    if id.is_empty() && folder.is_empty() {
        return Err("Name the workspace, by id or by folder.".into());
    }
    let snapshot = port.snapshot();
    let workspaces = &snapshot.state.workspaces;
    let found = if id.is_empty() {
        workspaces
            .iter()
            .find(|ws| folder_key(&ws.cwd) == folder_key(&folder))
    } else {
        workspaces.iter().find(|ws| ws.id == id)
    };
    let Some(ws) = found else {
        let which = if id.is_empty() {
            format!("at \"{folder}\"")
        } else {
            format!("with id \"{id}\"")
        };
        return Err(format!("No workspace {which} in Hangar."));
    };
    Ok(RemoteWorkspace {
        id: ws.id.clone(),
        name: ws.name.clone(),
        cwd: ws.cwd.clone(),
    })
}

#[cfg(test)]
mod tests {
    //! A Codex pane's permissions and questions answered from the phone or
    //! the chat: through the asks Hangar's client of its server holds
    //! (hangar-core agent/codex_attach.rs), reported here in its shapes, and
    //! never by keys unless the screen shows Codex's dialog. Claude Code's
    //! own paths are the suites' of mobile/tests/.

    use std::sync::Arc;
    use std::time::Duration;

    use hangar_core::bridge::{self as core_bridge, AskPoll};
    use hangar_core::chat::ChatPage;
    use hangar_core::git::{ChangedFile, Changes, FileDiff};
    use hangar_core::mobile_data::ActivityEntry;
    use hangar_core::stats::UsageStats;

    use super::*;
    use crate::control::fake::{Fake, lock};
    use crate::control::port::BoxFuture;
    use crate::live::BridgeEvent;
    use crate::mobile::answer::{CODEX_NOT_ON_SCREEN, answer_pane, choices_of};
    use crate::mobile::chat_live::pending_now as live_pending;
    use crate::mobile::engine::{ChatRead, Sources};
    use crate::orchestration::manager_feed::ManagerJournalEntry;
    use crate::orchestration::projects::ProjectSummary;
    use crate::state::PaneActivity;

    /// What a Codex pane reads beyond the port: nothing here (its pages
    /// come from its rollout, which a thread made up for a test has none of).
    struct Nothing;

    fn ready<T: Send + 'static>(value: T) -> BoxFuture<T> {
        Box::pin(std::future::ready(value))
    }

    impl Sources for Nothing {
        fn projects(&self) -> BoxFuture<Vec<ProjectSummary>> {
            ready(Vec::new())
        }
        fn manager_journal(&self) -> Vec<ManagerJournalEntry> {
            Vec::new()
        }
        fn history(&self, _: u64) -> BoxFuture<Result<Vec<ActivityEntry>, String>> {
            ready(Ok(Vec::new()))
        }
        fn chat(&self, _: ChatRead) -> BoxFuture<Result<Option<ChatPage>, String>> {
            ready(Err(
                "no Claude Code transcript is read for a Codex pane".into()
            ))
        }
        fn usage(&self) -> BoxFuture<Result<Arc<UsageStats>, String>> {
            ready(Err("no statistics".into()))
        }
        fn worktrees(&self, _: &str) -> BoxFuture<Result<Vec<String>, String>> {
            ready(Ok(Vec::new()))
        }
        fn stats_now(&self) -> i64 {
            0
        }
        fn changes(&self, _: &str, _: &str) -> BoxFuture<Result<Changes, String>> {
            ready(Err("no git".into()))
        }
        fn file_diff(
            &self,
            _: &str,
            _: &ChangedFile,
            _: &str,
        ) -> BoxFuture<Result<FileDiff, String>> {
            ready(Err("no git".into()))
        }
        fn wait(&self, _: u64) -> BoxFuture<()> {
            ready(())
        }
        fn listening(&self, _: Vec<u16>) -> BoxFuture<Result<Vec<u16>, String>> {
            ready(Ok(Vec::new()))
        }
        fn tasks(&self, _: &str) -> BoxFuture<Result<Vec<(String, String)>, String>> {
            ready(Ok(Vec::new()))
        }
    }

    const CWD: &str = "/repo";

    /// One workspace with Codex pane `pane`, its thread made up, waiting,
    /// its terminal showing a shell.
    fn codex(pane: &str) -> (Arc<Fake>, Arc<Mobile>, Workspace, Pane) {
        let cody = Pane {
            id: pane.into(),
            name: "Cody".into(),
            agent: AgentId::Codex,
            session_id: Some("019a7c2e-5b1d-7f43-9a61-000000000018".into()),
            ..Pane::default()
        };
        let ws = Workspace {
            id: "w1".into(),
            name: "Repo".into(),
            cwd: CWD.into(),
            panes: vec![cody.clone()],
            ..Workspace::default()
        };
        let fake = Fake::with_state(AppState {
            workspaces: vec![ws.clone()],
            ..AppState::default()
        });
        *lock(&fake.terminals) = vec![pane.to_string()];
        lock(&fake.activity).insert(
            pane.to_string(),
            PaneActivity {
                workspace_id: "w1".into(),
                activity: AgentActivity::Waiting,
                bridged: true,
            },
        );
        (fake, Mobile::new(Arc::new(Nothing)), ws, cody)
    }

    /// What Hangar's client of the pane's server reports, to the window and
    /// to core's held asks alike (bridge.rs, `report`).
    fn report(mobile: &Mobile, pane: &str, event: Value) {
        core_bridge::note_report(pane, &event);
        if let Some(parsed) = BridgeEvent::parse(&event) {
            mobile.note_bridge_event(pane, &parsed, 1_000);
        }
    }

    /// Codex's command approval, held: `item/commandExecution/requestApproval`
    /// as `describe` reports it.
    fn command_asked(mobile: &Mobile, pane: &str, ask: &str) {
        let input = json!({"command": "pnpm test", "cwd": CWD, "reason": "needs the network"});
        report(
            mobile,
            pane,
            json!({"kind": "tool", "phase": "start", "toolUseId": "item-7", "tool": "Bash",
                   "input": {"command": "pnpm test", "cwd": CWD}}),
        );
        report(
            mobile,
            pane,
            json!({"kind": "permission", "waiting": true, "tool": "Bash", "input": input,
                   "askId": ask, "toolUseId": "item-7"}),
        );
    }

    /// Codex's side of the held ask: what it is answered with.
    fn codex_takes(pane: &str, ask: &str) -> tokio::task::JoinHandle<AskPoll> {
        let (pane, ask) = (pane.to_string(), ask.to_string());
        tokio::spawn(async move {
            core_bridge::await_decision(&pane, &ask, Duration::from_secs(5)).await
        })
    }

    async fn shown(port: &Port, mobile: &Mobile, ws: &Workspace, pane: &Pane) -> String {
        match live_pending(port, mobile, ws, pane).await {
            Ok(Some(ChatPending::Permission { prompt, .. })) => prompt,
            other => panic!("no permission pending: {other:?}"),
        }
    }

    #[test]
    fn each_agent_s_side_channel_is_its_own_setting() {
        let mut settings = Settings::default();
        let pane = |agent| Pane {
            agent,
            ..Pane::default()
        };
        let on = |settings: &Settings, agent| side_channel(settings, &pane(agent)).map(|c| c.on);
        assert_eq!(on(&settings, AgentId::Claude), Some(true));
        assert_eq!(on(&settings, AgentId::Codex), Some(true));
        assert_eq!(on(&settings, AgentId::Shell), None);
        settings.codex_attach = false;
        assert_eq!(on(&settings, AgentId::Codex), Some(false));
        assert_eq!(on(&settings, AgentId::Claude), Some(true));
        settings.claude_bridge = false;
        settings.codex_attach = true;
        assert_eq!(on(&settings, AgentId::Claude), Some(false));
        assert_eq!(on(&settings, AgentId::Codex), Some(true));
    }

    #[test]
    fn the_phone_s_answer_to_a_permission_is_checked() {
        assert_eq!(
            permit_of(true, false, None),
            Ok(Permit::Allow { session: false })
        );
        assert_eq!(
            permit_of(true, true, Some("  ")),
            Ok(Permit::Allow { session: true })
        );
        assert_eq!(
            permit_of(false, false, Some("  use cargo  ")),
            Ok(Permit::Deny {
                words: Some("use cargo".into())
            })
        );
        assert_eq!(
            permit_of(false, false, None),
            Ok(Permit::Deny { words: None })
        );
        assert!(permit_of(true, false, Some("why")).is_err());
        assert!(permit_of(false, true, None).is_err());
    }

    #[tokio::test]
    async fn a_codex_command_is_allowed_for_the_session_through_the_ask_held() {
        const PANE: &str = "codex-held-session";
        let (fake, mobile, ws, cody) = codex(PANE);
        let port = fake.port();
        command_asked(&mobile, PANE, "codex-1-7");
        let codex = codex_takes(PANE, "codex-1-7");
        let prompt = shown(&port, &mobile, &ws, &cody).await;
        let answered = permit_as(
            &port,
            &mobile,
            &ws,
            &cody,
            Permit::Allow { session: true },
            Some(&prompt),
        )
        .await;
        assert_eq!(
            answered,
            Ok(RemoteAnswered {
                answered: true,
                queued: None
            })
        );
        assert_eq!(
            codex.await.unwrap(),
            AskPoll::Decision(json!({"behavior": "allow", "forSession": true}))
        );
        assert!(fake.typed().is_empty(), "no key typed");
        assert!(core_bridge::held_asks(PANE).is_empty());
    }

    #[tokio::test]
    async fn a_codex_command_denied_with_words_stops_the_turn_and_queues_them() {
        const PANE: &str = "codex-held-words";
        let (fake, mobile, ws, cody) = codex(PANE);
        let port = fake.port();
        command_asked(&mobile, PANE, "codex-2-7");
        let codex = codex_takes(PANE, "codex-2-7");
        let prompt = shown(&port, &mobile, &ws, &cody).await;
        let no = Permit::Deny {
            words: Some("Run cargo test instead.".into()),
        };
        assert!(
            permit_as(&port, &mobile, &ws, &cody, no, Some(&prompt))
                .await
                .is_ok()
        );
        assert_eq!(
            codex.await.unwrap(),
            AskPoll::Decision(json!({"behavior": "deny", "interrupt": true}))
        );
        // The person's own words, for the session to take once the turn is
        // over (codex_attach.rs delivers them as the next turn).
        assert_eq!(
            fake.logged("enqueue:"),
            [format!("enqueue:{PANE}:Run cargo test instead.")]
        );
        assert_eq!(
            *lock(&fake.senders),
            ["Run cargo test instead. <- the person"]
        );
        assert!(fake.typed().is_empty());
    }

    #[tokio::test]
    async fn a_codex_pane_on_its_plain_tui_gets_no_key_unless_its_dialog_shows() {
        const PANE: &str = "codex-floor";
        let (fake, mobile, ws, cody) = codex(PANE);
        let port = fake.port();
        // No side channel: no prompt to name, no ask held. The terminal
        // shows a shell, not Codex's dialog.
        for permit in [
            Permit::Allow { session: false },
            Permit::Allow { session: true },
            Permit::Deny {
                words: Some("stop".into()),
            },
        ] {
            assert_eq!(
                permit_as(&port, &mobile, &ws, &cody, permit, None).await,
                Err(CODEX_NOT_ON_SCREEN.to_string())
            );
        }
        assert!(fake.typed().is_empty(), "nothing typed: {:?}", fake.typed());
        assert!(fake.logged("enqueue:").is_empty());
        assert!(mobile.book().answering.is_empty(), "the pane let go");
    }

    #[tokio::test]
    async fn a_codex_question_is_answered_by_id_through_the_ask_held() {
        const PANE: &str = "codex-held-question";
        let (fake, mobile, ws, cody) = codex(PANE);
        let port = fake.port();
        let input = json!({"questions": [{
            "id": "deploy", "question": "Deploy now?", "header": "Deploy", "multiSelect": false,
            "options": [{"label": "Yes", "description": "Ship it"}, {"label": "No", "description": ""}],
            "isOther": true, "isSecret": false,
        }]});
        report(
            &mobile,
            PANE,
            json!({"kind": "tool", "phase": "start", "toolUseId": "item-11",
                   "tool": "AskUserQuestion", "input": input}),
        );
        report(
            &mobile,
            PANE,
            json!({"kind": "permission", "waiting": true, "tool": "AskUserQuestion", "input": input,
                   "askId": "codex-3-21", "toolUseId": "item-11"}),
        );
        let codex = codex_takes(PANE, "codex-3-21");
        let answers = json!([{"other": "after the freeze"}]);
        let answered = answer_pane(&port, &mobile, &ws, &cody, "item-11", |questions| {
            choices_of(questions, Some(&answers))
        })
        .await;
        assert_eq!(
            answered,
            Ok(RemoteAnswered {
                answered: true,
                queued: None
            })
        );
        assert_eq!(
            codex.await.unwrap(),
            AskPoll::Decision(json!({"behavior": "allow", "answers": {
                "deploy": ["None of the above", "user_note: after the freeze"],
            }}))
        );
        assert!(fake.typed().is_empty());

        // Asked of a question Codex no longer waits on, Codex is named.
        let gone = answer_pane(&port, &mobile, &ws, &cody, "item-99", |questions| {
            choices_of(questions, Some(&answers))
        })
        .await;
        assert_eq!(gone, Err("Codex now waits on another question.".into()));
    }
}
