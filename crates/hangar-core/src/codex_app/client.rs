//! The JSON-RPC client of `codex app-server` (docs/native/agents.md §4.3, D10): one connection,
//! its handshake, Hangar's requests and their answers, and what the server sends on its own —
//! notifications, and requests that wait for Hangar's answer (approvals, questions, MCP
//! elicitations).
//!
//! The reference for every behaviour is OpenAI's own app-server at the pinned tag (D22):
//! `app-server-protocol/src/rpc.rs` (the envelope: JSON-RPC without the `"jsonrpc"` field),
//! `app-server-transport/src/transport/` (stdio is one message per line; a pane's socket is a
//! WebSocket, websocket.rs), `app-server/src/request_processors/initialize_processor.rs` (the
//! handshake) and `app-server/src/outgoing_message.rs` (server requests and their answers).
//!
//! Three ways in ([`Endpoint`]):
//! - **stdio**: Hangar spawns `codex app-server --listen stdio://` (a headless child, a probe) and
//!   speaks on its stdin and stdout;
//! - **a pane's socket**: the pane's server listens on `unix://<socket>` and Hangar connects to it
//!   as a second client, beside the pane's TUI (§4.1, D5);
//! - **a proxy**: `codex app-server proxy --sock <socket>`, spawned (through `wsl.exe` for a WSL
//!   pane), relays the same socket's bytes on its stdio.
//!
//! Decoding is lenient (D10): an unknown field is ignored; a notification Hangar does not read, or
//! one it cannot read, is dropped and reported once per method; a server request Hangar does not
//! handle is refused with `methodNotFound` when Hangar is the server's only client
//! ([`Unhandled::Refuse`]) — never left unanswered — and left to the TUI when it is not
//! ([`Unhandled::Leave`]). Nothing a server sends makes the client panic.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::fmt;
use std::marker::PhantomData;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use super::gate::{self, Refusal, Verdict};
use super::generated::{
    notifications, requests, server_requests, ClientInfo, ClientNotification, ClientRequest,
    InitializeCapabilities, InitializeParams, InitializeResponse, RequestId, ServerNotification,
    ServerRequest, ServerRequestMethod,
};
use super::websocket::{self, Line};

/// JSON-RPC's code for a request the receiver will not take as it is: Codex's answer to a
/// `turn/steer` that finds no turn to steer, or to a second `initialize`.
pub const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC's code for a method the receiver does not have.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC's code for params the receiver cannot read.
pub const INVALID_PARAMS: i64 = -32602;
/// JSON-RPC's code for an error of the receiver's own.
pub const INTERNAL_ERROR: i64 = -32603;
/// Codex's code for a request it has no room for now ("Server overloaded; retry later",
/// `app-server/src/error_code.rs`).
pub const OVERLOADED: i64 = -32001;

/// How many of the server's last stderr lines are kept for an error message.
const STDERR_LINES: usize = 20;
/// The longest stderr line kept, in characters.
const STDERR_LINE_CHARS: usize = 2_000;
/// How long the end of a spawned server's output waits for the end of its stderr, so the reason
/// it gave is in the error.
const STDERR_SETTLE: Duration = Duration::from_millis(500);

/// What to do with a server request Hangar does not handle, or cannot read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unhandled {
    /// Answer it with an error (`methodNotFound`, or invalid params for a method Hangar handles
    /// but could not read), so the server does not wait: Hangar is the only client (headless
    /// children, probes). D10's rule.
    Refuse,
    /// Leave it unanswered: another client answers it. An attached pane's TUI receives every
    /// request of the thread too, and the server takes the first answer, an error included
    /// (`outgoing_message.rs`, `take_connection_callback`), so Hangar's refusal would cancel the
    /// TUI's dialog. Reported once all the same.
    Leave,
}

/// Where Hangar's reports of what it dropped go (once per kind and method): the log by default.
#[derive(Clone)]
pub struct Report(Arc<dyn Fn(&str) + Send + Sync>);

impl Report {
    pub fn new(report: impl Fn(&str) + Send + Sync + 'static) -> Report {
        Report(Arc::new(report))
    }

    fn say(&self, message: &str) {
        (self.0)(message)
    }
}

impl Default for Report {
    fn default() -> Report {
        Report::new(|message| eprintln!("[codex] {message}"))
    }
}

impl fmt::Debug for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Report")
    }
}

/// How a connection is made and kept.
#[derive(Debug, Clone)]
pub struct Options {
    /// `clientInfo.name`. Codex makes the first client's name the server process's
    /// originator (the `originator` header of its model requests, the start of its `userAgent`;
    /// `initialize_processor.rs`), so a server Hangar reaches first is "Hangar"'s for OpenAI.
    pub client_name: String,
    /// `clientInfo.version`.
    pub client_version: String,
    /// The longest a request waits for its answer.
    pub request_timeout: Duration,
    /// The longest the WebSocket upgrade and `initialize` wait (a first start through npm's
    /// shim can be slow).
    pub initialize_timeout: Duration,
    /// How long [`Client::shutdown`] lets a spawned server go by itself before killing it.
    pub shutdown_grace: Duration,
    /// Server requests Hangar does not handle.
    pub unhandled: Unhandled,
    /// Notifications this connection asks Codex not to send (`optOutNotificationMethods`).
    pub opt_out_notifications: Vec<String>,
    /// Where drops are reported.
    pub report: Report,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            client_name: "Teskad".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
            request_timeout: Duration::from_secs(60),
            initialize_timeout: Duration::from_secs(30),
            shutdown_grace: Duration::from_secs(5),
            unhandled: Unhandled::Refuse,
            opt_out_notifications: Vec::new(),
            report: Report::default(),
        }
    }
}

