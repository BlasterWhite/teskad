//! An agent pane's conversation for the phone (`remote_chat`), and what its
//! side channel reports that its history cannot say yet (`noteBridgeEvent`,
//! `liveQuestion`, `pendingOf`, `runningOf` of lib/remoteControl.ts): an
//! agent writes a call to its history only once it has its result, so
//! neither a call running nor the question or the permission the pane waits
//! on is in it.
//!
//! The history is the one the pane's agent writes, by its capability
//! (`transcript`, docs/native/agents.md D3): Claude Code's transcript, or a
//! Codex thread's rollout, which core's `chat/codex.rs` (T9) reads as the
//! same chat items. A Codex pane's own app-server writes that rollout as the
//! thread runs, whether Hangar is attached to it or not; what it has not
//! written yet is what Hangar's client of that server reports — the calls
//! running, the dialog open, the question asked (`agent/codex_attach.rs`) —
//! in the very reports the teskad-bridge mod sends, kept here alike.

use std::collections::HashMap;

use serde_json::Value;

use hangar_core::agent::TranscriptFormat;
use hangar_core::chat::{Body, CallState, ChatItem, ChatPage, PendingQuestion};

use crate::control::port::{Port, activity_of};
use crate::live::{BridgeEvent, RateLimit, ToolPhase};
use crate::state::{AgentActivity, Pane, PaneId, Workspace};

use super::calls::{asked_questions, describe_call};
use super::contract::{ChatPending, ChatReason, RemoteChat};
use super::engine::{ChatRead, Mobile};
use super::held::ASK_QUESTION as QUESTION_TOOL;
use super::ops::find_pane;

/// A call a pane's mod reported started and not yet ended.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveCall {
    pub tool_use_id: String,
    pub tool: String,
    pub input: Value,
    pub at: u64,
}

/// What one pane's mod reported (`Live`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaneLive {
    /// Oldest first.
    pub calls: Vec<LiveCall>,
    /// The permission dialog open, with the input of the call it asks about.
    pub permission: Option<(String, Value)>,
}

/// The plan's rate-limit windows, from whichever session reported last, as
/// the navigator shows them: one account behind every pane (`planLimits`).
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub limits: Vec<RateLimit>,
    pub at: u64,
}

#[derive(Debug, Default)]
pub struct ChatLive {
    panes: HashMap<PaneId, PaneLive>,
    pub plan: Option<Plan>,
}

impl ChatLive {
    /// Keeps what a pane's mod reports of its calls and dialogs, and of the
    /// plan.
    pub fn note(&mut self, pane: &str, event: &BridgeEvent, at: u64) {
        match event {
            BridgeEvent::Usage { rate_limits, .. } => {
                // A session without such limits (an API key) says nothing of
                // the plan.
                if !rate_limits.is_empty() {
                    self.plan = Some(Plan {
                        limits: rate_limits.clone(),
                        at,
                    });
                }
            }
            // A turn that starts or ends, a conversation that starts over:
            // nothing from before still runs or waits.
            BridgeEvent::Hello { .. }
            | BridgeEvent::Session { .. }
            | BridgeEvent::Turn(_)
            | BridgeEvent::End { .. } => {
                self.panes.remove(pane);
            }
            // How the session runs: nothing of its calls or dialogs.
            BridgeEvent::Settings { .. } => {}
            BridgeEvent::Permission {
                waiting,
                tool,
                input,
            } => {
                let live = self.panes.entry(pane.to_string()).or_default();
                live.permission = match tool {
                    Some(tool) if *waiting && !tool.is_empty() => {
                        Some((tool.clone(), input.clone().unwrap_or(Value::Null)))
                    }
                    _ => None,
                };
                self.prune(pane);
            }
            BridgeEvent::Tool(phase) => {
                let live = self.panes.entry(pane.to_string()).or_default();
                let id = match phase {
                    ToolPhase::Start { tool_use_id, .. } | ToolPhase::End { tool_use_id } => {
                        tool_use_id
                    }
                };
                live.calls.retain(|call| call.tool_use_id != *id);
                if let ToolPhase::Start {
                    tool_use_id,
                    tool,
                    input,
                } = phase
                {
                    live.calls.push(LiveCall {
                        tool_use_id: tool_use_id.clone(),
                        tool: tool.clone(),
                        input: if input.is_null() {
                            Value::Object(Default::default())
                        } else {
                            input.clone()
                        },
                        at,
                    });
                }
                self.prune(pane);
            }
            BridgeEvent::Edit { .. } | BridgeEvent::Dropped { .. } => {}
        }
    }

