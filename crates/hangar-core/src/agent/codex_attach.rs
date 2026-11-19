//! Codex's side channel: Hangar's client of a Codex pane's own app-server
//! (docs/native/agents.md §4.1, §4.2.3–§4.2.6; D5, D9). What the hangar-bridge
//! mod is for Claude Code, on Hangar's side of the wire.
//!
//! A Codex pane runs `codex --remote unix://<socket>` against a server of the
//! pane's own (`pty/codex_server.rs`); [`attach`] connects Hangar to that
//! server as a second client, beside the TUI, and from then on, for that
//! pane:
//!
//! - **follows its threads**: the ones loaded when it connects
//!   (`thread/loaded/list`), a new one (`thread/started`, the TUI's `/new`),
//!   one the TUI resumes (its `thread/status/changed`) — each joined with
//!   `thread/resume`, which attaches a second connection to a running thread
//!   rather than starting it again; a subagent's thread is left to the TUI.
//!   The thread the TUI works in is the pane's session: `hello` the first
//!   time, `session` when it changes;
//! - **reports what the session does**, as the mod reports it
//!   (`bridge::report`): `turn` start and end (with the last agent message as
//!   the headline), `permission` while it waits on the person, `tool` calls
//!   as they run, `settings`, `usage` (context and plan limits), `edit`,
//!   `end`;
//! - **holds what Codex asks the person** — approvals, questions, MCP
//!   elicitations — as the asks the mod holds (bridge.rs), and answers each
//!   with the response its method takes once the person chose in Hangar's
//!   chat, Mobile or Discord. The TUI shows its own dialog all the while:
//!   whichever answers first wins, and `serverRequest/resolved` ends the ask
//!   here when it was the TUI. Only the person answers: an agent cannot;
//! - **delivers the pane's queue** (D9): when the session is idle, the oldest
//!   message as a `turn/start`; in the middle of a turn, the oldest one an
//!   agent or Hangar sent, as a `turn/steer` of the running turn — one Codex
//!   refuses (the turn ended meanwhile) goes as a `turn/start`, which starts
//!   a turn or joins the one running. What the person queued waits for the
//!   turn's end and goes unlabelled; anyone else's carries the mod's label;
//! - **briefs the thread**: the state an orchestration hands a pane, as a
//!   developer message (`thread/inject_items`) when a thread is joined, and
//!   again after each compaction, which keeps only the thread's initial
//!   context and user messages (`core/src/compact.rs`,
//!   `build_compacted_history`) and so drops it.
//!
//! # Who connects first
//!
//! Codex makes the first client's `clientInfo.name` the server process's
//! originator, for good (`initialize_processor.rs`, `set_default_originator`),
//! and turns first-party features off for any other
//! (`core/src/mcp_skill_dependencies.rs`): a server Hangar reached first would
//! run the TUI's turns as "Hangar"'s. So Hangar connects after the TUI, and
//! never presents another client's name. Nothing in the protocol says a
//! client is there without initializing (`message_processor.rs` answers
//! anything before `initialize` with "Not initialized"), so the system does:
//! on Linux the kernel's socket table lists every connection accepted on the
//! server's socket ([`clients`]), and Hangar waits until there is one, for as
//! long as it takes: without a TUI there is no thread to follow, and
//! connecting anyway would only make Hangar the originator of the TUI that
//! comes later. Where that cannot be read (macOS, a socket the table does not
//! show) Hangar waits a fixed grace instead — long enough for a shell to type
//! the line and the TUI to start — and connects then. *Ordering is not
//! guaranteed there*: a TUI slower than the grace runs with Hangar as its
//! originator, which only costs it the first-party features named above;
//! refusing to attach at all would cost the pane its side channel. Connecting
//! second still leaves one mark, read in the same file and accepted: the end
//! of the server's user agent names the latest client to connect (`(Hangar;
//! <version>)`), the TUI's requests included, as it would any other client.
//!
//! The same reading tells, on Linux, when the TUI leaves: Hangar reports
//! `end` and lets go of the server until a TUI connects again, so that its
//! subscription neither keeps the person's threads loaded nor lets Hangar
//! start turns nobody sees. Where it cannot be read, Hangar stays attached
//! after the TUI quits, until the pane ends: what is queued for the pane then
//! still starts turns in its server, seen at the TUI's next `resume`. A
//! limitation of those systems, until Codex says when a client leaves.
//!
//! What Codex does with the second client was read in its source at the
//! pinned release (`agent::codex::CHECKED_AT`; paths under `codex-rs/`), never
//! run (§4.0): every `// From the source` names the file, and an
//! `// Assumption (from source, not run)` is one the code relies on.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::bridge::{self, AskPoll, Queued};
use crate::codex_app::client::{Cause, RpcError};
use crate::codex_app::generated::{
    AgentMessageThreadItem, AskForApproval, CommandExecutionApprovalDecision,
    CommandExecutionApprovalKind, CommandExecutionRequestApprovalResponse,
    FileChangeApprovalDecision, FileChangeRequestApprovalResponse, FileUpdateChange,
    GrantedPermissionProfile, McpServerElicitationAction, McpServerElicitationRequestResponse,
    MessagePhase, ModeKind, PatchApplyStatus, PatchChangeKind, PermissionGrantScope,
    PermissionsRequestApprovalResponse, RateLimitSnapshot, RateLimitWindow, RequestId,
    RequestPermissionProfile, SandboxPolicy, ServerNotification, TextUserInput, Thread,
    ThreadActiveFlag, ThreadInjectItemsParams, ThreadItem, ThreadLoadedListParams,
    ThreadReadParams, ThreadResumeParams, ThreadStatus, ToolRequestUserInputAnswer,
    ToolRequestUserInputOption, ToolRequestUserInputQuestion, ToolRequestUserInputResponse,
    TurnCompletedNotification, TurnInterruptParams, TurnStartParams, TurnStatus, TurnSteerParams,
    UserInput,
};
use crate::codex_app::{
    requests, server_requests, Client, Closed, Error, Event, Events, Incoming, Options, Pending,
    Unhandled,
};
use crate::host::Host;

/// Who sends Hangar's own events (`Queued::from`), as the mod names it.
const HANGAR: &str = "hangar";
/// The tool names the window already reads (the mod's, Claude Code's): a
/// question Codex asks reads as one, a command it runs as a shell command.
const QUESTION_TOOL: &str = "AskUserQuestion";
const COMMAND_TOOL: &str = "Bash";
/// Codex's own names for its patch and permission tools.
const PATCH_TOOL: &str = "apply_patch";
const PERMISSIONS_TOOL: &str = "request_permissions";
/// Past this, a text in what an ask is about is cut, as the mod cuts it.
const INPUT_TEXT: usize = 4096;
/// The longest headline a turn's end carries, as the mod's.
const HEADLINE_CHARS: usize = 200;
/// What the person queued that Codex would take as one of its slash
/// commands: those run in the TUI, not through the protocol.
const SLASH_REFUSED: &str = "Codex runs its slash commands in its own window: type it in the pane";
/// The longest a request to the pane's server waits.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The longest the handshake waits.
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(15);
/// The longest a briefing is waited for before a thread goes without it.
const BRIEFING_WAIT: Duration = Duration::from_secs(5);
/// What this connection asks Codex not to send: the streams of a turn, which
/// nothing here reads (the chat view reads items, T9).
const OPTED_OUT: &[&str] = &[
    "item/agentMessage/delta",
    "item/plan/delta",
    "item/commandExecution/outputDelta",
    "item/mcpToolCall/progress",
    "turn/diff/updated",
    "turn/plan/updated",
];

/// How often things are looked at, and for how long: the defaults, or a
/// test's.
#[derive(Clone, Debug)]
struct Pace {
    /// How often the socket table is read while no TUI is there yet.
    tui_poll: Duration,
    /// After this long without one, read it less often.
    tui_patience: Duration,
    tui_poll_slow: Duration,
    /// Where the socket table cannot tell: how long the TUI is given to
    /// connect before Hangar does.
    tui_grace: Duration,
    /// How often the socket table is read once attached, to see the TUI go.
    presence_poll: Duration,
    /// The waits between attempts to connect again, after which Hangar
    /// gives up on the pane until it is attached again.
    reconnects: Vec<Duration>,
}

impl Default for Pace {
    fn default() -> Pace {
        Pace {
            tui_poll: Duration::from_millis(250),
            tui_patience: Duration::from_secs(30),
            tui_poll_slow: Duration::from_secs(1),
            tui_grace: Duration::from_secs(10),
            presence_poll: Duration::from_secs(2),
            reconnects: [500, 1_000, 2_000, 4_000, 8_000]
                .into_iter()
                .map(Duration::from_millis)
                .collect(),
        }
    }
}

/// The state an orchestration hands the pane (orchestrator.rs's `context`),
/// or None.
type Briefer =
    Arc<dyn Fn(Host, String) -> Pin<Box<dyn Future<Output = Option<String>> + Send>> + Send + Sync>;
/// How many clients the pane's server has, or None where that cannot be read.
type Presence = Arc<dyn Fn() -> Option<usize> + Send + Sync>;
/// A connection to the pane's server, past its handshake.
type Connector = Arc<
    dyn Fn(Options) -> Pin<Box<dyn Future<Output = Result<(Client, Events), Error>> + Send>>
        + Send
        + Sync,
>;

/// One attached pane, as its client sees it.
#[derive(Clone)]
struct Ctx {
    host: Host,
    pane: String,
    name: String,
    briefing: Briefer,
    presence: Presence,
    pace: Pace,
}

impl Ctx {
    /// What the session does, the way the mod reports it.
    fn report(&self, event: Value) {
        bridge::report(&self.host, &self.pane, event);
    }

    fn say(&self, message: &str) {
        eprintln!("[codex attach] pane {}: {message}", self.pane);
    }
}

// ---------------------------------------------------------------------------
// The attached panes
// ---------------------------------------------------------------------------

struct Attachment {
    stop: watch::Sender<bool>,
    generation: u64,
}

fn attachments() -> MutexGuard<'static, HashMap<String, Attachment>> {
    static ATTACHMENTS: OnceLock<Mutex<HashMap<String, Attachment>>> = OnceLock::new();
    ATTACHMENTS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

/// Attaches Hangar to pane `pane_id`'s Codex server, listening on `socket`
/// (`pty::codex_socket`, `Spawned::server`): a task on the current tokio
/// runtime that follows the pane until it is detached — its shell ends or it
/// closes (`bridge::release_edits`, `bridge::forget`), or it is attached
/// again — or its server is gone for good. `pane_name` is what its edits are
/// signed with. None outside a runtime, or where Hangar does not attach to
/// Codex (not unix: `pty/codex_server.rs` never starts a server there).
///
/// The returned task is for a caller that wants to wait for its end; nothing
/// needs to keep it.
pub fn attach(
    host: &Host,
    pane_id: &str,
    pane_name: &str,
    socket: PathBuf,
) -> Option<JoinHandle<()>> {
    let runtime = tokio::runtime::Handle::try_current().ok()?;
    let connector = socket_connector(socket.clone())?;
    let ctx = Ctx {
        host: host.clone(),
        pane: pane_id.to_string(),
        name: pane_name.to_string(),
        briefing: window_briefing(),
        presence: Arc::new(move || clients(&socket)),
        pace: Pace::default(),
    };
    Some(start(&runtime, ctx, connector))
}

/// Lets go of pane `pane_id`'s Codex server, if Hangar is attached to it:
/// its task ends, asks it held with it. Called wherever a pane's shell ends.
pub fn detach(pane_id: &str) {
    let gone = attachments().remove(pane_id);
    if let Some(attachment) = gone {
        let _ = attachment.stop.send(true);
    }
}

/// Whether Hangar follows pane `pane_id`'s Codex server (or waits for its
/// TUI to be there first).
pub fn is_attached(pane_id: &str) -> bool {
    attachments().contains_key(pane_id)
}

#[cfg(unix)]
fn socket_connector(socket: PathBuf) -> Option<Connector> {
    use crate::codex_app::Endpoint;
    Some(Arc::new(move |options| {
        boxed(Client::connect(Endpoint::Socket(socket.clone()), options))
    }))
}

#[cfg(not(unix))]
fn socket_connector(_socket: PathBuf) -> Option<Connector> {
    None
}

/// The window's briefing of the pane (`orchestrator::briefing`): its state
/// part, the one handed to every conversation that starts. The role is on the
/// pane server's command line already (`developer_instructions`, §4.2.5).
fn window_briefing() -> Briefer {
    Arc::new(|host, pane| {
        boxed(async move {
            let briefing = crate::orchestrator::briefing(&host, &pane).await.ok()??;
            let context = briefing.get("context")?.as_str()?.trim();
            (!context.is_empty()).then(|| context.to_string())
        })
    })
}

/// A future as the closures above hand theirs out.
fn boxed<T>(
    future: impl Future<Output = T> + Send + 'static,
) -> Pin<Box<dyn Future<Output = T> + Send>> {
    Box::pin(future)
}

fn start(runtime: &tokio::runtime::Handle, ctx: Ctx, connector: Connector) -> JoinHandle<()> {
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    let (stop, stopped) = watch::channel(false);
    let earlier = attachments().insert(ctx.pane.clone(), Attachment { stop, generation });
    if let Some(earlier) = earlier {
        let _ = earlier.stop.send(true);
    }
    runtime.spawn(run(ctx, connector, stopped, generation))
}

/// Ends once `stop` says so, or its sender is gone.
async fn stopped(stop: &mut watch::Receiver<bool>) {
    while !*stop.borrow_and_update() {
        if stop.changed().await.is_err() {
            return;
        }
    }
}

/// The life of one attached pane: wait for its TUI, connect, follow; when
/// the TUI leaves, wait for it again; when the server goes, try again a few
/// times, then give up.
async fn run(ctx: Ctx, connector: Connector, mut stop: watch::Receiver<bool>, generation: u64) {
    let mut briefed = HashSet::new();
    let mut failures = 0;
    loop {
        if !wait_for_tui(&ctx, &mut stop).await {
            break;
        }
        let connected = tokio::select! {
            connected = connector(options()) => connected,
            () = stopped(&mut stop) => break,
        };
        match connected {
            Ok((client, events)) => {
                failures = 0;
                let client = Arc::new(client);
                let ended =
                    follow(&ctx, Arc::clone(&client), events, &mut stop, &mut briefed).await;
                client.shutdown().await;
                match ended {
                    Ended::Stopped => break,
                    // Quit in the pane: the shell has it back.
                    Ended::TuiLeft => ctx.report(json!({ "kind": "end", "reason": "exit" })),
                    Ended::Closed(closed) => {
                        ctx.say(&closed.to_string());
                        ctx.report(json!({ "kind": "end", "reason": "closed" }));
                    }
                }
            }
            // Too old, or listed broken (codex_app/gate.rs): nothing to speak to.
            Err(Error::Unsupported(refusal)) => {
                ctx.say(&refusal.to_string());
                break;
            }
            Err(error) => {
                let Some(delay) = ctx.pace.reconnects.get(failures).copied() else {
                    ctx.say(&format!("gave up attaching: {error}"));
                    break;
                };
                failures += 1;
                tokio::select! {
                    () = tokio::time::sleep(delay) => {}
                    () = stopped(&mut stop) => break,
                }
            }
        }
    }
    let mut attachments = attachments();
    if attachments
        .get(&ctx.pane)
        .is_some_and(|attachment| attachment.generation == generation)
    {
        attachments.remove(&ctx.pane);
    }
}