/// Where the server is.
#[derive(Debug)]
pub enum Endpoint {
    /// A server to spawn that speaks on its stdio, one JSON message per line:
    /// [`app_server_command`] plus the launch's `-c` settings.
    Stdio(Command),
    /// A command that relays a pane server's socket on its stdio ([`proxy_command`], possibly
    /// run through `wsl.exe`): the WebSocket goes through it.
    Proxy(Command),
    /// A pane server's socket (`--listen unix://<socket>`), connected to directly.
    #[cfg(unix)]
    Socket(PathBuf),
}

/// `<codex> app-server --listen stdio://`: a server of Hangar's own on stdio. Its `-c key=value`
/// settings are added after (`app-server` takes them: `cli/src/main.rs`, `AppServerCommand`).
pub fn app_server_command(codex: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(codex);
    command.args(["app-server", "--listen", "stdio://"]);
    command
}

/// `<codex> app-server proxy --sock <socket>`: the bytes of a pane server's socket on stdio.
pub fn proxy_command(codex: impl AsRef<OsStr>, socket: &Path) -> Command {
    let mut command = Command::new(codex);
    command.args(["app-server", "proxy", "--sock"]).arg(socket);
    command
}

/// How messages are cut on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    /// One JSON message per line (stdio).
    Lines,
    /// One JSON message per WebSocket text frame (a pane's socket, directly or through a proxy).
    WebSocket,
}

/// Codex's error answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Why a connection ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// The server closed its side (the stream ended, or a WebSocket close).
    ServerClosed,
    /// Reading or writing failed.
    Io(String),
    /// Hangar closed it ([`Client::shutdown`]).
    Shutdown,
}

/// A connection that ended, and what the server last wrote on stderr (spawned servers only),
/// for the notice and the bug report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closed {
    pub cause: Cause,
    pub stderr: Vec<String>,
}

impl fmt::Display for Closed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.cause {
            Cause::ServerClosed => f.write_str("Codex's app-server closed the connection")?,
            Cause::Io(error) => write!(f, "the connection to Codex's app-server failed: {error}")?,
            Cause::Shutdown => f.write_str("Teskad closed the connection to Codex's app-server")?,
        }
        if !self.stderr.is_empty() {
            write!(f, " (it said: {})", self.stderr.join(" / "))?;
        }
        Ok(())
    }
}

