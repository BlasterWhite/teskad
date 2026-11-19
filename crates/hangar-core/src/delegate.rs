//! `delegate`: a sub-task handed to an agent that runs with no terminal, and its result handed
//! back to the pane that asked (docs/native/agents.md §5.2, D14; the headless child, §4.3).
//!
//! A pane's agent asks for one with the `delegate` tool (task T17 exposes this module as MCP
//! tools): [`start`] records a [`Delegation`] — `d-1`, `d-2`… for the life of Hangar — prepares
//! where its child works and starts the child; [`status`] says where it is, waiting up to
//! [`WAIT_MAX`] for it to end; [`cancel`] stops it; [`list`] lists them all.
//!
//! - **The child** is picked by what its kind can do without a terminal (`Headless`), never by
//!   kind ([`runner_for`]): one that serves a protocol (`app_server`) runs as a thread of a server
//!   of its own — Codex's `codex app-server` on stdio (delegate/codex.rs); one that runs one
//!   prompt to its answer (`one_shot`) runs as such a run, streamed both ways — Claude Code's
//!   `claude -p` in stream-json, Hangar the host of its permission prompts (delegate/claude.rs).
//! - **Permissions**: the workspace's orchestration policy ([`Request::permission_mode`]), as the
//!   child's kind enforces it; a policy the kind does not enforce steps down to the next stricter
//!   one it does ([`policy_for`]). Hangar never imitates a policy by answering for the person.
//! - **Asks**: what the child asks the person — approvals, questions — is held as an ask of the
//!   delegation's own id (`bridge::held_asks("d-3")`), as a pane's are held under the pane's, and
//!   answered by the person only; the delegation says `needs_you` meanwhile.
//! - **Isolation** ([`Isolation`]): the caller's folder, or a worktree and branch of their own
//!   made from the caller's last commit (delegate/worktree.rs) — removed when the delegation is
//!   cancelled, unless it holds commits.
//! - **Result**: the child's final message, the files it changed, the images it generated, its
//!   branch; its model and tokens for the accounting.
//! - **Coming back**: when the child ends, however it ends, an event goes to the caller's queue
//!   (`bridge::enqueue_from`), from Hangar: "Delegation d-3 (codex) finished: …", which the
//!   caller's side channel labels as Hangar's on its way in.

mod claude;
mod codex;
mod worktree;

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::agent::{AgentKind, PermissionMode};
use crate::bridge;
use crate::codex_app::{Client, Error, Events, Options};
use crate::host::Host;

/// The longest [`status`] waits: an MCP call must answer before the agent's own limit (Codex's
/// `tool_timeout_sec` is 60 s by default, and Claude Code's may be as short), so a wait never
/// outlives one (D14).
pub const WAIT_MAX: Duration = Duration::from_secs(45);
/// How long [`cancel`] gives a child to stop before it answers anyway.
const CANCEL_WAIT: Duration = Duration::from_secs(15);
/// Who Hangar's own events come from (`bridge::Queued::from`).
const HANGAR: &str = "hangar";
/// Past this, the result an event carries is cut: [`status`] gives it whole.
const EVENT_RESULT_CHARS: usize = 4_000;
/// The longest a delegation's task reads in a listing.
const TITLE_CHARS: usize = 120;
/// Delegations kept for [`list`] and [`status`]; past it, the oldest that ended go.
const KEPT: usize = 256;

/// Where a child works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Isolation {
    /// In the caller's folder, beside the caller: work that reads more than it writes.
    #[default]
    Shared,
    /// In a worktree and branch of its own, made from the caller's last commit (what the caller
    /// has not committed is not there).
    Worktree,
}

/// Where a delegation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    /// Its child waits on the person: an ask of the delegation's id is held.
    NeedsYou,
    Done,
    Failed,
    Cancelled,
}

impl Status {
    /// It ended: nothing about it changes any more.
    pub fn is_over(self) -> bool {
        matches!(self, Status::Done | Status::Failed | Status::Cancelled)
    }
}