    fn prune(&mut self, pane: &str) {
        if self
            .panes
            .get(pane)
            .is_some_and(|live| live.calls.is_empty() && live.permission.is_none())
        {
            self.panes.remove(pane);
        }
    }

    /// The input of the call the pane's permission dialog asks about, as its
    /// mod reported it, when that dialog is on `tool`.
    pub fn permission_input(&self, pane: &str, tool: &str) -> Option<Value> {
        let (asked, input) = self.panes.get(pane)?.permission.as_ref()?;
        (asked == tool).then(|| input.clone())
    }

    /// What the pane's question call `tool_use_id` was given, as its side
    /// channel reported it: beyond the questions, what each takes — Codex
    /// says whether a question offers "None of the above" (`isOther`).
    pub fn question_input(&self, pane: &str, tool_use_id: &str) -> Option<Value> {
        self.panes
            .get(pane)?
            .calls
            .iter()
            .find(|call| call.tool == QUESTION_TOOL && call.tool_use_id == tool_use_id)
            .map(|call| call.input.clone())
    }

    /// The question a pane's AskUserQuestion call puts to the person, while
    /// it runs (`liveQuestion`).
    pub fn question(&self, pane: &str) -> Option<PendingQuestion> {
        let calls = &self.panes.get(pane)?.calls;
        calls.iter().rev().find_map(|call| {
            if call.tool != QUESTION_TOOL {
                return None;
            }
            let questions = asked_questions(&call.input);
            (!questions.is_empty()).then(|| PendingQuestion {
                tool_use_id: call.tool_use_id.clone(),
                questions,
            })
        })
    }

    /// What the pane waits on the person for: what its mod reports first,
    /// then what the transcript says — a question without its answer, or the
    /// last call without a result while the pane waits (`pendingOf`).
    pub fn pending_of(
        &self,
        pane: &str,
        page: &ChatPage,
        activity: Option<AgentActivity>,
        cwd: &str,
    ) -> Option<ChatPending> {
        if let Some(question) = self.question(pane).or_else(|| page.question.clone()) {
            return Some(ChatPending::Question {
                tool_use_id: question.tool_use_id,
                questions: question.questions,
            });
        }
        if activity != Some(AgentActivity::Waiting) {
            return None;
        }
        if let Some((tool, input)) = self
            .panes
            .get(pane)
            .and_then(|live| live.permission.as_ref())
        {
            let described = describe_call(tool, input, Some(cwd));
            // The call the dialog holds, when the mod reported it start: two
            // prompts for the same command are still two prompts.
            let call = self.panes.get(pane).and_then(|live| {
                live.calls
                    .iter()
                    .rev()
                    .find(|call| call.tool == *tool)
                    .map(|call| call.tool_use_id.as_str())
            });
            return Some(ChatPending::Permission {
                prompt: prompt_of(call, tool, &described.ask, described.detail.as_deref()),
                tool: tool.clone(),
                title: described.ask,
                detail: described.detail,
                plan: plan_of(tool, input),
            });
        }
        page.open_call.as_ref().map(|call| ChatPending::Permission {
            prompt: prompt_of(None, &call.tool, &call.title, call.detail.as_deref()),
            tool: call.tool.clone(),
            title: call.title.clone(),
            detail: call.detail.clone(),
            plan: None,
        })
    }

    /// Keeps what the mods of these panes reported, and forgets the panes
    /// gone (MOB-16).
    pub fn keep_only(&mut self, panes: &std::collections::HashSet<&str>) {
        self.panes.retain(|pane, _| panes.contains(pane.as_str()));
    }

    /// The calls the pane runs now, as actions after the transcript's: not a
    /// question (the pending card shows it), nor the call a permission
    /// dialog holds, which has not run yet (`runningOf`).
    pub fn running_of(
        &self,
        pane: &str,
        page: &ChatPage,
        pending: Option<&ChatPending>,
        cwd: &str,
    ) -> Vec<ChatItem> {
        let Some(live) = self.panes.get(pane) else {
            return Vec::new();
        };
        let mut calls: Vec<&LiveCall> = live
            .calls
            .iter()
            .filter(|call| {
                call.tool != QUESTION_TOOL
                    && !page.items.iter().any(|item| item.id == call.tool_use_id)
            })
            .collect();
        if let Some(ChatPending::Permission { tool, .. }) = pending
            && let Some(at) = calls.iter().rposition(|call| call.tool == *tool)
        {
            calls.remove(at);
        }
        calls
            .into_iter()
            .map(|call| {
                let described = describe_call(&call.tool, &call.input, Some(cwd));
                ChatItem {
                    id: call.tool_use_id.clone(),
                    at: call.at as i64,
                    body: Body::Action {
                        tool: call.tool.clone(),
                        title: described.title,
                        detail: described.detail,
                        result: None,
                        state: CallState::Running,
                    },
                }
            })
            .collect()
    }
}