/// What a connection or a request ran into.
#[derive(Debug, Clone)]
pub enum Error {
    /// The command could not be started, or the socket not reached.
    Start(String),
    /// The pane server's socket did not take the WebSocket upgrade.
    Handshake(String),
    /// The connection ended, or was never there.
    Closed(Closed),
    /// No answer in time. The request is forgotten: a late answer is dropped.
    Timeout {
        method: &'static str,
        after: Duration,
    },
    /// Codex answered with an error.
    Rpc {
        method: &'static str,
        error: RpcError,
    },
    /// Codex's answer is not what the pinned schema says it is.
    Decode { method: &'static str, error: String },
    /// What Hangar sends could not be written as JSON.
    Encode { method: &'static str, error: String },
    /// The version gate refused this Codex (gate.rs): the pane falls back to the plain TUI.
    Unsupported(Refusal),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Start(error) => write!(f, "could not start Codex's app-server: {error}"),
            Error::Handshake(error) => write!(f, "could not attach to Codex's app-server: {error}"),
            Error::Closed(closed) => closed.fmt(f),
            Error::Timeout { method, after } => write!(
                f,
                "Codex did not answer `{method}` within {} s",
                after.as_secs_f32()
            ),
            Error::Rpc { method, error } => write!(
                f,
                "Codex refused `{method}`: {} (code {})",
                error.message, error.code
            ),
            Error::Decode { method, error } => {
                write!(f, "could not read Codex's answer to `{method}`: {error}")
            }
            Error::Encode { method, error } => write!(f, "could not write `{method}`: {error}"),
            Error::Unsupported(refusal) => refusal.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

/// The server Hangar is connected to, as `initialize` described it.
#[derive(Debug, Clone)]
pub struct Server {
    /// Its answer: `userAgent`, `codexHome`, platform.
    pub info: InitializeResponse,
    /// What the version gate made of it.
    pub verdict: Verdict,
}

/// What the server sends on its own, in order.
// Not boxed: each event goes once through the channel to its one reader, and most are
// notifications, so a box would cost each an allocation and every `match` a deref, to save
// nothing that lasts.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event {
    /// A notification of Hangar's subset (scripts/codex-protocol/subset.json).
    Notification(ServerNotification),
    /// A request that waits for Hangar's answer.
    Request(Incoming),
    /// The connection ended; nothing comes after.
    Closed(Closed),
}

/// The stream of [`Event`]s of one connection.
#[derive(Debug)]
pub struct Events {
    rx: mpsc::UnboundedReceiver<Event>,
    /// [`Event::Closed`] was given: the stream is over, though the [`Client`] (which holds the
    /// sending side) may live on.
    ended: bool,
}

impl Events {
    /// The next event; `None` after [`Event::Closed`].
    pub async fn next(&mut self) -> Option<Event> {
        if self.ended {
            return None;
        }
        let event = self.rx.recv().await?;
        if matches!(event, Event::Closed(_)) {
            self.ended = true;
            self.rx.close();
        }
        Some(event)
    }
}

/// A server request Hangar handles, waiting for its answer. Each is answered once, with the type
/// its method takes; one dropped unanswered is refused under [`Unhandled::Refuse`], so the
/// server never waits forever.
#[derive(Debug)]
pub enum Incoming {
    /// `item/commandExecution/requestApproval`
    CommandExecutionApproval(Pending<server_requests::ItemCommandExecutionRequestApproval>),
    /// `item/fileChange/requestApproval`
    FileChangeApproval(Pending<server_requests::ItemFileChangeRequestApproval>),
    /// `item/permissions/requestApproval`
    PermissionsApproval(Pending<server_requests::ItemPermissionsRequestApproval>),
    /// `item/tool/requestUserInput`
    UserInput(Pending<server_requests::ItemToolRequestUserInput>),
    /// `mcpServer/elicitation/request`
    McpElicitation(Pending<server_requests::McpServerElicitationRequest>),
}

impl Incoming {
    /// The request's id, which `serverRequest/resolved` names when another client answered it.
    pub fn id(&self) -> &RequestId {
        match self {
            Incoming::CommandExecutionApproval(p) => p.id(),
            Incoming::FileChangeApproval(p) => p.id(),
            Incoming::PermissionsApproval(p) => p.id(),
            Incoming::UserInput(p) => p.id(),
            Incoming::McpElicitation(p) => p.id(),
        }
    }

    /// Its method.
    pub fn method(&self) -> &'static str {
        match self {
            Incoming::CommandExecutionApproval(_) => {
                <server_requests::ItemCommandExecutionRequestApproval as ServerRequestMethod>::METHOD
            }
            Incoming::FileChangeApproval(_) => {
                <server_requests::ItemFileChangeRequestApproval as ServerRequestMethod>::METHOD
            }
            Incoming::PermissionsApproval(_) => {
                <server_requests::ItemPermissionsRequestApproval as ServerRequestMethod>::METHOD
            }
            Incoming::UserInput(_) => {
                <server_requests::ItemToolRequestUserInput as ServerRequestMethod>::METHOD
            }
            Incoming::McpElicitation(_) => {
                <server_requests::McpServerElicitationRequest as ServerRequestMethod>::METHOD
            }
        }
    }
}

/// One server request of method `M`, and the means to answer it.
pub struct Pending<M: ServerRequestMethod> {
    params: M::Params,
    reply: Reply,
    method: PhantomData<M>,
}

impl<M: ServerRequestMethod> Pending<M> {
    pub fn id(&self) -> &RequestId {
        &self.reply.id
    }

    /// What the server asks.
    pub fn params(&self) -> &M::Params {
        &self.params
    }

    /// Answers with the response the method takes. An error means the connection is gone, and
    /// with it the request.
    pub fn respond(mut self, response: M::Response) -> Result<(), Error> {
        let result = serde_json::to_value(&response).map_err(|e| Error::Encode {
            method: M::METHOD,
            error: e.to_string(),
        })?;
        self.reply.send(Ok(result))
    }

    /// Answers with an error: Hangar cannot answer it. (A person's "no" is a response — a
    /// `decline` — not this.)
    pub fn reject(mut self, message: impl Into<String>) -> Result<(), Error> {
        self.reply.send(Err(RpcError {
            code: INTERNAL_ERROR,
            message: message.into(),
            data: None,
        }))
    }
}

impl<M: ServerRequestMethod> fmt::Debug for Pending<M>
where
    M::Params: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pending")
            .field("method", &M::METHOD)
            .field("id", &self.reply.id)
            .field("params", &self.params)
            .finish()
    }
}

/// The answer half of a server request.
struct Reply {
    id: RequestId,
    method: &'static str,
    shared: Weak<Shared>,
    answered: bool,
}