/// How Hangar's client connects: as "Hangar" (never as another client), and
/// leaving to the TUI what it does not answer itself — under
/// `Unhandled::Refuse`, Hangar's error would be the first answer, which
/// resolves the TUI's request too (`outgoing_message.rs`,
/// `take_connection_callback`).
fn options() -> Options {
    Options {
        request_timeout: REQUEST_TIMEOUT,
        initialize_timeout: INITIALIZE_TIMEOUT,
        unhandled: Unhandled::Leave,
        opt_out_notifications: OPTED_OUT.iter().map(|method| method.to_string()).collect(),
        ..Options::default()
    }
}

/// Waits until the pane's TUI is connected to its server (see "Who connects
/// first", above): true then, false when the pane was detached meanwhile. A
/// client is seen twice in a row first, so that a connection on its way out
/// — Hangar's own, just let go of — is not taken for a TUI.
async fn wait_for_tui(ctx: &Ctx, stop: &mut watch::Receiver<bool>) -> bool {
    let started = Instant::now();
    let mut seen = 0;
    loop {
        if *stop.borrow() {
            return false;
        }
        match (ctx.presence)() {
            Some(clients) if clients > 0 => {
                seen += 1;
                if seen >= 2 {
                    return true;
                }
            }
            Some(_) => seen = 0,
            None if started.elapsed() >= ctx.pace.tui_grace => return true,
            None => {}
        }
        let poll = if started.elapsed() < ctx.pace.tui_patience {
            ctx.pace.tui_poll
        } else {
            ctx.pace.tui_poll_slow
        };
        tokio::select! {
            () = tokio::time::sleep(poll) => {}
            () = stopped(stop) => return false,
        }
    }
}

/// How many connections the server listening on `socket` has accepted, as
/// the kernel's socket table lists them (`/proc/net/unix`): an accepted
/// connection carries the listening socket's address, the connecting side
/// none. `socket` may be the link Codex publishes to its real socket
/// (`pty/codex_server.rs`). None when the table does not show the listening
/// socket itself: then it cannot tell.
#[cfg(target_os = "linux")]
fn clients(socket: &Path) -> Option<usize> {
    // The table names a socket by the path it was bound at: the link's
    // target as Codex wrote it, or its resolved path, or the path itself.
    let mut paths = vec![socket.to_path_buf()];
    if let Ok(target) = std::fs::read_link(socket) {
        paths.push(
            socket
                .parent()
                .map_or(target.clone(), |dir| dir.join(&target)),
        );
    }
    paths.extend(std::fs::canonicalize(socket).ok());
    let table = std::fs::read_to_string("/proc/net/unix").ok()?;
    paths
        .iter()
        .filter_map(|path| path.to_str())
        .find_map(|path| clients_in(&table, path))
}

#[cfg(not(target_os = "linux"))]
fn clients(_socket: &Path) -> Option<usize> {
    None
}

/// [`clients`], from the table's text: `Num RefCount Protocol Flags Type St
/// Inode Path`, the listening socket flagged `__SO_ACCEPTCON` (0x10000), a
/// connection in state 02 (connecting: not accepted yet) or 03 (connected).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn clients_in(table: &str, path: &str) -> Option<usize> {
    let mut listening = false;
    let mut connected = 0;
    for line in table.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 8 || fields[7..].join(" ") != path {
            continue;
        }
        let flags = u32::from_str_radix(fields[3], 16).unwrap_or(0);
        if flags & 0x10000 != 0 {
            listening = true;
        } else if matches!(fields[5], "02" | "03") {
            connected += 1;
        }
    }
    listening.then_some(connected)
}

// ---------------------------------------------------------------------------
// One connection
// ---------------------------------------------------------------------------

/// Why a connection was let go of.
#[derive(Debug)]
enum Ended {
    /// The pane was detached.
    Stopped,
    /// Hangar's was the server's last connection: the TUI quit.
    TuiLeft,
    /// The server closed it, or it broke.
    Closed(Closed),
}

/// Follows the pane's server over one connection, until it ends.
async fn follow(
    ctx: &Ctx,
    client: Arc<Client>,
    mut events: Events,
    stop: &mut watch::Receiver<bool>,
    briefed: &mut HashSet<String>,
) -> Ended {
    let mut queue = bridge::queue_changes();
    let mut follower = Follower::new(ctx, Arc::clone(&client), briefed);
    let stopped_early = tokio::select! {
        () = follower.discover() => false,
        () = stopped(stop) => true,
    };
    if stopped_early {
        follower.finish();
        return Ended::Stopped;
    }
    let mut presence = tokio::time::interval(ctx.pace.presence_poll);
    presence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut alone = 0;
    let ended = loop {
        follower.deliver().await;
        tokio::select! {
            event = events.next() => match event {
                Some(Event::Notification(notification)) => {
                    follower.on_notification(notification).await;
                }
                Some(Event::Request(request)) => follower.on_request(request),
                Some(Event::Closed(closed)) => break Ended::Closed(closed),
                None => {
                    break Ended::Closed(client.closed().unwrap_or(Closed {
                        cause: Cause::ServerClosed,
                        stderr: Vec::new(),
                    }))
                }
            },
            // Something was queued for a pane: maybe this one.
            _ = queue.changed() => {}
            _ = presence.tick() => {
                // Read twice in a row, so a TUI reconnecting is not taken
                // for one that left.
                if (ctx.presence)().is_some_and(|clients| clients <= 1) {
                    alone += 1;
                    if alone >= 2 {
                        break Ended::TuiLeft;
                    }
                } else {
                    alone = 0;
                }
            }
            () = stopped(stop) => break Ended::Stopped,
        }
    };
    follower.finish();
    ended
}

/// Whether a turn runs in a thread, and which, when known (a thread joined
/// mid-turn says only that it is active).
#[derive(Debug, Clone, PartialEq)]
enum Running {
    Idle,
    Turn(Option<String>),
}

/// A thread Hangar follows.
#[derive(Debug)]
struct Followed {
    model: Option<String>,
    effort: Option<String>,
    mode: &'static str,
    cwd: String,
    running: Running,
    /// A `turn` start was reported for the running turn.
    started: bool,
    /// The turn's last agent message, and whether it was its final answer.
    answer: Option<(String, bool)>,
}

/// An ask held for the person (bridge.rs) on one of Codex's requests.
#[derive(Debug)]
struct Held {
    ask: String,
    thread: String,
    /// The call a question is about, reported as a running tool.
    question: Option<String>,
}

/// What follows the pane's threads over one connection.
struct Follower<'a> {
    ctx: &'a Ctx,
    client: Arc<Client>,
    /// Starts the ids of the asks held on this connection: Codex numbers its
    /// requests per process, and a new server numbers them again.
    tag: String,
    /// Codex's release, from its user agent.
    version: String,
    threads: HashMap<String, Followed>,
    /// Threads seen and not followed: subagents'.
    ignored: HashSet<String>,
    /// The thread the TUI works in: the pane's session.
    current: Option<String>,
    said_hello: bool,
    /// A `permission` with `waiting: true` was the last reported.
    waiting: bool,
    /// The current thread's status says it waits on the person.
    status_waiting: bool,
    /// Asks held, by Codex's request id.
    held: HashMap<String, Held>,
    waiters: Vec<JoinHandle<()>>,
    /// The changes of the patches in progress, by item: what an approval of
    /// one shows.
    patches: HashMap<String, Vec<FileUpdateChange>>,
    /// The current thread's context, as `usage` reports it.
    context: Option<Value>,
    /// The plan's limits, merged from Codex's sparse updates.
    limits: Option<RateLimitSnapshot>,
    /// The `settings` last reported.
    settings: Option<Value>,
    /// A delivery waits for the running turn's end: Codex cannot take one
    /// now (a compaction, a review), or the connection is going.
    blocked: bool,
    /// Threads handed their briefing, across reconnections: what is in a
    /// thread's history stays there.
    briefed: &'a mut HashSet<String>,
    /// Compactions handled, by thread and turn: Codex says it twice.
    compactions: HashSet<String>,
}

impl<'a> Follower<'a> {
    fn new(ctx: &'a Ctx, client: Arc<Client>, briefed: &'a mut HashSet<String>) -> Self {
        let server = client.server();
        let version = server
            .verdict
            .version()
            .map(ToString::to_string)
            .unwrap_or_default();
        let tag: String = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        Follower {
            ctx,
            client,
            tag,
            version,
            threads: HashMap::new(),
            ignored: HashSet::new(),
            current: None,
            said_hello: false,
            waiting: false,
            status_waiting: false,
            held: HashMap::new(),
            waiters: Vec::new(),
            patches: HashMap::new(),
            context: None,
            limits: None,
            settings: None,
            blocked: false,
            briefed,
            compactions: HashSet::new(),
        }
    }

    // ----- threads

    /// The threads the server has loaded, joined; the latest becomes the
    /// pane's session.
    async fn discover(&mut self) {
        let mut ids = Vec::new();
        let mut cursor = None;
        loop {
            let params = ThreadLoadedListParams {
                cursor: cursor.take(),
                limit: None,
            };
            match self
                .client
                .request::<requests::ThreadLoadedList>(params)
                .await
            {
                Ok(page) => {
                    ids.extend(page.data);
                    match page.next_cursor {
                        Some(next) => cursor = Some(next),
                        None => break,
                    }
                }
                Err(error) => {
                    self.ctx
                        .say(&format!("could not list its threads: {error}"));
                    break;
                }
            }
        }
        let mut latest: Option<(i64, String)> = None;
        for id in ids {
            if let Some(at) = self.take_up(&id, None).await {
                if latest.as_ref().is_none_or(|(last, _)| at >= *last) {
                    latest = Some((at, id));
                }
            }
        }
        if let Some((_, id)) = latest {
            self.switch_to(&id, "resume");
        }
    }

    /// Joins thread `id` (`thread/resume`), unless it is a subagent's or no
    /// longer loaded: its last activity, when joined. `known` is the thread
    /// as a `thread/started` gave it, else it is read first (`thread/read`,
    /// which neither loads nor joins).
    ///
    /// From the source (`thread_processor.rs`, `resume_running_thread`): a
    /// running thread is rejoined, its overrides ignored — Hangar sends none
    /// — its pending requests replayed to the new connection; it also takes
    /// Hangar as the thread's latest client until the TUI's next
    /// `turn/start` names itself again (`set_app_server_client_info`).
    async fn take_up(&mut self, id: &str, known: Option<Thread>) -> Option<i64> {
        if self.threads.contains_key(id) || self.ignored.contains(id) {
            return None;
        }
        let thread = match known {
            Some(thread) => thread,
            None => {
                let params = ThreadReadParams {
                    include_turns: Some(false),
                    thread_id: id.to_string(),
                };
                match self.client.request::<requests::ThreadRead>(params).await {
                    Ok(read) => read.thread,
                    Err(error) => {
                        self.ctx
                            .say(&format!("could not read thread {id}: {error}"));
                        return None;
                    }
                }
            }
        };
        if thread.parent_thread_id.is_some() {
            // A subagent's: the TUI answers what it asks.
            self.ignored.insert(id.to_string());
            return None;
        }
        // Resuming a thread that is not loaded would load it: one the TUI
        // left alone stays so.
        if matches!(thread.status, ThreadStatus::NotLoaded) {
            return None;
        }
        let params = ThreadResumeParams {
            thread_id: id.to_string(),
            // Its metadata only: a paginated thread's whole history is
            // deprecated on resume, and nothing here reads it.
            exclude_turns: Some(true),
            ..ThreadResumeParams::default()
        };
        let resumed = match self.client.request::<requests::ThreadResume>(params).await {
            Ok(resumed) => resumed,
            Err(error) => {
                self.ctx
                    .say(&format!("could not join thread {id}: {error}"));
                return None;
            }
        };
        let running = if matches!(resumed.thread.status, ThreadStatus::Active(_)) {
            Running::Turn(None)
        } else {
            Running::Idle
        };
        self.threads.insert(
            id.to_string(),
            Followed {
                model: Some(resumed.model.clone()).filter(|model| !model.is_empty()),
                effort: resumed.reasoning_effort.clone(),
                mode: mode_of(
                    resumed.collaboration_mode.as_ref().map(|mode| &mode.mode),
                    &resumed.approval_policy,
                    &resumed.sandbox,
                ),
                cwd: resumed.cwd.clone(),
                running,
                started: false,
                answer: None,
            },
        );
        if !self.briefed.contains(id) {
            self.brief(id).await;
        }
        Some(
            resumed
                .thread
                .recency_at
                .unwrap_or(resumed.thread.updated_at),
        )
    }

    /// Makes thread `id` the pane's session: `hello` the first time, `session`
    /// after, then how it runs.
    fn switch_to(&mut self, id: &str, source: &str) {
        if self.current.as_deref() == Some(id) {
            return;
        }
        let Some(thread) = self.threads.get(id) else {
            return;
        };
        let model = thread.model.clone().unwrap_or_default();
        let running = thread.running != Running::Idle;
        self.current = Some(id.to_string());
        self.status_waiting = false;
        if self.said_hello {
            self.ctx
                .report(json!({ "kind": "session", "sessionId": id, "source": source }));
        } else {
            self.said_hello = true;
            self.ctx.report(json!({
                "kind": "hello",
                "sessionId": id,
                "model": model,
                "version": self.version,
            }));
        }
        // A session starts in its own mode, until it says (pane_agent.rs).
        self.settings = None;
        self.report_settings();
        if running {
            if let Some(thread) = self.threads.get_mut(id) {
                thread.started = true;
            }
            self.ctx.report(json!({ "kind": "turn", "phase": "start" }));
        }
    }

    /// A thread unloaded: its asks end with it.
    fn drop_thread(&mut self, id: &str) {
        self.threads.remove(id);
        if self.current.as_deref() == Some(id) {
            self.current = None;
        }
        self.settle_thread(id);
    }

    fn report_settings(&mut self) {
        let Some(thread) = self.current.as_ref().and_then(|id| self.threads.get(id)) else {
            return;
        };
        let event = json!({
            "kind": "settings",
            "model": thread.model,
            "mode": thread.mode,
            "effort": thread.effort,
        });
        if self.settings.as_ref() != Some(&event) {
            self.settings = Some(event.clone());
            self.ctx.report(event);
        }
    }