/// The plan an ExitPlanMode dialog presents, whole, as the mod reported it.
fn plan_of(tool: &str, input: &Value) -> Option<String> {
    (tool == PLAN_TOOL)
        .then(|| input.get("plan").and_then(Value::as_str))
        .flatten()
        .filter(|plan| !plan.trim().is_empty())
        .map(str::to_string)
}

/// The tool that presents a plan for the person to approve.
pub const PLAN_TOOL: &str = "ExitPlanMode";

/// Which permission prompt the phone shows: its call when known, and what it
/// shows of it — the tool, the title, the detail. The phone sends it back
/// with its answer, and the PC types nothing unless the pane still asks the
/// same (MOB-02). Two prompts the PC cannot tell apart ask for the same
/// thing, word for word.
pub fn prompt_of(call: Option<&str>, tool: &str, title: &str, detail: Option<&str>) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (call, tool, title, detail).hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// The history the pane's agent writes, which its chat is read from; none
/// for a pane without an agent. Read from the agent's capabilities, never
/// its name (docs/native/agents.md D3).
pub fn transcript_of(pane: &Pane) -> Option<TranscriptFormat> {
    pane.agent.kind()?.info().caps.transcript
}

/// A page of a pane's conversation, from the history its agent writes on
/// this machine: a Claude Code transcript (the sources' `chat`, as ever),
/// or a Codex thread's rollout (core's `chat::codex::chat_page`, T9). `None`
/// while the session has none yet.
pub async fn history_page(
    mobile: &Mobile,
    format: TranscriptFormat,
    read: ChatRead,
) -> Result<Option<ChatPage>, String> {
    match format {
        TranscriptFormat::ClaudeJsonl => mobile.sources().chat(read).await,
        TranscriptFormat::CodexRollout => {
            hangar_core::chat::codex::chat_page(read.session_id, read.cwd, read.before, read.limit)
                .await
        }
    }
}

/// What an agent pane waits on the person for now, as `remote_chat` shows
/// it: its side channel's report and the end of its history. `None` for a
/// pane the PC cannot say this of: no agent, or no session yet.
pub async fn pending_now(
    port: &Port,
    mobile: &Mobile,
    ws: &Workspace,
    pane: &Pane,
) -> Result<Option<ChatPending>, String> {
    let Some(format) = transcript_of(pane) else {
        return Ok(None);
    };
    let Some(session_id) = pane.session_id.clone().filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    let cwd = cwd_of(ws, pane);
    let read = ChatRead {
        session_id,
        cwd: cwd.clone(),
        before: None,
        limit: None,
    };
    let page = history_page(mobile, format, read)
        .await?
        .unwrap_or_else(|| ChatPage {
            items: Vec::new(),
            older: false,
            cursor: None,
            question: None,
            open_call: None,
        });
    let activity = activity_of(&port.snapshot(), &pane.id).map(|entry| entry.activity);
    Ok(mobile
        .book()
        .chat
        .pending_of(&pane.id, &page, activity, &cwd))
}

/// The folder a pane's session runs in: its own, else its workspace's.
fn cwd_of(ws: &Workspace, pane: &Pane) -> String {
    pane.cwd
        .clone()
        .filter(|cwd| !cwd.is_empty())
        .unwrap_or_else(|| ws.cwd.clone())
}

/// `remote_chat {pane, before?, limit?}`: a page of an agent pane's
/// conversation, the newest unless `before` asks for an older one
/// (`chat_pane`).
pub async fn remote_chat(port: &Port, mobile: &Mobile, args: &Value) -> Result<RemoteChat, String> {
    let (ws, pane) = find_pane(&port.snapshot().state, args.get("pane"))?;
    let before = match args.get("before") {
        None => None,
        Some(Value::String(cursor)) => Some(cursor.clone()),
        Some(_) => return Err("\"before\" is a cursor from an earlier answer.".into()),
    };
    let limit = args
        .get("limit")
        .and_then(Value::as_f64)
        .map(|limit| limit.max(0.0).floor() as usize);
    chat_pane(port, mobile, &ws, &pane, before, limit).await
}

