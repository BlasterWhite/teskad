//! A Claude Code child (docs/native/agents.md §4.3, §5.2 D14): a `claude -p` of its own that
//! speaks stream-json on stdio — the task goes in as its one user message, its messages come out
//! one per line — with Hangar as the host its permission prompts go to, answered by the person
//! only.
//!
//! - **Its run** is a pane's launch (agent/claude.rs, through `launch::build`): the settings'
//!   command, the model, the workspace's permission policy, the account's `CLAUDE_CONFIG_DIR`
//!   (accounts.rs). Added to it is what makes it headless: `-p`, stream-json both ways
//!   (`--verbose` too, without which Claude Code refuses stream-json output), its permission
//!   prompts sent to Hangar over stdio (`--permission-prompt-tool stdio`, as the Agent SDK does
//!   for its `canUseTool`), and the role at the end of its system prompt
//!   (`--append-system-prompt`), there being no mod to put it there.
//! - **Its work**: `system/init` says its session and model; its `assistant` messages carry its
//!   texts, its calls and what each API call spent; a file tool's call (`Edit`, `Write`…) whose
//!   `tool_result` is not an error changed that file; `result` ends it, its `result` the final
//!   message and its `modelUsage` what the whole run spent. The run is then over: its input
//!   closes, and a process still there after a moment is stopped — background work it left
//!   behind goes with it, as a Codex child's goes with its server.
//! - **What it asks** (a `control_request` `can_use_tool`) is held for the person as a pane's
//!   asks are, under the delegation's id, and answered with the person's choice in the mod's
//!   terms (teskad-bridge, `decisionOf`). Nothing here answers for the person: an ask is answered
//!   when they answer it, withdrawn when Claude Code withdraws it (`control_cancel_request`).
//!   Any other request is refused, Hangar having declared no hooks and no SDK servers.
//! - **Cancelled**, its turn is interrupted (a `control_request` `interrupt`), its input closed,
//!   and a process still there after a moment asked to stop (SIGTERM, on which Claude Code ends
//!   its commands' processes), then killed.
//!
//! The messages and their fields are those of Claude Code's stream-json docs (code.claude.com:
//! `headless`; the Agent SDK's TypeScript reference — `SDKSystemMessage`, `SDKAssistantMessage`,
//! `SDKUserMessage`, `SDKResultMessage`, `PermissionResult`, `SDKControlInterruptResponse`), and
//! the envelope of the control messages, which the docs name without spelling, the Agent SDK's
//! own client (claude-agent-sdk-python, `_internal/query.py`: `control_request {request_id,
//! request}`, `control_response {response: {subtype, request_id, response | error}}`,
//! `control_cancel_request {request_id}`; `types.py`: a `canUseTool` is
//! `--permission-prompt-tool stdio`). The flags were checked against Claude Code 2.1.328
//! (`claude --help`, and a run with no prompt); no session was recorded (§7, the person's
//! decision).

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::{worktree, Live, Outcome, Request, Status, Tokens};
use crate::agent::codex_attach::{absolute, stopped};
use crate::agent::launch::{self, LaunchRequest};
use crate::agent::{AgentKind, PermissionMode};
use crate::bridge::{self, AskPoll};
use crate::host::Host;

/// The child's role, at the end of its system prompt: what it is for, and that nobody watches it.
const ROLE: &str = "You are working on a sub-task another agent handed over to you through \
Hangar. Nobody watches this session while it runs: do the task on your own, then end with your \
final answer, which is all the other agent reads of your work. Say in it what you did and what \
you changed. What you need the person to allow, ask for as you would anyway: Hangar shows it to \
them.";
/// Added to [`ROLE`] for a child in a worktree of its own.
const WORKTREE_ROLE: &str = "You work in a git worktree of your own, on its branch, made from \
the other agent's last commit: commit your work there before you answer.";
/// What makes the launch headless: one run (`-p`), stream-json in and out (`--verbose`: Claude
/// Code refuses stream-json output without it), its permission prompts to the host on stdio.
const HEADLESS: [&str; 8] = [
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--permission-prompt-tool",
    "stdio",
];
const ROLE_FLAG: &str = "--append-system-prompt";
/// How long a cancelled run's turn is given to end once interrupted.
const INTERRUPT_WAIT: Duration = Duration::from_secs(5);
/// How long a run whose input closed is given to go by itself, then (on Unix) to go once asked
/// to stop: together with [`INTERRUPT_WAIT`], within the wait of `cancel`.
const EXIT_GRACE: Duration = Duration::from_secs(3);
#[cfg(unix)]
const TERM_GRACE: Duration = Duration::from_secs(3);
/// How long the end of the run's output waits for the end of its stderr.
const STDERR_SETTLE: Duration = Duration::from_millis(500);
/// How many of its last stderr lines are kept for a message, and how long each.
const STDERR_LINES: usize = 20;
const STDERR_LINE_CHARS: usize = 2_000;
/// Past this, a text in a call's input is cut in what is reported: the mod's limits
/// (teskad-bridge, `INPUT_TEXT`, `PLAN_TEXT`). The answer goes back with the input whole.
const INPUT_TEXT: usize = 4_096;
const PLAN_TEXT: usize = 65_536;
/// A no without words, as the mod words it.
const DECLINED: &str = "The person declined this from Hangar.";
/// The tools whose call, once it succeeded, changed a file, and the input that names the file.
const FILE_TOOLS: [(&str, &str); 4] = [
    ("Edit", "file_path"),
    ("MultiEdit", "file_path"),
    ("Write", "file_path"),
    ("NotebookEdit", "notebook_path"),
];
/// The tool whose answer rides in its input (`answers`), and the one whose plan is read whole.
const QUESTION_TOOL: &str = "AskUserQuestion";
const PLAN_TOOL: &str = "ExitPlanMode";
/// The id of Hangar's one request of a run: the interruption of a cancelled one.
const INTERRUPT_ID: &str = "hangar-interrupt";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The run a Claude Code child is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Spawn {
    pub program: String,
    /// Its arguments, each exactly as Claude Code receives it: no shell reads them.
    pub args: Vec<String>,
    pub cwd: String,
    /// The launch's variables (its MCP file's, the account's): set, or taken out.
    pub env: Vec<(String, Option<OsString>)>,
}