/// What a pane asks to have done.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Request {
    /// The caller's pane: where the event goes when the child ends.
    pub parent: String,
    /// The caller's folder: where a shared child works, and what a worktree is made from.
    pub cwd: String,
    /// The whole brief: the child sees nothing else.
    pub task: String,
    /// The child's agent (the tool's default, the caller's kind, is T17's to fill in).
    pub kind: AgentKind,
    /// Its model; absent, the agent's own default.
    pub model: Option<String>,
    pub isolation: Isolation,
    /// It may generate images (a Codex child: `features.image_generation`, D15).
    pub images: bool,
    /// The caller's workspace's orchestration policy; absent, the person's own configuration.
    pub permission_mode: Option<PermissionMode>,
    /// The account of the child's kind the caller's workspace runs on (an id of accounts.json);
    /// absent, main.
    pub account: Option<String>,
    /// The kind's launch command from the settings, when customised.
    pub command: Option<String>,
}

/// What a child spent, as its agent counts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub input: u64,
    /// Of `input`, read from the cache.
    pub cached_input: u64,
    pub output: u64,
    /// Of `output`, spent reasoning.
    pub reasoning_output: u64,
    pub total: u64,
}

/// One delegation, as [`status`] and [`list`] give it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delegation {
    /// `d-<n>`.
    pub id: String,
    /// The caller's pane.
    pub parent: String,
    pub kind: AgentKind,
    /// The model asked for, then the one the child runs on once it says.
    pub model: Option<String>,
    pub status: Status,
    /// The first line of its task.
    pub title: String,
    /// When it started and ended, in milliseconds since the Unix epoch.
    pub started: u64,
    pub ended: Option<u64>,
    /// What it spent so far.
    pub tokens: Option<Tokens>,
    pub isolation: Isolation,
    /// The folder the child works in: the caller's, or its worktree.
    pub cwd: String,
    /// The child's session (a Codex thread id, a Claude Code session id), once started.
    pub session: Option<String>,
    /// Its final message.
    pub result: Option<String>,
    /// The files it changed, as absolute paths.
    pub files: Vec<String>,
    /// The images it generated, where its agent saved them.
    pub images: Vec<String>,
    /// Its worktree's branch, where its commits are; none once that worktree was removed.
    pub branch: Option<String>,
    /// Why it failed, or what stopped it.
    pub error: Option<String>,
}

/// [`status`]'s answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Polled {
    pub delegation: Delegation,
    /// The wait asked for ran out with the child still running.
    pub waited_out: bool,
}

// ---------------------------------------------------------------------------
// The registry
// ---------------------------------------------------------------------------

struct Entry {
    delegation: Delegation,
    /// Tells its child to stop; none once it ended.
    stop: Option<watch::Sender<bool>>,
    /// [`cancel`] was asked: its worktree goes, unless it holds commits.
    cancelled: bool,
}

#[derive(Default)]
struct Registry {
    by_number: BTreeMap<u64, Entry>,
}

impl Registry {
    fn get(&self, id: &str) -> Option<&Entry> {
        self.by_number.get(&number_of(id)?)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Entry> {
        self.by_number.get_mut(&number_of(id)?)
    }

    fn insert(&mut self, number: u64, entry: Entry) {
        self.by_number.insert(number, entry);
        while self.by_number.len() > KEPT {
            let oldest_over = self
                .by_number
                .iter()
                .find(|(_, entry)| entry.delegation.status.is_over())
                .map(|(number, _)| *number);
            match oldest_over {
                Some(number) => self.by_number.remove(&number),
                None => break,
            };
        }
    }
}

fn registry() -> MutexGuard<'static, Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Counts every change of any delegation, for the waits of [`status`] and [`cancel`].
fn changes() -> &'static watch::Sender<u64> {
    static CHANGES: OnceLock<watch::Sender<u64>> = OnceLock::new();
    CHANGES.get_or_init(|| watch::channel(0).0)
}

fn changed() {
    changes().send_modify(|count| *count = count.wrapping_add(1));
}

static NEXT: AtomicU64 = AtomicU64::new(1);