    /// Hands thread `id` the pane's briefing, as a developer message in its
    /// history. From the source (`core/src/codex_thread.rs`,
    /// `inject_response_items`): the items are Responses API items, recorded
    /// at once, the thread's initial context first when it has none yet.
    async fn brief(&mut self, id: &str) {
        let briefing = (self.ctx.briefing)(self.ctx.host.clone(), self.ctx.pane.clone());
        let Ok(Some(context)) = tokio::time::timeout(BRIEFING_WAIT, briefing).await else {
            return;
        };
        let text = format!("[Hangar] Briefing — not typed by the person.\n\n{context}");
        let params = ThreadInjectItemsParams {
            items: vec![developer_message(&text)],
            thread_id: id.to_string(),
        };
        match self
            .client
            .request::<requests::ThreadInjectItems>(params)
            .await
        {
            Ok(_) => {
                self.briefed.insert(id.to_string());
            }
            Err(error) => self
                .ctx
                .say(&format!("could not brief thread {id}: {error}")),
        }
    }

    /// A compaction in thread `id`: what was injected went with the history
    /// it replaced, so the briefing is handed again — once per compaction,
    /// which Codex reports as an item and, still, as `thread/compacted`.
    async fn compacted(&mut self, id: &str, turn: &str) {
        if !self.threads.contains_key(id) || !self.compactions.insert(format!("{id}/{turn}")) {
            return;
        }
        self.briefed.remove(id);
        self.brief(id).await;
    }

    // ----- what Codex says

    async fn on_notification(&mut self, notification: ServerNotification) {
        match notification {
            // A new thread — the TUI's `/new`, a fork — is broadcast to every
            // client (thread_processor.rs, `send_server_notification` of the
            // process-wide sender).
            ServerNotification::ThreadStarted(started) => {
                let id = started.thread.id.clone();
                if self.take_up(&id, Some(started.thread)).await.is_some() {
                    self.switch_to(&id, "startup");
                }
            }
            ServerNotification::ThreadStatusChanged(changed) => {
                self.on_status(&changed.thread_id, &changed.status).await;
            }
            ServerNotification::ThreadClosed(closed) => self.drop_thread(&closed.thread_id),
            ServerNotification::ThreadSettingsUpdated(updated) => {
                let settings = updated.thread_settings;
                if let Some(thread) = self.threads.get_mut(&updated.thread_id) {
                    thread.model = Some(settings.model).filter(|model| !model.is_empty());
                    thread.effort = settings.effort;
                    thread.mode = mode_of(
                        Some(&settings.collaboration_mode.mode),
                        &settings.approval_policy,
                        &settings.sandbox_policy,
                    );
                }
                if self.current.as_deref() == Some(updated.thread_id.as_str()) {
                    self.report_settings();
                }
            }
            ServerNotification::ThreadTokenUsageUpdated(usage) => {
                if self.current.as_deref() == Some(usage.thread_id.as_str()) {
                    // From the source (protocol/src/protocol.rs,
                    // `tokens_in_context_window`): what the last request
                    // carried is what fills the window.
                    let usage = usage.token_usage;
                    self.context = Some(json!({
                        "tokens": usage.last.total_tokens.max(0),
                        "window": usage.model_context_window.unwrap_or(0).max(0),
                        "percent": null,
                    }));
                    self.report_usage();
                }
            }
            ServerNotification::AccountRateLimitsUpdated(updated) => {
                self.limits = merge_limits(self.limits.take(), updated.rate_limits);
                self.report_usage();
            }
            ServerNotification::ThreadCompacted(compacted) => {
                self.compacted(&compacted.thread_id, &compacted.turn_id)
                    .await;
            }
            ServerNotification::TurnStarted(started) => {
                let id = started.thread_id;
                let Some(thread) = self.threads.get_mut(&id) else {
                    return;
                };
                let fresh = !thread.started
                    || thread.running != Running::Turn(Some(started.turn.id.clone()));
                thread.running = Running::Turn(Some(started.turn.id));
                thread.answer = None;
                if self.current.as_deref() != Some(id.as_str()) {
                    // A turn in another thread of the pane's: the TUI works
                    // there now.
                    self.switch_to(&id, "resume");
                } else if fresh {
                    thread.started = true;
                    self.ctx.report(json!({ "kind": "turn", "phase": "start" }));
                }
            }
            ServerNotification::TurnCompleted(completed) => self.on_turn_completed(completed),
            ServerNotification::ItemStarted(started) => {
                if self.current.as_deref() != Some(started.thread_id.as_str()) {
                    return;
                }
                if let ThreadItem::FileChange(item) = &started.item {
                    self.patches.insert(item.id.clone(), item.changes.clone());
                }
                if let Some((id, tool, input)) = tool_of(&started.item) {
                    self.ctx.report(json!({
                        "kind": "tool",
                        "phase": "start",
                        "toolUseId": id,
                        "tool": tool,
                        "input": clipped(&input, 0),
                    }));
                }
            }
            ServerNotification::ItemFileChangePatchUpdated(updated) => {
                self.patches.insert(updated.item_id, updated.changes);
            }
            ServerNotification::ItemCompleted(completed) => {
                let thread = completed.thread_id;
                match &completed.item {
                    ThreadItem::AgentMessage(message) => self.on_agent_message(&thread, message),
                    ThreadItem::ContextCompaction(_) => {
                        self.compacted(&thread, &completed.turn_id).await;
                    }
                    ThreadItem::FileChange(item) => {
                        self.patches.remove(&item.id);
                        if item.status == PatchApplyStatus::Completed {
                            self.on_patched(&thread, &item.changes).await;
                        }
                    }
                    _ => {}
                }
                if self.current.as_deref() == Some(thread.as_str()) {
                    if let Some((id, _, _)) = tool_of(&completed.item) {
                        self.ctx
                            .report(json!({ "kind": "tool", "phase": "end", "toolUseId": id }));
                    }
                }
            }
            // From the source (outgoing_message.rs, `resolve_server_request`):
            // a request answered — by the TUI, by Hangar, or ended with its
            // turn — is said resolved to every client of its thread.
            ServerNotification::ServerRequestResolved(resolved) => {
                let Some(held) = self.held.remove(&key_of(&resolved.request_id)) else {
                    return;
                };
                let thread = held.thread.clone();
                self.end_ask(held);
                if self.current.as_deref() == Some(thread.as_str())
                    && !self.status_waiting
                    && !self.holds_on(&thread)
                {
                    self.set_waiting(false, None);
                }
            }
            _ => {}
        }
    }

    async fn on_status(&mut self, id: &str, status: &ThreadStatus) {
        if self.ignored.contains(id) {
            return;
        }
        if !self.threads.contains_key(id) {
            // A thread the TUI resumed (`/resume`, `codex --remote … resume`):
            // no `thread/started` comes for one, but its status does, to
            // every client (thread_status.rs, `ThreadWatchManager` on the
            // process-wide sender).
            if !matches!(status, ThreadStatus::NotLoaded) && self.take_up(id, None).await.is_some()
            {
                self.switch_to(id, "resume");
            }
            return;
        }
        if matches!(status, ThreadStatus::NotLoaded) {
            self.drop_thread(id);
            return;
        }
        if let (ThreadStatus::Idle, Some(thread)) = (status, self.threads.get_mut(id)) {
            // A turn Hangar started that never said it started.
            if !thread.started {
                thread.running = Running::Idle;
            }
        }
        if self.current.as_deref() != Some(id) {
            return;
        }
        let waits = matches!(status, ThreadStatus::Active(active) if active.active_flags.iter().any(|flag| {
            matches!(flag, ThreadActiveFlag::WaitingOnApproval | ThreadActiveFlag::WaitingOnUserInput)
        }));
        self.status_waiting = waits;
        if waits {
            self.set_waiting(true, None);
        } else if !self.holds_on(id) {
            self.set_waiting(false, None);
        }
    }

    fn on_turn_completed(&mut self, completed: TurnCompletedNotification) {
        let id = completed.thread_id;
        let Some(thread) = self.threads.get_mut(&id) else {
            return;
        };
        let answer = thread.answer.take().map(|(text, _)| text);
        thread.running = Running::Idle;
        thread.started = false;
        self.blocked = false;
        // The turn's asks ended with it; so did any dialog on screen.
        self.settle_thread(&id);
        if self.current.as_deref() != Some(id.as_str()) {
            return;
        }
        self.waiting = false;
        self.status_waiting = false;
        let turn = completed.turn;
        let reason = match turn.status {
            TurnStatus::Completed => "answer",
            TurnStatus::Interrupted => "aborted",
            _ => "error",
        };
        let said = answer
            .or_else(|| turn.error.as_ref().map(|error| error.message.clone()))
            .unwrap_or_default();
        self.ctx.report(json!({
            "kind": "turn",
            "phase": "end",
            "reason": reason,
            "durationMs": turn.duration_ms.unwrap_or(0).max(0),
            "answer": headline(&said),
        }));
    }

    /// The turn's last agent message, its final answer above any other.
    fn on_agent_message(&mut self, thread: &str, message: &AgentMessageThreadItem) {
        let Some(thread) = self.threads.get_mut(thread) else {
            return;
        };
        if message.text.trim().is_empty() {
            return;
        }
        let last_word = matches!(message.phase, Some(MessagePhase::FinalAnswer));
        if last_word
            || !thread
                .answer
                .as_ref()
                .is_some_and(|(_, final_answer)| *final_answer)
        {
            thread.answer = Some((message.text.clone(), last_word));
        }
    }

    /// Files a patch changed: the pane's edits, and a word to the model about
    /// another pane's recent edit of the same file, as the mod gives it.
    async fn on_patched(&mut self, thread: &str, changes: &[FileUpdateChange]) {
        let cwd = self
            .threads
            .get(thread)
            .map(|followed| followed.cwd.clone())
            .unwrap_or_default();
        let mut notes = Vec::new();
        for change in changes {
            let mut files = vec![absolute(&cwd, &change.path)];
            if let PatchChangeKind::Update(update) = &change.kind {
                files.extend(update.move_path.as_deref().map(|to| absolute(&cwd, to)));
            }
            for file in files {
                let mut event = json!({ "kind": "edit", "file": file, "name": self.ctx.name });
                if let Some(owner) = bridge::conflict(&self.ctx.pane, &file) {
                    event["overlap"] = json!(owner.name);
                    event["overlapPane"] = json!(owner.pane_id);
                    notes.push(overlap_note(&owner.name, &file, owner.ago_ms));
                }
                self.ctx.report(event);
            }
        }
        let Some(Running::Turn(Some(turn))) = self.threads.get(thread).map(|t| t.running.clone())
        else {
            return;
        };
        if notes.is_empty() {
            return;
        }
        let note = Queued {
            id: String::new(),
            text: notes.join("\n\n"),
            from: Some(HANGAR.to_string()),
        };
        let params = TurnSteerParams {
            client_user_message_id: None,
            expected_turn_id: turn,
            input: vec![text_input(&labelled(&note))],
            thread_id: thread.to_string(),
        };
        if let Err(error) = self.client.request::<requests::TurnSteer>(params).await {
            self.ctx
                .say(&format!("could not pass on an edit overlap: {error}"));
        }
    }

    fn report_usage(&self) {
        // A context gauge without its context would be cleared: the limits
        // wait for the first token count.
        let Some(context) = &self.context else {
            return;
        };
        self.ctx.report(json!({
            "kind": "usage",
            "context": context,
            "rateLimits": limits_of(self.limits.as_ref()),
            "costUsd": null,
        }));
    }

    // ----- what Codex asks

    /// A request of Codex's for the person: held as an ask, and answered
    /// with the person's choice once given (see the top of this file). One
    /// about a thread Hangar does not follow is left to the TUI.
    fn on_request(&mut self, incoming: Incoming) {
        let key = key_of(incoming.id());
        let ask = format!("codex-{}-{}", self.tag, key.trim_matches('"'));
        let Some(asked) = describe(incoming, &self.patches) else {
            return;
        };
        if !self.threads.contains_key(&asked.thread) {
            return;
        }
        if asked.question {
            if let Some(item) = &asked.tool_use_id {
                self.ctx.report(json!({
                    "kind": "tool",
                    "phase": "start",
                    "toolUseId": item,
                    "tool": QUESTION_TOOL,
                    "input": asked.input,
                }));
            }
        }
        let mut detail = Map::new();
        detail.insert("tool".into(), json!(asked.tool));
        detail.insert("input".into(), asked.input.clone());
        detail.insert("askId".into(), json!(ask));
        if let Some(item) = &asked.tool_use_id {
            detail.insert("toolUseId".into(), json!(item));
        }
        self.held.insert(
            key,
            Held {
                ask: ask.clone(),
                thread: asked.thread.clone(),
                question: asked.question.then(|| asked.tool_use_id.clone()).flatten(),
            },
        );
        // Held by the bridge as it is reported (`note_report`), before its
        // wait starts below.
        self.set_waiting(true, Some(detail));
        let waiter = tokio::spawn(wait_for_answer(
            self.ctx.clone(),
            Arc::clone(&self.client),
            ask,
            asked,
        ));
        self.waiters.retain(|waiter| !waiter.is_finished());
        self.waiters.push(waiter);
    }

    /// A dialog that opens is always reported, with what it asks about; one
    /// that closes, once (the mod's `setWaiting`).
    fn set_waiting(&mut self, value: bool, detail: Option<Map<String, Value>>) {
        if self.waiting == value && detail.is_none() {
            return;
        }
        self.waiting = value;
        let mut event = Map::new();
        event.insert("kind".into(), json!("permission"));
        event.insert("waiting".into(), json!(value));
        event.extend(detail.unwrap_or_default());
        self.ctx.report(Value::Object(event));
    }

    fn holds_on(&self, thread: &str) -> bool {
        self.held.values().any(|held| held.thread == thread)
    }

    /// The ask is over here (bridge.rs's `settle_ask`, through the report).
    fn end_ask(&self, held: Held) {
        self.ctx
            .report(json!({ "kind": "ask", "phase": "end", "askId": held.ask }));
        if let Some(item) = held.question {
            self.ctx
                .report(json!({ "kind": "tool", "phase": "end", "toolUseId": item }));
        }
    }