impl Reply {
    fn send(&mut self, answer: Result<Value, RpcError>) -> Result<(), Error> {
        self.answered = true;
        match self.shared.upgrade() {
            Some(shared) => shared.answer(&self.id, answer),
            None => Err(Error::Closed(Closed {
                cause: Cause::Shutdown,
                stderr: Vec::new(),
            })),
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        if self.answered {
            return;
        }
        if let Some(shared) = self.shared.upgrade() {
            if shared.options.unhandled == Unhandled::Refuse {
                let _ = shared.answer(
                    &self.id,
                    Err(RpcError {
                        code: INTERNAL_ERROR,
                        message: format!("Teskad dropped `{}` without an answer", self.method),
                        data: None,
                    }),
                );
            }
        }
    }
}

/// A connection to a `codex app-server`, past its handshake. Requests may be sent from several
/// tasks at once (`&self`); the [`Events`] that came with it are read by one.
pub struct Client {
    shared: Arc<Shared>,
    server: Server,
    child: tokio::sync::Mutex<Option<Child>>,
    tasks: Mutex<Tasks>,
}

/// The tasks of a connection.
#[derive(Default)]
struct Tasks {
    /// Writes Hangar's messages; finishes by closing Hangar's side.
    writer: Option<JoinHandle<()>>,
    /// The reader, and a spawned server's stderr.
    others: Vec<JoinHandle<()>>,
}

impl Tasks {
    fn abort(&mut self) {
        if let Some(writer) = self.writer.take() {
            writer.abort();
        }
        for task in self.others.drain(..) {
            task.abort();
        }
    }
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("server", &self.server)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Connects to `endpoint`, then `initialize` / `initialized`, and checks the server's version
    /// against the gate: a refused Codex is disconnected and comes back as
    /// [`Error::Unsupported`].
    pub async fn connect(endpoint: Endpoint, options: Options) -> Result<(Client, Events), Error> {
        match endpoint {
            Endpoint::Stdio(command) => {
                let (read, write, process) = spawn(command)?;
                open(read, write, Framing::Lines, Some(process), options).await
            }
            Endpoint::Proxy(command) => {
                let (read, write, process) = spawn(command)?;
                open(read, write, Framing::WebSocket, Some(process), options).await
            }
            #[cfg(unix)]
            Endpoint::Socket(path) => {
                // tokio's `net` comes with axum's `tokio` feature (core's Cargo.toml).
                let stream = tokio::net::UnixStream::connect(&path)
                    .await
                    .map_err(|e| Error::Start(format!("{}: {e}", path.display())))?;
                let (read, write) = stream.into_split();
                open(
                    Box::new(read),
                    Box::new(write),
                    Framing::WebSocket,
                    None,
                    options,
                )
                .await
            }
        }
    }

    /// [`Client::connect`] over a stream of the caller's: tests, or a transport the endpoints do
    /// not cover.
    pub async fn connect_over(
        read: impl AsyncRead + Send + Unpin + 'static,
        write: impl AsyncWrite + Send + Unpin + 'static,
        framing: Framing,
        options: Options,
    ) -> Result<(Client, Events), Error> {
        open(Box::new(read), Box::new(write), framing, None, options).await
    }

    /// The server, as its handshake described it.
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// Sends request `R` and waits for its answer, up to [`Options::request_timeout`].
    pub async fn request<R: ClientRequest>(&self, params: R::Params) -> Result<R::Response, Error> {
        self.request_within::<R>(params, self.shared.options.request_timeout)
            .await
    }

    /// [`Client::request`] with a time limit of its own.
    pub async fn request_within<R: ClientRequest>(
        &self,
        params: R::Params,
        timeout: Duration,
    ) -> Result<R::Response, Error> {
        self.shared.request::<R>(params, timeout).await
    }

    /// Why the connection ended, once it has.
    pub fn closed(&self) -> Option<Closed> {
        lock(&self.shared.closed).clone()
    }

    /// Closes the connection: requests still waiting fail, a WebSocket is closed, the server's
    /// input ends (Codex then stops by itself: `transport/stdio.rs`); a spawned server that is
    /// still there after [`Options::shutdown_grace`] is killed. Gives the spawned server's exit
    /// status. Calling it again does nothing more.
    pub async fn shutdown(&self) -> Option<ExitStatus> {
        stop(
            &self.shared,
            &mut *self.child.lock().await,
            &self.tasks,
            self.shared.options.shutdown_grace,
        )
        .await
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // A spawned server is killed with its handle (`kill_on_drop`); the tasks reading and
        // writing it are stopped here, so a socket is let go of too.
        lock(&self.tasks).abort();
    }
}

/// What both halves of a connection share.
struct Shared {
    /// To the writer task.
    out: mpsc::UnboundedSender<Outgoing>,
    /// To the [`Events`].
    events: mpsc::UnboundedSender<Event>,
    /// Hangar's requests waiting for their answer, by id; `None` once the connection is closed.
    pending: Mutex<Option<HashMap<i64, oneshot::Sender<Answer>>>>,
    next_id: AtomicI64,
    closed: Mutex<Option<Closed>>,
    /// What has been reported already (kind and method).
    reported: Mutex<HashSet<String>>,
    /// The server's version, for reports.
    version: Mutex<Option<String>>,
    /// The last lines a spawned server wrote on stderr.
    stderr: Arc<Mutex<VecDeque<String>>>,
    /// Turns true when a spawned server's stderr has ended.
    stderr_done: Option<watch::Receiver<bool>>,
    options: Options,
}

/// What goes to the writer task.
enum Outgoing {
    Message(String),
    Pong(Vec<u8>),
    /// Close our side and stop.
    Finish,
}

/// The answer to one of Hangar's requests.
enum Answer {
    Result(Value),
    Error(RpcError),
    Closed(Closed),
}

impl Shared {
    async fn request<R: ClientRequest>(
        &self,
        params: R::Params,
        timeout: Duration,
    ) -> Result<R::Response, Error> {
        let params = serde_json::to_value(&params).map_err(|e| Error::Encode {
            method: R::METHOD,
            error: e.to_string(),
        })?;
        let result = self.call(R::METHOD, params, timeout).await?;
        serde_json::from_value(result).map_err(|e| {
            self.report_once(format!("answer {}", R::METHOD), || {
                format!(
                    "could not read the answer to `{}` from Codex {}: {e}",
                    R::METHOD,
                    self.version_text()
                )
            });
            Error::Decode {
                method: R::METHOD,
                error: e.to_string(),
            }
        })
    }