fn number_of(id: &str) -> Option<u64> {
    id.strip_prefix("d-")?.parse().ok()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// A running delegation, as its child's runner sees it: what it reports goes into the registry.
#[derive(Clone)]
struct Live {
    host: Host,
    id: String,
}

impl Live {
    fn update(&self, change: impl FnOnce(&mut Delegation)) {
        if let Some(entry) = registry().get_mut(&self.id) {
            change(&mut entry.delegation);
        }
        changed();
    }

    /// Its child waits on the person, or no longer does.
    fn needs_you(&self, waiting: bool) {
        self.update(|delegation| {
            if !delegation.status.is_over() {
                delegation.status = if waiting {
                    Status::NeedsYou
                } else {
                    Status::Running
                };
            }
        });
    }
}

/// How a child ended, as its runner tells it.
struct Outcome {
    status: Status,
    result: Option<String>,
    files: Vec<String>,
    images: Vec<String>,
    error: Option<String>,
}

/// A connection to a Codex child's server, past its handshake: spawned on stdio, or a test's.
type Connect = Box<
    dyn FnOnce(Options) -> Pin<Box<dyn Future<Output = Result<(Client, Events), Error>> + Send>>
        + Send,
>;

/// How a delegation's child runs, by what its kind can do without a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Runner {
    /// A thread of a protocol server of its own (delegate/codex.rs).
    AppServer,
    /// One prompt run to its answer, streamed both ways (delegate/claude.rs).
    OneShot,
}

/// The runner of a child of `kind`: its protocol server when it serves one, else its one-shot
/// run; none when it has neither.
fn runner_for(kind: AgentKind) -> Option<Runner> {
    let headless = kind.info().caps.headless;
    if headless.app_server {
        Some(Runner::AppServer)
    } else if headless.one_shot {
        Some(Runner::OneShot)
    } else {
        None
    }
}

/// A child ready to start: the connection to its server, or its run's stdio — spawned, or a
/// test's.
enum Child {
    AppServer(Connect),
    OneShot(claude::Open),
}

// ---------------------------------------------------------------------------
// The calls
// ---------------------------------------------------------------------------

/// Starts a delegation: checks the request, prepares the child's folder, starts the child, and
/// answers at once with the delegation running. An error says why nothing was started.
pub async fn start(host: &Host, request: Request) -> Result<Delegation, String> {
    begin(host, request, None).await
}

/// The delegation `id` now — or, given a `wait`, once it no longer runs (it ended, or waits on the
/// person), waiting at most that long and never more than [`WAIT_MAX`]. `None` for an id Hangar
/// does not know.
pub async fn status(id: &str, wait: Option<Duration>) -> Option<Polled> {
    let wait = capped(wait);
    let deadline = tokio::time::Instant::now() + wait;
    let mut seen = changes().subscribe();
    loop {
        seen.borrow_and_update();
        let delegation = get(id)?;
        if delegation.status != Status::Running || wait.is_zero() {
            return Some(Polled {
                delegation,
                waited_out: false,
            });
        }
        if tokio::time::timeout_at(deadline, seen.changed())
            .await
            .is_err()
        {
            let delegation = get(id)?;
            let waited_out = delegation.status == Status::Running;
            return Some(Polled {
                delegation,
                waited_out,
            });
        }
    }
}

/// Stops delegation `id`: its child's turn is interrupted and its process stopped, what it asked
/// the person is withdrawn, and its worktree removed unless it holds commits. Answers once it
/// stopped (or after a few seconds, still stopping); one that had ended already is left as it
/// is.
pub async fn cancel(id: &str) -> Result<Delegation, String> {
    let stop = {
        let mut registry = registry();
        let entry = registry
            .get_mut(id)
            .ok_or_else(|| format!("there is no delegation {id}"))?;
        if entry.delegation.status.is_over() {
            return Ok(entry.delegation.clone());
        }
        entry.cancelled = true;
        entry.stop.clone()
    };
    if let Some(stop) = stop {
        let _ = stop.send(true);
    }
    let deadline = tokio::time::Instant::now() + CANCEL_WAIT;
    let mut seen = changes().subscribe();
    loop {
        seen.borrow_and_update();
        let delegation = get(id).ok_or_else(|| format!("there is no delegation {id}"))?;
        if delegation.status.is_over()
            || tokio::time::timeout_at(deadline, seen.changed())
                .await
                .is_err()
        {
            return get(id).ok_or_else(|| format!("there is no delegation {id}"));
        }
    }
}

/// Every delegation Hangar knows, oldest first.
pub fn list() -> Vec<Delegation> {
    registry()
        .by_number
        .values()
        .map(|entry| entry.delegation.clone())
        .collect()
}