    fn settle_thread(&mut self, thread: &str) {
        let keys: Vec<String> = self
            .held
            .iter()
            .filter(|(_, held)| held.thread == thread)
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            if let Some(held) = self.held.remove(&key) {
                self.end_ask(held);
            }
        }
    }

    // ----- the pane's queue

    /// Hands the session the next message it can take now (D9), if any.
    async fn deliver(&mut self) {
        if self.blocked || self.waiting {
            return;
        }
        let Some(id) = self.current.clone() else {
            return;
        };
        let Some(running) = self.threads.get(&id).map(|thread| thread.running.clone()) else {
            return;
        };
        let mid_turn = running != Running::Idle;
        let Some(message) = bridge::take_next(&self.ctx.host, &self.ctx.pane, mid_turn) else {
            return;
        };
        if message.from.is_none() && message.text.trim_start().starts_with('/') {
            self.ctx.report(json!({
                "kind": "dropped",
                "text": message.text,
                "reason": SLASH_REFUSED,
            }));
            return;
        }
        let text = if message.from.is_some() {
            labelled(&message)
        } else {
            message.text.clone()
        };
        if let Running::Turn(Some(turn)) = &running {
            let params = TurnSteerParams {
                client_user_message_id: None,
                expected_turn_id: turn.clone(),
                input: vec![text_input(&text)],
                thread_id: id.clone(),
            };
            match self.client.request::<requests::TurnSteer>(params).await {
                Ok(_) => return,
                // The turn ended, or another took its place: `turn/start`
                // starts one or joins the one running (turn_processor.rs,
                // `start_or_steer_turn`).
                Err(Error::Rpc { .. }) => {}
                Err(error) => return self.undelivered(message, error),
            }
        }
        let params = TurnStartParams {
            input: vec![text_input(&text)],
            thread_id: id.clone(),
            ..TurnStartParams::default()
        };
        match self.client.request::<requests::TurnStart>(params).await {
            Ok(started) => {
                if let Some(thread) = self.threads.get_mut(&id) {
                    if started.turn.status == TurnStatus::InProgress {
                        thread.running = Running::Turn(Some(started.turn.id));
                    }
                }
            }
            Err(error) => self.undelivered(message, error),
        }
    }

    /// A message Codex did not take: back at the head of the queue when it
    /// can be taken later, dropped (and said, as the mod says it) otherwise.
    fn undelivered(&mut self, message: Queued, error: Error) {
        match &error {
            // Nothing more goes through this connection.
            Error::Closed(_) => {
                self.blocked = true;
                bridge::put_back(&self.ctx.host, &self.ctx.pane, message);
            }
            Error::Rpc { error: refusal, .. } if not_steerable(refusal) => {
                self.blocked = true;
                bridge::put_back(&self.ctx.host, &self.ctx.pane, message);
            }
            _ => self.ctx.report(json!({
                "kind": "dropped",
                "text": message.text,
                "reason": error.to_string(),
            })),
        }
    }

    /// The connection is over: the asks held on it end, their waits with
    /// them (a request dropped unanswered is left to the TUI).
    fn finish(&mut self) {
        for waiter in self.waiters.drain(..) {
            waiter.abort();
        }
        for held in std::mem::take(&mut self.held).into_values() {
            self.end_ask(held);
        }
    }
}

/// Codex's request ids as one text, whatever their shape.
fn key_of(id: &RequestId) -> String {
    serde_json::to_string(id).unwrap_or_default()
}

/// What Codex calls a mode, as the window names one: `plan`, the bypass of
/// every check (`--dangerously-bypass-approvals-and-sandbox`, §4.2.1), or
/// the default.
fn mode_of(
    collaboration: Option<&ModeKind>,
    approval: &AskForApproval,
    sandbox: &SandboxPolicy,
) -> &'static str {
    if matches!(collaboration, Some(ModeKind::Plan)) {
        "plan"
    } else if matches!(approval, AskForApproval::Never)
        && matches!(sandbox, SandboxPolicy::DangerFullAccess)
    {
        "bypassPermissions"
    } else {
        "default"
    }
}

/// A call an item is, as the window reads a running tool: its id, its tool's
/// name, its input.
fn tool_of(item: &ThreadItem) -> Option<(String, String, Value)> {
    match item {
        ThreadItem::CommandExecution(item) => Some((
            item.id.clone(),
            COMMAND_TOOL.to_string(),
            json!({ "command": item.command, "cwd": item.cwd }),
        )),
        ThreadItem::FileChange(item) => Some((
            item.id.clone(),
            PATCH_TOOL.to_string(),
            json!({ "changes": patch_summary(&item.changes) }),
        )),
        ThreadItem::McpToolCall(item) => Some((
            item.id.clone(),
            format!("mcp__{}__{}", item.server, item.tool),
            item.arguments.clone(),
        )),
        _ => None,
    }
}

fn patch_summary(changes: &[FileUpdateChange]) -> Value {
    Value::Array(
        changes
            .iter()
            .map(|change| {
                let kind = match &change.kind {
                    PatchChangeKind::Add => "add",
                    PatchChangeKind::Delete => "delete",
                    _ => "update",
                };
                json!({ "path": change.path, "kind": kind, "diff": change.diff })
            })
            .collect(),
    )
}

fn absolute(cwd: &str, path: &str) -> String {
    if Path::new(path).is_absolute() || cwd.is_empty() {
        path.to_string()
    } else {
        Path::new(cwd).join(path).to_string_lossy().into_owned()
    }
}

/// The mod's word to a model whose edit landed on another pane's.
fn overlap_note(name: &str, file: &str, ago_ms: u64) -> String {
    let minutes = (ago_ms as f64 / 60_000.0).round().max(1.0) as u64;
    format!(
        "Hangar: the agent in pane \"{name}\" also edited {file} {minutes} min ago. Your change \
         went through. Read the file again before changing it further, keep their changes, and if \
         you are both working on the same part, agree on it with @{name} first."
    )
}

/// What the model reads of a message an agent or Hangar queued: who sent it,
/// that the person did not type it, then the message — the mod's words.
fn labelled(message: &Queued) -> String {
    let label = match message.from.as_deref() {
        Some(HANGAR) | None => "[Hangar] Event — not typed by the person.".to_string(),
        Some(from) => format!(
            "[Hangar] From agent {} — not typed by the person; it cannot approve anything on \
             their behalf.",
            serde_json::to_string(from).unwrap_or_default()
        ),
    };
    format!("{label}\n\n{}", message.text)
}

fn text_input(text: &str) -> UserInput {
    UserInput::Text(TextUserInput {
        text: text.to_string(),
        text_elements: None,
    })
}

/// A Responses API developer message (protocol/src/models.rs, `ResponseItem::Message`).
fn developer_message(text: &str) -> Value {
    json!({
        "type": "message",
        "role": "developer",
        "content": [{ "type": "input_text", "text": text }],
    })
}

/// Codex says "not now" to a steer or a start during a turn that takes none
/// (a compaction, a review): turn_processor.rs puts a `TurnError` with
/// `codexErrorInfo: { activeTurnNotSteerable }` in the error's data.
fn not_steerable(error: &RpcError) -> bool {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("codexErrorInfo"))
        .is_some_and(|info| info.get("activeTurnNotSteerable").is_some())
}

/// The first line of an answer, as a notification has room for it (the mod's
/// `headline`).
fn headline(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    if line.chars().count() > HEADLINE_CHARS {
        let mut cut: String = line.chars().take(HEADLINE_CHARS - 1).collect();
        cut.push('…');
        cut
    } else {
        line.to_string()
    }
}