    async fn call(
        &self,
        method: &'static str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, Error> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        match lock(&self.pending).as_mut() {
            Some(pending) => {
                pending.insert(id, tx);
            }
            None => return Err(Error::Closed(self.closed_now())),
        }
        let mut message = Map::new();
        message.insert("id".into(), json!(id));
        message.insert("method".into(), json!(method));
        if !params.is_null() {
            message.insert("params".into(), params);
        }
        if self.send(Value::Object(message)).is_err() {
            self.forget(id);
            return Err(Error::Closed(self.closed_now()));
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Answer::Result(result))) => Ok(result),
            Ok(Ok(Answer::Error(error))) => Err(Error::Rpc { method, error }),
            Ok(Ok(Answer::Closed(closed))) => Err(Error::Closed(closed)),
            Ok(Err(_)) => Err(Error::Closed(self.closed_now())),
            Err(_) => {
                self.forget(id);
                Err(Error::Timeout {
                    method,
                    after: timeout,
                })
            }
        }
    }

    fn notify<N: ClientNotification>(&self, params: N::Params) -> Result<(), Error> {
        let params = serde_json::to_value(&params).map_err(|e| Error::Encode {
            method: N::METHOD,
            error: e.to_string(),
        })?;
        let mut message = Map::new();
        message.insert("method".into(), json!(N::METHOD));
        if !params.is_null() {
            message.insert("params".into(), params);
        }
        self.send(Value::Object(message))
            .map_err(|()| Error::Closed(self.closed_now()))
    }

    fn answer(&self, id: &RequestId, answer: Result<Value, RpcError>) -> Result<(), Error> {
        let message = match answer {
            Ok(result) => json!({ "id": id, "result": result }),
            Err(error) => json!({ "id": id, "error": error }),
        };
        self.send(message)
            .map_err(|()| Error::Closed(self.closed_now()))
    }

    fn send(&self, message: Value) -> Result<(), ()> {
        if lock(&self.closed).is_some() {
            return Err(());
        }
        self.out
            .send(Outgoing::Message(message.to_string()))
            .map_err(|_| ())
    }

    fn forget(&self, id: i64) {
        if let Some(pending) = lock(&self.pending).as_mut() {
            pending.remove(&id);
        }
    }

    /// Why the connection is closed, or a closure seen from the sending side (the writer
    /// stopped) before the reader noticed.
    fn closed_now(&self) -> Closed {
        lock(&self.closed).clone().unwrap_or_else(|| Closed {
            cause: Cause::ServerClosed,
            stderr: self.stderr_now(),
        })
    }

    fn stderr_now(&self) -> Vec<String> {
        lock(&self.stderr).iter().cloned().collect()
    }

    /// Ends the connection once: requests waiting fail, and [`Event::Closed`] is the last event.
    fn close(&self, cause: Cause) {
        let closed = {
            let mut slot = lock(&self.closed);
            if slot.is_some() {
                return;
            }
            let closed = Closed {
                cause,
                stderr: self.stderr_now(),
            };
            *slot = Some(closed.clone());
            closed
        };
        let pending = lock(&self.pending).take().unwrap_or_default();
        for (_, tx) in pending {
            let _ = tx.send(Answer::Closed(closed.clone()));
        }
        let _ = self.events.send(Event::Closed(closed));
    }

    fn report_once(&self, key: String, message: impl FnOnce() -> String) {
        if lock(&self.reported).insert(key) {
            self.options.report.say(&message());
        }
    }

    fn version_text(&self) -> String {
        lock(&self.version)
            .clone()
            .unwrap_or_else(|| "(version not known yet)".into())
    }

    /// One message from the server.
    fn dispatch(self: &Arc<Self>, bytes: &[u8]) {
        if lock(&self.closed).is_some() {
            return;
        }
        let message = match serde_json::from_slice::<Value>(bytes) {
            Ok(Value::Object(message)) => message,
            _ => {
                self.report_once("unreadable".into(), || {
                    format!(
                        "dropped a message from Codex {} that is not a JSON-RPC object: {}",
                        self.version_text(),
                        preview(bytes)
                    )
                });
                return;
            }
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let id = message.get("id").cloned();
        match (method, id) {
            (Some(method), Some(id)) => self.on_request(method, id, message),
            (Some(method), None) => self.on_notification(method, message),
            (None, Some(id)) => self.on_answer(id, message),
            (None, None) => self.report_once("unreadable".into(), || {
                format!(
                    "dropped a message from Codex {} with neither a method nor an id: {}",
                    self.version_text(),
                    preview(bytes)
                )
            }),
        }
    }

    fn on_answer(&self, id: Value, mut message: Map<String, Value>) {
        let tx = id
            .as_i64()
            .and_then(|id| lock(&self.pending).as_mut()?.remove(&id));
        let Some(tx) = tx else {
            // An answer after its time limit, or to an id Hangar never sent.
            self.report_once("answer unknown".into(), || {
                format!("dropped an answer from Codex to a request no longer waiting (id {id})")
            });
            return;
        };
        let answer = match message.remove("error") {
            Some(error) => Answer::Error(serde_json::from_value(error.clone()).unwrap_or_else(
                |_| RpcError {
                    code: INTERNAL_ERROR,
                    message: error.to_string(),
                    data: None,
                },
            )),
            // `turn/interrupt` answers `{}`; a missing result reads as null.
            None => Answer::Result(message.remove("result").unwrap_or(Value::Null)),
        };
        let _ = tx.send(answer);
    }

    fn on_notification(&self, method: String, message: Map<String, Value>) {
        let notification =
            match serde_json::from_value::<ServerNotification>(Value::Object(message)) {
                Ok(ServerNotification::Unrecognized(_)) | Err(_) => {
                    let known = ServerNotification::METHODS.contains(&method.as_str());
                    self.report_once(format!("notification {method}"), || {
                        if known {
                            format!(
                                "could not read a `{method}` notification from Codex {}: dropped",
                                self.version_text()
                            )
                        } else {
                            format!(
                                "Codex {} sends `{method}`, a notification Teskad does not read: \
                             ignored",
                                self.version_text()
                            )
                        }
                    });
                    return;
                }
                Ok(notification) => notification,
            };
        let _ = self.events.send(Event::Notification(notification));
    }

    fn on_request(self: &Arc<Self>, method: String, id: Value, message: Map<String, Value>) {
        // Codex numbers its requests (`next_request_id`); any other id is answered as it came.
        let id =
            serde_json::from_value::<RequestId>(id.clone()).unwrap_or(RequestId::Unrecognized(id));
        let request = serde_json::from_value::<ServerRequest>(Value::Object(message))
            .unwrap_or(ServerRequest::Unrecognized(Value::Null));
        let reply = |method: &'static str| Reply {
            id: id.clone(),
            method,
            shared: Arc::downgrade(self),
            answered: false,
        };
        let incoming = match request {
            ServerRequest::ItemCommandExecutionRequestApproval(params) => {
                Incoming::CommandExecutionApproval(pending(params, reply))
            }
            ServerRequest::ItemFileChangeRequestApproval(params) => {
                Incoming::FileChangeApproval(pending(params, reply))
            }
            ServerRequest::ItemPermissionsRequestApproval(params) => {
                Incoming::PermissionsApproval(pending(params, reply))
            }
            ServerRequest::ItemToolRequestUserInput(params) => {
                Incoming::UserInput(pending(params, reply))
            }
            ServerRequest::McpServerElicitationRequest(params) => {
                Incoming::McpElicitation(pending(params, reply))
            }
            ServerRequest::Unrecognized(_) => {
                self.unhandled(&method, &id);
                return;
            }
        };
        // Events dropped: the request is dropped with it, and refused under `Refuse`.
        let _ = self.events.send(Event::Request(incoming));
    }

    /// A server request outside the subset, or one Hangar could not read.
    fn unhandled(&self, method: &str, id: &RequestId) {
        let known = ServerRequest::METHODS.contains(&method);
        let refuse = self.options.unhandled == Unhandled::Refuse;
        self.report_once(format!("request {method}"), || {
            let what = if known {
                format!(
                    "could not read a `{method}` request from Codex {}",
                    self.version_text()
                )
            } else {
                format!(
                    "Codex {} asks `{method}`, a request Teskad does not handle",
                    self.version_text()
                )
            };
            if refuse {
                format!("{what}: refused")
            } else {
                format!("{what}: left to the other client")
            }
        });
        if refuse {
            let error = if known {
                RpcError {
                    code: INVALID_PARAMS,
                    message: format!("Teskad could not read this `{method}` request"),
                    data: None,
                }
            } else {
                RpcError {
                    code: METHOD_NOT_FOUND,
                    message: format!("Teskad does not handle `{method}`"),
                    data: None,
                }
            };
            let _ = self.answer(id, Err(error));
        }
    }
}

