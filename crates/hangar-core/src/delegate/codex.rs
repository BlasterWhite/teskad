//! A Codex child (docs/native/agents.md §4.3, "Headless children"): a `codex app-server` of its
//! own on stdio, Hangar its only client, one thread, one turn.
//!
//! - **Its server** is the pane server's launch (agent/codex.rs, T4a) — the same command, the
//!   workspace's permission policy, the model, the role as developer instructions — listening on
//!   stdio instead of a socket, on the workspace's Codex account (`CODEX_HOME`, accounts.rs), with
//!   image generation on or off as the delegation says (D15).
//! - **Its work**: `thread/start` in the child's folder, `turn/start` with the task; the turn's
//!   items say what it changed (`fileChange`) and generated (`imageGeneration`), its last agent
//!   message is the result, `thread/tokenUsage/updated` what it spent; `turn/completed` ends it.
//!   Cancelled, the turn is interrupted (`turn/interrupt`) and the server stopped.
//! - **What it asks** is held for the person as a pane's asks are, by the code of Hangar's client
//!   of a pane's server (agent/codex_attach.rs: `describe`, `answer_when_decided`), under the
//!   delegation's id; nothing here answers for the person. A request Hangar cannot read is
//!   refused (`Unhandled::Refuse`): Hangar is the server's only client, and nothing else would
//!   answer it.
//!
//! Every method, field and order here is the protocol schema's at the pinned release
//! (`codex_app::CODEX_PINNED`) — `thread/start`, `turn/start`, `turn/interrupt` and the
//! notifications of a turn — and the order of a turn's notifications the docs' (`app-server`).
//! Nothing was run against a Codex (§4.0).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::process::Command;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::{worktree, Connect, Live, Outcome, Request, Status, Tokens};
use crate::agent::codex_attach::{
    absolute, answer_when_decided, ask_detail, describe, key_of, stopped,
};
use crate::agent::launch::{self, LaunchRequest};
use crate::agent::{AgentKind, PermissionMode};
use crate::bridge;
use crate::codex_app::generated::{
    FileUpdateChange, MessagePhase, PatchApplyStatus, ServerNotification, TextUserInput,
    ThreadItem, ThreadStartParams, TokenUsageBreakdown, TurnInterruptParams, TurnStartParams,
    TurnStatus, UserInput,
};
use crate::codex_app::{
    requests, Cause, Client, Closed, Endpoint, Event, Events, Incoming, Options, Unhandled,
};

/// The child's role, as developer instructions: what it is for, and that nobody watches it.
const ROLE: &str = "You are working on a sub-task another agent handed over to you through \
Hangar. Nobody watches this session while it runs: do the task on your own, then end with your \
final answer, which is all the other agent reads of your work. Say in it what you did and what \
you changed. What you need the person to allow, ask for as you would anyway: Hangar shows it to \
them.";
/// Added to [`ROLE`] for a child in a worktree of its own.
const WORKTREE_ROLE: &str = "You work in a git worktree of your own, on its branch, made from \
the other agent's last commit: commit your work there before you answer.";
/// `app-server --listen`: the address T4a's launch gives (a pane's socket), replaced by stdio.
const LISTEN_FLAG: &str = "--listen";
const STDIO: &str = "stdio://";
/// A socket name for T4a's launch to have one: it makes the server's settings, then stdio takes
/// its place. Never created.
const NO_SOCKET: &str = "delegate";
/// `-c features.image_generation=…`, the stable feature (agent/codex.rs): the launch turns it
/// on; given again after, it says what the delegation allows — `-c` builds its layer in the order
/// given, a later key replacing an earlier one (config/src/overrides.rs, as agent/codex.rs reads
/// it).
const IMAGE_GENERATION: &str = "features.image_generation";
/// How long the child's server is given to answer a request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a cancelled turn's interruption is waited for before the server is stopped anyway.
const INTERRUPT_TIMEOUT: Duration = Duration::from_secs(5);
/// What the child's connection asks Codex not to send: the streams of a turn, which nothing here
/// reads (the completed items say it all).
const OPTED_OUT: &[&str] = &[
    "item/agentMessage/delta",
    "item/plan/delta",
    "item/commandExecution/outputDelta",
    "item/mcpToolCall/progress",
    "turn/diff/updated",
    "turn/plan/updated",
];