/// The run for `request`, working in `cwd` under the stepped-down `mode`: a pane's launch of
/// Claude Code, made headless.
pub(super) fn spawn_spec(
    request: &Request,
    cwd: &str,
    mode: Option<PermissionMode>,
    made: Option<&worktree::Made>,
) -> Result<Spawn, String> {
    let account =
        crate::accounts::for_launch(Some(AgentKind::Claude.id()), request.account.as_deref());
    let account_dir = account.and_then(|account| account.dir);
    let mut spawn = run_spec(request, cwd, mode, made, account_dir)?;
    // The settings' command as it is; the bare program, where its installer put it when PATH
    // does not say (`install::agent`): `claude.exe` or `claude.cmd` on Windows.
    if request
        .command
        .as_deref()
        .is_none_or(|command| command.trim().is_empty())
    {
        if let Some(found) = crate::install::agent::find(AgentKind::Claude) {
            spawn.program = found.to_string_lossy().into_owned();
        }
    }
    Ok(spawn)
}

/// [`spawn_spec`] without what it reads of the machine (the account's folder, where Claude Code
/// is).
fn run_spec(
    request: &Request,
    cwd: &str,
    mode: Option<PermissionMode>,
    made: Option<&worktree::Made>,
    account_dir: Option<PathBuf>,
) -> Result<Spawn, String> {
    if request.kind != AgentKind::Claude {
        return Err(format!(
            "Hangar runs one-shot children of Claude Code only, not of {}",
            request.kind.info().label
        ));
    }
    let launch = LaunchRequest {
        kind: AgentKind::Claude,
        command: request.command.clone(),
        cwd: cwd.to_string(),
        model: request.model.clone(),
        permission_mode: mode,
        account_dir,
        ..LaunchRequest::default()
    };
    let spec = launch::build(&launch).map_err(|error| error.to_string())?;
    // One line: a newline is no argument for a Windows command shim (`claude.cmd`).
    let role = match made {
        Some(_) => format!("{ROLE} {WORKTREE_ROLE}"),
        None => ROLE.to_string(),
    };
    let mut args: Vec<String> = HEADLESS.iter().map(|arg| arg.to_string()).collect();
    args.push(ROLE_FLAG.into());
    args.push(role);
    args.extend(spec.args.iter().map(|word| unquoted(word).to_string()));
    Ok(Spawn {
        program: spec.program,
        args,
        cwd: spec.cwd,
        env: spec.env,
    })
}

/// A word of the launch's line as Claude Code receives it. The builder words it for a shell: a
/// value between double quotes when a shell would read it otherwise, and only a value holding
/// nothing a shell reads between them (agent/claude.rs, `word`; `launch::is_quotable`) — so the
/// value is what they hold.
fn unquoted(word: &str) -> &str {
    word.strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(word)
}

type Reader = Box<dyn AsyncRead + Send + Unpin>;
type Writer = Box<dyn AsyncWrite + Send + Unpin>;
type Output = Lines<BufReader<Reader>>;

/// A child's stdio, as the runner speaks to it.
pub(super) struct Io {
    /// What it says: its stdout, one JSON message per line.
    pub output: Reader,
    /// What it is told: its stdin.
    pub input: Writer,
    /// The process behind them; none for a test's.
    pub process: Option<Process>,
}

/// Starts a child: the run of a [`Spawn`], or a test's.
pub(super) type Open = Box<dyn FnOnce() -> Result<Io, String> + Send>;

/// The child spawned from `spawn`, once the runner starts it.
pub(super) fn stdio(spawn: Spawn) -> Open {
    Box::new(move || start(&spawn))
}

/// The process of `spawn`, its stdio piped.
fn command(spawn: &Spawn) -> Command {
    crate::path_env::ensure();
    let mut command = std::process::Command::new(&spawn.program);
    command.args(&spawn.args).current_dir(&spawn.cwd);
    // Not the identity of the pane Hangar may have been started from: the mod stays quiet.
    crate::delivery::scrub(&mut command);
    for (name, value) in &spawn.env {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut command = Command::from(command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

fn start(spawn: &Spawn) -> Result<Io, String> {
    let mut child = command(spawn)
        .spawn()
        .map_err(|error| format!("{}: {error}", spawn.program))?;
    let (Some(input), Some(output), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err("Claude Code's run has no stdio to speak on".into());
    };
    // Read all along, a full pipe would stop it; its last lines are kept.
    let said = Arc::new(Mutex::new(VecDeque::new()));
    let kept = Arc::clone(&said);
    let reader = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line: String = line.trim_end().chars().take(STDERR_LINE_CHARS).collect();
            if line.is_empty() {
                continue;
            }
            let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
            if kept.len() == STDERR_LINES {
                kept.pop_front();
            }
            kept.push_back(line);
        }
    });
    Ok(Io {
        output: Box::new(output),
        input: Box::new(input),
        process: Some(Process {
            child,
            said,
            reader,
        }),
    })
}

