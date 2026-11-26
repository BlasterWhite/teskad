//! What happens in a workspace that its orchestrator should hear about
//! (`lib/orchestratorEvents.ts`), and the engine that tells it.
//!
//! Events come from two sides: the code that causes them posts them (worker
//! starts and stops, deliveries), and the engine makes the rest out of what it
//! watches — the board of every workspace that has an orchestrator, read again
//! whenever an agent changes a board and on every tick (the window's own edits
//! announce nothing), and the activity of its workers.
//!
//! Nothing is sent as it happens. A workspace's events wait a few seconds for
//! the ones that come with them, and for the orchestrator to be between turns;
//! then they go as one message, read again just before — a worker that stopped
//! waiting in the meantime is no longer news — deduplicated, ranked, capped.
//! Only what is urgent goes that fast (`is_urgent`): the rest — plain
//! comments, idle workers, tasks unblocked — waits for a digest, at most every
//! few minutes, which an urgent event takes along when it goes first.
//! What the orchestrator did itself, it is not told about. Everything, told or
//! not, is kept in a short journal its briefing hands back after a compaction.
//! A workspace without an orchestrator is never looked at.
//!
//! A worker is called idle once, when it ended a turn without delivering a
//! while ago and has nothing of its own running — a build, a benchmark, a
//! watch on its CI it waits on (`EngineIo::pane_job`) — and again only once it
//! did something since (a comment, a progress report, a commit, a delivery) or
//! after a long while: the orchestrator nudging it is not the worker doing
//! something.
//!
//! The engine is a plain state machine: every input takes the time it happens
//! at, timers are deadlines it keeps (`next_wake`, `fire_due`), and what it
//! reads or sends goes through `EngineIo`. The runtime (runtime.rs) drives it
//! with the real clock; the tests below drive it with a fake one, as the TS
//! tests did with their fake timers. Board reads are synchronous here, so the
//! TS's `reading`/`readAgain` guard against two reads at once has nothing left
//! to guard.

use hangar_core::instructions::APP_NAME;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use hangar_core::board::Task;
use hangar_core::pty::PaneJob;

use super::text::{MAX_TEXT, is_hangar_author, minutes, one_line, same_name, task_ref};
use crate::live::{BridgeEvent, TurnPhase};
use crate::state::roles::{NEEDS_YOU_LABEL, orchestrator_of, workers_of};
use crate::state::types::{AppState, Pane, PaneId, Workspace, WsId};
use crate::state::{AgentActivity, PaneActivity};

/// How a delivery ended, as the orchestrator is told it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryOutcome {
    Landed,
    /// A worker in a workspace that is not a git repository: nothing to merge.
    Done,
    Ready,
    Conflict,
    CheckFailed,
    ChangesRequested,
    Dirty,
    Stale,
    Error,
}