fn pending<M: ServerRequestMethod>(
    params: M::Params,
    reply: impl FnOnce(&'static str) -> Reply,
) -> Pending<M> {
    Pending {
        params,
        reply: reply(M::METHOD),
        method: PhantomData,
    }
}

type BoxRead = Box<dyn AsyncRead + Send + Unpin>;
type BoxWrite = Box<dyn AsyncWrite + Send + Unpin>;

/// A spawned server or proxy.
struct Process {
    child: Child,
    /// Its last stderr lines, the task that reads them, and when it is done.
    stderr: Arc<Mutex<VecDeque<String>>>,
    stderr_task: JoinHandle<()>,
    stderr_done: watch::Receiver<bool>,
}

/// Spawns `command` with its stdio piped: Hangar's side of it, and the process.
fn spawn(mut command: Command) -> Result<(BoxRead, BoxWrite, Process), Error> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| {
        let program = command
            .as_std()
            .get_program()
            .to_string_lossy()
            .into_owned();
        Error::Start(format!("{program}: {e}"))
    })?;
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err(Error::Start("no stdio to speak on".into()));
    };
    // Read stderr all along: a full pipe would stop the server. Its last lines are kept.
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    let kept = Arc::clone(&tail);
    let (done_tx, stderr_done) = watch::channel(false);
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line: String = line.trim_end().chars().take(STDERR_LINE_CHARS).collect();
            if line.is_empty() {
                continue;
            }
            let mut kept = lock(&kept);
            if kept.len() == STDERR_LINES {
                kept.pop_front();
            }
            kept.push_back(line);
        }
        let _ = done_tx.send(true);
    });
    Ok((
        Box::new(stdout),
        Box::new(stdin),
        Process {
            child,
            stderr: tail,
            stderr_task,
            stderr_done,
        },
    ))
}