/// A spawned run.
pub(super) struct Process {
    child: Child,
    /// Its last lines on stderr.
    said: Arc<Mutex<VecDeque<String>>>,
    reader: JoinHandle<()>,
}

impl Process {
    /// Its input closed: it is let go by itself, then asked to stop, then killed. How it exited,
    /// when that is known, and its last lines on stderr.
    async fn end(mut self) -> (Option<ExitStatus>, Vec<String>) {
        let status = self.stop().await;
        let _ = tokio::time::timeout(STDERR_SETTLE, &mut self.reader).await;
        self.reader.abort();
        let said = self
            .said
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect();
        (status, said)
    }

    async fn stop(&mut self) -> Option<ExitStatus> {
        if let Ok(Ok(status)) = tokio::time::timeout(EXIT_GRACE, self.child.wait()).await {
            return Some(status);
        }
        // SIGTERM: Claude Code ends the process trees of the commands it runs, then itself.
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            // SAFETY: a signal to the child's own pid, which is not reaped while `child` holds
            // it.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            if let Ok(Ok(status)) = tokio::time::timeout(TERM_GRACE, self.child.wait()).await {
                return Some(status);
            }
        }
        let _ = self.child.start_kill();
        self.child.wait().await.ok()
    }
}

fn outcome(status: Status, error: Option<String>) -> Outcome {
    Outcome {
        status,
        result: None,
        files: Vec::new(),
        images: Vec::new(),
        error,
    }
}

/// Runs the child to its end: starts it, tells it the task, follows its messages, holds what it
/// asks for the person, and stops it when `stop` says so.
pub(super) async fn run(
    live: Live,
    open: Open,
    task: String,
    cwd: String,
    mut stop: watch::Receiver<bool>,
) -> Outcome {
    if *stop.borrow_and_update() {
        return outcome(Status::Cancelled, None);
    }
    let Io {
        output,
        input,
        process,
    } = match open() {
        Ok(io) => io,
        Err(error) => return outcome(Status::Failed, Some(error)),
    };
    let mut child = Run::new(live, cwd, input);
    let ended = child
        .work(task, BufReader::new(output).lines(), &mut stop)
        .await;
    child.finish();
    child.close_input().await;
    let (exit, said) = match process {
        Some(process) => process.end().await,
        None => (None, Vec::new()),
    };
    child.outcome(ended, exit, &said)
}

/// How the run ended, as [`Run::work`] saw it.
enum Ended {
    /// Its `result` message.
    Result(Value),
    /// Cancelled from Hangar.
    Cancelled,
    /// Its output ended before a result.
    Closed,
}

/// An ask held for the person.
struct Held {
    ask: String,
    waiter: JoinHandle<()>,
}

/// The person's answer to Claude Code's request `request`, as Claude Code takes it; none when
/// its ask ended without one.
struct Answered {
    request: String,
    response: Option<Value>,
}

/// One child, over its stdio.
struct Run {
    live: Live,
    cwd: String,
    /// Its stdin, until it is closed.
    input: Option<Writer>,
    /// Its last text in the main conversation: the answer, unless its result says one.
    last_text: Option<String>,
    files: Vec<String>,
    /// The calls of a file tool waiting for their result: the file each changes.
    editing: HashMap<String, String>,
    /// What each of its API calls spent, by message id: a message's blocks come one per line,
    /// each with the message's usage.
    spent: HashMap<String, Tokens>,
    /// Asks held for the person, by Claude Code's request id.
    held: HashMap<String, Held>,
    /// Asks held so far, for their ids.
    asked: u64,
    answers_tx: mpsc::UnboundedSender<Answered>,
    answers_rx: mpsc::UnboundedReceiver<Answered>,
    /// The person's no said to stop: the turn ends there.
    interrupted: bool,
}

impl Run {
    fn new(live: Live, cwd: String, input: Writer) -> Run {
        let (answers_tx, answers_rx) = mpsc::unbounded_channel();
        Run {
            live,
            cwd,
            input: Some(input),
            last_text: None,
            files: Vec::new(),
            editing: HashMap::new(),
            spent: HashMap::new(),
            held: HashMap::new(),
            asked: 0,
            answers_tx,
            answers_rx,
            interrupted: false,
        }
    }

    /// Tells the task, then follows the run to its result.
    async fn work(
        &mut self,
        task: String,
        mut output: Output,
        stop: &mut watch::Receiver<bool>,
    ) -> Ended {
        // An `SDKUserMessage`: the task as the user's prompt.
        self.send(&json!({
            "type": "user",
            "message": { "role": "user", "content": task },
            "parent_tool_use_id": null,
        }))
        .await;
        loop {
            tokio::select! {
                line = output.next_line() => match line {
                    Ok(Some(line)) => {
                        if let Some(message) = parse(&line) {
                            if let Some(ended) = self.on_message(message).await {
                                return ended;
                            }
                        }
                    }
                    Ok(None) | Err(_) => return Ended::Closed,
                },
                Some(answered) = self.answers_rx.recv() => self.on_answer(answered).await,
                () = stopped(stop) => return self.interrupt(&mut output).await,
            }
        }
    }