/// A page of a pane's conversation, the newest unless `before` (a cursor
/// from an earlier page) asks for an older one, with what the pane waits on
/// the person for and, on the newest page, the calls it runs now.
pub async fn chat_pane(
    port: &Port,
    mobile: &Mobile,
    ws: &Workspace,
    pane: &Pane,
    before: Option<String>,
    limit: Option<usize>,
) -> Result<RemoteChat, String> {
    let activity = activity_of(&port.snapshot(), &pane.id).map(|entry| entry.activity);
    let chat = |items, older, cursor, pending, reason| RemoteChat {
        pane_id: pane.id.clone(),
        pane: pane.name.clone(),
        workspace: ws.name.clone(),
        agent: pane.agent.as_str().into(),
        activity,
        session_id: pane.session_id.clone(),
        items,
        older,
        cursor,
        pending,
        reason,
    };
    let empty = |reason| chat(Vec::new(), false, None, None, Some(reason));
    // No history to read: a shell (the contract's `notClaude`, which the
    // phone takes as "show the terminal").
    let Some(format) = transcript_of(pane) else {
        return Ok(empty(ChatReason::NotClaude));
    };
    let Some(session_id) = pane.session_id.clone().filter(|id| !id.is_empty()) else {
        return Ok(empty(ChatReason::NoSession));
    };
    let cwd = cwd_of(ws, pane);
    let read = ChatRead {
        session_id,
        cwd: cwd.clone(),
        before: before.clone(),
        limit,
    };
    let page = history_page(mobile, format, read).await?;
    let Some(page) = page else {
        return Ok(empty(ChatReason::NoTranscript));
    };
    let book = mobile.book();
    let pending = book.chat.pending_of(&pane.id, &page, activity, &cwd);
    // What runs now comes last: on the newest page alone.
    let mut items = page.items.clone();
    if before.is_none() {
        items.extend(
            book.chat
                .running_of(&pane.id, &page, pending.as_ref(), &cwd),
        );
    }
    Ok(chat(items, page.older, page.cursor.clone(), pending, None))
}

#[cfg(test)]
mod tests {
    //! A Codex pane attached to its server: what Hangar's client of that
    //! server reports, in the shapes it reports them (hangar-core
    //! agent/codex_attach.rs: `tool_of`, `on_request`), kept beside a page of
    //! T9's chat items (core chat/codex.rs), as the mod's reports are kept
    //! beside a Claude Code transcript.

    use serde_json::json;

    use hangar_core::chat::codex;
    use hangar_core::codex_app::generated::ThreadItem;

    use super::*;
    use crate::state::AgentId;

    const PANE: &str = "codex-pane";
    const CWD: &str = "/repo";

    fn note(live: &mut ChatLive, event: Value, at: u64) {
        let event = BridgeEvent::parse(&event).expect("a report the window reads");
        live.note(PANE, &event, at);
    }

    /// A thread item as T9 makes it a chat item.
    fn item(json: Value, at: i64) -> ChatItem {
        let item: ThreadItem = serde_json::from_value(json).expect("a protocol item");
        codex::item(&item, at, Some(CWD)).expect("an item the chat shows")
    }

    fn page() -> ChatPage {
        ChatPage {
            items: vec![
                item(
                    json!({"type": "userMessage", "id": "u1",
                           "content": [{"type": "text", "text": "Run the tests"}]}),
                    1_000,
                ),
                item(
                    json!({"type": "commandExecution", "id": "item-3", "command": "cargo build",
                           "commandActions": [], "cwd": CWD, "status": "completed",
                           "aggregatedOutput": "Finished"}),
                    2_000,
                ),
            ],
            older: false,
            cursor: None,
            question: None,
            open_call: None,
        }
    }

    #[test]
    fn a_pane_s_history_is_the_one_its_agent_writes() {
        let pane = |agent| Pane {
            agent,
            ..Pane::default()
        };
        assert_eq!(
            transcript_of(&pane(AgentId::Claude)),
            Some(TranscriptFormat::ClaudeJsonl)
        );
        assert_eq!(
            transcript_of(&pane(AgentId::Codex)),
            Some(TranscriptFormat::CodexRollout)
        );
        assert_eq!(transcript_of(&pane(AgentId::Shell)), None);
    }