/// Starts the reader and the writer, then the handshake.
async fn open(
    read: BoxRead,
    mut write: BoxWrite,
    framing: Framing,
    process: Option<Process>,
    options: Options,
) -> Result<(Client, Events), Error> {
    let mut read = BufReader::with_capacity(64 * 1024, read);
    let (mut child, stderr, stderr_done, mut tasks) = match process {
        Some(process) => (
            Some(process.child),
            process.stderr,
            Some(process.stderr_done),
            Tasks {
                writer: None,
                others: vec![process.stderr_task],
            },
        ),
        None => (
            None,
            Arc::new(Mutex::new(VecDeque::new())),
            None,
            Tasks::default(),
        ),
    };
    if framing == Framing::WebSocket {
        let upgraded = tokio::time::timeout(
            options.initialize_timeout,
            websocket::handshake(&mut read, &mut write),
        )
        .await;
        let failure = match upgraded {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e.to_string()),
            Err(_) => Some(format!(
                "no answer to the WebSocket upgrade within {} s",
                options.initialize_timeout.as_secs_f32()
            )),
        };
        if let Some(failure) = failure {
            if let Some(child) = child.as_mut() {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
            if let Some(mut done) = stderr_done {
                let _ = tokio::time::timeout(STDERR_SETTLE, done.wait_for(|done| *done)).await;
            }
            tasks.abort();
            let said: Vec<String> = lock(&stderr).iter().cloned().collect();
            return Err(Error::Handshake(if said.is_empty() {
                failure
            } else {
                format!("{failure} (it said: {})", said.join(" / "))
            }));
        }
    }

    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let shared = Arc::new(Shared {
        out: out_tx,
        events: events_tx,
        pending: Mutex::new(Some(HashMap::new())),
        next_id: AtomicI64::new(0),
        closed: Mutex::new(None),
        reported: Mutex::new(HashSet::new()),
        version: Mutex::new(None),
        stderr,
        stderr_done,
        options,
    });
    tasks.writer = Some(tokio::spawn(write_loop(write, framing, out_rx)));
    tasks
        .others
        .push(tokio::spawn(read_loop(read, framing, Arc::clone(&shared))));
    let tasks = Mutex::new(tasks);

    match handshake(&shared).await {
        Ok(server) => Ok((
            Client {
                shared,
                server,
                child: tokio::sync::Mutex::new(child),
                tasks,
            },
            Events {
                rx: events_rx,
                ended: false,
            },
        )),
        Err(error) => {
            stop(&shared, &mut child, &tasks, shared.options.shutdown_grace).await;
            Err(error)
        }
    }
}

/// `initialize`, the version gate, `initialized` (§4.3).
async fn handshake(shared: &Shared) -> Result<Server, Error> {
    let options = &shared.options;
    let params = InitializeParams {
        client_info: ClientInfo {
            name: options.client_name.clone(),
            title: None,
            version: options.client_version.clone(),
        },
        // The stable surface only: subset.json's `"experimental": false`.
        capabilities: Some(InitializeCapabilities {
            experimental_api: Some(false),
            opt_out_notification_methods: (!options.opt_out_notifications.is_empty())
                .then(|| options.opt_out_notifications.clone()),
            ..InitializeCapabilities::default()
        }),
    };
    let info = shared
        .request::<requests::Initialize>(params, options.initialize_timeout)
        .await?;
    let verdict = match gate::version_from_user_agent(&info.user_agent) {
        Some(version) => gate::check_version(version),
        None => Ok(Verdict::Unknown(info.user_agent.clone())),
    }
    .map_err(Error::Unsupported)?;
    *lock(&shared.version) = Some(match verdict.version() {
        Some(version) => version.to_string(),
        None => "of an unknown version".into(),
    });
    match &verdict {
        Verdict::Supported(_) => {}
        Verdict::Newer(version) => shared.report_once("newer".into(), || {
            format!(
                "Codex {version} is newer than {}, the release Teskad's protocol types come \
                 from: what changed is read leniently",
                gate::CODEX_PINNED
            )
        }),
        Verdict::Unknown(user_agent) => shared.report_once("unknown version".into(), || {
            format!(
                "could not read Codex's version from its user agent `{user_agent}`: spoken to as \
                 a release newer than {}",
                gate::CODEX_PINNED
            )
        }),
    }
    shared.notify::<notifications::Initialized>(())?;
    Ok(Server { info, verdict })
}