    /// What a message tells of the run; its end, when it ends it.
    async fn on_message(&mut self, message: Value) -> Option<Ended> {
        let kind = message["type"].as_str()?.to_owned();
        match kind.as_str() {
            "system" => self.on_system(&message),
            "assistant" => self.on_assistant(&message),
            "user" => self.on_user(&message),
            "result" => {
                self.on_result(&message);
                return Some(Ended::Result(message));
            }
            "control_request" => self.on_request(&message).await,
            "control_cancel_request" => {
                if let Some(request) = message["request_id"].as_str() {
                    self.settle(request);
                }
            }
            _ => {}
        }
        None
    }

    /// `system/init`: its session and the model it runs on.
    fn on_system(&self, message: &Value) {
        if message["subtype"] != "init" {
            return;
        }
        let session = text_of(&message["session_id"]);
        let model = text_of(&message["model"]);
        self.live.update(|delegation| {
            if session.is_some() {
                delegation.session = session;
            }
            if model.is_some() {
                delegation.model = model;
            }
        });
    }

    /// An `assistant` message: its text in the main conversation, its calls of a file tool, what
    /// its API call spent. A subagent's (`parent_tool_use_id` set) are the child's work too.
    fn on_assistant(&mut self, message: &Value) {
        let main = message["parent_tool_use_id"].is_null();
        let body = &message["message"];
        if let (Some(id), Some(tokens)) = (body["id"].as_str(), usage_tokens(&body["usage"])) {
            self.spent.insert(id.to_string(), tokens);
            let spent = sum(self.spent.values());
            self.live
                .update(|delegation| delegation.tokens = Some(spent));
        }
        for block in body["content"].as_array().into_iter().flatten() {
            match block["type"].as_str() {
                Some("text") if main => {
                    if let Some(text) = block["text"].as_str().filter(|t| !t.trim().is_empty()) {
                        self.last_text = Some(text.to_string());
                    }
                }
                Some("tool_use") => {
                    if let (Some(id), Some(file)) = (block["id"].as_str(), edited_file(block)) {
                        self.editing
                            .insert(id.to_string(), absolute(&self.cwd, file));
                    }
                }
                _ => {}
            }
        }
    }