/// The server a Codex child runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Spawn {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    /// The account's variables (accounts.rs, `env_for`): set, or taken out.
    pub env: Vec<(&'static str, Option<PathBuf>)>,
}

/// The child's server for `request`, working in `cwd` under the stepped-down `mode`: T4a's launch
/// of a pane's server, on stdio, with the delegation's image switch last.
pub(super) fn spawn_spec(
    request: &Request,
    cwd: &str,
    mode: Option<PermissionMode>,
    made: Option<&worktree::Made>,
) -> Result<Spawn, String> {
    let account =
        crate::accounts::for_launch(Some(AgentKind::Codex.id()), request.account.as_deref());
    let account_dir = account.and_then(|account| account.dir);
    let mut spawn = server_spec(request, cwd, mode, made, account_dir.clone())?;
    spawn.env = crate::accounts::env_for(Some(AgentKind::Codex.id()), account_dir.as_deref());
    // The settings' command as it is; the bare program, where its installer put it when PATH
    // does not say (`install::agent`).
    if request
        .command
        .as_deref()
        .is_none_or(|command| command.trim().is_empty())
    {
        if let Some(found) = crate::install::agent::find(AgentKind::Codex) {
            spawn.program = found.to_string_lossy().into_owned();
        }
    }
    Ok(spawn)
}

/// [`spawn_spec`] without what it reads of the machine (the account's folder, where Codex is).
fn server_spec(
    request: &Request,
    cwd: &str,
    mode: Option<PermissionMode>,
    made: Option<&worktree::Made>,
    account_dir: Option<PathBuf>,
) -> Result<Spawn, String> {
    let role = match made {
        Some(_) => format!("{ROLE}\n\n{WORKTREE_ROLE}"),
        None => ROLE.to_string(),
    };
    let launch = LaunchRequest {
        kind: AgentKind::Codex,
        command: request.command.clone(),
        cwd: cwd.to_string(),
        model: request.model.clone(),
        permission_mode: mode,
        account_dir,
        role: Some(role),
        server_socket: Some(NO_SOCKET.into()),
        ..LaunchRequest::default()
    };
    let server = launch::server(&launch)
        .ok_or_else(|| "Hangar could not make the launch of Codex's app-server".to_string())?;
    let mut args = server.args;
    let at = args
        .iter()
        .position(|arg| arg == LISTEN_FLAG)
        .filter(|at| at + 1 < args.len())
        .ok_or_else(|| "Codex's app-server launch has no address to listen on".to_string())?;
    args[at + 1] = STDIO.into();
    args.push("-c".into());
    args.push(format!("{IMAGE_GENERATION}={}", request.images));
    Ok(Spawn {
        program: server.program,
        args,
        cwd: server.cwd,
        env: Vec::new(),
    })
}

/// The connection to a child's server spawned from `spawn`.
pub(super) fn stdio(spawn: Spawn) -> Connect {
    Box::new(move |options| {
        Box::pin(async move { Client::connect(Endpoint::Stdio(command(spawn)), options).await })
    })
}