/// Ends a connection (once): see [`Client::shutdown`].
async fn stop(
    shared: &Shared,
    child: &mut Option<Child>,
    tasks: &Mutex<Tasks>,
    grace: Duration,
) -> Option<ExitStatus> {
    shared.close(Cause::Shutdown);
    let _ = shared.out.send(Outgoing::Finish);
    let mut writer = lock(tasks).writer.take();
    let status = match child.as_mut() {
        // Its input ended: it stops by itself, or is killed.
        Some(child) => match tokio::time::timeout(grace, child.wait()).await {
            Ok(Ok(status)) => Some(status),
            _ => {
                let _ = child.start_kill();
                child.wait().await.ok()
            }
        },
        // A socket: the writer closes the WebSocket and Hangar's side, then it is let go of.
        None => {
            if let Some(writer) = writer.as_mut() {
                let _ = tokio::time::timeout(grace, writer).await;
            }
            None
        }
    };
    if let Some(writer) = writer {
        writer.abort();
    }
    lock(tasks).abort();
    status
}

async fn write_loop(
    mut write: BoxWrite,
    framing: Framing,
    mut rx: mpsc::UnboundedReceiver<Outgoing>,
) {
    while let Some(outgoing) = rx.recv().await {
        let bytes = match (outgoing, framing) {
            (Outgoing::Message(text), Framing::Lines) => {
                let mut bytes = text.into_bytes();
                bytes.push(b'\n');
                bytes
            }
            (Outgoing::Message(text), Framing::WebSocket) => {
                websocket::frame(websocket::TEXT, text.as_bytes())
            }
            (Outgoing::Pong(payload), Framing::WebSocket) => {
                websocket::frame(websocket::PONG, &payload)
            }
            (Outgoing::Pong(_), Framing::Lines) => continue,
            (Outgoing::Finish, framing) => {
                if framing == Framing::WebSocket {
                    let close = websocket::frame(websocket::CLOSE, &1000u16.to_be_bytes());
                    let _ = write.write_all(&close).await;
                }
                let _ = write.shutdown().await;
                return;
            }
        };
        if write.write_all(&bytes).await.is_err() || write.flush().await.is_err() {
            return;
        }
    }
    let _ = write.shutdown().await;
}

async fn read_loop(mut read: BufReader<BoxRead>, framing: Framing, shared: Arc<Shared>) {
    let mut line = Vec::new();
    let mut frames = websocket::Reader::default();
    let cause = loop {
        let bytes = match framing {
            Framing::Lines => {
                line.clear();
                match websocket::read_line(&mut read, &mut line, websocket::MAX_MESSAGE).await {
                    Ok(Line::Read) => {
                        if line.iter().all(u8::is_ascii_whitespace) {
                            continue;
                        }
                        std::mem::take(&mut line)
                    }
                    Ok(Line::TooLong) => {
                        shared.report_once("too long".into(), || {
                            format!(
                                "dropped a message from Codex larger than {} MiB",
                                websocket::MAX_MESSAGE >> 20
                            )
                        });
                        continue;
                    }
                    Ok(Line::Eof) => break Cause::ServerClosed,
                    Err(e) => break Cause::Io(e.to_string()),
                }
            }
            Framing::WebSocket => match frames.next(&mut read).await {
                Ok(websocket::Received::Text(bytes)) => bytes,
                Ok(websocket::Received::Ping(payload)) => {
                    let _ = shared.out.send(Outgoing::Pong(payload));
                    continue;
                }
                Ok(websocket::Received::Skipped(why)) => {
                    shared.report_once(format!("skipped {why}"), || {
                        format!("dropped {why} from Codex")
                    });
                    continue;
                }
                Ok(websocket::Received::Closed) => break Cause::ServerClosed,
                Err(e) => break Cause::Io(e.to_string()),
            },
        };
        shared.dispatch(&bytes);
    };
    if cause == Cause::ServerClosed {
        // A server that went closed its stderr too: wait for its last words.
        if let Some(mut done) = shared.stderr_done.clone() {
            let _ = tokio::time::timeout(STDERR_SETTLE, done.wait_for(|done| *done)).await;
        }
    }
    shared.close(cause);
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The start of a message, for a report.
fn preview(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out: String = text.chars().take(200).collect();
    if text.chars().count() > 200 {
        out.push('…');
    }
    out
}