    /// A `user` message: the results of its calls. A file tool's that is not an error changed
    /// its file.
    fn on_user(&mut self, message: &Value) {
        for block in message["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if block["type"] != "tool_result" {
                continue;
            }
            let Some(file) = block["tool_use_id"]
                .as_str()
                .and_then(|id| self.editing.remove(id))
            else {
                continue;
            };
            if block["is_error"] != true && !self.files.contains(&file) {
                self.files.push(file);
            }
        }
    }

    /// Its `result`: what the whole run spent, and its session should `init` have said none.
    fn on_result(&self, result: &Value) {
        let tokens = result_tokens(result);
        let session = text_of(&result["session_id"]);
        self.live.update(|delegation| {
            if tokens.is_some() {
                delegation.tokens = tokens;
            }
            if delegation.session.is_none() {
                delegation.session = session;
            }
        });
    }

    /// A request of Claude Code's: a permission prompt is held for the person; anything else is
    /// refused.
    async fn on_request(&mut self, message: &Value) {
        let Some(request) = message["request_id"].as_str() else {
            return;
        };
        let body = &message["request"];
        match body["subtype"].as_str() {
            Some("can_use_tool") => self.hold(request, body),
            other => {
                let why = format!(
                    "Hangar does not answer {} for a delegated run",
                    other.unwrap_or("this request")
                );
                self.send(&refusal(request, &why)).await;
            }
        }
    }

    /// A permission prompt (`can_use_tool`), held as an ask of the delegation's — reported as the
    /// mod reports a pane's (`permission`, `waiting`) — and answered with the person's choice
    /// once given.
    fn hold(&mut self, request: &str, body: &Value) {
        let tool = body["tool_name"]
            .as_str()
            .filter(|tool| !tool.is_empty())
            .unwrap_or("unknown")
            .to_string();
        let input = match &body["input"] {
            Value::Object(input) => Value::Object(input.clone()),
            _ => Value::Object(Map::new()),
        };
        self.asked += 1;
        let ask = format!("claude-{}-{}", self.live.id, self.asked);
        let limit = if tool == PLAN_TOOL {
            PLAN_TEXT
        } else {
            INPUT_TEXT
        };
        let mut event = Map::new();
        event.insert("kind".into(), json!("permission"));
        event.insert("waiting".into(), json!(true));
        event.insert("tool".into(), json!(tool));
        event.insert("input".into(), clipped(&input, limit, 0));
        event.insert("askId".into(), json!(ask));
        if let Some(call) = body["tool_use_id"].as_str().filter(|id| !id.is_empty()) {
            event.insert("toolUseId".into(), json!(call));
        }
        let event = Value::Object(event);
        // Held by the bridge as it is reported (`note_report`), before its wait starts below.
        bridge::report(&self.live.host, &self.live.id, event.clone());
        self.live.needs_you(true);
        let waiter = tokio::spawn(wait_for_answer(
            self.live.host.clone(),
            self.live.id.clone(),
            Asked {
                request: request.to_string(),
                ask: ask.clone(),
                tool,
                input,
                event,
            },
            self.answers_tx.clone(),
        ));
        if let Some(earlier) = self.held.insert(request.to_string(), Held { ask, waiter }) {
            earlier.waiter.abort();
            self.end_ask(&earlier.ask);
        }
    }

    /// The person answered an ask, or it ended without them: their answer goes to Claude Code.
    async fn on_answer(&mut self, answered: Answered) {
        if !self.held.contains_key(&answered.request) {
            return;
        }
        if let Some(response) = answered.response {
            if response["interrupt"] == true {
                self.interrupted = true;
            }
            self.send(&json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": answered.request,
                    "response": response,
                },
            }))
            .await;
        }
        self.settle(&answered.request);
    }

    /// The ask on request `request` is over: answered, or withdrawn by Claude Code.
    fn settle(&mut self, request: &str) {
        let Some(held) = self.held.remove(request) else {
            return;
        };
        held.waiter.abort();
        self.end_ask(&held.ask);
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

    /// Cancelled: the turn is interrupted, and its end waited for a moment, what it does
    /// meanwhile still read. Nothing more is asked of the person.
    async fn interrupt(&mut self, output: &mut Output) -> Ended {
        self.send(&json!({
            "type": "control_request",
            "request_id": INTERRUPT_ID,
            "request": { "subtype": "interrupt" },
        }))
        .await;
        let deadline = tokio::time::Instant::now() + INTERRUPT_WAIT;
        while let Ok(Ok(Some(line))) = tokio::time::timeout_at(deadline, output.next_line()).await {
            let Some(message) = parse(&line) else {
                continue;
            };
            let kind = message["type"].as_str().unwrap_or_default().to_owned();
            match kind.as_str() {
                "result" => {
                    self.on_result(&message);
                    break;
                }
                "control_request" | "control_response" => {}
                _ => {
                    self.on_message(message).await;
                }
            }
        }
        Ended::Cancelled
    }

    /// Writes one message to its input. One it cannot take any more ended: its output says so.
    async fn send(&mut self, message: &Value) {
        let Some(input) = self.input.as_mut() else {
            return;
        };
        let mut line = message.to_string();
        line.push('\n');
        let written = async {
            input.write_all(line.as_bytes()).await?;
            input.flush().await
        };
        if written.await.is_err() {
            self.input = None;
        }
    }

    /// Its input ends: Claude Code ends with it.
    async fn close_input(&mut self) {
        if let Some(mut input) = self.input.take() {
            let _ = tokio::time::timeout(EXIT_GRACE, input.shutdown()).await;
        }
    }

    /// The run is over: its asks are withdrawn, their waits with them.
    fn finish(&mut self) {
        for (_, held) in std::mem::take(&mut self.held) {
            held.waiter.abort();
            self.end_ask(&held.ask);
        }
    }

    fn outcome(self, ended: Ended, exit: Option<ExitStatus>, said: &[String]) -> Outcome {
        let last = self.last_text;
        let (status, result, error) = match ended {
            Ended::Cancelled => (Status::Cancelled, last, None),
            // Stopped by the person's answer to one of its asks ("no, and stop").
            Ended::Result(_) if self.interrupted => (
                Status::Cancelled,
                last,
                Some("its turn was stopped before its end".to_string()),
            ),
            Ended::Result(result) => ended_as(&result, last),
            Ended::Closed => (Status::Failed, last, Some(closed(exit, said))),
        };
        Outcome {
            status,
            result,
            files: self.files,
            images: Vec::new(),
            error,
        }
    }
}

/// A permission prompt, as its wait for the person holds it.
struct Asked {
    request: String,
    ask: String,
    tool: String,
    /// The call's input, whole: what an allow gives back.
    input: Value,
    /// Its report, for when it is asked again.
    event: Value,
}

/// Waits for the person's answer to `asked`, held under `holder`, and hands it over. An answer
/// that does not fit the question (a question allowed with no answers) asks it again; an ask that
/// ended without an answer ends the wait.
async fn wait_for_answer(
    host: Host,
    holder: String,
    asked: Asked,
    answered: mpsc::UnboundedSender<Answered>,
) {
    let wait = Duration::from_millis(bridge::ASK_WAIT_MAX_MS);
    let response = loop {
        match bridge::await_decision(&holder, &asked.ask, wait).await {
            AskPoll::Pending => {}
            AskPoll::Gone => break None,
            AskPoll::Decision(decision) => {
                match permission_result(&asked.tool, &asked.input, &decision) {
                    Some(response) => break Some(response),
                    None => bridge::report(&host, &holder, asked.event.clone()),
                }
            }
        }
    };
    let _ = answered.send(Answered {
        request: asked.request,
        response,
    });
}