/// The process of `spawn`: its arguments already say `app-server --listen stdio://`
/// (`codex_app::app_server_command`'s), with the launch's settings after.
fn command(spawn: Spawn) -> Command {
    crate::path_env::ensure();
    let mut command = std::process::Command::new(&spawn.program);
    command.args(&spawn.args).current_dir(&spawn.cwd);
    // Not the identity of the pane Hangar may have been started from.
    crate::delivery::scrub(&mut command);
    for (name, value) in &spawn.env {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    Command::from(command)
}

/// How Hangar's client of a child's server connects: the only client, so what it does not handle
/// is refused rather than left waiting (D10).
fn options() -> Options {
    Options {
        request_timeout: REQUEST_TIMEOUT,
        unhandled: Unhandled::Refuse,
        opt_out_notifications: OPTED_OUT.iter().map(|method| method.to_string()).collect(),
        ..Options::default()
    }
}

/// Runs the child to its end: connects, starts its thread and its turn, follows the turn, holds
/// what it asks for the person, and stops it when `stop` says so.
pub(super) async fn run(
    live: Live,
    connect: Connect,
    task: String,
    cwd: String,
    mut stop: watch::Receiver<bool>,
) -> Outcome {
    let connected = tokio::select! {
        connected = connect(options()) => connected,
        () = stopped(&mut stop) => return Outcome::cancelled(),
    };
    let (client, events) = match connected {
        Ok(connected) => connected,
        Err(error) => return Outcome::failed(error.to_string()),
    };
    let client = Arc::new(client);
    let mut child = Child::new(live, Arc::clone(&client), cwd);
    let ended = child.work(task, events, &mut stop).await;
    child.finish();
    client.shutdown().await;
    child.outcome(ended)
}

/// How the turn ended, as [`Child::work`] saw it.
enum Ended {
    Turn(TurnStatus, Option<String>),
    Cancelled,
    Failed(String),
}

impl Outcome {
    fn cancelled() -> Outcome {
        Outcome {
            status: Status::Cancelled,
            result: None,
            files: Vec::new(),
            images: Vec::new(),
            error: None,
        }
    }

    fn failed(error: String) -> Outcome {
        Outcome {
            status: Status::Failed,
            error: Some(error),
            ..Outcome::cancelled()
        }
    }
}

/// One child, over its connection.
struct Child {
    live: Live,
    client: Arc<Client>,
    cwd: String,
    thread: Option<String>,
    turn: Option<String>,
    /// The thread's last agent message, and whether it was its final answer.
    answer: Option<(String, bool)>,
    files: Vec<String>,
    images: Vec<String>,
    /// The changes of the patches in progress, by item: what an approval of one shows.
    patches: HashMap<String, Vec<FileUpdateChange>>,
    /// Asks held for the person, by Codex's request id.
    held: HashMap<String, String>,
    waiters: Vec<JoinHandle<()>>,
    /// The request ids whose wait for the person is over, from the waits.
    over_tx: mpsc::UnboundedSender<String>,
    over_rx: mpsc::UnboundedReceiver<String>,
    /// The last error Codex reported that it will not retry.
    error: Option<String>,
}

impl Child {
    fn new(live: Live, client: Arc<Client>, cwd: String) -> Child {
        let (over_tx, over_rx) = mpsc::unbounded_channel();
        Child {
            live,
            client,
            cwd,
            thread: None,
            turn: None,
            answer: None,
            files: Vec::new(),
            images: Vec::new(),
            patches: HashMap::new(),
            held: HashMap::new(),
            waiters: Vec::new(),
            over_tx,
            over_rx,
            error: None,
        }
    }

    /// Starts the thread and the turn, then follows the turn to its end.
    async fn work(
        &mut self,
        task: String,
        mut events: Events,
        stop: &mut watch::Receiver<bool>,
    ) -> Ended {
        let params = ThreadStartParams {
            cwd: Some(self.cwd.clone()),
            ..ThreadStartParams::default()
        };
        let started = tokio::select! {
            started = self.client.request::<requests::ThreadStart>(params) => started,
            () = stopped(stop) => return Ended::Cancelled,
        };
        let started = match started {
            Ok(started) => started,
            Err(error) => return Ended::Failed(error.to_string()),
        };
        let thread = started.thread.id.clone();
        let model = started.model.clone();
        let session = thread.clone();
        self.live.update(|delegation| {
            delegation.session = Some(session);
            if !model.is_empty() {
                delegation.model = Some(model);
            }
        });
        self.thread = Some(thread.clone());

        let params = TurnStartParams {
            input: vec![UserInput::Text(TextUserInput {
                text: task,
                text_elements: None,
            })],
            thread_id: thread,
            ..TurnStartParams::default()
        };
        let turn = tokio::select! {
            turn = self.client.request::<requests::TurnStart>(params) => turn,
            () = stopped(stop) => return Ended::Cancelled,
        };
        match turn {
            Ok(turn) => {
                if self.turn.is_none() {
                    self.turn = Some(turn.turn.id);
                }
            }
            Err(error) => return Ended::Failed(error.to_string()),
        }

        loop {
            tokio::select! {
                event = events.next() => match event {
                    Some(Event::Notification(notification)) => {
                        if let Some(ended) = self.on_notification(notification) {
                            return ended;
                        }
                    }
                    Some(Event::Request(request)) => self.on_request(request),
                    Some(Event::Closed(closed)) => return Ended::Failed(closed.to_string()),
                    None => {
                        let closed = self.client.closed().unwrap_or(Closed {
                            cause: Cause::ServerClosed,
                            stderr: Vec::new(),
                        });
                        return Ended::Failed(closed.to_string());
                    }
                },
                Some(key) = self.over_rx.recv() => self.settle(&key),
                () = stopped(stop) => {
                    self.interrupt().await;
                    return Ended::Cancelled;
                }
            }
        }
    }

    fn ours(&self, thread: &str) -> bool {
        self.thread.as_deref() == Some(thread)
    }

    /// What a notification tells of the turn; its end, when it ends it.
    fn on_notification(&mut self, notification: ServerNotification) -> Option<Ended> {
        match notification {
            ServerNotification::TurnStarted(started) => {
                if self.ours(&started.thread_id) {
                    self.turn = Some(started.turn.id);
                }
            }
            ServerNotification::ItemStarted(started) => {
                if let ThreadItem::FileChange(item) = &started.item {
                    self.patches.insert(item.id.clone(), item.changes.clone());
                }
            }
            ServerNotification::ItemFileChangePatchUpdated(updated) => {
                self.patches.insert(updated.item_id, updated.changes);
            }
            ServerNotification::ItemCompleted(completed) => {
                let ours = self.ours(&completed.thread_id);
                self.on_item(ours, completed.item);
            }
            ServerNotification::ThreadTokenUsageUpdated(updated) => {
                if self.ours(&updated.thread_id) {
                    let tokens = tokens_of(&updated.token_usage.total);
                    self.live
                        .update(|delegation| delegation.tokens = Some(tokens));
                }
            }
            ServerNotification::Error(error) => {
                if self.ours(&error.thread_id) && !error.will_retry {
                    self.error = Some(error.error.message);
                }
            }
            ServerNotification::ServerRequestResolved(resolved) => {
                self.settle(&key_of(&resolved.request_id));
            }
            ServerNotification::TurnCompleted(completed) => {
                let ours = self.ours(&completed.thread_id)
                    && self
                        .turn
                        .as_deref()
                        .is_none_or(|turn| turn == completed.turn.id);
                if ours {
                    let error = completed.turn.error.map(|error| error.message);
                    return Some(Ended::Turn(completed.turn.status, error));
                }
            }
            ServerNotification::ThreadClosed(closed) if self.ours(&closed.thread_id) => {
                return Some(Ended::Failed(
                    "Codex closed the thread before its turn ended".into(),
                ));
            }
            _ => {}
        }
        None
    }

    /// A completed item: the answer when it is the thread's agent message; changes and images
    /// from any thread of the child's (a subagent's included: it is the child's work too).
    fn on_item(&mut self, ours: bool, item: ThreadItem) {
        match item {
            ThreadItem::AgentMessage(message) if ours => {
                if message.text.trim().is_empty() {
                    return;
                }
                // A final answer stays the answer: a remark after it does not replace it.
                let last_word = matches!(message.phase, Some(MessagePhase::FinalAnswer));
                if last_word || !self.answer.as_ref().is_some_and(|(_, last)| *last) {
                    self.answer = Some((message.text, last_word));
                }
            }
            ThreadItem::FileChange(item) => {
                self.patches.remove(&item.id);
                if item.status == PatchApplyStatus::Completed {
                    for change in &item.changes {
                        let file = absolute(&self.cwd, &change.path);
                        if !self.files.contains(&file) {
                            self.files.push(file);
                        }
                    }
                }
            }
            // D15: only an image completed without a failure counts; its file stays where Codex
            // saved it.
            ThreadItem::ImageGeneration(image)
                if image.status == "completed" && image.failure.is_none() =>
            {
                if let Some(path) = image.saved_path.filter(|path| !path.is_empty()) {
                    if !self.images.contains(&path) {
                        self.images.push(path);
                    }
                }
            }
            _ => {}
        }
    }

    /// A request of Codex's for the person, held as an ask of the delegation's, and answered with
    /// the person's choice once given. One Hangar cannot read is refused as it is dropped.
    fn on_request(&mut self, incoming: Incoming) {
        let key = key_of(incoming.id());
        let ask = format!("codex-{}-{}", self.live.id, key.trim_matches('"'));
        let Some(asked) = describe(incoming, &self.patches) else {
            return;
        };
        let mut event = ask_detail(&asked, &ask);
        event.insert("kind".into(), json!("permission"));
        event.insert("waiting".into(), json!(true));
        // Held by the bridge as it is reported (`note_report`), before its wait starts below.
        bridge::report(&self.live.host, &self.live.id, Value::Object(event));
        self.held.insert(key.clone(), ask.clone());
        self.live.needs_you(true);
        let holder = self.live.id.clone();
        let client = Arc::clone(&self.client);
        let over = self.over_tx.clone();
        let waiter = tokio::spawn(async move {
            if let Err(why) = answer_when_decided(&holder, &client, &ask, asked).await {
                eprintln!("[delegate] {holder}: {why}");
            }
            let _ = over.send(key);
        });
        self.waiters.retain(|waiter| !waiter.is_finished());
        self.waiters.push(waiter);
    }

    /// The ask on request `key` is over: answered, or ended by Codex.
    fn settle(&mut self, key: &str) {
        let Some(ask) = self.held.remove(key) else {
            return;
        };
        self.end_ask(&ask);
        if self.held.is_empty() {
            self.live.needs_you(false);
        }
    }

    fn end_ask(&self, ask: &str) {
        bridge::report(
            &self.live.host,
            &self.live.id,
            json!({ "kind": "ask", "phase": "end", "askId": ask }),
        );
    }

    /// Cancelled: the running turn is asked to stop, briefly; the server goes after anyway.
    async fn interrupt(&self) {
        let (Some(thread), Some(turn)) = (self.thread.clone(), self.turn.clone()) else {
            return;
        };
        let params = TurnInterruptParams {
            thread_id: thread,
            turn_id: turn,
        };
        if let Err(error) = self
            .client
            .request_within::<requests::TurnInterrupt>(params, INTERRUPT_TIMEOUT)
            .await
        {
            eprintln!(
                "[delegate] {}: could not stop the turn: {error}",
                self.live.id
            );
        }
    }

    /// The child is over: its asks are withdrawn, their waits with them.
    fn finish(&mut self) {
        for waiter in self.waiters.drain(..) {
            waiter.abort();
        }
        for ask in std::mem::take(&mut self.held).into_values() {
            self.end_ask(&ask);
        }
    }

    fn outcome(self, ended: Ended) -> Outcome {
        let (status, error) = match ended {
            Ended::Turn(TurnStatus::Completed, _) => (Status::Done, None),
            Ended::Turn(TurnStatus::Failed, error) => (
                Status::Failed,
                Some(
                    error
                        .or(self.error)
                        .unwrap_or_else(|| "Codex's turn failed".into()),
                ),
            ),
            // Stopped by the person's answer to one of its asks ("no, and stop").
            Ended::Turn(TurnStatus::Interrupted, _) => (
                Status::Cancelled,
                Some("its turn was stopped before its end".into()),
            ),
            Ended::Turn(other, error) => (
                Status::Failed,
                Some(error.unwrap_or_else(|| format!("Codex's turn ended as {other:?}"))),
            ),
            Ended::Cancelled => (Status::Cancelled, None),
            Ended::Failed(error) => (Status::Failed, Some(error)),
        };
        Outcome {
            status,
            result: self.answer.map(|(text, _)| text),
            files: self.files,
            images: self.images,
            error,
        }
    }
}

fn tokens_of(usage: &TokenUsageBreakdown) -> Tokens {
    let count = |value: i64| u64::try_from(value).unwrap_or(0);
    Tokens {
        input: count(usage.input_tokens),
        cached_input: count(usage.cached_input_tokens),
        output: count(usage.output_tokens),
        reasoning_output: count(usage.reasoning_output_tokens),
        total: count(usage.total_tokens),
    }
}

#[cfg(test)]
mod spec_tests {
    //! The child's server, as T4a's launch makes it and this file changes it.

    use std::path::PathBuf;

    use super::*;

    fn request(images: bool) -> Request {
        Request {
            parent: "p1".into(),
            cwd: "/work/hangar".into(),
            task: "Draw the icon.".into(),
            kind: AgentKind::Codex,
            model: Some("gpt-5.5-codex".into()),
            images,
            ..Request::default()
        }
    }

    fn pairs(args: &[String]) -> Vec<String> {
        args.windows(2)
            .filter(|pair| pair[0] == "-c")
            .map(|pair| pair[1].clone())
            .collect()
    }

    #[test]
    fn the_child_server_listens_on_stdio_with_the_launch_settings() {
        let spawn = server_spec(
            &request(false),
            "/work/hangar",
            Some(PermissionMode::AcceptEdits),
            None,
            None,
        )
        .unwrap();
        assert_eq!(spawn.program, "codex");
        assert_eq!(spawn.cwd, "/work/hangar");
        assert_eq!(&spawn.args[..3], ["app-server", "--listen", "stdio://"]);
        assert!(!spawn.args.iter().any(|arg| arg.contains("unix://")));
        let settings = pairs(&spawn.args);
        // The workspace's policy as Codex's own settings (agent/codex.rs).
        assert!(settings.contains(&"approval_policy=on-request".to_string()));
        assert!(settings.contains(&"sandbox_mode=workspace-write".to_string()));
        assert!(settings.contains(&"model=gpt-5.5-codex".to_string()));
        assert!(settings
            .iter()
            .any(|setting| setting.starts_with("developer_instructions=")
                && setting.contains("Nobody watches this session")));
    }

    /// D15: off for a child not asked to make images, and said after the launch's own `true`.
    #[test]
    fn image_generation_follows_the_delegation_and_comes_last() {
        for images in [false, true] {
            let spawn = server_spec(&request(images), "/work/hangar", None, None, None).unwrap();
            let settings = pairs(&spawn.args);
            assert_eq!(
                settings.last().unwrap(),
                &format!("features.image_generation={images}")
            );
            assert_eq!(
                spawn.args.last().unwrap(),
                &format!("features.image_generation={images}")
            );
        }
    }

    #[test]
    fn a_policy_that_lets_everything_through_is_codexs_bypass() {
        let spawn = server_spec(
            &request(false),
            "/work/hangar",
            Some(PermissionMode::BypassPermissions),
            None,
            None,
        )
        .unwrap();
        let settings = pairs(&spawn.args);
        assert!(settings.contains(&"approval_policy=never".to_string()));
        assert!(settings.contains(&"sandbox_mode=danger-full-access".to_string()));
    }

    #[test]
    fn no_policy_leaves_the_persons_own() {
        let spawn = server_spec(&request(false), "/work/hangar", None, None, None).unwrap();
        let settings = pairs(&spawn.args);
        assert!(!settings.iter().any(|s| s.starts_with("approval_policy=")));
        assert!(!settings.iter().any(|s| s.starts_with("sandbox_mode=")));
    }

    #[test]
    fn a_child_in_a_worktree_is_told_to_commit_there() {
        let made = worktree::Made {
            repo: "/work/hangar".into(),
            path: "/work/hangar-worktrees/delegate-d-1-abcdef".into(),
            branch: "delegate/d-1-abcdef".into(),
            base: "1a2b3c4".into(),
        };
        let spawn = server_spec(&request(false), &made.path, None, Some(&made), None).unwrap();
        assert_eq!(spawn.cwd, made.path);
        assert!(pairs(&spawn.args)
            .iter()
            .any(|setting| setting.contains("commit your work there")));
    }

    #[test]
    fn the_settings_command_is_the_servers() {
        let request = Request {
            command: Some("  /opt/codex/bin/codex  ".into()),
            ..request(false)
        };
        let spawn = server_spec(&request, "/work/hangar", None, None, None).unwrap();
        assert_eq!(spawn.program, "/opt/codex/bin/codex");
    }

    /// A shadow home's `CODEX_HOME` (accounts.rs), as a pane on that account gets it.
    #[test]
    fn the_account_is_the_childs_codex_home() {
        let home = PathBuf::from("/home/ada/.hangar/accounts/codex/team");
        let env = crate::accounts::env_for(Some(AgentKind::Codex.id()), Some(&home));
        assert!(
            env.iter()
                .any(|(name, value)| *name == "CODEX_HOME"
                    && value.as_deref() == Some(home.as_path()))
        );
        let spawn = Spawn {
            program: "codex".into(),
            args: vec!["app-server".into(), "--listen".into(), "stdio://".into()],
            cwd: "/work/hangar".into(),
            env,
        };
        let command = command(spawn);
        let set: Vec<_> = command.as_std().get_envs().collect();
        assert!(set
            .iter()
            .any(|(name, value)| name.to_str() == Some("CODEX_HOME")
                && value.map(PathBuf::from).as_deref() == Some(home.as_path())));
        // Never the identity of a pane Hangar was started from.
        assert!(set
            .iter()
            .any(|(name, value)| name.to_str() == Some("HANGAR_PANE_ID") && value.is_none()));
        let args: Vec<_> = command.as_std().get_args().collect();
        assert_eq!(args, ["app-server", "--listen", "stdio://"]);
    }
}