    #[test]
    fn a_codex_command_waiting_on_its_approval_is_pending_and_not_running() {
        let mut live = ChatLive::default();
        // The build finished: in the page already, its live call left out.
        note(
            &mut live,
            json!({"kind": "tool", "phase": "start", "toolUseId": "item-3", "tool": "Bash",
                   "input": {"command": "cargo build", "cwd": CWD}}),
            1_500,
        );
        note(
            &mut live,
            json!({"kind": "tool", "phase": "start", "toolUseId": "item-7", "tool": "Bash",
                   "input": {"command": "pnpm test", "cwd": CWD}}),
            3_000,
        );
        note(
            &mut live,
            json!({"kind": "tool", "phase": "start", "toolUseId": "item-8",
                   "tool": "mcp__github__list_issues", "input": {"state": "open"}}),
            3_100,
        );
        note(
            &mut live,
            json!({"kind": "permission", "waiting": true, "tool": "Bash",
                   "input": {"command": "pnpm test", "cwd": CWD, "reason": "needs the network"},
                   "askId": "codex-1-7", "toolUseId": "item-7"}),
            3_200,
        );
        let page = page();
        let pending = live.pending_of(PANE, &page, Some(AgentActivity::Waiting), CWD);
        let Some(ChatPending::Permission {
            tool,
            title,
            detail,
            prompt,
            plan,
        }) = pending.clone()
        else {
            panic!("no permission pending: {pending:?}");
        };
        assert_eq!(tool, "Bash");
        assert_eq!(title, "Run pnpm test");
        assert_eq!(detail.as_deref(), Some("pnpm test"));
        assert_eq!(plan, None);
        assert_eq!(
            prompt,
            prompt_of(Some("item-7"), "Bash", &title, detail.as_deref())
        );
        let running = live.running_of(PANE, &page, pending.as_ref(), CWD);
        let ids: Vec<&str> = running.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, ["item-8"], "the call the dialog holds has not run yet");
        assert!(matches!(
            &running[0].body,
            Body::Action { title, state: CallState::Running, .. } if title == "Used list_issues"
        ));
        assert_eq!(
            live.permission_input(PANE, "Bash"),
            Some(json!({"command": "pnpm test", "cwd": CWD, "reason": "needs the network"}))
        );
        // The turn over: nothing waits, nothing runs.
        note(
            &mut live,
            json!({"kind": "turn", "phase": "end", "reason": "answer"}),
            4_000,
        );
        assert_eq!(
            live.pending_of(PANE, &page, Some(AgentActivity::Waiting), CWD),
            None
        );
        assert!(live.running_of(PANE, &page, None, CWD).is_empty());
    }

    #[test]
    fn a_codex_patch_waiting_on_its_approval_reads_as_the_edit_it_asks() {
        let mut live = ChatLive::default();
        note(
            &mut live,
            json!({"kind": "permission", "waiting": true, "tool": "apply_patch",
                   "input": {"changes": [{"path": "/repo/src/lib.rs", "kind": "update", "diff": "+pub mod x;"}]},
                   "askId": "codex-1-9", "toolUseId": "item-9"}),
            5_000,
        );
        let pending = live.pending_of(PANE, &page(), Some(AgentActivity::Waiting), CWD);
        assert!(matches!(
            pending,
            Some(ChatPending::Permission { ref tool, ref title, ref detail, .. })
                if tool == "apply_patch" && title == "Edit src/lib.rs"
                    && detail.as_deref() == Some("+pub mod x;")
        ));
    }

    #[test]
    fn a_codex_question_is_pending_with_what_codex_said_of_it() {
        let mut live = ChatLive::default();
        let input = json!({"questions": [{
            "id": "deploy", "question": "Deploy now?", "header": "Deploy", "multiSelect": false,
            "options": [{"label": "Yes", "description": "Ship it"}, {"label": "No", "description": ""}],
            "isOther": true, "isSecret": false,
        }]});
        note(
            &mut live,
            json!({"kind": "tool", "phase": "start", "toolUseId": "item-11",
                   "tool": "AskUserQuestion", "input": input}),
            6_000,
        );
        note(
            &mut live,
            json!({"kind": "permission", "waiting": true, "tool": "AskUserQuestion",
                   "input": input, "askId": "codex-1-21", "toolUseId": "item-11"}),
            6_001,
        );
        let pending = live.pending_of(PANE, &page(), Some(AgentActivity::Waiting), CWD);
        let Some(ChatPending::Question {
            tool_use_id,
            questions,
        }) = pending
        else {
            panic!("no question pending: {pending:?}");
        };
        assert_eq!(tool_use_id, "item-11");
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].question, "Deploy now?");
        assert!(!questions[0].multi_select);
        assert_eq!(live.question_input(PANE, "item-11"), Some(input));
        assert_eq!(live.question_input(PANE, "item-12"), None);
        // Not a call that runs: the pending card shows it.
        assert!(live.running_of(PANE, &page(), None, CWD).is_empty());
        // Answered: the call ends with the ask.
        note(
            &mut live,
            json!({"kind": "tool", "phase": "end", "toolUseId": "item-11"}),
            7_000,
        );
        assert_eq!(live.question(PANE), None);
    }
}