/// The person's choice (bridge.rs, `answer_ask`) as Claude Code takes it (`PermissionResult`),
/// as the mod gives it (`decisionOf`): an allow gives the call's input back, a question's answers
/// in it; a no says why, and stops the turn when the person said so. An allow "for the session"
/// is a plain allow, as the mod's is. `None` for a choice that does not fit.
fn permission_result(tool: &str, input: &Value, decision: &Value) -> Option<Value> {
    match decision.get("behavior")?.as_str()? {
        "allow" => {
            let mut updated = input.as_object().cloned().unwrap_or_default();
            if tool == QUESTION_TOOL {
                let answers = decision
                    .get("answers")
                    .filter(|answers| answers.as_object().is_some_and(|map| !map.is_empty()))?;
                updated.insert("answers".into(), answers.clone());
            }
            Some(json!({ "behavior": "allow", "updatedInput": updated }))
        }
        "deny" => {
            let message = decision
                .get("message")
                .and_then(Value::as_str)
                .filter(|message| !message.trim().is_empty())
                .unwrap_or(DECLINED);
            let mut denied = json!({ "behavior": "deny", "message": message });
            if decision.get("interrupt") == Some(&Value::Bool(true)) {
                denied["interrupt"] = json!(true);
            }
            Some(denied)
        }
        _ => None,
    }
}

/// A request Hangar does not answer, refused rather than left waiting.
fn refusal(request: &str, why: &str) -> Value {
    json!({
        "type": "control_response",
        "response": { "subtype": "error", "request_id": request, "error": why },
    })
}

/// How a `result` ends the delegation: done with its answer, or failed with its reason (and its
/// last text, unless that is the reason again).
fn ended_as(result: &Value, last: Option<String>) -> (Status, Option<String>, Option<String>) {
    let said = text_of(&result["result"]);
    let subtype = result["subtype"].as_str().unwrap_or("");
    let why = match subtype {
        "success" if result["is_error"] != true => return (Status::Done, said.or(last), None),
        // An error said as its result: the API's, the account's limit.
        "success" => said.unwrap_or_else(|| "Claude Code's run failed".into()),
        _ => {
            let errors: Vec<&str> = result["errors"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|error| !error.is_empty())
                .collect();
            if !errors.is_empty() {
                errors.join("; ")
            } else {
                match subtype {
                    "error_max_turns" => "it reached its limit of turns".into(),
                    "error_max_budget_usd" => "it reached its budget".into(),
                    "" => "Claude Code's run failed".into(),
                    other => format!("Claude Code's run ended as {other}"),
                }
            }
        }
    };
    let last = last.filter(|text| text.trim() != why.trim());
    (Status::Failed, last, Some(why))
}

/// Why a run whose output ended without a result failed.
fn closed(exit: Option<ExitStatus>, said: &[String]) -> String {
    let mut why = match exit {
        Some(status) => format!("Claude Code ended before its answer ({status})"),
        None => "Claude Code ended before its answer".to_string(),
    };
    if !said.is_empty() {
        why.push_str(&format!(" (it said: {})", said.join(" / ")));
    }
    why
}

fn parse(line: &str) -> Option<Value> {
    serde_json::from_str::<Value>(line.trim())
        .ok()
        .filter(Value::is_object)
}

fn text_of(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .map(str::to_string)
}

/// The file a call of a file tool changes, as its input names it.
fn edited_file(call: &Value) -> Option<&str> {
    let tool = call["name"].as_str()?;
    let (_, key) = FILE_TOOLS.iter().find(|(name, _)| *name == tool)?;
    call["input"][*key].as_str().filter(|path| !path.is_empty())
}

/// What one API call spent, from the Messages API's `usage`: the input read from the cache and
/// written to it are counted apart from `input_tokens` there, and are input here.
fn usage_tokens(usage: &Value) -> Option<Tokens> {
    usage.as_object()?;
    let count = |key: &str| usage[key].as_u64().unwrap_or(0);
    let cached = count("cache_read_input_tokens");
    let input = count("input_tokens") + count("cache_creation_input_tokens") + cached;
    let output = count("output_tokens");
    Some(Tokens {
        input,
        cached_input: cached,
        output,
        reasoning_output: 0,
        total: input + output,
    })
}

/// What the whole run spent, from its `result`: each model's (`modelUsage`, a subagent's on
/// another model included), else its `usage`.
fn result_tokens(result: &Value) -> Option<Tokens> {
    let Some(models) = result["modelUsage"]
        .as_object()
        .filter(|models| !models.is_empty())
    else {
        return usage_tokens(&result["usage"]);
    };
    let mut tokens = Tokens::default();
    for usage in models.values() {
        let count = |key: &str| usage[key].as_u64().unwrap_or(0);
        let cached = count("cacheReadInputTokens");
        tokens.input += count("inputTokens") + count("cacheCreationInputTokens") + cached;
        tokens.cached_input += cached;
        tokens.output += count("outputTokens");
    }
    tokens.total = tokens.input + tokens.output;
    Some(tokens)
}

fn sum<'a>(spent: impl Iterator<Item = &'a Tokens>) -> Tokens {
    spent.fold(Tokens::default(), |total, tokens| Tokens {
        input: total.input + tokens.input,
        cached_input: total.cached_input + tokens.cached_input,
        output: total.output + tokens.output,
        reasoning_output: total.reasoning_output + tokens.reasoning_output,
        total: total.total + tokens.total,
    })
}