fn get(id: &str) -> Option<Delegation> {
    registry().get(id).map(|entry| entry.delegation.clone())
}

/// A wait as [`status`] keeps it: none is none, and none longer than [`WAIT_MAX`].
fn capped(wait: Option<Duration>) -> Duration {
    wait.unwrap_or(Duration::ZERO).min(WAIT_MAX)
}

/// [`start`], with a test's fake child in place of a spawned one.
async fn begin(host: &Host, request: Request, child: Option<Child>) -> Result<Delegation, String> {
    let task = request.task.trim().to_string();
    if task.is_empty() {
        return Err("a delegation needs a task".into());
    }
    if request.parent.trim().is_empty() {
        return Err("a delegation needs the pane it reports to".into());
    }
    if !Path::new(&request.cwd).is_dir() {
        return Err(format!("{} is not a folder", request.cwd));
    }
    let Some(runner) = runner_for(request.kind) else {
        return Err(format!(
            "Hangar cannot run a delegation on {} without a terminal",
            request.kind.info().label
        ));
    };
    let mode = policy_for(request.kind, request.permission_mode)?;

    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let id = format!("d-{number}");
    let made = match request.isolation {
        Isolation::Shared => None,
        Isolation::Worktree => Some(worktree::make(&request.cwd, &id).await?),
    };
    let cwd = made
        .as_ref()
        .map_or_else(|| request.cwd.clone(), |made| made.path.clone());
    let child = match child {
        Some(child) => child,
        None => {
            let spawn = match runner {
                Runner::AppServer => codex::spawn_spec(&request, &cwd, mode, made.as_ref())
                    .map(|spawn| Child::AppServer(codex::stdio(spawn))),
                Runner::OneShot => claude::spawn_spec(&request, &cwd, mode, made.as_ref())
                    .map(|spawn| Child::OneShot(claude::stdio(spawn))),
            };
            match spawn {
                Ok(child) => child,
                Err(error) => {
                    if let Some(made) = &made {
                        let _ = worktree::remove(made).await;
                    }
                    return Err(error);
                }
            }
        }
    };

    let delegation = Delegation {
        id: id.clone(),
        parent: request.parent.clone(),
        kind: request.kind,
        model: request
            .model
            .clone()
            .filter(|model| !model.trim().is_empty()),
        status: Status::Running,
        title: title_of(&task),
        started: now_ms(),
        ended: None,
        tokens: None,
        isolation: request.isolation,
        cwd: cwd.clone(),
        session: None,
        result: None,
        files: Vec::new(),
        images: Vec::new(),
        branch: made.as_ref().map(|made| made.branch.clone()),
        error: None,
    };
    let (stop, stopped) = watch::channel(false);
    registry().insert(
        number,
        Entry {
            delegation: delegation.clone(),
            stop: Some(stop),
            cancelled: false,
        },
    );
    changed();

    let live = Live {
        host: host.clone(),
        id: id.clone(),
    };
    tokio::spawn(async move {
        let outcome = match child {
            Child::AppServer(connect) => {
                codex::run(live.clone(), connect, task, cwd, stopped).await
            }
            Child::OneShot(open) => claude::run(live.clone(), open, task, cwd, stopped).await,
        };
        finish(&live, outcome, made).await;
    });
    Ok(delegation)
}

/// The orchestration policies from the most leeway to the least: bypass every check, let the
/// agent's own reviewer approve, ask before anything beyond editing.
const STRICTER: [PermissionMode; 3] = [
    PermissionMode::BypassPermissions,
    PermissionMode::Auto,
    PermissionMode::AcceptEdits,
];

/// The policy a child of `kind` runs under for the workspace's `policy`: the same, or the next
/// stricter one the kind enforces. An error when it enforces none as strict: the child does not
/// run at all rather than with more leeway than the workspace gives its agents.
fn policy_for(
    kind: AgentKind,
    policy: Option<PermissionMode>,
) -> Result<Option<PermissionMode>, String> {
    let Some(policy) = policy else {
        return Ok(None);
    };
    step_down(policy, kind.info().caps.permission_modes)
        .map(Some)
        .ok_or_else(|| {
            format!(
                "{} cannot run under the workspace's permission policy ({})",
                kind.info().label,
                policy.id()
            )
        })
}