/// One thing that happened in a workspace (`OrchestratorEvent`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OrchestratorEvent {
    #[serde(rename_all = "camelCase")]
    WorkerStarted {
        worker: String,
        task_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    WorkerStopped {
        worker: String,
        task_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    WorkerExited {
        worker: String,
        task_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    WorkerWaiting {
        worker: String,
        task_id: Option<String>,
        for_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    WorkerIdle {
        worker: String,
        task_id: Option<String>,
        for_ms: u64,
        /// A command it started that still runs in its pane after
        /// `JOB_HOLD_MS`: it no longer holds the alert back.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        job: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    TaskMoved {
        task_id: String,
        title: String,
        from: String,
        to: String,
        by: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    TaskCommented {
        task_id: String,
        title: String,
        by: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    TaskNeedsYou {
        task_id: String,
        title: String,
        reason: String,
    },
    #[serde(rename_all = "camelCase")]
    TaskBlocked {
        task_id: String,
        title: String,
        by: String,
        reason: String,
        human_only: bool,
    },
    #[serde(rename_all = "camelCase")]
    TaskUnblocked {
        task_id: String,
        title: String,
        how: String,
    },
    #[serde(rename_all = "camelCase")]
    TaskRecheckDue {
        task_id: String,
        title: String,
        reason: String,
    },
    #[serde(rename_all = "camelCase")]
    Delivery {
        task_id: String,
        title: String,
        worker: String,
        outcome: DeliveryOutcome,
        detail: String,
    },
}

/// The kinds, for the comparisons `settle` makes between events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    WorkerStarted,
    WorkerStopped,
    WorkerExited,
    WorkerWaiting,
    WorkerIdle,
    TaskMoved,
    TaskCommented,
    TaskNeedsYou,
    TaskBlocked,
    TaskUnblocked,
    TaskRecheckDue,
    Delivery,
}

impl OrchestratorEvent {
    pub fn kind(&self) -> EventKind {
        match self {
            OrchestratorEvent::WorkerStarted { .. } => EventKind::WorkerStarted,
            OrchestratorEvent::WorkerStopped { .. } => EventKind::WorkerStopped,
            OrchestratorEvent::WorkerExited { .. } => EventKind::WorkerExited,
            OrchestratorEvent::WorkerWaiting { .. } => EventKind::WorkerWaiting,
            OrchestratorEvent::WorkerIdle { .. } => EventKind::WorkerIdle,
            OrchestratorEvent::TaskMoved { .. } => EventKind::TaskMoved,
            OrchestratorEvent::TaskCommented { .. } => EventKind::TaskCommented,
            OrchestratorEvent::TaskNeedsYou { .. } => EventKind::TaskNeedsYou,
            OrchestratorEvent::TaskBlocked { .. } => EventKind::TaskBlocked,
            OrchestratorEvent::TaskUnblocked { .. } => EventKind::TaskUnblocked,
            OrchestratorEvent::TaskRecheckDue { .. } => EventKind::TaskRecheckDue,
            OrchestratorEvent::Delivery { .. } => EventKind::Delivery,
        }
    }

    /// The task it is about, when it is about one.
    pub fn task_id(&self) -> Option<&str> {
        match self {
            OrchestratorEvent::WorkerStarted { task_id, .. }
            | OrchestratorEvent::WorkerStopped { task_id, .. }
            | OrchestratorEvent::WorkerExited { task_id, .. }
            | OrchestratorEvent::WorkerWaiting { task_id, .. }
            | OrchestratorEvent::WorkerIdle { task_id, .. } => task_id.as_deref(),
            OrchestratorEvent::TaskMoved { task_id, .. }
            | OrchestratorEvent::TaskCommented { task_id, .. }
            | OrchestratorEvent::TaskNeedsYou { task_id, .. }
            | OrchestratorEvent::TaskBlocked { task_id, .. }
            | OrchestratorEvent::TaskUnblocked { task_id, .. }
            | OrchestratorEvent::TaskRecheckDue { task_id, .. }
            | OrchestratorEvent::Delivery { task_id, .. } => Some(task_id),
        }
    }

    /// The worker it names, when it names one.
    pub fn worker(&self) -> Option<&str> {
        match self {
            OrchestratorEvent::WorkerStarted { worker, .. }
            | OrchestratorEvent::WorkerStopped { worker, .. }
            | OrchestratorEvent::WorkerExited { worker, .. }
            | OrchestratorEvent::WorkerWaiting { worker, .. }
            | OrchestratorEvent::WorkerIdle { worker, .. }
            | OrchestratorEvent::Delivery { worker, .. } => Some(worker),
            _ => None,
        }
    }
}

/// One event as the journal keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Milliseconds since the epoch.
    pub at: u64,
    pub event: OrchestratorEvent,
    /// Kept for the briefing, never woken for: the orchestrator caused it.
    pub quiet: bool,
}

/// The engine's pace (`TIMING`), in milliseconds.
pub mod timing {
    /// Quiet this long after the last event, the batch goes.
    pub const DEBOUNCE_MS: u64 = 5_000;
    /// …unless events keep coming: the first one never waits longer.
    pub const MAX_WAIT_MS: u64 = 30_000;
    /// Between two messages to one orchestrator, at least.
    pub const MIN_GAP_MS: u64 = 15_000;
    /// Looked at again this often while the orchestrator is in a turn.
    pub const HOLD_MS: u64 = 3_000;
    /// A worker waiting for the person this long is worth saying.
    pub const WAITING_MS: u64 = 2 * 60_000;
    /// A worker that ended its turn without delivering, this long ago.
    pub const IDLE_MS: u64 = 10 * 60_000;
    /// Told idle, a worker is told so again only after it did something, or
    /// this long after.
    pub const IDLE_AGAIN_MS: u64 = 30 * 60_000;
    /// A worker's pane is looked into for a job of its own at most this
    /// often, and this often while one runs.
    pub const JOB_POLL_MS: u64 = 60_000;
    /// A job of its own holds a worker's idle alert back this long at most
    /// after its turn: past it, the alert goes and names the job.
    pub const JOB_HOLD_MS: u64 = 60 * 60_000;
    /// What is not urgent waits this long after the first of it, at most,
    /// and goes as one digest.
    pub const DIGEST_MS: u64 = 5 * 60_000;
    /// Two events about one task this close are one story.
    pub const NEAR_MS: u64 = 60_000;
    /// A tool call often writes a board twice (claim, then comment): one read.
    pub const BOARD_SETTLE_MS: u64 = 250;
}

use timing::*;

const JOURNAL_SIZE: usize = 50;
const MAX_PENDING: usize = 200;
/// Event lines in one message; the rest is counted, not listed.
const MAX_LINES: usize = 12;
const MAX_DETAIL: usize = 400;
/// Plain comments on one task in one message: from this many on, one line.
const MERGED_FROM: usize = 3;
/// How an idle line goes on when a command of the worker's still runs
/// (`heard` reads the line up to it).
pub const IDLE_JOB: &str = "; a command it started still runs in its pane, over an hour later: ";

/* ── Wording ───────────────────────────────────────────────────── */

fn on_task(task_id: Option<&str>, titles: &dyn Fn(&str) -> Option<String>) -> String {
    match task_id {
        Some(id) if !id.is_empty() => format!(" on {}", task_ref(titles(id).as_deref(), id)),
        _ => String::new(),
    }
}

fn delivery_words(outcome: DeliveryOutcome) -> String {
    match outcome {
        DeliveryOutcome::Landed => "landed: merged on the branch, task done".into(),
        // A worker in a workspace that is not a git repository: nothing to merge.
        DeliveryOutcome::Done => "is in: its changes are in the workspace's folder, task done".into(),
        DeliveryOutcome::Ready => {
            "passed its check and review; it waits for the person to land it".into()
        }
        DeliveryOutcome::Conflict => {
            format!("hit a merge conflict; {APP_NAME} sent the worker what to fix")
        }
        DeliveryOutcome::CheckFailed => {
            format!("failed the check; {APP_NAME} sent the worker the output")
        }
        DeliveryOutcome::ChangesRequested => format!(
            "came back from review with changes requested; {APP_NAME} sent the worker the review"
        ),
        // The worker's worktree or the person's checkout: the detail says which.
        DeliveryOutcome::Dirty => "was held back by uncommitted changes".into(),
        DeliveryOutcome::Stale => format!(
            "was not landed: the worker committed after it was checked and reviewed, so {APP_NAME} asked it to submit again"
        ),
        DeliveryOutcome::Error => "failed".into(),
    }
}

/// One event as a line a model reads: who, what, which task, what it means
/// (`describeEvent`).
pub fn describe_event(
    event: &OrchestratorEvent,
    titles: &dyn Fn(&str) -> Option<String>,
) -> String {
    use OrchestratorEvent as E;
    match event {
        E::Delivery {
            task_id,
            title,
            worker,
            outcome,
            detail,
        } => {
            let detail = if detail.trim().is_empty() {
                String::new()
            } else {
                format!(" — {}", one_line(detail, MAX_DETAIL))
            };
            format!(
                "Delivery of {} by {worker} {}{detail}",
                task_ref(Some(title), task_id),
                delivery_words(*outcome)
            )
        }
        E::TaskNeedsYou {
            task_id,
            title,
            reason,
        } => format!(
            "{} needs the person: {}",
            task_ref(Some(title), task_id),
            one_line(reason, MAX_TEXT)
        ),
        E::TaskBlocked {
            task_id,
            title,
            by,
            reason,
            human_only,
        } => format!(
            "{} blocked {}: {}{}",
            if by == "user" { "The person" } else { by },
            task_ref(Some(title), task_id),
            one_line(reason, MAX_TEXT),
            if *human_only {
                " (only the person can unblock it)"
            } else {
                ""
            }
        ),
        E::TaskUnblocked {
            task_id,
            title,
            how,
        } => format!(
            "{} unblocked: {}",
            task_ref(Some(title), task_id),
            one_line(how, MAX_TEXT)
        ),
        E::TaskRecheckDue {
            task_id,
            title,
            reason,
        } => format!(
            "{}: its recheck day passed and it stays blocked until the person lifts it ({})",
            task_ref(Some(title), task_id),
            one_line(reason, MAX_TEXT)
        ),
        E::TaskCommented {
            task_id,
            title,
            by,
            text,
        } => {
            let who = if by == "user" { "The person" } else { by };
            let title = title.clone();
            format!(
                "{who} commented{}: \"{}\"",
                on_task(Some(task_id), &move |_| Some(title.clone())),
                one_line(text, MAX_TEXT)
            )
        }
        E::TaskMoved {
            task_id,
            title,
            from,
            to,
            by,
        } => format!(
            "{} moved {from} → {to}{}",
            task_ref(Some(title), task_id),
            by.as_ref()
                .map(|by| format!(" (by {by})"))
                .unwrap_or_default()
        ),
        E::WorkerStarted { worker, task_id } => {
            format!("{worker} started{}", on_task(task_id.as_deref(), titles))
        }
        E::WorkerStopped { worker, task_id } => {
            format!(
                "{worker} was stopped{}",
                on_task(task_id.as_deref(), titles)
            )
        }
        E::WorkerExited { worker, task_id } => format!(
            "{worker}'s session ended{}; the task is still assigned to it",
            on_task(task_id.as_deref(), titles)
        ),
        E::WorkerWaiting {
            worker,
            task_id,
            for_ms,
        } => format!(
            "{worker} has been waiting for the person for {}{}: a permission or a question in its pane, which only the person can answer",
            minutes(*for_ms),
            on_task(task_id.as_deref(), titles)
        ),
        E::WorkerIdle {
            worker,
            task_id,
            for_ms,
            job,
        } => format!(
            "{worker} ended its turn {} ago without delivering{}{}",
            minutes(*for_ms),
            on_task(task_id.as_deref(), titles),
            job.as_deref()
                .map(|job| format!("{IDLE_JOB}{}", one_line(job, MAX_TEXT)))
                .unwrap_or_default()
        ),
    }
}

/// Several plain comments on one task, as one line: how many, by whom, and
/// the last one. `comments` are `TaskCommented`, on one task.
fn describe_comments(comments: &[&OrchestratorEvent]) -> String {
    let mut authors: Vec<&str> = Vec::new();
    let mut last: Option<(&str, &str, &str, &str)> = None;
    for event in comments {
        if let OrchestratorEvent::TaskCommented {
            task_id,
            title,
            by,
            text,
        } = event
        {
            if !authors.contains(&by.as_str()) {
                authors.push(by.as_str());
            }
            last = Some((task_id.as_str(), title.as_str(), by.as_str(), text.as_str()));
        }
    }
    let Some((task_id, title, by, text)) = last else {
        return String::new();
    };
    let authors = match authors.split_last() {
        Some((only, [])) => only.to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    };
    format!(
        "{} comments on {} by {authors}; the last, {by}'s: \"{}\" (board_list_tasks has them all)",
        comments.len(),
        task_ref(Some(title), task_id),
        one_line(text, MAX_TEXT)
    )
}

/* ── What cannot wait ──────────────────────────────────────────── */

/// Whether an event goes to the orchestrator within seconds, or waits for
/// the next digest (`DIGEST_MS`): what needs it or the person now does — a
/// delivery, a worker waiting for the person or gone, a task that needs the
/// person, got blocked or is done (others may be ready), a comment of the
/// person's, or one that asks something or says its author is stuck.
pub fn is_urgent(event: &OrchestratorEvent) -> bool {
    use OrchestratorEvent as E;
    match event {
        E::Delivery { .. }
        | E::WorkerWaiting { .. }
        | E::WorkerExited { .. }
        | E::TaskNeedsYou { .. }
        | E::TaskBlocked { .. } => true,
        E::TaskMoved { to, .. } => to == "done",
        E::TaskCommented { by, text, .. } => by == "user" || asks(text),
        E::WorkerStarted { .. }
        | E::WorkerStopped { .. }
        | E::WorkerIdle { .. }
        | E::TaskUnblocked { .. }
        | E::TaskRecheckDue { .. } => false,
    }
}

/// Words that say a comment's author is stuck, in the languages the person
/// and the agents write in.
const STUCK: [&str; 7] = [
    "blocked",
    "blocker",
    "stuck",
    "bloqué",
    "bloque",
    "coincé",
    "can't continue",
];

/// A comment that asks something — a question mark that ends a sentence, not
/// the `?` of some code — or says its author is stuck.
fn asks(text: &str) -> bool {
    let lower = text.to_lowercase();
    let question = lower.char_indices().any(|(at, c)| {
        c == '?'
            && lower[at + 1..].chars().next().is_none_or(|next| {
                next.is_whitespace() || matches!(next, '"' | '\'' | ')' | '*' | '_' | '»')
            })
    });
    question || STUCK.iter().any(|word| lower.contains(word))
}

/* ── Batching ──────────────────────────────────────────────────── */

fn rank(kind: EventKind) -> f64 {
    match kind {
        EventKind::Delivery => 0.0,
        EventKind::TaskNeedsYou => 1.0,
        EventKind::TaskRecheckDue => 1.5,
        EventKind::TaskBlocked => 1.6,
        EventKind::TaskUnblocked => 1.7,
        EventKind::TaskCommented => 2.0,
        EventKind::WorkerExited => 3.0,
        EventKind::WorkerWaiting => 4.0,
        EventKind::WorkerIdle => 5.0,
        EventKind::TaskMoved => 6.0,
        EventKind::WorkerStarted => 7.0,
        EventKind::WorkerStopped => 8.0,
    }
}

/// What a batch is checked against just before it goes (`Live`).
pub trait LiveView {
    /// A worker's activity now, by pane name; `None` once it is gone.
    fn activity(&self, worker: &str) -> Option<AgentActivity>;
    /// A task's column now; `None` when the board does not have it.
    fn column(&self, task_id: &str) -> Option<String>;
}

/// The events of a batch worth sending, in the order they matter: the ones
/// the moment has overtaken left out, the ones a more telling event already
/// covers left out, repeats folded into one (`settle`).
///
/// `journal` is everything recent, sent or not, the batch included: a move to
/// `doing` is told by the worker it started, and the worker's own event may be
/// in an earlier batch. The TS also skipped the entry itself when it looked
/// for a near one (`other !== entry`); every such look is for another kind
/// than the entry's, or strictly earlier, so the entry can never match itself
/// and no identity is needed.
pub fn settle(
    batch: &[JournalEntry],
    journal: &[JournalEntry],
    live: &dyn LiveView,
) -> Vec<OrchestratorEvent> {
    let near =
        |entry: &JournalEntry, kind: EventKind, matches: &dyn Fn(&OrchestratorEvent) -> bool| {
            journal.iter().any(|other| {
                other.event.kind() == kind
                    && other.at.abs_diff(entry.at) <= NEAR_MS
                    && matches(&other.event)
            })
        };

    let kept = batch.iter().filter(|entry| {
        use OrchestratorEvent as E;
        match &entry.event {
            E::WorkerWaiting { worker, .. } => live.activity(worker) == Some(AgentActivity::Waiting),
            E::WorkerIdle { worker, task_id, .. } => {
                let now = live.activity(worker);
                now != Some(AgentActivity::Working)
                    && now != Some(AgentActivity::Waiting)
                    && task_id
                        .as_deref()
                        .is_none_or(|id| live.column(id).as_deref() == Some("doing"))
            }
            E::WorkerExited { worker, task_id } => {
                // Stopped on purpose: worker_stop says so itself.
                if near(entry, EventKind::WorkerStopped, &|e| e.worker() == Some(worker)) {
                    return false;
                }
                // Closed because its delivery landed, or off a task no longer
                // in doing: nothing is left hanging on it.
                let column = task_id.as_deref().and_then(|id| live.column(id));
                column.is_none() || column.as_deref() == Some("doing")
            }
            E::TaskMoved { task_id, to, .. } => {
                let same = |e: &OrchestratorEvent| e.task_id() == Some(task_id.as_str());
                if to == "doing" && near(entry, EventKind::WorkerStarted, &same) {
                    return false;
                }
                // Landed, waiting for the person, or back with its worker: the
                // delivery says so, and why.
                if to != "todo" && near(entry, EventKind::Delivery, &same) {
                    return false;
                }
                if to == "todo" && near(entry, EventKind::WorkerStopped, &same) {
                    return false;
                }
                true
            }
            // Posted by whoever labelled it, then seen again on the board.
            E::TaskNeedsYou { task_id, .. } => !journal.iter().any(|other| {
                other.at < entry.at
                    && matches!(&other.event, E::TaskNeedsYou { task_id: other_id, .. } if other_id == task_id)
                    && entry.at - other.at <= NEAR_MS
            }),
            _ => true,
        }
    });

    let mut out: Vec<OrchestratorEvent> = Vec::new();
    let mut seen: HashSet<&OrchestratorEvent> = HashSet::new();
    // In the order they were first counted, as the TS's Map iterated them.
    let mut comments: Vec<(String, usize)> = Vec::new();
    for entry in kept {
        let event = &entry.event;
        if !seen.insert(event) {
            continue;
        }
        match event {
            OrchestratorEvent::TaskMoved {
                task_id, to, by, ..
            } => {
                // todo → doing → review is one move, todo → review.
                if let Some(first) = out.iter_mut().find(
                    |e| matches!(e, OrchestratorEvent::TaskMoved { task_id: id, .. } if id == task_id),
                ) {
                    if let OrchestratorEvent::TaskMoved { to: first_to, by: first_by, .. } = first {
                        *first_to = to.clone();
                        if first_by != by {
                            *first_by = None;
                        }
                    }
                    continue;
                }
            }
            OrchestratorEvent::WorkerWaiting { worker, .. }
            | OrchestratorEvent::WorkerIdle { worker, .. } => {
                let kind = event.kind();
                if let Some(at) = out
                    .iter()
                    .position(|e| e.kind() == kind && e.worker() == Some(worker.as_str()))
                {
                    out[at] = event.clone();
                    continue;
                }
            }
            OrchestratorEvent::TaskCommented { task_id, by, .. } => {
                let key = format!("{task_id}\n{by}");
                match comments.iter_mut().find(|(known, _)| *known == key) {
                    Some((_, count)) => *count += 1,
                    None => comments.push((key, 1)),
                }
            }
            _ => {}
        }
        out.push(event.clone());
    }
    // A worker writing five comments in a row: its last two say where it is.
    for (key, count) in &comments {
        let mut drop = count.saturating_sub(2);
        let mut i = 0;
        while i < out.len() && drop > 0 {
            if let OrchestratorEvent::TaskCommented { task_id, by, .. } = &out[i]
                && format!("{task_id}\n{by}") == *key
            {
                out.remove(i);
                drop -= 1;
                continue;
            }
            i += 1;
        }
    }
    // "Needs the person: <its last comment>" already quotes that comment.
    let quoted: HashSet<String> = out
        .iter()
        .filter_map(|e| match e {
            OrchestratorEvent::TaskNeedsYou {
                task_id, reason, ..
            } => Some(format!("{task_id}\n{reason}")),
            _ => None,
        })
        .collect();
    let mut out: Vec<OrchestratorEvent> = out
        .into_iter()
        .filter(|e| !matches!(e, OrchestratorEvent::TaskMoved { from, to, .. } if from == to))
        .filter(|e| match e {
            OrchestratorEvent::TaskCommented {
                task_id, by, text, ..
            } => !quoted.contains(&format!("{task_id}\n{by}: {}", one_line(text, MAX_TEXT))),
            _ => true,
        })
        .collect();
    // Stable: equal ranks keep their order.
    out.sort_by(|a, b| rank(a.kind()).total_cmp(&rank(b.kind())));
    out
}

fn is_done(task: Option<&Task>) -> bool {
    task.is_some_and(|task| task.column == "done")
}

/// Higher priority first, then the board's own order.
pub fn by_rank(a: &Task, b: &Task) -> std::cmp::Ordering {
    b.priority
        .cmp(&a.priority)
        .then(a.order.total_cmp(&b.order))
}

/// Tasks nobody holds, in todo, with every dependency done: the next spawns
/// (`readyTasks`).
pub fn ready_tasks(tasks: &[Task]) -> Vec<&Task> {
    let by_id: HashMap<&str, &Task> = tasks.iter().map(|task| (task.id.as_str(), task)).collect();
    let mut ready: Vec<&Task> = tasks
        .iter()
        .filter(|task| {
            task.column == "todo"
                && task.assignee.as_deref().is_none_or(str::is_empty)
                && !task.labels.iter().any(|label| label == NEEDS_YOU_LABEL)
                && task.blocked.is_none()
                && task
                    .depends_on
                    .iter()
                    .all(|id| is_done(by_id.get(id.as_str()).copied()))
        })
        .collect();
    ready.sort_by(|a, b| by_rank(a, b));
    ready
}

/// The lines a message tells `events` in, one each but for the plain
/// comments on one task: from `MERGED_FROM` on, they are one line. A comment
/// that cannot wait (`is_urgent`) keeps a line of its own.
fn lines_of(events: &[OrchestratorEvent], titles: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let plain =
        |event: &OrchestratorEvent| event.kind() == EventKind::TaskCommented && !is_urgent(event);
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for event in events.iter().filter(|event| plain(event)) {
        *counts
            .entry(event.task_id().unwrap_or_default())
            .or_default() += 1;
    }
    let mut merged: HashSet<&str> = HashSet::new();
    let mut lines = Vec::new();
    for event in events {
        let task_id = event.task_id().unwrap_or_default();
        if plain(event)
            && counts
                .get(task_id)
                .is_some_and(|count| *count >= MERGED_FROM)
        {
            if merged.insert(task_id) {
                let on_it: Vec<&OrchestratorEvent> = events
                    .iter()
                    .filter(|other| plain(other) && other.task_id() == Some(task_id))
                    .collect();
                lines.push(describe_comments(&on_it));
            }
            continue;
        }
        lines.push(describe_event(event, titles));
    }
    lines
}

/// The message a batch becomes (`renderDelta`).
pub fn render_delta(events: &[OrchestratorEvent], tasks: &[Task], workspace: &str) -> String {
    render(events, tasks, workspace, false)
}

/// The message a batch becomes; `digest`: nothing in it is urgent, it waited
/// for the others.
fn render(events: &[OrchestratorEvent], tasks: &[Task], workspace: &str, digest: bool) -> String {
    let titles = |id: &str| {
        tasks
            .iter()
            .find(|task| task.id == id)
            .map(|task| task.title.clone())
    };
    let all = lines_of(events, &titles);
    let shown = all.len().min(MAX_LINES);
    let mut lines: Vec<String> = all[..shown]
        .iter()
        .map(|line| format!("- {line}"))
        .collect();
    let hidden = all.len() - shown;
    if hidden > 0 {
        lines.push(format!(
            "- …and {hidden} more: board_list_tasks and worker_list have the rest."
        ));
    }
    let count = if events.len() == 1 {
        "1 update".to_string()
    } else {
        format!("{} updates", events.len())
    };
    let head = if digest {
        format!(
            "{count} in workspace \"{workspace}\", a digest of the last minutes (nothing urgent):"
        )
    } else {
        format!("{count} in workspace \"{workspace}\":")
    };
    let mut out = vec![head];
    out.extend(lines);

    let finished = events.iter().any(|e| match e {
        OrchestratorEvent::Delivery { outcome, .. } => {
            matches!(outcome, DeliveryOutcome::Landed | DeliveryOutcome::Done)
        }
        OrchestratorEvent::TaskMoved { to, .. } => to == "done",
        _ => false,
    });
    if finished && !tasks.is_empty() {
        let ready = ready_tasks(tasks);
        if tasks.iter().all(|task| task.column == "done") {
            out.push(
                "Every task on the board is done: check the plan's validation criteria, then report to the person."
                    .into(),
            );
        } else if !ready.is_empty() {
            let names: Vec<String> = ready
                .iter()
                .take(6)
                .map(|task| task_ref(Some(&task.title), &task.id))
                .collect();
            out.push(format!("Ready to start: {}.", names.join(", ")));
        }
    }
    out.join("\n")
}

/* ── The board, compared with what it was ──────────────────────── */

/// What of a task the engine compares between two reads (`TaskSnap`).
#[derive(Debug, Clone, PartialEq)]
pub struct TaskSnap {
    pub column: String,
    pub labels: Vec<String>,
    pub comments: Vec<String>,
    pub blocked: Option<BlockSnap>,
    /// When its progress was last reported.
    pub progress_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockSnap {
    pub reason: String,
    pub reminded: bool,
}

pub fn snap_board(tasks: &[Task]) -> HashMap<String, TaskSnap> {
    tasks
        .iter()
        .map(|task| {
            (
                task.id.clone(),
                TaskSnap {
                    column: task.column.clone(),
                    labels: task.labels.clone(),
                    comments: task.comments.iter().map(|c| c.id.clone()).collect(),
                    blocked: task.blocked.as_ref().map(|blocked| BlockSnap {
                        reason: blocked.reason.clone(),
                        reminded: blocked.reminded.is_some_and(|at| at != 0),
                    }),
                    progress_at: task.progress.as_ref().map(|progress| progress.at),
                },
            )
        })
        .collect()
}

/// Who reported progress on a task since the last read: the reporter, else
/// the task's assignee. No event: the orchestrator reads progress on the
/// board; it only tells that a worker told idle did something since.
fn progress_reported(before: &HashMap<String, TaskSnap>, after: &[Task]) -> Vec<String> {
    after
        .iter()
        .filter_map(|task| {
            let progress = task.progress.as_ref()?;
            let was = before.get(&task.id)?;
            if was.progress_at == Some(progress.at) {
                return None;
            }
            progress.by.clone().or_else(|| task.assignee.clone())
        })
        .collect()
}

/// The orchestrator a board is read for: its name, and whether it is in a
/// turn (a move made then is taken for its own).
#[derive(Debug, Clone, Copy)]
pub struct Reader<'a> {
    pub name: &'a str,
    pub busy: bool,
}

/// What changed on a board between two reads, as events, each with whether
/// it is the orchestrator's own doing (`diffBoard`). The board does not record
/// who moved a task, so a move made while the orchestrator is in a turn is
/// taken for its own — it is the one moving tasks — and kept quiet; comments
/// are signed, and the orchestrator's and Hangar's own are no news. New and
/// deleted tasks are no event: the orchestrator creates them.
pub fn diff_board(
    before: &HashMap<String, TaskSnap>,
    after: &[Task],
    orchestrator: Reader<'_>,
) -> Vec<(OrchestratorEvent, bool)> {
    let mut out = Vec::new();
    let own = orchestrator.busy;
    for task in after {
        let Some(was) = before.get(&task.id) else {
            continue;
        };
        if was.column != task.column {
            out.push((
                OrchestratorEvent::TaskMoved {
                    task_id: task.id.clone(),
                    title: task.title.clone(),
                    from: was.column.clone(),
                    to: task.column.clone(),
                    by: own.then(|| orchestrator.name.to_string()),
                },
                own,
            ));
        }
        let known: HashSet<&str> = was.comments.iter().map(String::as_str).collect();
        for comment in &task.comments {
            if known.contains(comment.id.as_str()) {
                continue;
            }
            if same_name(&comment.author, orchestrator.name) || is_hangar_author(&comment.author) {
                continue;
            }
            out.push((
                OrchestratorEvent::TaskCommented {
                    task_id: task.id.clone(),
                    title: task.title.clone(),
                    by: comment.author.clone(),
                    text: comment.text.clone(),
                },
                false,
            ));
        }
        let last = task.comments.last();
        match (&task.blocked, &was.blocked) {
            (Some(blocked), None) => out.push((
                OrchestratorEvent::TaskBlocked {
                    task_id: task.id.clone(),
                    title: task.title.clone(),
                    by: if blocked.by.is_empty() {
                        "an agent".into()
                    } else {
                        blocked.by.clone()
                    },
                    reason: blocked.reason.clone(),
                    human_only: blocked.human_only,
                },
                own && same_name(&blocked.by, orchestrator.name),
            )),
            // Hangar signs why it was lifted: who, or "recheck passed".
            (None, Some(_)) => out.push((
                OrchestratorEvent::TaskUnblocked {
                    task_id: task.id.clone(),
                    title: task.title.clone(),
                    how: match last {
                        Some(last) if is_hangar_author(&last.author) => last.text.clone(),
                        _ => "unblocked".into(),
                    },
                },
                false,
            )),
            (Some(blocked), Some(was_blocked))
                if blocked.reminded.is_some_and(|at| at != 0) && !was_blocked.reminded =>
            {
                out.push((
                    OrchestratorEvent::TaskRecheckDue {
                        task_id: task.id.clone(),
                        title: task.title.clone(),
                        reason: blocked.reason.clone(),
                    },
                    false,
                ))
            }
            _ => {}
        }
        let labelled = |labels: &[String]| labels.iter().any(|label| label == NEEDS_YOU_LABEL);
        if labelled(&task.labels) && !labelled(&was.labels) {
            out.push((
                OrchestratorEvent::TaskNeedsYou {
                    task_id: task.id.clone(),
                    title: task.title.clone(),
                    reason: match last {
                        Some(last) => {
                            format!("{}: {}", last.author, one_line(&last.text, MAX_TEXT))
                        }
                        None => format!("labelled {NEEDS_YOU_LABEL}"),
                    },
                },
                own,
            ));
        }
    }
    out
}

/* ── The engine ────────────────────────────────────────────────── */

/// What the engines read and send (`EngineDeps`, less the timers, which are
/// theirs now). Synchronous: a board is a small file, and the runtime calls
/// the engines off the async threads.
pub trait EngineIo {
    /// The persisted model as it stands (`snapshot()`).
    fn state(&self) -> Arc<AppState>;
    fn activity(&self, pane: &str) -> Option<PaneActivity>;
    fn load_board(&self, cwd: &str) -> Result<Vec<Task>, String>;
    /// `bridgeEnqueue`: the pane's mod submits it once the session is idle.
    fn enqueue(&self, pane: &str, text: &str, from: &str) -> Result<(), String>;
    /// What the agent of pane `pane` has running of its own between two
    /// turns — a build, a benchmark, a watch on its CI it started in the
    /// background (`hangar_core::pty::pane_job`). `Unknown`, the default: a
    /// pane on a node, or one nobody can look into, is called idle as it
    /// always was.
    fn pane_job(&self, pane: &str) -> PaneJob {
        let _ = pane;
        PaneJob::Unknown
    }
    /// How far the HEAD of the checkout at `cwd` has moved (`head::moves`):
    /// a worker that committed moved it. `None`, the default: a checkout on
    /// a node, or none.
    fn head_moves(&self, cwd: &str) -> Option<u64> {
        let _ = cwd;
        None
    }
}

/// A deadline the engine keeps; `seq` orders two that fall on the same
/// millisecond by when they were set, as the TS's timers fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Timer {
    at: u64,
    seq: u64,
}

#[derive(Debug, Default)]
struct Space {
    journal: Vec<JournalEntry>,
    pending: Vec<JournalEntry>,
    first_pending_at: Option<u64>,
    /// The first urgent event pending: the batch goes within `MAX_WAIT_MS`
    /// of it. `None`: what is pending waits for the digest.
    first_urgent_at: Option<u64>,
    last_sent_at: u64,
    timer: Option<Timer>,
    /// The batch waits for the orchestrator's turn to end.
    holding: bool,
    /// The board as last read; `None` until the first read, the baseline.
    board: Option<HashMap<String, TaskSnap>>,
    tasks: Vec<Task>,
}

#[derive(Debug, Clone)]
struct WorkerWatch {
    workspace_id: WsId,
    name: String,
    task_id: Option<String>,
    activity: AgentActivity,
    since: u64,
    /// The worker had a turn and handed back; not merely never started.
    after_turn: bool,
    waiting_sent: bool,
    /// Told idle since its activity last changed: once a spell.
    idle_sent: bool,
    exit_sent: bool,
    /// When it was last told idle, and its checkout's HEAD then
    /// (`EngineIo::head_moves`): the next time waits for it to do something.
    idle_told_at: Option<u64>,
    head_told: Option<u64>,
    /// When it last commented, reported progress or delivered.
    progress_at: Option<u64>,
    /// A command of its own that runs between its turns, as last seen, and
    /// when it was; when its pane was last looked into.
    job: Option<String>,
    job_seen_at: Option<u64>,
    job_checked_at: Option<u64>,
}

impl WorkerWatch {
    fn new(workspace_id: WsId, pane: &Pane, activity: AgentActivity, now: u64) -> Self {
        WorkerWatch {
            workspace_id,
            name: pane.name.clone(),
            task_id: pane.task_id.clone(),
            activity,
            since: now,
            after_turn: false,
            waiting_sent: false,
            idle_sent: false,
            exit_sent: false,
            idle_told_at: None,
            head_told: None,
            progress_at: None,
            job: None,
            job_seen_at: None,
            job_checked_at: None,
        }
    }

    /// Whether it may be told idle again: never told, told long enough ago,
    /// or it did something since — commented, reported, delivered (noted as
    /// they come), committed (`head` is its HEAD now).
    fn may_be_told_idle(&self, head: impl FnOnce() -> Option<u64>, now: u64) -> bool {
        let Some(told) = self.idle_told_at else {
            return true;
        };
        now.saturating_sub(told) >= IDLE_AGAIN_MS
            || self.progress_at.is_some_and(|at| at > told)
            || head().is_some_and(|head| Some(head) != self.head_told)
    }
}

/// Where a pane's session stands, from its mod's own reports.
#[derive(Debug, Clone, Copy)]
struct Turn {
    in_turn: bool,
    waiting: bool,
}

/// Which timer fires next.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Due {
    Space(WsId),
    Board,
}

/// The engine (`createOrchestratorEngine`).
#[derive(Debug, Default)]
pub struct OrchestratorEngine {
    spaces: HashMap<WsId, Space>,
    /// In the order first seen, as the TS's Map kept them.
    workers: Vec<(PaneId, WorkerWatch)>,
    /// The pane's activity is the fallback only: a session that has just
    /// started keeps the "working" its startup output suggested until its
    /// first turn, and events held for it would wait for the person to type
    /// something.
    turns: HashMap<PaneId, Turn>,
    board_timer: Option<Timer>,
    seq: u64,
    stopped: bool,
}

fn workspace_in(state: &AppState, id: &str) -> Option<Workspace> {
    state.workspaces.iter().find(|ws| ws.id == id).cloned()
}

fn orchestrated(state: &AppState) -> Vec<(Workspace, Pane)> {
    state
        .workspaces
        .iter()
        .filter_map(|ws| orchestrator_of(ws).map(|pane| (ws.clone(), pane.clone())))
        .collect()
}

impl OrchestratorEngine {
    #[cfg(any(test, feature = "testing"))]
    pub fn new() -> Self {
        Self::default()
    }

    fn timer(&mut self, at: u64) -> Timer {
        self.seq += 1;
        Timer { at, seq: self.seq }
    }

    fn space(&mut self, id: &str) -> &mut Space {
        self.spaces.entry(id.to_string()).or_default()
    }

    fn in_turn(&self, io: &dyn EngineIo, pane: &Pane) -> bool {
        match self.turns.get(&pane.id) {
            Some(turn) => turn.in_turn,
            None => io.activity(&pane.id).map(|a| a.activity) == Some(AgentActivity::Working),
        }
    }

    fn busy(&self, io: &dyn EngineIo, pane: &Pane) -> bool {
        match self.turns.get(&pane.id) {
            Some(turn) => turn.in_turn || turn.waiting,
            None => matches!(
                io.activity(&pane.id).map(|a| a.activity),
                Some(AgentActivity::Working | AgentActivity::Waiting)
            ),
        }
    }

    /// Hands `event` to the workspace's orchestrator, if it has one
    /// (`post`). `quiet`: journal it, never wake for it.
    pub fn post(
        &mut self,
        io: &dyn EngineIo,
        workspace_id: &str,
        event: OrchestratorEvent,
        quiet: bool,
        now: u64,
    ) {
        if self.stopped {
            return;
        }
        let state = io.state();
        let Some(ws) = workspace_in(&state, workspace_id) else {
            return;
        };
        let Some(orchestrator) = orchestrator_of(&ws) else {
            return;
        };
        // Starting and stopping workers is the orchestrator's own doing when
        // it happens during its turn: it already has the tool's answer.
        let own = matches!(
            event.kind(),
            EventKind::WorkerStarted | EventKind::WorkerStopped
        ) && self.in_turn(io, orchestrator);
        let entry = JournalEntry {
            at: now,
            event,
            quiet: quiet || own,
        };
        self.note_progress(workspace_id, &entry.event, now);
        let s = self.space(workspace_id);
        s.journal.push(entry.clone());
        if s.journal.len() > JOURNAL_SIZE {
            let over = s.journal.len() - JOURNAL_SIZE;
            s.journal.drain(..over);
        }
        if entry.quiet {
            return;
        }
        s.first_pending_at.get_or_insert(entry.at);
        if is_urgent(&entry.event) {
            s.first_urgent_at.get_or_insert(entry.at);
        }
        s.pending.push(entry);
        if s.pending.len() > MAX_PENDING {
            let over = s.pending.len() - MAX_PENDING;
            s.pending.drain(..over);
        }
        // Already held for the orchestrator's turn: the hold looks again soon.
        if !s.holding {
            self.schedule(workspace_id, false, now);
        }
    }

    /// A worker that commented or delivered did something: told idle, it
    /// may be told so again (`WorkerWatch::may_be_told_idle`).
    fn note_progress(&mut self, workspace_id: &str, event: &OrchestratorEvent, now: u64) {
        let who = match event {
            OrchestratorEvent::TaskCommented { by, .. } => by,
            OrchestratorEvent::Delivery { worker, .. } => worker,
            _ => return,
        };
        self.progressed(workspace_id, who, now);
    }

    fn progressed(&mut self, workspace_id: &str, who: &str, now: u64) {
        for (_, w) in &mut self.workers {
            if w.workspace_id == workspace_id && same_name(&w.name, who) {
                w.progress_at = Some(now);
            }
        }
    }

    /// The next look at a workspace's batch: with something urgent in it,
    /// once things quieten (never later than the first urgent event's
    /// deadline); without, when the digest is due; or — `hold` — a few
    /// seconds on, while the orchestrator is in a turn. Never closer than the
    /// gap to the last message.
    fn schedule(&mut self, workspace_id: &str, hold: bool, now: u64) {
        let (last_sent_at, first_pending_at, first_urgent_at) = {
            let s = self.space(workspace_id);
            (s.last_sent_at, s.first_pending_at, s.first_urgent_at)
        };
        let t = now as i128;
        let gap = last_sent_at as i128 + MIN_GAP_MS as i128 - t;
        let wanted = if hold {
            HOLD_MS as i128
        } else if let Some(urgent) = first_urgent_at {
            let deadline = urgent as i128 + MAX_WAIT_MS as i128 - t;
            (DEBOUNCE_MS as i128).min(deadline)
        } else {
            first_pending_at.map_or(t, |at| at as i128) + DIGEST_MS as i128 - t
        };
        let delay = wanted.max(gap).max(0) as u64;
        let timer = self.timer(now + delay);
        let s = self.space(workspace_id);
        s.holding = hold;
        s.timer = Some(timer);
    }

    /// The batch goes, or waits for the orchestrator's turn to end (`fire`).
    fn fire(&mut self, io: &dyn EngineIo, workspace_id: &str, now: u64) {
        if self.stopped {
            return;
        }
        let state = io.state();
        let ws = workspace_in(&state, workspace_id);
        let orchestrator = ws.as_ref().and_then(|ws| orchestrator_of(ws).cloned());
        let (Some(ws), Some(orchestrator)) = (ws, orchestrator) else {
            let s = self.space(workspace_id);
            s.pending.clear();
            s.first_pending_at = None;
            s.first_urgent_at = None;
            return;
        };
        // Queued now, it would sit in the pane's queue while the news aged;
        // held here, what comes meanwhile joins it and the whole is read
        // again later.
        if self.busy(io, &orchestrator) {
            self.schedule(workspace_id, true, now);
            return;
        }
        self.send(io, &ws, &orchestrator, now);
    }

    fn send(
        &mut self,
        io: &dyn EngineIo,
        ws: &Workspace,
        orchestrator: &Pane,
        now: u64,
    ) -> Option<String> {
        // The board as it is now: a landing moved its task, and the next
        // spawns are read off it.
        let _ = self.read(io, ws, now);
        let s = self.space(&ws.id);
        let batch = std::mem::take(&mut s.pending);
        s.first_pending_at = None;
        s.first_urgent_at = None;
        if batch.is_empty() {
            return None;
        }
        let live = SpaceLive {
            io,
            ws,
            tasks: &s.tasks,
        };
        let events = settle(&batch, &s.journal, &live);
        if events.is_empty() {
            return None;
        }
        // Nothing urgent left in it: it waited for the digest.
        let digest = !events.iter().any(is_urgent);
        let text = render(&events, &s.tasks, &ws.name, digest);
        s.last_sent_at = now;
        // A pane that cannot take it now gets the next batch; nothing to undo.
        let _ = io.enqueue(&orchestrator.id, &text, "hangar");
        Some(text)
    }

    /// Reads a workspace's board, and tells what changed since the last read
    /// (`read`). The first read is the baseline.
    fn read(&mut self, io: &dyn EngineIo, ws: &Workspace, now: u64) -> Result<(), String> {
        let tasks = io.load_board(&ws.cwd)?;
        let state = io.state();
        let Some(current) = workspace_in(&state, &ws.id) else {
            return Ok(());
        };
        let Some(orchestrator) = orchestrator_of(&current).cloned() else {
            return Ok(());
        };
        let busy = self.in_turn(io, &orchestrator);
        let s = self.space(&ws.id);
        let changes = s.board.as_ref().map(|before| {
            diff_board(
                before,
                &tasks,
                Reader {
                    name: &orchestrator.name,
                    busy,
                },
            )
        });
        let reported = s
            .board
            .as_ref()
            .map(|before| progress_reported(before, &tasks))
            .unwrap_or_default();
        s.board = Some(snap_board(&tasks));
        s.tasks = tasks;
        for who in reported {
            self.progressed(&ws.id, &who, now);
        }
        for (event, quiet) in changes.unwrap_or_default() {
            self.post(io, &ws.id, event, quiet, now);
        }
        Ok(())
    }

    fn read_all(&mut self, io: &dyn EngineIo, now: u64) {
        let state = io.state();
        for (ws, _) in orchestrated(&state) {
            let _ = self.read(io, &ws, now);
        }
    }

    fn watch_workers(&mut self, io: &dyn EngineIo, now: u64) {
        let state = io.state();
        let mut present: HashSet<PaneId> = HashSet::new();
        for (ws, _) in orchestrated(&state) {
            for pane in workers_of(&ws) {
                present.insert(pane.id.clone());
                let activity = io
                    .activity(&pane.id)
                    .map_or(AgentActivity::Idle, |a| a.activity);
                let handed_back = matches!(activity, AgentActivity::Yours | AgentActivity::Idle);
                let at = match self.workers.iter().position(|(id, _)| *id == pane.id) {
                    Some(at) => at,
                    None => {
                        let mut w = WorkerWatch::new(ws.id.clone(), pane, activity, now);
                        w.after_turn = activity == AgentActivity::Yours;
                        self.workers.push((pane.id.clone(), w));
                        self.workers.len() - 1
                    }
                };
                let w = &mut self.workers[at].1;
                w.name = pane.name.clone();
                w.task_id = pane.task_id.clone();
                if activity != w.activity {
                    let was_in_turn =
                        matches!(w.activity, AgentActivity::Working | AgentActivity::Waiting);
                    w.after_turn = handed_back && (was_in_turn || w.after_turn);
                    w.activity = activity;
                    w.since = now;
                    w.waiting_sent = false;
                    w.idle_sent = false;
                }
                let for_ms = now.saturating_sub(w.since);
                if activity == AgentActivity::Waiting && !w.waiting_sent && for_ms >= WAITING_MS {
                    w.waiting_sent = true;
                    let event = OrchestratorEvent::WorkerWaiting {
                        worker: pane.name.clone(),
                        task_id: w.task_id.clone(),
                        for_ms,
                    };
                    self.post(io, &ws.id, event, false, now);
                }
                let w = self.workers[at].1.clone();
                if handed_back && w.after_turn && !w.idle_sent && for_ms >= IDLE_MS {
                    let s = self.space(&ws.id);
                    let task = w
                        .task_id
                        .as_deref()
                        .and_then(|id| s.tasks.iter().find(|task| task.id == id));
                    let delivered = s.journal.iter().any(|entry| {
                        entry.at >= w.since
                            && entry.event.kind() == EventKind::Delivery
                            && entry.event.task_id() == w.task_id.as_deref()
                    });
                    // A task that left `doing`, or waits for the person, is not idling.
                    let open = task.is_none_or(|task| {
                        task.column == "doing" && !task.labels.iter().any(|l| l == NEEDS_YOU_LABEL)
                    });
                    let cwd = pane.cwd.clone().unwrap_or_else(|| ws.cwd.clone());
                    if !delivered
                        && open
                        && w.may_be_told_idle(|| io.head_moves(&cwd), now)
                        && let Some(job) = self.idle_now(io, at, &pane.id, for_ms, now)
                    {
                        let w = &mut self.workers[at].1;
                        w.idle_sent = true;
                        w.idle_told_at = Some(now);
                        w.head_told = io.head_moves(&cwd);
                        let event = OrchestratorEvent::WorkerIdle {
                            worker: pane.name.clone(),
                            task_id: w.task_id.clone(),
                            for_ms,
                            job,
                        };
                        self.post(io, &ws.id, event, false, now);
                    }
                }
            }
        }
        // A worker pane gone without worker_stop — closed by hand — exited.
        // One that restarted under a new id is still there by name.
        let gone: Vec<(PaneId, WorkerWatch)> = self
            .workers
            .iter()
            .filter(|(id, _)| !present.contains(id))
            .cloned()
            .collect();
        self.workers.retain(|(id, _)| present.contains(id));
        for (_, w) in gone {
            let Some(ws) = workspace_in(&state, &w.workspace_id) else {
                continue;
            };
            if orchestrator_of(&ws).is_none() || w.exit_sent {
                continue;
            }
            if workers_of(&ws)
                .iter()
                .any(|pane| same_name(&pane.name, &w.name))
            {
                continue;
            }
            let event = OrchestratorEvent::WorkerExited {
                worker: w.name.clone(),
                task_id: w.task_id.clone(),
            };
            self.post(io, &ws.id, event, false, now);
        }
    }

    /// Whether worker `at`, which ended its turn `for_ms` ago, is idle now:
    /// `Some(None)` when nothing of its own runs in its pane, and nothing has
    /// for `IDLE_MS`; `Some(Some(job))` when a job of its own has run for
    /// `JOB_HOLD_MS`, long enough to tell; `None` while it waits on its job.
    /// Its pane is looked into once a `JOB_POLL_MS` at most, which is also
    /// how often a running job is looked at again until it ends.
    fn idle_now(
        &mut self,
        io: &dyn EngineIo,
        at: usize,
        pane: &str,
        for_ms: u64,
        now: u64,
    ) -> Option<Option<String>> {
        let w = &mut self.workers[at].1;
        let looked = w
            .job_checked_at
            .is_some_and(|checked| now.saturating_sub(checked) < JOB_POLL_MS);
        if !looked {
            w.job_checked_at = Some(now);
            match io.pane_job(pane) {
                PaneJob::Running(job) => {
                    w.job = Some(job);
                    w.job_seen_at = Some(now);
                }
                // Unknown: a pane on a node, or one nobody can look into;
                // idle as it always was.
                PaneJob::Nothing | PaneJob::Unknown => w.job = None,
            }
        }
        match &w.job {
            Some(job) if for_ms >= JOB_HOLD_MS => Some(Some(job.clone())),
            Some(_) => None,
            // Its job ended a moment ago: it is quiet from then on.
            None if w
                .job_seen_at
                .is_some_and(|seen| now.saturating_sub(seen) < IDLE_MS) =>
            {
                None
            }
            None => Some(None),
        }
    }

    fn exited(&mut self, io: &dyn EngineIo, pane_id: &str, now: u64) {
        let state = io.state();
        for (ws, _) in orchestrated(&state) {
            let Some(pane) = workers_of(&ws)
                .into_iter()
                .find(|p| p.id == pane_id)
                .cloned()
            else {
                continue;
            };
            match self.workers.iter_mut().find(|(id, _)| id == pane_id) {
                Some((_, w)) if w.exit_sent => return,
                Some((_, w)) => w.exit_sent = true,
                None => {
                    let mut w = WorkerWatch::new(ws.id.clone(), &pane, AgentActivity::Idle, now);
                    w.exit_sent = true;
                    self.workers.push((pane_id.to_string(), w));
                }
            }
            let event = OrchestratorEvent::WorkerExited {
                worker: pane.name.clone(),
                task_id: pane.task_id.clone(),
            };
            self.post(io, &ws.id, event, false, now);
            return;
        }
    }

    /// An agent changed a board: the watched ones are read again shortly
    /// (`boardChanged`).
    pub fn board_changed(&mut self, now: u64) {
        if self.stopped || self.board_timer.is_some() {
            return;
        }
        self.board_timer = Some(self.timer(now + BOARD_SETTLE_MS));
    }

    /// A report of a pane's hangar-bridge mod (`bridgeEvent`).
    pub fn bridge_event(
        &mut self,
        io: &dyn EngineIo,
        pane_id: &str,
        event: &BridgeEvent,
        now: u64,
    ) {
        match event {
            BridgeEvent::Hello { .. } => {
                self.turns.insert(
                    pane_id.to_string(),
                    Turn {
                        in_turn: false,
                        waiting: false,
                    },
                );
                if let Some((_, w)) = self.workers.iter_mut().find(|(id, _)| id == pane_id) {
                    w.exit_sent = false;
                }
            }
            BridgeEvent::Turn(phase) => {
                self.turns.insert(
                    pane_id.to_string(),
                    Turn {
                        in_turn: matches!(phase, TurnPhase::Start),
                        waiting: false,
                    },
                );
            }
            BridgeEvent::Permission { waiting, .. } => {
                if let Some(turn) = self.turns.get_mut(pane_id) {
                    turn.waiting = *waiting;
                }
            }
            BridgeEvent::End { .. } => {
                self.turns.remove(pane_id);
                self.exited(io, pane_id, now);
            }
            _ => {}
        }
    }

    /// A pane's shell ended by itself (`ptyExited`).
    pub fn pty_exited(&mut self, io: &dyn EngineIo, pane_id: &str, now: u64) {
        self.turns.remove(pane_id);
        self.exited(io, pane_id, now);
    }

    /// Reads the boards, looks at the workers: called on every tick.
    pub fn tick(&mut self, io: &dyn EngineIo, now: u64) {
        if self.stopped || orchestrated(&io.state()).is_empty() {
            return;
        }
        self.read_all(io, now);
        self.watch_workers(io, now);
    }

    /// The boards a tick or a timer of this engine reads: every orchestrated
    /// workspace's. The runtime reads them before it locks the engine.
    pub fn watched_boards(&self, io: &dyn EngineIo) -> Vec<String> {
        if self.stopped {
            return Vec::new();
        }
        orchestrated(&io.state())
            .into_iter()
            .map(|(ws, _)| ws.cwd)
            .collect()
    }

    /// What happened lately in the workspace, oldest first, for its briefing.
    pub fn journal(&self, workspace_id: &str) -> Vec<JournalEntry> {
        self.spaces
            .get(workspace_id)
            .map(|s| s.journal.clone())
            .unwrap_or_default()
    }

    #[allow(dead_code)]
    /// The workspace's board as last read.
    pub fn tasks(&self, workspace_id: &str) -> Vec<Task> {
        self.spaces
            .get(workspace_id)
            .map(|s| s.tasks.clone())
            .unwrap_or_default()
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn stop(&mut self) {
        self.stopped = true;
        self.spaces.clear();
        self.workers.clear();
        self.turns.clear();
        self.board_timer = None;
    }

    fn next_due(&self) -> Option<(Timer, Due)> {
        let spaces = self
            .spaces
            .iter()
            .filter_map(|(id, s)| s.timer.map(|timer| (timer, Due::Space(id.clone()))));
        let board = self.board_timer.map(|timer| (timer, Due::Board));
        spaces
            .chain(board)
            .min_by_key(|(timer, _)| (timer.at, timer.seq))
    }

    /// When the next timer falls due, if one is set.
    pub fn next_wake(&self) -> Option<u64> {
        self.next_due().map(|(timer, _)| timer.at)
    }

    /// Fires the earliest timer due by `now`, if one is. Returns whether one
    /// fired; the runtime calls it until none does.
    pub fn fire_due(&mut self, io: &dyn EngineIo, now: u64) -> bool {
        let Some((timer, due)) = self.next_due() else {
            return false;
        };
        if timer.at > now {
            return false;
        }
        match due {
            Due::Board => {
                self.board_timer = None;
                self.read_all(io, now);
            }
            Due::Space(id) => {
                let s = self.space(&id);
                s.timer = None;
                s.holding = false;
                self.fire(io, &id, now);
            }
        }
        true
    }
}

/// The board and the panes a batch is checked against (`liveOf`).
struct SpaceLive<'a> {
    io: &'a dyn EngineIo,
    ws: &'a Workspace,
    tasks: &'a [Task],
}

impl LiveView for SpaceLive<'_> {
    fn activity(&self, worker: &str) -> Option<AgentActivity> {
        let pane = workers_of(self.ws)
            .into_iter()
            .find(|p| same_name(&p.name, worker))?;
        Some(
            self.io
                .activity(&pane.id)
                .map_or(AgentActivity::Idle, |a| a.activity),
        )
    }

    fn column(&self, task_id: &str) -> Option<String> {
        self.tasks
            .iter()
            .find(|task| task.id == task_id)
            .map(|task| task.column.clone())
    }
}

#[cfg(test)]
pub mod tests;