/// A call's input as reported: long texts cut past `max`, long lists shortened (the mod's
/// `clipped`).
fn clipped(value: &Value, max: usize, depth: usize) -> Value {
    match value {
        Value::String(text) if text.chars().count() > max => {
            let mut cut: String = text.chars().take(max).collect();
            cut.push('…');
            Value::String(cut)
        }
        Value::Array(list) if depth <= 4 => Value::Array(
            list.iter()
                .take(50)
                .map(|entry| clipped(entry, max, depth + 1))
                .collect(),
        ),
        Value::Object(map) if depth <= 4 => Value::Object(
            map.iter()
                .map(|(key, entry)| (key.clone(), clipped(entry, max, depth + 1)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod spec_tests {
    //! The child's run, as the launch builder makes it and this file makes it headless; how its
    //! messages and the person's answers read.

    use std::path::PathBuf;

    use super::*;

    fn request() -> Request {
        Request {
            parent: "p1".into(),
            cwd: "/work/hangar".into(),
            task: "Review the parser.".into(),
            kind: AgentKind::Claude,
            model: Some("sonnet".into()),
            ..Request::default()
        }
    }

    fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        let at = args.iter().position(|arg| arg == flag)?;
        args.get(at + 1).map(String::as_str)
    }

    #[test]
    fn the_child_runs_headless_with_the_launch_settings() {
        let spawn = run_spec(
            &request(),
            "/work/hangar",
            Some(PermissionMode::AcceptEdits),
            None,
            None,
        )
        .unwrap();
        assert_eq!(spawn.program, "claude");
        assert_eq!(spawn.cwd, "/work/hangar");
        assert_eq!(
            &spawn.args[..8],
            [
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-prompt-tool",
                "stdio"
            ]
        );
        assert!(after(&spawn.args, "--append-system-prompt")
            .is_some_and(|role| role.contains("Nobody watches this session")));
        assert_eq!(after(&spawn.args, "--model"), Some("sonnet"));
        // The workspace's policy, as Claude Code enforces it itself.
        assert_eq!(after(&spawn.args, "--permission-mode"), Some("acceptEdits"));
        // No MCP file of a pane's: its variable is taken out, never handed down.
        assert!(spawn
            .env
            .iter()
            .any(|(name, value)| name == "TESKAD_MCP_CONFIG" && value.is_none()));
    }

    #[test]
    fn no_policy_leaves_the_persons_own() {
        let spawn = run_spec(&request(), "/work/hangar", None, None, None).unwrap();
        assert!(!spawn.args.iter().any(|arg| arg == "--permission-mode"));
    }

    /// The builder quotes a model a shell would read as a glob; no shell reads this run's
    /// arguments, so Claude Code gets the model as it is.
    #[test]
    fn a_model_the_builder_quotes_for_a_shell_reaches_claude_code_as_it_is() {
        let spawn = run_spec(
            &Request {
                model: Some("opus[1m]".into()),
                ..request()
            },
            "/work/hangar",
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(after(&spawn.args, "--model"), Some("opus[1m]"));
        assert_eq!(unquoted("\"/x y\""), "/x y");
        assert_eq!(unquoted("\""), "\"");
        assert_eq!(unquoted("sonnet"), "sonnet");
    }

    #[test]
    fn a_child_in_a_worktree_is_told_to_commit_there() {
        let made = worktree::Made {
            repo: "/work/hangar".into(),
            path: "/work/hangar-worktrees/delegate-d-1-abcdef".into(),
            branch: "delegate/d-1-abcdef".into(),
            base: "1a2b3c4".into(),
        };
        let spawn = run_spec(&request(), &made.path, None, Some(&made), None).unwrap();
        assert_eq!(spawn.cwd, made.path);
        let role = after(&spawn.args, "--append-system-prompt").unwrap();
        assert!(role.contains("commit your work there"), "{role}");
        assert!(!role.contains('\n'), "one line: {role}");
    }

    #[test]
    fn the_settings_command_is_the_runs() {
        let spawn = run_spec(
            &Request {
                command: Some("  /opt/claude/bin/claude  ".into()),
                ..request()
            },
            "/work/hangar",
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(spawn.program, "/opt/claude/bin/claude");
    }

    #[test]
    fn only_claude_code_runs_as_this_child() {
        let refused = run_spec(
            &Request {
                kind: AgentKind::Codex,
                ..request()
            },
            "/work/hangar",
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(refused.contains("Codex"), "{refused}");
    }

    /// A profile's `CLAUDE_CONFIG_DIR` (accounts.rs), as a pane on that account gets it, and never
    /// the identity of a pane Hangar was started from.
    #[test]
    fn the_account_is_the_childs_config_dir() {
        let home = PathBuf::from("/home/ada/.teskad/accounts/claude/team");
        let spawn = run_spec(&request(), "/work/hangar", None, None, Some(home.clone())).unwrap();
        let command = command(&spawn);
        let set: Vec<_> = command.as_std().get_envs().collect();
        assert!(set
            .iter()
            .any(|(name, value)| name.to_str() == Some("CLAUDE_CONFIG_DIR")
                && value.map(PathBuf::from).as_deref() == Some(home.as_path())));
        assert!(set
            .iter()
            .any(|(name, value)| name.to_str() == Some("TESKAD_PANE_ID") && value.is_none()));
        let args: Vec<_> = command.as_std().get_args().collect();
        assert_eq!(args[0], "-p");
    }

    // From Claude Code's stream-json docs (the Agent SDK's `PermissionResult`) and the mod's
    // `decisionOf`.
    #[test]
    fn the_persons_choice_reads_as_claude_code_takes_it() {
        let input = json!({ "command": "cargo test", "description": "Run the tests" });
        assert_eq!(
            permission_result("Bash", &input, &json!({ "behavior": "allow" })),
            Some(json!({ "behavior": "allow", "updatedInput": input }))
        );
        // For the session: a plain allow, as the mod's.
        assert_eq!(
            permission_result(
                "Bash",
                &input,
                &json!({ "behavior": "allow", "forSession": true })
            ),
            Some(json!({ "behavior": "allow", "updatedInput": input }))
        );
        assert_eq!(
            permission_result("Bash", &input, &json!({ "behavior": "deny" })),
            Some(json!({ "behavior": "deny", "message": DECLINED }))
        );
        assert_eq!(
            permission_result(
                "Bash",
                &input,
                &json!({ "behavior": "deny", "message": "Not on main.", "interrupt": true })
            ),
            Some(json!({ "behavior": "deny", "message": "Not on main.", "interrupt": true }))
        );
        assert_eq!(
            permission_result("Bash", &input, &json!({ "behavior": "maybe" })),
            None
        );
        let question = json!({ "questions": [{ "question": "Which crate?", "header": "Crate",
            "options": [{ "label": "core", "description": "" }, { "label": "engine", "description": "" }],
            "multiSelect": false }] });
        // A question needs its answers.
        assert_eq!(
            permission_result(QUESTION_TOOL, &question, &json!({ "behavior": "allow" })),
            None
        );
        let mut answered = question.clone();
        answered["answers"] = json!({ "Which crate?": "core" });
        assert_eq!(
            permission_result(
                QUESTION_TOOL,
                &question,
                &json!({ "behavior": "allow", "answers": { "Which crate?": "core" } })
            ),
            Some(json!({ "behavior": "allow", "updatedInput": answered }))
        );
    }

    // From Claude Code's stream-json docs (the Agent SDK's `SDKResultMessage`).
    #[test]
    fn a_result_ends_the_delegation_as_it_says() {
        let done = json!({ "type": "result", "subtype": "success", "is_error": false,
            "result": "Reviewed.", "session_id": "s" });
        assert_eq!(
            ended_as(&done, Some("Looking.".into())),
            (Status::Done, Some("Reviewed.".into()), None)
        );
        let limit = json!({ "type": "result", "subtype": "success", "is_error": true,
            "result": "You've hit your usage limit · resets 4pm", "session_id": "s" });
        assert_eq!(
            ended_as(
                &limit,
                Some("You've hit your usage limit · resets 4pm".into())
            ),
            (
                Status::Failed,
                None,
                Some("You've hit your usage limit · resets 4pm".into())
            )
        );
        let turns = json!({ "type": "result", "subtype": "error_max_turns", "is_error": true,
            "errors": [], "session_id": "s" });
        assert_eq!(
            ended_as(&turns, Some("Half done.".into())),
            (
                Status::Failed,
                Some("Half done.".into()),
                Some("it reached its limit of turns".into())
            )
        );
        let failed = json!({ "type": "result", "subtype": "error_during_execution",
            "is_error": true, "errors": ["Tool permission stream closed"], "session_id": "s" });
        assert_eq!(
            ended_as(&failed, None).2.as_deref(),
            Some("Tool permission stream closed")
        );
    }

    // From Claude Code's stream-json docs: `SDKResultMessage.usage` (the Messages API's) and
    // `modelUsage` (`ModelUsage`).
    #[test]
    fn what_a_run_spent_counts_every_model_and_the_cache_as_input() {
        let result = json!({
            "usage": { "input_tokens": 12, "cache_creation_input_tokens": 300,
                "cache_read_input_tokens": 4000, "output_tokens": 50 },
            "modelUsage": {
                "claude-sonnet-4-5-20250929": { "inputTokens": 12, "outputTokens": 50,
                    "cacheReadInputTokens": 4000, "cacheCreationInputTokens": 300,
                    "webSearchRequests": 0, "costUSD": 0.01, "contextWindow": 200000 },
                "claude-haiku-4-5-20251001": { "inputTokens": 100, "outputTokens": 10,
                    "cacheReadInputTokens": 0, "cacheCreationInputTokens": 0,
                    "webSearchRequests": 0, "costUSD": 0.0001, "contextWindow": 200000 },
            },
        });
        assert_eq!(
            result_tokens(&result),
            Some(Tokens {
                input: 4412,
                cached_input: 4000,
                output: 60,
                reasoning_output: 0,
                total: 4472,
            })
        );
        let usage_only = json!({ "usage": result["usage"].clone() });
        assert_eq!(result_tokens(&usage_only).unwrap().total, 4362);
        assert_eq!(result_tokens(&json!({})), None);
    }

    #[test]
    fn a_reported_input_is_cut_as_the_mod_cuts_it() {
        let long = "x".repeat(INPUT_TEXT + 5);
        let cut = clipped(
            &json!({ "content": long, "list": vec![1; 80] }),
            INPUT_TEXT,
            0,
        );
        assert_eq!(
            cut["content"].as_str().unwrap().chars().count(),
            INPUT_TEXT + 1
        );
        assert_eq!(cut["list"].as_array().unwrap().len(), 50);
    }
}