/// `policy`, or the next stricter one of `enforced`.
fn step_down(policy: PermissionMode, enforced: &[PermissionMode]) -> Option<PermissionMode> {
    STRICTER
        .iter()
        .skip_while(|mode| **mode != policy)
        .find(|mode| enforced.contains(mode))
        .copied()
}

/// The first line of a task, as a listing shows it.
fn title_of(task: &str) -> String {
    let line = task
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    cut(line, TITLE_CHARS)
}

fn cut(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(chars.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The child ended: what its worktree holds is added to its files, the worktree goes when the
/// delegation was cancelled and holds no commits, the registry learns how it ended, and the
/// caller hears of it.
async fn finish(live: &Live, outcome: Outcome, made: Option<worktree::Made>) {
    let Outcome {
        status,
        result,
        mut files,
        images,
        mut error,
    } = outcome;
    let cancelled = registry()
        .get(&live.id)
        .is_some_and(|entry| entry.cancelled);
    let mut branch = made.as_ref().map(|made| made.branch.clone());
    let mut removed = None;
    if let Some(made) = &made {
        for file in worktree::changed(made).await {
            if !files.contains(&file) {
                files.push(file);
            }
        }
        if status == Status::Cancelled && cancelled {
            match worktree::remove(made).await {
                Ok(true) => {
                    branch = None;
                    removed = Some(true);
                }
                Ok(false) => removed = Some(false),
                Err(why) => {
                    let note = format!("its worktree could not be removed: {why}");
                    error = Some(match error {
                        Some(error) => format!("{error}; {note}"),
                        None => note,
                    });
                }
            }
        } else {
            worktree::release(made).await;
        }
    }
    {
        let mut registry = registry();
        let Some(entry) = registry.get_mut(&live.id) else {
            return;
        };
        let delegation = &mut entry.delegation;
        delegation.status = status;
        delegation.ended = Some(now_ms());
        delegation.result = result;
        delegation.files = files;
        delegation.images = images;
        delegation.branch = branch;
        delegation.error = error;
        entry.stop = None;
        // Queued before the registry is let go of: whoever sees it ended finds the event there.
        bridge::enqueue_from(
            &live.host,
            &delegation.parent,
            event_text(delegation, removed),
            Some(HANGAR.into()),
        );
    }
    changed();
}

/// What the caller reads when its delegation ends. `removed`: whether a cancelled delegation's
/// worktree went (it held no commits) or stayed (it did).
fn event_text(delegation: &Delegation, removed: Option<bool>) -> String {
    let who = format!("Delegation {} ({})", delegation.id, delegation.kind.id());
    let mut text = match delegation.status {
        Status::Done => format!(
            "{who} finished: {}",
            delegation
                .result
                .as_deref()
                .map(|result| cut(result.trim(), EVENT_RESULT_CHARS))
                .filter(|result| !result.is_empty())
                .unwrap_or_else(|| "it ended without a final message.".into())
        ),
        Status::Failed => format!(
            "{who} failed: {}",
            delegation.error.as_deref().unwrap_or("its agent stopped.")
        ),
        _ => match &delegation.error {
            Some(why) => format!("{who} was cancelled: {why}"),
            None => format!("{who} was cancelled."),
        },
    };
    if delegation.status != Status::Done {
        if let Some(result) = delegation
            .result
            .as_deref()
            .filter(|r| !r.trim().is_empty())
        {
            text.push_str(&format!(
                "\n\nIts last message: {}",
                cut(result.trim(), EVENT_RESULT_CHARS)
            ));
        }
    }
    if !delegation.files.is_empty() {
        text.push_str(&format!(
            "\n\nFiles changed: {}",
            delegation.files.join(", ")
        ));
    }
    if !delegation.images.is_empty() {
        text.push_str(&format!("\n\nImages: {}", delegation.images.join(", ")));
    }
    match (&delegation.branch, removed) {
        (_, Some(true)) => text.push_str("\n\nIts worktree was removed: it held no commits."),
        (Some(branch), _) => text.push_str(&format!(
            "\n\nIts work is on branch {branch}, in {}.",
            delegation.cwd
        )),
        (None, _) => {}
    }
    text
}

#[cfg(test)]
mod tests;