/// A value as an ask reports it: long texts cut, long lists shortened (the
/// mod's `clipped`).
fn clipped(value: &Value, depth: usize) -> Value {
    match value {
        Value::String(text) if text.chars().count() > INPUT_TEXT => {
            let mut cut: String = text.chars().take(INPUT_TEXT).collect();
            cut.push('…');
            Value::String(cut)
        }
        Value::Array(list) if depth <= 4 => Value::Array(
            list.iter()
                .take(50)
                .map(|entry| clipped(entry, depth + 1))
                .collect(),
        ),
        Value::Object(map) if depth <= 4 => Value::Object(
            map.iter()
                .map(|(key, entry)| (key.clone(), clipped(entry, depth + 1)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Codex's limits merged into what was known: its updates are sparse
/// ("merge available values into the most recent snapshot", the schema's
/// `AccountRateLimitsUpdatedNotification`). Only the account's own bucket
/// (`codex`) is the plan's; a model's own bucket is left out.
fn merge_limits(
    known: Option<RateLimitSnapshot>,
    update: RateLimitSnapshot,
) -> Option<RateLimitSnapshot> {
    if update
        .limit_id
        .as_deref()
        .is_some_and(|bucket| bucket != "codex")
    {
        return known;
    }
    let Some(mut known) = known else {
        return Some(update);
    };
    if update.primary.is_some() {
        known.primary = update.primary;
    }
    if update.secondary.is_some() {
        known.secondary = update.secondary;
    }
    if update.plan_type.is_some() {
        known.plan_type = update.plan_type;
    }
    Some(known)
}

/// The plan's windows as the window reads them (`RateLimit` of the engine's
/// live.rs): `five_hour`, `seven_day`, else named by their length.
fn limits_of(snapshot: Option<&RateLimitSnapshot>) -> Vec<Value> {
    let Some(snapshot) = snapshot else {
        return Vec::new();
    };
    [
        (snapshot.primary.as_ref(), "primary"),
        (snapshot.secondary.as_ref(), "secondary"),
    ]
    .into_iter()
    .filter_map(|(window, fallback)| window.map(|window| limit_of(window, fallback)))
    .collect()
}

fn limit_of(window: &RateLimitWindow, fallback: &str) -> Value {
    let kind = match window.window_duration_mins {
        Some(300) => "five_hour".to_string(),
        Some(10_080) => "seven_day".to_string(),
        Some(mins) if mins > 0 && mins % 1_440 == 0 => format!("{}_day", mins / 1_440),
        Some(mins) if mins > 0 && mins % 60 == 0 => format!("{}_hour", mins / 60),
        Some(mins) if mins > 0 => format!("{mins}_minute"),
        _ => fallback.to_string(),
    };
    let resets_at = window
        .resets_at
        .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    json!({
        "kind": kind,
        "percentUsed": window.used_percent as f64,
        "resetsAt": resets_at,
    })
}

// ---------------------------------------------------------------------------
// Codex's requests, held for the person
// ---------------------------------------------------------------------------

/// How a request is answered once the person chose.
// Not boxed: one per request, moved once into the task that waits for its
// answer; the variants differ by what Codex sends with each.
#[allow(clippy::large_enum_variant)]
enum Answerer {
    Command {
        pending: Pending<server_requests::ItemCommandExecutionRequestApproval>,
        /// A managed checkpoint: only accept, decline and cancel are valid.
        human: bool,
    },
    FileChange(Pending<server_requests::ItemFileChangeRequestApproval>),
    Permissions {
        pending: Pending<server_requests::ItemPermissionsRequestApproval>,
        requested: RequestPermissionProfile,
    },
    Question {
        pending: Pending<server_requests::ItemToolRequestUserInput>,
        questions: Vec<ToolRequestUserInputQuestion>,
    },
    Elicitation {
        pending: Pending<server_requests::McpServerElicitationRequest>,
        /// A form to fill (not a URL to open).
        form: bool,
        /// An MCP tool's call to approve (`codex_approval_kind`).
        tool_call: bool,
    },
}

/// A request, as an ask reports it.
struct Asked {
    thread: String,
    turn: Option<String>,
    tool: String,
    input: Value,
    tool_use_id: Option<String>,
    question: bool,
    answerer: Answerer,
}

fn describe(incoming: Incoming, patches: &HashMap<String, Vec<FileUpdateChange>>) -> Option<Asked> {
    Some(match incoming {
        Incoming::CommandExecutionApproval(pending) => {
            let params = pending.params();
            let mut input = Map::new();
            if let Some(command) = &params.command {
                input.insert("command".into(), json!(command));
            }
            if let Some(cwd) = &params.cwd {
                input.insert("cwd".into(), json!(cwd));
            }
            if let Some(reason) = &params.reason {
                input.insert("reason".into(), json!(reason));
            }
            if let Some(kind) = params
                .kind
                .as_ref()
                .filter(|kind| **kind != CommandExecutionApprovalKind::Command)
            {
                input.insert("kind".into(), json!(kind));
            }
            let human = params.kind == Some(CommandExecutionApprovalKind::HumanCommand);
            let (thread, turn, item) = (
                params.thread_id.clone(),
                params.turn_id.clone(),
                params.item_id.clone(),
            );
            Asked {
                thread,
                turn: Some(turn),
                tool: COMMAND_TOOL.into(),
                input: clipped(&Value::Object(input), 0),
                tool_use_id: Some(item),
                question: false,
                answerer: Answerer::Command { pending, human },
            }
        }
        Incoming::FileChangeApproval(pending) => {
            let params = pending.params();
            let mut input = Map::new();
            let changes = patches
                .get(&params.item_id)
                .map(|changes| patch_summary(changes));
            input.insert("changes".into(), changes.unwrap_or_else(|| json!([])));
            if let Some(reason) = &params.reason {
                input.insert("reason".into(), json!(reason));
            }
            if let Some(root) = &params.grant_root {
                input.insert("grantRoot".into(), json!(root));
            }
            let (thread, turn, item) = (
                params.thread_id.clone(),
                params.turn_id.clone(),
                params.item_id.clone(),
            );
            Asked {
                thread,
                turn: Some(turn),
                tool: PATCH_TOOL.into(),
                input: clipped(&Value::Object(input), 0),
                tool_use_id: Some(item),
                question: false,
                answerer: Answerer::FileChange(pending),
            }
        }
        Incoming::PermissionsApproval(pending) => {
            let params = pending.params();
            let input = json!({
                "permissions": params.permissions,
                "reason": params.reason,
                "cwd": params.cwd,
            });
            let requested = params.permissions.clone();
            let (thread, turn, item) = (
                params.thread_id.clone(),
                params.turn_id.clone(),
                params.item_id.clone(),
            );
            Asked {
                thread,
                turn: Some(turn),
                tool: PERMISSIONS_TOOL.into(),
                input: clipped(&input, 0),
                tool_use_id: Some(item),
                question: false,
                answerer: Answerer::Permissions { pending, requested },
            }
        }
        Incoming::UserInput(pending) => {
            let params = pending.params();
            // AskUserQuestion's shape, which the window reads (chat_live.rs,
            // `asked_questions`); Codex's ids kept beside.
            let questions: Vec<Value> = params
                .questions
                .iter()
                .map(|question| {
                    json!({
                        "id": question.id,
                        "question": question.question,
                        "header": question.header,
                        "multiSelect": false,
                        "options": question.options.as_deref().unwrap_or_default().iter().map(|option| {
                            json!({ "label": option.label, "description": option.description })
                        }).collect::<Vec<_>>(),
                        "isOther": question.is_other.unwrap_or(false),
                        "isSecret": question.is_secret.unwrap_or(false),
                    })
                })
                .collect();
            let (thread, turn, item, asked) = (
                params.thread_id.clone(),
                params.turn_id.clone(),
                params.item_id.clone(),
                params.questions.clone(),
            );
            Asked {
                thread,
                turn: Some(turn),
                tool: QUESTION_TOOL.into(),
                input: clipped(&json!({ "questions": questions }), 0),
                tool_use_id: Some(item),
                question: true,
                answerer: Answerer::Question {
                    pending,
                    questions: asked,
                },
            }
        }
        Incoming::McpElicitation(pending) => {
            // Read through its JSON: its modes share these fields.
            let params = serde_json::to_value(pending.params()).ok()?;
            let text = |key: &str| params.get(key).and_then(Value::as_str).map(str::to_string);
            let thread = text("threadId")?;
            let turn = text("turnId");
            let server = text("serverName").unwrap_or_default();
            let meta = params.get("_meta").cloned().unwrap_or(Value::Null);
            // From the source (protocol/src/mcp_approval_meta.rs): an MCP
            // tool's call to approve is a form whose `_meta` names it.
            let tool_call =
                meta.get("codex_approval_kind").and_then(Value::as_str) == Some("mcp_tool_call");
            let form = text("mode").as_deref() != Some("url");
            let (tool, input) = if tool_call {
                let name = meta
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let input = meta
                    .get("tool_params")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                (format!("mcp__{server}__{name}"), input)
            } else {
                let mut input = Map::new();
                for key in ["message", "url", "requestedSchema"] {
                    if let Some(value) = params.get(key) {
                        input.insert(key.into(), value.clone());
                    }
                }
                (format!("mcp__{server}__elicitation"), Value::Object(input))
            };
            Asked {
                thread,
                turn,
                tool,
                input: clipped(&input, 0),
                tool_use_id: None,
                question: false,
                answerer: Answerer::Elicitation {
                    pending,
                    form,
                    tool_call,
                },
            }
        }
    })
}

/// Waits for the person's answer to ask `ask` and gives it to Codex; ends
/// with the ask when it ends otherwise (the TUI answered, the turn ended, the
/// pane went), leaving the request unanswered by Hangar.
async fn wait_for_answer(ctx: Ctx, client: Arc<Client>, ask: String, asked: Asked) {
    let wait = Duration::from_millis(bridge::ASK_WAIT_MAX_MS);
    let decision = loop {
        match bridge::await_decision(&ctx.pane, &ask, wait).await {
            AskPoll::Decision(decision) => break decision,
            AskPoll::Pending => {}
            AskPoll::Gone => return,
        }
    };
    let interrupt = match answer(asked.answerer, &decision) {
        Ok(interrupt) => interrupt,
        Err(why) => {
            ctx.say(&format!("could not give an answer to Codex: {why}"));
            return;
        }
    };
    // The person said no and to stop, where Codex's answer cannot say both.
    if let (true, Some(turn)) = (interrupt, asked.turn) {
        let params = TurnInterruptParams {
            thread_id: asked.thread,
            turn_id: turn,
        };
        if let Err(error) = client.request::<requests::TurnInterrupt>(params).await {
            ctx.say(&format!("could not stop the turn: {error}"));
        }
    }
}

/// What the person chose, from Hangar's decision (bridge.rs, `answer_ask`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Choice {
    Allow { session: bool },
    Deny { interrupt: bool },
}

fn choice(decision: &Value) -> Option<Choice> {
    let flag = |key: &str| decision.get(key) == Some(&Value::Bool(true));
    match decision.get("behavior")?.as_str()? {
        "allow" => Some(Choice::Allow {
            session: flag("forSession"),
        }),
        "deny" => Some(Choice::Deny {
            interrupt: flag("interrupt"),
        }),
        _ => None,
    }
}

/// Gives Codex the person's answer, as the request's method takes it: true
/// when the turn is to be stopped as well. An answer that does not fit the
/// request is not sent: the TUI's dialog stays, as the mod leaves Claude
/// Code's. A "no" carries no reason to Codex: its answers have no room for
/// one.
fn answer(answerer: Answerer, decision: &Value) -> Result<bool, String> {
    let unfit = || "an answer that does not fit the question".to_string();
    let sent = match answerer {
        Answerer::Command { pending, human } => {
            let decision = command_decision(choice(decision).ok_or_else(unfit)?, human);
            pending.respond(CommandExecutionRequestApprovalResponse { decision })
        }
        Answerer::FileChange(pending) => {
            let decision = file_decision(choice(decision).ok_or_else(unfit)?);
            pending.respond(FileChangeRequestApprovalResponse { decision })
        }
        Answerer::Permissions { pending, requested } => {
            let chosen = choice(decision).ok_or_else(unfit)?;
            let response = permissions_response(chosen, &requested);
            return pending
                .respond(response)
                .map(|()| chosen == Choice::Deny { interrupt: true })
                .map_err(|error| error.to_string());
        }
        Answerer::Question { pending, questions } => {
            let (response, interrupt) =
                question_response(decision, &questions).ok_or_else(unfit)?;
            return pending
                .respond(response)
                .map(|()| interrupt)
                .map_err(|error| error.to_string());
        }
        Answerer::Elicitation {
            pending,
            form,
            tool_call,
        } => {
            let response = elicitation_response(decision, form, tool_call).ok_or_else(unfit)?;
            pending.respond(response)
        }
    };
    sent.map(|()| false).map_err(|error| error.to_string())
}

/// Allow, allow for the session, decline (the turn goes on), cancel (the
/// turn stops), as the schema's `CommandExecutionApprovalDecision` says each.
fn command_decision(choice: Choice, human: bool) -> CommandExecutionApprovalDecision {
    match choice {
        Choice::Allow { session: true } if !human => {
            CommandExecutionApprovalDecision::AcceptForSession
        }
        Choice::Allow { .. } => CommandExecutionApprovalDecision::Accept,
        Choice::Deny { interrupt: true } => CommandExecutionApprovalDecision::Cancel,
        Choice::Deny { .. } => CommandExecutionApprovalDecision::Decline,
    }
}

fn file_decision(choice: Choice) -> FileChangeApprovalDecision {
    match choice {
        Choice::Allow { session: true } => FileChangeApprovalDecision::AcceptForSession,
        Choice::Allow { .. } => FileChangeApprovalDecision::Accept,
        Choice::Deny { interrupt: true } => FileChangeApprovalDecision::Cancel,
        Choice::Deny { .. } => FileChangeApprovalDecision::Decline,
    }
}

/// What was asked, granted for the turn or the session; a no grants nothing,
/// which is how Codex itself declines (bespoke_event_handling.rs,
/// `request_permissions_response_from_client_result`).
fn permissions_response(
    choice: Choice,
    requested: &RequestPermissionProfile,
) -> PermissionsRequestApprovalResponse {
    match choice {
        Choice::Allow { session } => PermissionsRequestApprovalResponse {
            permissions: GrantedPermissionProfile {
                file_system: requested.file_system.clone(),
                network: requested.network.clone(),
            },
            scope: Some(if session {
                PermissionGrantScope::Session
            } else {
                PermissionGrantScope::Turn
            }),
            strict_auto_review: None,
        },
        Choice::Deny { .. } => PermissionsRequestApprovalResponse {
            permissions: GrantedPermissionProfile::default(),
            scope: Some(PermissionGrantScope::Turn),
            strict_auto_review: None,
        },
    }
}

/// The answers by question id. Hangar's decision gives them by question
/// text (held.rs, `question_decision`) or by id, each as the labels picked
/// then the person's own words joined with ", ", or as a list. A no answers
/// nothing — what Codex itself sends when a client fails one
/// (bespoke_event_handling.rs, `on_request_user_input_response`).
fn question_response(
    decision: &Value,
    questions: &[ToolRequestUserInputQuestion],
) -> Option<(ToolRequestUserInputResponse, bool)> {
    match choice(decision)? {
        Choice::Deny { interrupt } => Some((ToolRequestUserInputResponse::default(), interrupt)),
        Choice::Allow { .. } => {
            let given = decision.get("answers")?.as_object()?;
            let mut answers = BTreeMap::new();
            for question in questions {
                let said = given
                    .get(&question.id)
                    .or_else(|| given.get(&question.question))
                    .or_else(|| {
                        given
                            .get(&question.header)
                            .filter(|_| !question.header.is_empty())
                    });
                let Some(said) = said else {
                    continue;
                };
                let options = question.options.as_deref().unwrap_or_default();
                let list = match said {
                    Value::String(text) => split_answer(text, options),
                    Value::Array(list) => list
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(str::to_string)
                        .collect(),
                    _ => Vec::new(),
                };
                if !list.is_empty() {
                    answers.insert(
                        question.id.clone(),
                        ToolRequestUserInputAnswer { answers: list },
                    );
                }
            }
            (!answers.is_empty()).then_some((ToolRequestUserInputResponse { answers }, false))
        }
    }
}

/// "Red, Blue, my own words" → the labels, then the words: the labels picked
/// come first in what Hangar's dialog gives (held.rs, `question_decision`).
fn split_answer(text: &str, options: &[ToolRequestUserInputOption]) -> Vec<String> {
    let mut said = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let label = options
            .iter()
            .map(|option| option.label.as_str())
            .filter(|label| !label.is_empty())
            .filter(|label| {
                rest == *label
                    || rest
                        .strip_prefix(label)
                        .is_some_and(|after| after.starts_with(", "))
            })
            .max_by_key(|label| label.len());
        let Some(label) = label else {
            break;
        };
        said.push(label.to_string());
        rest = &rest[label.len()..];
        rest = rest.strip_prefix(", ").unwrap_or(rest);
    }
    if !rest.is_empty() {
        said.push(rest.to_string());
    }
    said
}

/// Accept (with what the form asks, when Hangar's answer carries it as
/// `content`), decline, or cancel. An MCP tool's call allowed for the session
/// says so in `_meta` — Assumption (from source, not run):
/// protocol/src/mcp_approval_meta.rs names `persist: "session"`.
fn elicitation_response(
    decision: &Value,
    form: bool,
    tool_call: bool,
) -> Option<McpServerElicitationRequestResponse> {
    Some(match choice(decision)? {
        Choice::Allow { session } => McpServerElicitationRequestResponse {
            meta: (session && tool_call).then(|| json!({ "persist": "session" })),
            action: McpServerElicitationAction::Accept,
            content: decision
                .get("content")
                .filter(|content| content.is_object())
                .cloned()
                .or_else(|| form.then(|| json!({}))),
        },
        Choice::Deny { interrupt } => McpServerElicitationRequestResponse {
            meta: None,
            action: if interrupt {
                McpServerElicitationAction::Cancel
            } else {
                McpServerElicitationAction::Decline
            },
            content: None,
        },
    })
}

#[cfg(test)]
mod tests {
    //! Hangar as the second client of a pane's Codex server, against a fake server in the test
    //! process. No Codex session was ever recorded (the person's decision, docs/native/agents.md
    //! §7): every exchange below is written by hand from the protocol schema of the pinned
    //! release (codex_app/generated.rs, Codex 0.170.0) and, where the schema says nothing — who
    //! receives what, in which order — from OpenAI's source at rust-v0.170.0, named at the test.
    //! The TUI that `codex --remote` runs is the server's first client throughout: its name is
    //! the originator the user agent starts with, Hangar's the client it ends with.

    use std::sync::atomic::AtomicUsize;

    use tokio::io::{
        AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
    };
    use tokio::sync::broadcast;

    use super::*;
    use crate::codex_app::Framing;
    use crate::host::Event as HostEvent;

    const PATIENCE: Duration = Duration::from_secs(10);
    const THREAD: &str = "019a7c2e-5b1d-7f43-9a61-3c8e2f4d6b10";
    const SECOND: &str = "019a7c31-0c4e-7a12-8d55-6f1b2e9c4a77";
    const THIRD: &str = "019a7c35-77d0-7e81-b3c2-0a4f6d8e1b29";
    /// From the source (initialize_processor.rs): the first client's name is the originator for
    /// good — the TUI's — and the user agent's last part names the latest client, Hangar.
    const USER_AGENT: &str =
        "codex-tui/0.170.0 (Ubuntu 24.4.0; x86_64) xterm-256color (Hangar; test)";

    /// The server's side of one connection, one JSON message per line.
    struct Fake {
        lines: Lines<BufReader<ReadHalf<DuplexStream>>>,
        write: WriteHalf<DuplexStream>,
    }

    impl Fake {
        async fn recv(&mut self) -> Value {
            let line = tokio::time::timeout(PATIENCE, self.lines.next_line())
                .await
                .expect("Hangar says something in time")
                .expect("readable")
                .expect("the connection is open");
            serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"))
        }

        async fn expect(&mut self, method: &str) -> Value {
            let message = self.recv().await;
            assert_eq!(message["method"], method, "{message}");
            message
        }

        async fn send(&mut self, message: Value) {
            let mut line = message.to_string();
            line.push('\n');
            self.write.write_all(line.as_bytes()).await.unwrap();
        }

        async fn reply(&mut self, request: &Value, result: Value) {
            self.send(json!({ "id": request["id"], "result": result }))
                .await;
        }

        async fn refuse(&mut self, request: &Value, error: Value) {
            self.send(json!({ "id": request["id"], "error": error }))
                .await;
        }

        async fn notify(&mut self, method: &str, params: Value) {
            self.send(json!({ "method": method, "params": params }))
                .await;
        }

        async fn ask(&mut self, id: i64, method: &str, params: Value) {
            self.send(json!({ "id": id, "method": method, "params": params }))
                .await;
        }

        /// Hangar says nothing for a moment.
        async fn quiet(&mut self) {
            if let Ok(line) =
                tokio::time::timeout(Duration::from_millis(300), self.lines.next_line()).await
            {
                panic!("Hangar said {line:?}");
            }
        }
    }

    /// A connection past its handshake, as the TUI's second client makes it.
    async fn connected() -> (Client, Events, Fake) {
        let (hangar, codex) = tokio::io::duplex(1 << 20);
        let (read, write) = tokio::io::split(hangar);
        let (codex_read, codex_write) = tokio::io::split(codex);
        let mut fake = Fake {
            lines: BufReader::new(codex_read).lines(),
            write: codex_write,
        };
        let handshake = async {
            let init = fake.expect("initialize").await;
            assert_eq!(
                init["params"]["clientInfo"]["name"], "Hangar",
                "never another client's name"
            );
            let opted = &init["params"]["capabilities"]["optOutNotificationMethods"];
            assert!(opted
                .as_array()
                .is_some_and(|methods| methods.contains(&json!("item/agentMessage/delta"))));
            fake.reply(
                &init,
                json!({
                    "userAgent": USER_AGENT,
                    "codexHome": "/home/ada/.codex",
                    "platformFamily": "unix",
                    "platformOs": "linux",
                }),
            )
            .await;
            fake.expect("initialized").await;
        };
        let (connected, ()) = tokio::join!(
            Client::connect_over(read, write, Framing::Lines, options()),
            handshake
        );
        let (client, events) = connected.expect("connected");
        (client, events, fake)
    }

    fn fast() -> Pace {
        Pace {
            tui_poll: Duration::from_millis(10),
            tui_patience: Duration::from_secs(1),
            tui_poll_slow: Duration::from_millis(10),
            tui_grace: Duration::from_millis(50),
            presence_poll: Duration::from_millis(20),
            reconnects: vec![Duration::from_millis(10), Duration::from_millis(10)],
        }
    }

    fn test_ctx(
        pane: &str,
        briefing: Option<&'static str>,
    ) -> (Ctx, broadcast::Receiver<HostEvent>) {
        let host = Host::with_paths(crate::paths::Paths::rooted(&std::env::temp_dir()));
        let events = host.subscribe();
        let ctx = Ctx {
            host,
            pane: pane.to_string(),
            name: "Cody".into(),
            briefing: Arc::new(move |_, _| boxed(async move { briefing.map(str::to_string) })),
            presence: Arc::new(|| None),
            pace: fast(),
        };
        (ctx, events)
    }

    struct Following {
        stop: watch::Sender<bool>,
        task: JoinHandle<Ended>,
    }

    fn following(ctx: &Ctx, client: Client, events: Events) -> Following {
        let (stop, mut stopped) = watch::channel(false);
        let ctx = ctx.clone();
        let task = tokio::spawn(async move {
            let mut briefed = HashSet::new();
            follow(&ctx, Arc::new(client), events, &mut stopped, &mut briefed).await
        });
        Following { stop, task }
    }

    impl Following {
        async fn stop(self) -> Ended {
            let _ = self.stop.send(true);
            tokio::time::timeout(PATIENCE, self.task)
                .await
                .expect("it ends at once")
                .unwrap()
        }
    }

    /// The next report of pane `pane` to the window.
    async fn report(events: &mut broadcast::Receiver<HostEvent>, pane: &str) -> Value {
        loop {
            let event = tokio::time::timeout(PATIENCE, events.recv())
                .await
                .expect("a report in time")
                .expect("the bus");
            if let HostEvent::BridgeEvent { pane_id, event } = event {
                if pane_id == pane {
                    return event;
                }
            }
        }
    }

    /// The next report of kind `kind`, past the others.
    async fn report_of(
        events: &mut broadcast::Receiver<HostEvent>,
        pane: &str,
        kind: &str,
    ) -> Value {
        loop {
            let event = report(events, pane).await;
            if event["kind"] == kind {
                return event;
            }
        }
    }

    /// A `Thread` as the schema has it.
    fn thread(id: &str, status: Value) -> Value {
        json!({
            "id": id,
            "sessionId": id,
            "cliVersion": "0.170.0",
            "createdAt": 1795050000,
            "updatedAt": 1795050000,
            "cwd": "/work/hangar",
            "ephemeral": false,
            "modelProvider": "openai",
            "preview": "",
            "projectId": null,
            "source": "cli",
            "status": status,
            "turns": [],
        })
    }

    /// `thread/resume`'s answer: with `excludeTurns`, the thread without its turns.
    fn resumed(id: &str, status: Value) -> Value {
        json!({
            "approvalPolicy": "on-request",
            "approvalsReviewer": "user",
            "cwd": "/work/hangar",
            "model": "gpt-5.5-codex",
            "modelProvider": "openai",
            "reasoningEffort": "high",
            "sandbox": { "type": "workspaceWrite", "writableRoots": ["/work/hangar"], "networkAccess": false },
            "thread": thread(id, status),
        })
    }

    fn idle() -> Value {
        json!({ "type": "idle" })
    }

    /// Thread `id`, loaded when Hangar connects: listed, read, rejoined.
    async fn join(fake: &mut Fake, id: &str) {
        let list = fake.expect("thread/loaded/list").await;
        fake.reply(&list, json!({ "data": [id] })).await;
        take_up(fake, id, idle()).await;
    }

    /// Thread `id` read, then rejoined with no override (thread_processor.rs,
    /// `resume_running_thread`: a running thread is rejoined as it is).
    async fn take_up(fake: &mut Fake, id: &str, status: Value) {
        let read = fake.expect("thread/read").await;
        assert_eq!(
            read["params"],
            json!({ "threadId": id, "includeTurns": false })
        );
        fake.reply(&read, json!({ "thread": thread(id, status.clone()) }))
            .await;
        resume(fake, id, status).await;
    }

    async fn resume(fake: &mut Fake, id: &str, status: Value) {
        let resume = fake.expect("thread/resume").await;
        assert_eq!(
            resume["params"],
            json!({ "threadId": id, "excludeTurns": true })
        );
        fake.reply(&resume, resumed(id, status)).await;
    }

    fn turn(id: &str, status: &str) -> Value {
        json!({ "id": id, "items": [], "status": status })
    }

    async fn hello(events: &mut broadcast::Receiver<HostEvent>, pane: &str, id: &str) {
        assert_eq!(
            report(events, pane).await,
            json!({ "kind": "hello", "sessionId": id, "model": "gpt-5.5-codex", "version": "0.170.0" })
        );
        assert_eq!(
            report(events, pane).await,
            json!({ "kind": "settings", "model": "gpt-5.5-codex", "mode": "default", "effort": "high" })
        );
    }

    // ----- following the pane's threads

    #[tokio::test]
    async fn hangar_joins_the_tuis_thread_and_says_hello() {
        let pane = "p-attach-hello";
        let (ctx, mut events) = test_ctx(pane, None);
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;
        fake.quiet().await;
        assert!(matches!(run.stop().await, Ended::Stopped));
    }

    // From the docs (app-server: a turn starts, its items complete, the turn completes) and the
    // schema: `turn/completed` carries no items, so the answer is the turn's last agent message.
    #[tokio::test]
    async fn a_turn_reads_as_working_then_yours_with_its_final_answer() {
        let pane = "p-attach-turn";
        let (ctx, mut events) = test_ctx(pane, None);
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;

        fake.notify(
            "turn/started",
            json!({ "threadId": THREAD, "turn": turn("turn-1", "inProgress") }),
        )
        .await;
        assert_eq!(
            report(&mut events, pane).await,
            json!({ "kind": "turn", "phase": "start" })
        );
        for (id, text, phase) in [
            ("item-1", "Looking at the tests first.", "commentary"),
            (
                "item-2",
                "All 42 tests pass.\n\nDetails below.",
                "final_answer",
            ),
            ("item-3", "One more remark.", "commentary"),
        ] {
            fake.notify(
                "item/completed",
                json!({
                    "threadId": THREAD, "turnId": "turn-1", "completedAtMs": 1795050004200i64,
                    "item": { "type": "agentMessage", "id": id, "text": text, "phase": phase },
                }),
            )
            .await;
        }
        fake.notify(
            "turn/completed",
            json!({ "threadId": THREAD, "turn": {
                "id": "turn-1", "items": [], "status": "completed", "durationMs": 3150,
            }}),
        )
        .await;
        assert_eq!(
            report(&mut events, pane).await,
            json!({ "kind": "turn", "phase": "end", "reason": "answer", "durationMs": 3150, "answer": "All 42 tests pass." })
        );

        fake.notify(
            "turn/started",
            json!({ "threadId": THREAD, "turn": turn("turn-2", "inProgress") }),
        )
        .await;
        report_of(&mut events, pane, "turn").await;
        fake.notify(
            "turn/completed",
            json!({ "threadId": THREAD, "turn": turn("turn-2", "interrupted") }),
        )
        .await;
        let end = report(&mut events, pane).await;
        assert_eq!(
            (&end["reason"], &end["answer"]),
            (&json!("aborted"), &json!(""))
        );

        fake.notify(
            "turn/started",
            json!({ "threadId": THREAD, "turn": turn("turn-3", "inProgress") }),
        )
        .await;
        report_of(&mut events, pane, "turn").await;
        fake.notify(
            "turn/completed",
            json!({ "threadId": THREAD, "turn": {
                "id": "turn-3", "items": [], "status": "failed",
                "error": { "message": "You've hit your usage limit.", "codexErrorInfo": "usageLimitExceeded" },
            }}),
        )
        .await;
        let end = report(&mut events, pane).await;
        assert_eq!(
            (&end["reason"], &end["answer"]),
            (&json!("error"), &json!("You've hit your usage limit."))
        );
        run.stop().await;
    }

    #[tokio::test]
    async fn usage_reports_the_context_and_the_plans_windows() {
        let pane = "p-attach-usage";
        let (ctx, mut events) = test_ctx(pane, None);
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;
        // Limits before any token count wait for one: a gauge without its context is cleared.
        fake.notify(
            "account/rateLimits/updated",
            json!({ "rateLimits": {
                "limitId": "codex",
                "primary": { "usedPercent": 34, "windowDurationMins": 300, "resetsAt": 1795060800 },
            }}),
        )
        .await;
        fake.notify("thread/tokenUsage/updated", json!({
            "threadId": THREAD, "turnId": "turn-1",
            "tokenUsage": {
                "total": { "inputTokens": 9000, "cachedInputTokens": 3072, "outputTokens": 251, "reasoningOutputTokens": 12, "totalTokens": 9251 },
                "last": { "inputTokens": 5210, "cachedInputTokens": 3072, "outputTokens": 41, "reasoningOutputTokens": 12, "totalTokens": 5251 },
                "modelContextWindow": 272000,
            },
        }))
        .await;
        assert_eq!(
            report(&mut events, pane).await,
            json!({
                "kind": "usage",
                "context": { "tokens": 5251, "window": 272000, "percent": null },
                "rateLimits": [{ "kind": "five_hour", "percentUsed": 34.0, "resetsAt": "2026-11-19T04:00:00Z" }],
                "costUsd": null,
            })
        );
        // A sparse update keeps what it does not repeat; a model's own bucket is not the plan's.
        fake.notify(
            "account/rateLimits/updated",
            json!({ "rateLimits": { "limitId": "codex", "secondary": { "usedPercent": 12, "windowDurationMins": 10080 } } }),
        )
        .await;
        let usage = report(&mut events, pane).await;
        assert_eq!(
            usage["rateLimits"],
            json!([
                { "kind": "five_hour", "percentUsed": 34.0, "resetsAt": "2026-11-19T04:00:00Z" },
                { "kind": "seven_day", "percentUsed": 12.0, "resetsAt": null },
            ])
        );
        fake.notify(
            "account/rateLimits/updated",
            json!({ "rateLimits": { "limitId": "gpt-5.5-codex-mini", "primary": { "usedPercent": 90, "windowDurationMins": 300 } } }),
        )
        .await;
        let usage = report(&mut events, pane).await;
        assert_eq!(usage["rateLimits"][0]["percentUsed"], json!(34.0));
        run.stop().await;
    }

    // From the source: `thread/started` goes to every client (thread_processor.rs, the
    // process-wide sender), a thread the TUI resumes says so only by its status
    // (thread_status.rs), and a subagent's thread names its parent (the schema's `Thread`).
    #[tokio::test]
    async fn a_new_thread_and_a_resumed_one_become_the_session_a_subagents_does_not() {
        let pane = "p-attach-threads";
        let (ctx, mut events) = test_ctx(pane, None);
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;

        // The TUI's `/new`.
        fake.notify(
            "thread/started",
            json!({ "thread": thread(SECOND, idle()) }),
        )
        .await;
        resume(&mut fake, SECOND, idle()).await;
        assert_eq!(
            report(&mut events, pane).await,
            json!({ "kind": "session", "sessionId": SECOND, "source": "startup" })
        );
        report_of(&mut events, pane, "settings").await;

        // A subagent's thread: neither read nor joined.
        let mut child = thread("019a7c40-1111-7222-8333-944455556666", idle());
        child["parentThreadId"] = json!(SECOND);
        fake.notify("thread/started", json!({ "thread": child }))
            .await;
        fake.quiet().await;

        // The TUI's `/resume` of an older thread.
        fake.notify(
            "thread/status/changed",
            json!({ "threadId": THIRD, "status": idle() }),
        )
        .await;
        take_up(&mut fake, THIRD, idle()).await;
        assert_eq!(
            report(&mut events, pane).await,
            json!({ "kind": "session", "sessionId": THIRD, "source": "resume" })
        );
        report_of(&mut events, pane, "settings").await;

        // A turn in a thread Hangar follows: the TUI works there now.
        fake.notify(
            "turn/started",
            json!({ "threadId": THREAD, "turn": turn("turn-7", "inProgress") }),
        )
        .await;
        assert_eq!(
            report(&mut events, pane).await,
            json!({ "kind": "session", "sessionId": THREAD, "source": "resume" })
        );
        report_of(&mut events, pane, "settings").await;
        assert_eq!(
            report(&mut events, pane).await,
            json!({ "kind": "turn", "phase": "start" })
        );
        run.stop().await;
    }

    // ----- the briefing

    // From the source: `thread/inject_items` takes Responses API items (turn_processor.rs,
    // `thread_inject_items_response_inner`), and a compaction keeps only the thread's initial
    // context and user messages (core/src/compact.rs), which it reports as a `contextCompaction`
    // item and, deprecated, as `thread/compacted`.
    #[tokio::test]
    async fn the_briefing_is_handed_on_join_and_again_after_a_compaction() {
        let pane = "p-attach-briefing";
        let (ctx, mut events) = test_ctx(pane, Some("Board: 3 tasks in doing."));
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        let briefing = json!({
            "threadId": THREAD,
            "items": [{
                "type": "message",
                "role": "developer",
                "content": [{ "type": "input_text", "text": "[Hangar] Briefing — not typed by the person.\n\nBoard: 3 tasks in doing." }],
            }],
        });
        let inject = fake.expect("thread/inject_items").await;
        assert_eq!(inject["params"], briefing);
        fake.reply(&inject, json!({})).await;
        hello(&mut events, pane, THREAD).await;

        fake.notify(
            "item/completed",
            json!({
                "threadId": THREAD, "turnId": "turn-4", "completedAtMs": 1795050090000i64,
                "item": { "type": "contextCompaction", "id": "item-30" },
            }),
        )
        .await;
        let inject = fake.expect("thread/inject_items").await;
        assert_eq!(inject["params"], briefing);
        fake.reply(&inject, json!({})).await;
        // The same compaction, said again the old way: once is enough.
        fake.notify(
            "thread/compacted",
            json!({ "threadId": THREAD, "turnId": "turn-4" }),
        )
        .await;
        fake.quiet().await;
        run.stop().await;
    }

    // ----- asks held for the person

    // From the source (bespoke_event_handling.rs): a thread's status says it waits before its
    // request goes out, and stops saying so once the request is resolved, which every client
    // of the thread hears (outgoing_message.rs, `resolve_server_request`).
    #[tokio::test]
    async fn an_approval_is_held_and_answered_from_hangar() {
        let pane = "p-attach-approval";
        let (ctx, mut events) = test_ctx(pane, None);
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;
        fake.notify(
            "turn/started",
            json!({ "threadId": THREAD, "turn": turn("turn-1", "inProgress") }),
        )
        .await;
        report_of(&mut events, pane, "turn").await;

        fake.notify(
            "thread/status/changed",
            json!({ "threadId": THREAD, "status": { "type": "active", "activeFlags": ["waitingOnApproval"] } }),
        )
        .await;
        assert_eq!(
            report(&mut events, pane).await,
            json!({ "kind": "permission", "waiting": true })
        );
        fake.ask(
            7,
            "item/commandExecution/requestApproval",
            json!({
                "threadId": THREAD, "turnId": "turn-1", "itemId": "item-3",
                "startedAtMs": 1795050050000i64, "command": "cargo test", "cwd": "/work/hangar",
            }),
        )
        .await;
        let asked = report(&mut events, pane).await;
        let ask = asked["askId"].as_str().expect("an ask id").to_string();
        assert!(ask.starts_with("codex-") && ask.ends_with("-7"), "{ask}");
        assert_eq!(
            asked,
            json!({
                "kind": "permission", "waiting": true, "tool": "Bash",
                "input": { "command": "cargo test", "cwd": "/work/hangar" },
                "askId": ask, "toolUseId": "item-3",
            })
        );
        let held = bridge::held_asks(pane);
        assert_eq!(held.len(), 1);
        assert_eq!(
            (held[0].tool.as_str(), held[0].tool_use_id.as_deref()),
            ("Bash", Some("item-3"))
        );

        // What the person chose in Hangar's chat or on the phone.
        assert!(bridge::answer_ask(
            pane,
            &ask,
            json!({ "behavior": "allow", "forSession": true })
        ));
        let answered = fake.recv().await;
        assert_eq!(
            answered,
            json!({ "id": 7, "result": { "decision": "acceptForSession" } })
        );
        fake.notify(
            "serverRequest/resolved",
            json!({ "threadId": THREAD, "requestId": 7 }),
        )
        .await;
        assert_eq!(
            report(&mut events, pane).await,
            json!({ "kind": "ask", "phase": "end", "askId": ask })
        );
        fake.notify(
            "thread/status/changed",
            json!({ "threadId": THREAD, "status": { "type": "active", "activeFlags": [] } }),
        )
        .await;
        assert_eq!(
            report(&mut events, pane).await,
            json!({ "kind": "permission", "waiting": false })
        );
        assert!(bridge::held_asks(pane).is_empty());
        run.stop().await;
    }

    // The second-client case. From the source (outgoing_message.rs): a thread's requests go to
    // each of its clients and the first answer resolves them — an error included, so Hangar
    // leaves what it does not handle (codex_app's `Unhandled::Leave`) — then
    // `serverRequest/resolved` goes to all.
    #[tokio::test]
    async fn what_the_tui_answers_first_ends_the_ask_here_unanswered_by_hangar() {
        let pane = "p-attach-tui-first";
        let (ctx, mut events) = test_ctx(pane, None);
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;
        fake.notify("item/started", json!({
            "threadId": THREAD, "turnId": "turn-2", "startedAtMs": 1795050060000i64,
            "item": { "type": "fileChange", "id": "item-5", "status": "inProgress", "changes": [
                { "path": "src/main.rs", "kind": { "type": "update" }, "diff": "@@ -1 +1 @@\n-a\n+b\n" },
            ]},
        }))
        .await;
        report_of(&mut events, pane, "tool").await;
        fake.ask(
            12,
            "item/tool/call",
            json!({ "threadId": THREAD, "turnId": "turn-2", "callId": "call-7", "tool": "browser_open", "arguments": {} }),
        )
        .await;
        fake.ask(
            13,
            "item/fileChange/requestApproval",
            json!({
                "threadId": THREAD, "turnId": "turn-2", "itemId": "item-5",
                "startedAtMs": 1795050061000i64, "reason": "needs write access",
            }),
        )
        .await;
        let asked = report_of(&mut events, pane, "permission").await;
        assert_eq!(asked["tool"], "apply_patch");
        assert_eq!(
            asked["input"]["changes"][0],
            json!({ "path": "src/main.rs", "kind": "update", "diff": "@@ -1 +1 @@\n-a\n+b\n" })
        );
        let ask = asked["askId"].as_str().unwrap().to_string();

        // The person answered in the TUI.
        fake.notify(
            "serverRequest/resolved",
            json!({ "threadId": THREAD, "requestId": 13 }),
        )
        .await;
        assert_eq!(
            report(&mut events, pane).await,
            json!({ "kind": "ask", "phase": "end", "askId": ask })
        );
        assert_eq!(
            report(&mut events, pane).await,
            json!({ "kind": "permission", "waiting": false })
        );
        assert!(bridge::held_asks(pane).is_empty());
        assert!(!bridge::answer_ask(
            pane,
            &ask,
            json!({ "behavior": "allow" })
        ));
        // Neither request gets an answer of Hangar's.
        fake.quiet().await;
        run.stop().await;
    }

    #[tokio::test]
    async fn a_question_is_answered_by_question_id() {
        let pane = "p-attach-question";
        let (ctx, mut events) = test_ctx(pane, None);
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;
        fake.ask(21, "item/tool/requestUserInput", json!({
            "threadId": THREAD, "turnId": "turn-3", "itemId": "call-4", "isBlocking": true,
            "questions": [
                { "id": "color", "header": "Color", "question": "Which color?",
                  "options": [{ "label": "Red", "description": "warm" }, { "label": "Blue", "description": "cold" }] },
                { "id": "sizes", "header": "Sizes", "question": "Which sizes?", "isOther": true,
                  "options": [{ "label": "S", "description": "" }, { "label": "M", "description": "" }, { "label": "L", "description": "" }] },
            ],
        }))
        .await;
        // As the window reads AskUserQuestion: a running call, then its dialog.
        let call = report(&mut events, pane).await;
        assert_eq!(
            (
                &call["kind"],
                &call["phase"],
                &call["tool"],
                &call["toolUseId"]
            ),
            (
                &json!("tool"),
                &json!("start"),
                &json!("AskUserQuestion"),
                &json!("call-4")
            )
        );
        assert_eq!(call["input"]["questions"][0]["question"], "Which color?");
        assert_eq!(call["input"]["questions"][1]["options"][2]["label"], "L");
        let asked = report(&mut events, pane).await;
        assert_eq!(asked["tool"], "AskUserQuestion");
        let ask = asked["askId"].as_str().unwrap().to_string();

        // The window's answer (held.rs, `question_decision`): by question text, the labels
        // picked then the person's own words.
        assert!(bridge::answer_ask(
            pane,
            &ask,
            json!({ "behavior": "allow", "answers": { "Which color?": "Blue", "Which sizes?": "S, L, XXL too" } })
        ));
        assert_eq!(
            fake.recv().await,
            json!({ "id": 21, "result": { "answers": {
                "color": { "answers": ["Blue"] },
                "sizes": { "answers": ["S", "L", "XXL too"] },
            }}})
        );
        fake.notify(
            "serverRequest/resolved",
            json!({ "threadId": THREAD, "requestId": 21 }),
        )
        .await;
        assert_eq!(report(&mut events, pane).await["kind"], "ask");
        assert_eq!(
            report(&mut events, pane).await,
            json!({ "kind": "tool", "phase": "end", "toolUseId": "call-4" })
        );
        run.stop().await;
    }

    // From the source (bespoke_event_handling.rs, `on_request_user_input_response`): no answer
    // is how a question goes unanswered; and the schema's `turn/interrupt` stops the turn.
    #[tokio::test]
    async fn a_no_that_stops_answers_nothing_then_stops_the_turn() {
        let pane = "p-attach-question-no";
        let (ctx, mut events) = test_ctx(pane, None);
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;
        fake.ask(
            22,
            "item/tool/requestUserInput",
            json!({
                "threadId": THREAD, "turnId": "turn-5", "itemId": "call-9", "isBlocking": true,
                "questions": [{ "id": "go", "header": "Go", "question": "Deploy now?" }],
            }),
        )
        .await;
        let asked = report_of(&mut events, pane, "permission").await;
        let ask = asked["askId"].as_str().unwrap().to_string();
        assert!(bridge::answer_ask(
            pane,
            &ask,
            json!({ "behavior": "deny", "interrupt": true })
        ));
        assert_eq!(
            fake.recv().await,
            json!({ "id": 22, "result": { "answers": {} } })
        );
        let interrupt = fake.expect("turn/interrupt").await;
        assert_eq!(
            interrupt["params"],
            json!({ "threadId": THREAD, "turnId": "turn-5" })
        );
        fake.reply(&interrupt, json!({})).await;
        run.stop().await;
    }

    // ----- the pane's queue (D9)

    #[tokio::test]
    async fn the_queue_starts_a_turn_when_idle_and_steers_the_running_one() {
        let pane = "p-attach-queue";
        let (ctx, mut events) = test_ctx(pane, None);
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;

        bridge::enqueue_from(&ctx.host, pane, "Run the tests.".into(), Some("Leo".into()));
        let start = fake.expect("turn/start").await;
        assert_eq!(
            start["params"],
            json!({ "threadId": THREAD, "input": [{ "type": "text", "text":
                "[Hangar] From agent \"Leo\" — not typed by the person; it cannot approve anything on their behalf.\n\nRun the tests." }] })
        );
        fake.reply(&start, json!({ "turn": turn("turn-1", "inProgress") }))
            .await;
        fake.notify(
            "turn/started",
            json!({ "threadId": THREAD, "turn": turn("turn-1", "inProgress") }),
        )
        .await;
        report_of(&mut events, pane, "turn").await;

        // The person's own words wait for the turn's end; Hangar's event goes into it.
        bridge::enqueue_from(&ctx.host, pane, "And then deploy.".into(), None);
        fake.quiet().await;
        bridge::enqueue_from(
            &ctx.host,
            pane,
            "Task 12 done.".into(),
            Some("hangar".into()),
        );
        let steer = fake.expect("turn/steer").await;
        assert_eq!(
            steer["params"],
            json!({ "threadId": THREAD, "expectedTurnId": "turn-1", "input": [{ "type": "text", "text":
                "[Hangar] Event — not typed by the person.\n\nTask 12 done." }] })
        );
        fake.reply(&steer, json!({ "turnId": "turn-1" })).await;
        fake.quiet().await;

        fake.notify(
            "turn/completed",
            json!({ "threadId": THREAD, "turn": turn("turn-1", "completed") }),
        )
        .await;
        let start = fake.expect("turn/start").await;
        assert_eq!(
            start["params"]["input"],
            json!([{ "type": "text", "text": "And then deploy." }]),
            "the person's words go unlabelled"
        );
        fake.reply(&start, json!({ "turn": turn("turn-2", "inProgress") }))
            .await;
        assert!(bridge::bridge_queue(pane.into()).is_empty());
        run.stop().await;
    }

    // From the source (turn_processor.rs): a steer that finds no turn, or another, is an
    // invalid request; `turn/start` then starts a turn or joins the running one
    // (`start_or_steer_turn`).
    #[tokio::test]
    async fn a_refused_steer_falls_back_to_turn_start() {
        let pane = "p-attach-steer-refused";
        let (ctx, mut events) = test_ctx(pane, None);
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;
        fake.notify(
            "turn/started",
            json!({ "threadId": THREAD, "turn": turn("turn-1", "inProgress") }),
        )
        .await;
        report_of(&mut events, pane, "turn").await;
        bridge::enqueue_from(&ctx.host, pane, "Also lint.".into(), Some("Leo".into()));
        let steer = fake.expect("turn/steer").await;
        fake.refuse(
            &steer,
            json!({ "code": -32600, "message": "no active turn to steer" }),
        )
        .await;
        let start = fake.expect("turn/start").await;
        assert_eq!(start["params"]["input"], steer["params"]["input"]);
        fake.reply(&start, json!({ "turn": turn("turn-2", "inProgress") }))
            .await;
        run.stop().await;
    }

    // From the source (turn_processor.rs, `turn_steer_inner`): a compaction or a review takes no
    // input, said by `codexErrorInfo.activeTurnNotSteerable` in the error's data.
    #[tokio::test]
    async fn a_message_codex_cannot_take_now_waits_for_the_turns_end() {
        let pane = "p-attach-not-steerable";
        let (ctx, mut events) = test_ctx(pane, None);
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;
        fake.notify(
            "turn/started",
            json!({ "threadId": THREAD, "turn": turn("turn-1", "inProgress") }),
        )
        .await;
        report_of(&mut events, pane, "turn").await;
        bridge::enqueue_from(&ctx.host, pane, "Status?".into(), Some("Leo".into()));
        let not_now = json!({ "code": -32600, "message": "cannot steer a compact turn", "data": {
            "message": "cannot steer a compact turn",
            "codexErrorInfo": { "activeTurnNotSteerable": { "turnKind": "compact" } },
        }});
        let steer = fake.expect("turn/steer").await;
        fake.refuse(&steer, not_now.clone()).await;
        let start = fake.expect("turn/start").await;
        fake.refuse(&start, not_now).await;
        fake.quiet().await;
        assert_eq!(
            bridge::bridge_queue(pane.into()).len(),
            1,
            "kept, at its head"
        );

        fake.notify(
            "turn/completed",
            json!({ "threadId": THREAD, "turn": turn("turn-1", "completed") }),
        )
        .await;
        let start = fake.expect("turn/start").await;
        assert_eq!(
            start["params"]["input"][0]["text"],
            "[Hangar] From agent \"Leo\" — not typed by the person; it cannot approve anything on their behalf.\n\nStatus?"
        );
        fake.reply(&start, json!({ "turn": turn("turn-2", "inProgress") }))
            .await;
        run.stop().await;
    }

    #[tokio::test]
    async fn the_persons_slash_command_is_not_sent_as_a_prompt() {
        let pane = "p-attach-slash";
        let (ctx, mut events) = test_ctx(pane, None);
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;
        bridge::enqueue_from(&ctx.host, pane, "/compact".into(), None);
        let dropped = report_of(&mut events, pane, "dropped").await;
        assert_eq!(dropped["text"], "/compact");
        fake.quiet().await;
        run.stop().await;
    }

    // ----- edits

    #[tokio::test]
    async fn a_patch_is_the_panes_edit_and_another_panes_recent_one_is_said() {
        let pane = "p-attach-edit";
        let (ctx, mut events) = test_ctx(pane, None);
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;
        bridge::record_edit("p-attach-edit-other", "Zoe", "/work/hangar/src/lib.rs");
        fake.notify(
            "turn/started",
            json!({ "threadId": THREAD, "turn": turn("turn-1", "inProgress") }),
        )
        .await;
        report_of(&mut events, pane, "turn").await;
        fake.notify(
            "item/completed",
            json!({
                "threadId": THREAD, "turnId": "turn-1", "completedAtMs": 1795050070000i64,
                "item": { "type": "fileChange", "id": "item-8", "status": "completed", "changes": [
                    { "path": "src/lib.rs", "kind": { "type": "update" }, "diff": "" },
                    { "path": "/work/hangar/README.md", "kind": { "type": "add" }, "diff": "" },
                ]},
            }),
        )
        .await;
        let first = report_of(&mut events, pane, "edit").await;
        assert_eq!(
            first,
            json!({ "kind": "edit", "file": "/work/hangar/src/lib.rs", "name": "Cody",
                    "overlap": "Zoe", "overlapPane": "p-attach-edit-other" })
        );
        let second = report_of(&mut events, pane, "edit").await;
        assert_eq!(
            second,
            json!({ "kind": "edit", "file": "/work/hangar/README.md", "name": "Cody" })
        );
        let steer = fake.expect("turn/steer").await;
        let said = steer["params"]["input"][0]["text"].as_str().unwrap();
        assert!(
            said.starts_with("[Hangar] Event — not typed by the person.\n\nHangar: the agent in pane \"Zoe\" also edited /work/hangar/src/lib.rs"),
            "{said}"
        );
        fake.reply(&steer, json!({ "turnId": "turn-1" })).await;
        assert_eq!(
            bridge::conflict("p-attach-edit-other", "/work/hangar/README.md").map(|o| o.name),
            Some("Cody".to_string())
        );
        run.stop().await;
    }

    // ----- the connection's life

    #[tokio::test]
    async fn the_tui_leaving_ends_the_session_here() {
        let pane = "p-attach-tui-left";
        let (mut ctx, mut events) = test_ctx(pane, None);
        let clients = Arc::new(AtomicUsize::new(2));
        let seen = Arc::clone(&clients);
        ctx.presence = Arc::new(move || Some(seen.load(Ordering::Relaxed)));
        let (client, codex, mut fake) = connected().await;
        let run = following(&ctx, client, codex);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;
        // Hangar's own connection is all the server has left.
        clients.store(1, Ordering::Relaxed);
        let ended = tokio::time::timeout(PATIENCE, run.task)
            .await
            .expect("it lets go")
            .unwrap();
        assert!(matches!(ended, Ended::TuiLeft), "{ended:?}");
    }

    /// A connector handing out fake connections, one per call, and counting the calls.
    fn fake_connector(streams: Vec<DuplexStream>) -> (Connector, Arc<AtomicUsize>) {
        let streams = Arc::new(Mutex::new(std::collections::VecDeque::from(streams)));
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let connector: Connector = Arc::new(move |options| {
            counted.fetch_add(1, Ordering::Relaxed);
            let stream = streams.lock().unwrap().pop_front();
            boxed(async move {
                let Some(stream) = stream else {
                    return Err(Error::Start("nothing listens".into()));
                };
                let (read, write) = tokio::io::split(stream);
                Client::connect_over(read, write, Framing::Lines, options).await
            })
        });
        (connector, calls)
    }

    /// The server's side of a connection Hangar opens through `fake_connector`.
    async fn accept(codex: DuplexStream) -> Fake {
        let (read, write) = tokio::io::split(codex);
        let mut fake = Fake {
            lines: BufReader::new(read).lines(),
            write,
        };
        let init = fake.expect("initialize").await;
        fake.reply(
            &init,
            json!({ "userAgent": USER_AGENT, "codexHome": "/home/ada/.codex", "platformFamily": "unix", "platformOs": "linux" }),
        )
        .await;
        fake.expect("initialized").await;
        fake
    }

    // The --remote case: the TUI connects first, Hangar after it (initialize_processor.rs makes
    // the first client the originator); a server that goes is connected to again, then given up.
    #[tokio::test]
    async fn hangar_connects_after_the_tui_again_after_a_drop_and_gives_up_on_a_server_gone() {
        let pane = "p-attach-run";
        let (mut ctx, mut events) = test_ctx(pane, None);
        let clients = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&clients);
        ctx.presence = Arc::new(move || Some(seen.load(Ordering::Relaxed)));
        let (first, first_codex) = tokio::io::duplex(1 << 20);
        let (second, second_codex) = tokio::io::duplex(1 << 20);
        let (connector, calls) = fake_connector(vec![first, second]);
        let task = start(&tokio::runtime::Handle::current(), ctx.clone(), connector);
        assert!(is_attached(pane));

        // No TUI yet: Hangar does not connect.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(calls.load(Ordering::Relaxed), 0, "never the first client");
        clients.store(1, Ordering::Relaxed);
        let mut fake = accept(first_codex).await;
        clients.store(2, Ordering::Relaxed);
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;

        // The server goes: the session ends, and Hangar connects again.
        drop(fake);
        assert_eq!(
            report_of(&mut events, pane, "end").await,
            json!({ "kind": "end", "reason": "closed" })
        );
        let mut fake = accept(second_codex).await;
        join(&mut fake, THREAD).await;
        hello(&mut events, pane, THREAD).await;

        // Gone for good: a few attempts, then the pane is let go of.
        drop(fake);
        report_of(&mut events, pane, "end").await;
        tokio::time::timeout(PATIENCE, task)
            .await
            .expect("it gives up")
            .unwrap();
        assert_eq!(
            calls.load(Ordering::Relaxed),
            2 + fast().reconnects.len() + 1
        );
        assert!(!is_attached(pane));
    }

    #[tokio::test]
    async fn a_pane_whose_shell_ends_takes_its_client_with_it() {
        let pane = "p-attach-detach";
        let (mut ctx, _events) = test_ctx(pane, None);
        ctx.presence = Arc::new(|| Some(0));
        let (connector, calls) = fake_connector(Vec::new());
        let task = start(&tokio::runtime::Handle::current(), ctx.clone(), connector);
        assert!(is_attached(pane));
        // What `pty_kill` does for a pane's shell that ends.
        bridge::release_edits(pane);
        tokio::time::timeout(PATIENCE, task)
            .await
            .expect("it ends at once")
            .unwrap();
        assert!(!is_attached(pane));
        assert_eq!(calls.load(Ordering::Relaxed), 0);

        // Attached again (a restart): the earlier client goes, the new one stays.
        let (connector, _) = fake_connector(Vec::new());
        let earlier = start(&tokio::runtime::Handle::current(), ctx.clone(), connector);
        let (connector, _) = fake_connector(Vec::new());
        let later = start(&tokio::runtime::Handle::current(), ctx.clone(), connector);
        tokio::time::timeout(PATIENCE, earlier)
            .await
            .expect("the earlier one ends")
            .unwrap();
        assert!(is_attached(pane));
        detach(pane);
        tokio::time::timeout(PATIENCE, later)
            .await
            .expect("the later one ends")
            .unwrap();
    }

    #[tokio::test]
    async fn where_the_socket_table_cannot_tell_hangar_waits_a_grace_then_connects() {
        let pane = "p-attach-grace";
        let (ctx, _events) = test_ctx(pane, None);
        let (connector, calls) = fake_connector(Vec::new());
        let started = Instant::now();
        let task = start(&tokio::runtime::Handle::current(), ctx.clone(), connector);
        tokio::time::timeout(PATIENCE, task)
            .await
            .expect("it gives up")
            .unwrap();
        assert!(started.elapsed() >= fast().tui_grace);
        assert_eq!(calls.load(Ordering::Relaxed), fast().reconnects.len() + 1);
    }

    // ----- the socket table

    /// /proc/net/unix as Linux prints it (net/unix/af_unix.c, `unix_seq_show`): the listening
    /// socket flagged __SO_ACCEPTCON, each accepted connection under the same path, the
    /// connecting sides with none.
    #[test]
    fn the_socket_table_counts_the_connections_a_server_accepted() {
        let path = "/tmp/codex-daemon-1000/9f86d081884c7d659a2feaa0c55ad015";
        let table = format!(
            "Num       RefCount Protocol Flags    Type St Inode Path\n\
             0000000000000000: 00000002 00000000 00010000 0001 01 41001 {path}\n\
             0000000000000000: 00000003 00000000 00000000 0001 03 41002 {path}\n\
             0000000000000000: 00000003 00000000 00000000 0001 03 41003\n\
             0000000000000000: 00000002 00000000 00010000 0001 01 41004 /run/user/1000/bus\n\
             0000000000000000: 00000003 00000000 00000000 0001 03 41005 /run/user/1000/bus\n"
        );
        assert_eq!(clients_in(&table, path), Some(1));
        assert_eq!(clients_in(&table, "/run/user/1000/bus"), Some(1));
        assert_eq!(clients_in(&table, "/tmp/elsewhere"), None, "cannot tell");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_tui_connecting_to_the_socket_is_seen_through_codexs_link() {
        use std::os::unix::net::{UnixListener, UnixStream};
        let folder = crate::testdir::scratch("attach-socket");
        let real = folder.join("real.sock");
        let link = folder.join("codex.sock");
        let listener = UnixListener::bind(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(clients(&link), Some(0));
        let tui = UnixStream::connect(&link).unwrap();
        let (accepted, _) = listener.accept().unwrap();
        assert_eq!(clients(&link), Some(1));
        drop(tui);
        drop(accepted);
        assert_eq!(clients(&link), Some(0));
        assert_eq!(clients(&folder.join("nothing.sock")), None);
    }

    // ----- the mappings

    #[test]
    fn decisions_read_as_codexs_answers() {
        let allow = choice(&json!({ "behavior": "allow" })).unwrap();
        let session = choice(&json!({ "behavior": "allow", "forSession": true })).unwrap();
        let no = choice(&json!({ "behavior": "deny", "message": "not now" })).unwrap();
        let stop = choice(&json!({ "behavior": "deny", "interrupt": true })).unwrap();
        assert_eq!(choice(&json!({ "behavior": "maybe" })), None);
        assert_eq!(
            command_decision(allow, false),
            CommandExecutionApprovalDecision::Accept
        );
        assert_eq!(
            command_decision(session, false),
            CommandExecutionApprovalDecision::AcceptForSession
        );
        // A managed checkpoint takes only accept, decline and cancel (the schema).
        assert_eq!(
            command_decision(session, true),
            CommandExecutionApprovalDecision::Accept
        );
        assert_eq!(
            command_decision(no, false),
            CommandExecutionApprovalDecision::Decline
        );
        assert_eq!(
            command_decision(stop, false),
            CommandExecutionApprovalDecision::Cancel
        );
        assert_eq!(
            file_decision(session),
            FileChangeApprovalDecision::AcceptForSession
        );
        assert_eq!(file_decision(stop), FileChangeApprovalDecision::Cancel);

        let requested: RequestPermissionProfile =
            serde_json::from_value(json!({ "network": { "enabled": true } })).unwrap();
        assert_eq!(
            serde_json::to_value(permissions_response(session, &requested)).unwrap(),
            json!({ "permissions": { "network": { "enabled": true } }, "scope": "session" })
        );
        assert_eq!(
            serde_json::to_value(permissions_response(no, &requested)).unwrap(),
            json!({ "permissions": {}, "scope": "turn" })
        );

        let tool_call = elicitation_response(
            &json!({ "behavior": "allow", "forSession": true }),
            true,
            true,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(tool_call).unwrap(),
            json!({ "_meta": { "persist": "session" }, "action": "accept", "content": {} })
        );
        let url = elicitation_response(&json!({ "behavior": "allow" }), false, false).unwrap();
        assert_eq!(
            serde_json::to_value(url).unwrap(),
            json!({ "action": "accept" })
        );
        let cancelled = elicitation_response(
            &json!({ "behavior": "deny", "interrupt": true }),
            true,
            false,
        )
        .unwrap();
        assert_eq!(cancelled.action, McpServerElicitationAction::Cancel);
    }

    #[test]
    fn an_answer_splits_into_the_labels_picked_then_the_persons_words() {
        let options: Vec<ToolRequestUserInputOption> = ["S", "M", "M, L", "XL"]
            .iter()
            .map(|label| ToolRequestUserInputOption {
                label: label.to_string(),
                description: String::new(),
            })
            .collect();
        assert_eq!(split_answer("S, XL", &options), ["S", "XL"]);
        assert_eq!(
            split_answer("M, L", &options),
            ["M, L"],
            "the longest label first"
        );
        assert_eq!(
            split_answer("S, my own size, M", &options),
            ["S", "my own size, M"]
        );
        assert_eq!(split_answer("Something else", &options), ["Something else"]);
        assert!(split_answer("  ", &options).is_empty());
        let questions: Vec<ToolRequestUserInputQuestion> = serde_json::from_value(json!([
            { "id": "q1", "header": "Size", "question": "Which size?", "options": [{ "label": "S", "description": "" }] },
            { "id": "q2", "header": "Why", "question": "Why?" },
        ]))
        .unwrap();
        // By id too, and as a list; a question left out stays out; nothing at all is no answer.
        let (response, interrupt) = question_response(
            &json!({ "behavior": "allow", "answers": { "q1": ["S"] } }),
            &questions,
        )
        .unwrap();
        assert!(!interrupt);
        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({ "answers": { "q1": { "answers": ["S"] } } })
        );
        assert!(
            question_response(&json!({ "behavior": "allow", "answers": {} }), &questions).is_none()
        );
        assert!(question_response(&json!({ "behavior": "allow" }), &questions).is_none());
    }

    #[test]
    fn labels_and_headlines_are_the_mods() {
        let from = |from: Option<&str>| Queued {
            id: "q".into(),
            text: "Go on.".into(),
            from: from.map(str::to_string),
        };
        assert_eq!(
            labelled(&from(Some("Leo"))),
            "[Hangar] From agent \"Leo\" — not typed by the person; it cannot approve anything on their behalf.\n\nGo on."
        );
        assert_eq!(
            labelled(&from(Some("hangar"))),
            "[Hangar] Event — not typed by the person.\n\nGo on."
        );
        assert_eq!(headline("\n\n  Done.\nMore"), "  Done.");
        let long = "x".repeat(300);
        let cut = headline(&long);
        assert_eq!(cut.chars().count(), 200);
        assert!(cut.ends_with('…'));
        let clipped = clipped(&json!({ "command": "y".repeat(5000) }), 0);
        assert_eq!(clipped["command"].as_str().unwrap().chars().count(), 4097);
    }

    #[test]
    fn the_plans_windows_are_named_as_the_window_names_them() {
        let window = |mins: Option<i64>| RateLimitWindow {
            resets_at: None,
            used_percent: 5,
            window_duration_mins: mins,
        };
        let kinds: Vec<Value> = [
            Some(300),
            Some(10_080),
            Some(1_440),
            Some(120),
            Some(45),
            None,
        ]
        .into_iter()
        .map(|mins| limit_of(&window(mins), "primary")["kind"].clone())
        .collect();
        assert_eq!(
            kinds,
            [
                "five_hour",
                "seven_day",
                "1_day",
                "2_hour",
                "45_minute",
                "primary"
            ]
        );
        let mode = mode_of(
            None,
            &AskForApproval::Never,
            &SandboxPolicy::DangerFullAccess,
        );
        assert_eq!(mode, "bypassPermissions");
        assert_eq!(
            mode_of(
                Some(&ModeKind::Plan),
                &AskForApproval::Never,
                &SandboxPolicy::DangerFullAccess
            ),
            "plan"
        );
    }
}
