//! A Codex thread's conversation as chat items: the `ChatItem`s and
//! `ChatPage`s a Claude Code transcript gives (mod.rs), so the chat view,
//! Hangar Mobile and Discord read either agent the same way
//! (docs/native/agents.md §4.2.7, T9). Two sources, in two shapes:
//!
//! - the app server's protocol (`codex_app::generated`): the `ThreadItem`s of
//!   a `thread/items/list` page or of an `item/completed` notification, for a
//!   pane Hangar is attached to;
//! - the thread's rollout file, for a thread no server holds: read span by
//!   span from its end like a Claude Code transcript (mod.rs), or whole when
//!   it is compressed.
//!
//! **A rollout's items are not the protocol's.** As read in openai/codex at
//! rust-v0.170.0: `rollout/src/policy.rs` (`persisted_rollout_item`) writes
//! core's `EventMsg` as it is (`history/src/rollout_payload.rs`, an
//! `event_msg` record), and its `item_completed` carries core's `TurnItem`
//! (`protocol/src/items.rs`): PascalCase tags (`"UserMessage"`), snake_case
//! fields except `McpToolCallItem`'s, a message's text in a `content` array, a
//! command as its words, a patch as a map of paths, and web searches and
//! generated images also as `Extension` items tagged by `kind`
//! (`ext/items/src/lib.rs`). The app server turns those into its camelCase
//! `ThreadItem`s on the way out (`app-server-protocol/src/protocol/v2/item.rs`,
//! `impl From<CoreTurnItem> for ThreadItemPayload`). So the generated
//! decoders read the protocol only; a rollout's shapes are decoded here from
//! their own definitions, and both end in the same descriptions (tools.rs):
//! a conversation reads the same whether Hangar is attached or not.
//!
//! **Legacy and paginated rollouts.** Since Codex 0.158 a new thread's history
//! is "paginated": its messages and tool calls are `item_completed` events. An
//! older thread's is "legacy": `user_message` and `agent_message` events and
//! its tools' `*_end` events. `policy.rs` writes each set in its own mode
//! only, so both are read wherever they are, as the app server replays a
//! rollout (`app-server-protocol/src/protocol/thread_history.rs`). Like that
//! replay, the `response_item`s — what the model was sent — are left out: a
//! legacy thread's commands show where Codex kept an `exec_command_end`.
//!
//! What shows: the person's and Hangar's messages, Codex's messages, command
//! runs, patches, MCP and client tool calls, web searches, generated images
//! (an image item once Codex completed and saved one, by agent/images.rs's
//! rule; the call while it runs, or when it failed or went unsaved),
//! a proposed plan (as Claude Code's "Presented a plan"), a sub-agent started
//! (as Claude Code's Agent), compactions and interruptions as notices. Left
//! out on purpose: reasoning (as Claude Code's thinking), hook prompts and
//! bare function outputs (Codex's plumbing), a sub-agent's later activity and
//! the other calls between agents, a look at an image, sleeps, review-mode
//! markers — and any item kind a later Codex adds (`Unrecognized` in the
//! protocol, an unknown tag in a rollout), or a rollout line that does not
//! decode: skipped, never a failed page. A rollback leaves the turns it undid
//! in the file, and they show, as both branches of a Claude Code rewind do.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::thread::{from_app, COMPACTED, INTERRUPTED};
use super::tools::{self, clip, Change, ChangeKind, Described, WebAction, CLIP_BYTES, TEXT_BYTES};
use super::{
    Body, CacheOf, CallResult, CallState, ChatItem, ChatPage, Lines, OpenCall, PendingQuestion,
    Sender, Span,
};
use crate::agent::images;
use crate::codex_app::generated as protocol;
use crate::sessions::codex::Record;

// ---------------------------------------------------------------------------
// What both shapes become
// ---------------------------------------------------------------------------

/// A message from the person, or from Hangar when it starts with "[Teskad]" (or "[Hangar]", as it did before the rename).
fn user(text: &str, images: u32) -> Option<Body> {
    let text = text.trim();
    if text.is_empty() && images == 0 {
        return None;
    }
    let from = if from_app(text) {
        Sender::Hangar
    } else {
        Sender::Person
    };
    Some(Body::User {
        text: clip(text, TEXT_BYTES),
        from,
        images,
    })
}

fn assistant(text: &str) -> Option<Body> {
    (!text.trim().is_empty()).then(|| Body::Assistant {
        text: clip(text, TEXT_BYTES),
    })
}

fn notice(text: &str) -> Body {
    Body::Notice {
        text: text.to_string(),
    }
}

fn action(tool: &str, described: Described, result: Option<CallResult>, state: CallState) -> Body {
    Body::Action {
        tool: tool.to_string(),
        title: described.title,
        detail: described.detail,
        result,
        state,
    }
}

/// A finished call's result; none while it runs.
fn result(state: CallState, text: &str) -> Option<CallResult> {
    (state != CallState::Running).then(|| CallResult {
        ok: state == CallState::Done,
        text: clip(text, CLIP_BYTES),
    })
}

/// What a Codex status says of its call. The protocol spells statuses in
/// camelCase, a rollout in snake_case or camelCase (`McpToolCallStatus`); one
/// this version does not know is taken for finished.
fn state_of(status: &str) -> CallState {
    match status {
        "inProgress" | "in_progress" | "generating" => CallState::Running,
        "failed" | "declined" | "interrupted" | "incomplete" => CallState::Error,
        _ => CallState::Done,
    }
}

/// `state_of` a protocol status enum, by the name it goes by on the wire.
fn state_in(status: &impl Serialize) -> CallState {
    match serde_json::to_value(status) {
        Ok(Value::String(status)) => state_of(&status),
        _ => CallState::Done,
    }
}

fn command(script: String, state: CallState, output: Option<&str>) -> Body {
    action(
        "commandExecution",
        tools::command(&script),
        result(state, output.unwrap_or("")),
        state,
    )
}

fn patch(changes: &[Change], state: CallState, cwd: Option<&str>) -> Body {
    action("fileChange", tools::patch(changes, cwd), None, state)
}

/// An MCP tool's call, named as Claude Code names one: `mcp__<server>__<tool>`.
fn mcp(server: &str, tool: &str, arguments: &Value, state: CallState, text: &str) -> Body {
    action(
        &format!("mcp__{server}__{tool}"),
        tools::used(tool, arguments),
        result(state, text),
        state,
    )
}

/// What an MCP result's content says, and whether the server called it an
/// error.
fn mcp_outcome(content: &[Value], is_error: Option<bool>) -> (String, bool) {
    (
        tools::result_text(Some(&Value::Array(content.to_vec()))),
        is_error == Some(true),
    )
}

/// A web search's action, from either shape: the protocol's and the search
/// extension's camelCase tags, the Responses API's snake_case ones in core's
/// hosted `WebSearch` item and legacy `web_search_end`.
fn web_action(action: Option<&Value>) -> WebAction {
    let Some(action) = action else {
        return WebAction::Other;
    };
    let text = |key: &str| action.get(key).and_then(Value::as_str).map(str::to_string);
    match action.get("type").and_then(Value::as_str) {
        Some("search") => {
            let mut queries: Vec<String> = action
                .get("queries")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|q| q.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if queries.is_empty() {
                queries.extend(text("query"));
            }
            WebAction::Search { queries }
        }
        Some("openPage" | "open_page") => WebAction::OpenPage { url: text("url") },
        Some("findInPage" | "find_in_page") => WebAction::FindInPage {
            url: text("url"),
            pattern: text("pattern"),
        },
        _ => WebAction::Other,
    }
}

fn web_search(query: &str, action: Option<&Value>) -> Body {
    action_done("webSearch", tools::web_search(query, &web_action(action)))
}

fn action_done(tool: &str, described: Described) -> Body {
    action(tool, described, None, CallState::Done)
}

/// A generation of an image: the image, when Codex completed and saved it
/// (the rule is agent/images.rs's); otherwise the call, the prompt as its
/// detail, where it was saved, if anywhere, as its result.
fn image(
    status: &str,
    prompt: Option<&str>,
    saved: Option<&str>,
    failed: bool,
    cwd: Option<&str>,
) -> Body {
    if let Some(path) = images::saved(status, failed, saved) {
        return Body::Image {
            path: path.to_string(),
            prompt: prompt
                .map(str::trim)
                .filter(|prompt| !prompt.is_empty())
                .map(|prompt| clip(prompt, TEXT_BYTES)),
        };
    }
    let state = if failed {
        CallState::Error
    } else {
        state_of(status)
    };
    let saved = saved
        .map(|path| tools::shorten(path, cwd))
        .unwrap_or_default();
    action(
        "imageGeneration",
        tools::image_generation(prompt),
        result(state, &saved),
        state,
    )
}

fn plan(text: &str) -> Option<Body> {
    // An empty plan is the start of one: the app server leaves it out too.
    (!text.trim().is_empty()).then(|| action_done("plan", tools::plan(text)))
}

fn agent(prompt: Option<&str>, state: CallState) -> Body {
    action("collabAgentToolCall", tools::agent(prompt), None, state)
}

// ---------------------------------------------------------------------------
// The protocol's items
// ---------------------------------------------------------------------------

/// One protocol item as a chat item written at `at` (ms), its paths relative
/// to `cwd`; `None` for an item the chat leaves out (see the module).
pub fn item(item: &protocol::ThreadItem, at: i64, cwd: Option<&str>) -> Option<ChatItem> {
    use protocol::ThreadItem as Item;
    let (id, body) = match item {
        Item::UserMessage(message) => {
            let mut texts = Vec::new();
            let mut images = 0;
            for input in &message.content {
                match input {
                    protocol::UserInput::Text(text) => texts.push(text.text.as_str()),
                    protocol::UserInput::Image(_) | protocol::UserInput::LocalImage(_) => {
                        images += 1
                    }
                    _ => {}
                }
            }
            (&message.id, user(&texts.join("\n\n"), images)?)
        }
        Item::AgentMessage(message) => (&message.id, assistant(&message.text)?),
        Item::Plan(item) => (&item.id, plan(&item.text)?),
        Item::CommandExecution(run) => (
            &run.id,
            command(
                tools::script_of_line(&run.command),
                state_in(&run.status),
                run.aggregated_output.as_deref(),
            ),
        ),
        Item::FileChange(item) => {
            let changes: Vec<Change> = item
                .changes
                .iter()
                .map(|change| Change {
                    path: change.path.clone(),
                    kind: match change.kind {
                        protocol::PatchChangeKind::Add => ChangeKind::Add,
                        protocol::PatchChangeKind::Delete => ChangeKind::Delete,
                        _ => ChangeKind::Update,
                    },
                    diff: change.diff.clone(),
                })
                .collect();
            (&item.id, patch(&changes, state_in(&item.status), cwd))
        }
        Item::McpToolCall(call) => {
            let (text, failed) = match (&call.error, &call.result) {
                (Some(error), _) => (error.message.clone(), true),
                (None, Some(result)) => mcp_outcome(&result.content, None),
                (None, None) => (String::new(), false),
            };
            let state = match state_in(&call.status) {
                CallState::Done if failed => CallState::Error,
                state => state,
            };
            (
                &call.id,
                mcp(&call.server, &call.tool, &call.arguments, state, &text),
            )
        }
        Item::DynamicToolCall(call) => {
            let text = call
                .content_items
                .iter()
                .flatten()
                .filter_map(|part| match part {
                    protocol::DynamicToolCallOutputContentItem::InputText(text) => {
                        Some(text.text.clone())
                    }
                    protocol::DynamicToolCallOutputContentItem::InputImage(_) => {
                        Some("[image]".to_string())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            let state = match state_in(&call.status) {
                CallState::Done if call.success == Some(false) => CallState::Error,
                state => state,
            };
            let described = tools::used(&call.tool, &call.arguments);
            (
                &call.id,
                action(&call.tool, described, result(state, &text), state),
            )
        }
        Item::CollabAgentToolCall(call) if call.tool == protocol::CollabAgentTool::SpawnAgent => (
            &call.id,
            agent(call.prompt.as_deref(), state_in(&call.status)),
        ),
        Item::WebSearch(search) => {
            let action = search
                .action
                .as_ref()
                .and_then(|action| serde_json::to_value(action).ok());
            (&search.id, web_search(&search.query, action.as_ref()))
        }
        Item::ImageGeneration(item) => (
            &item.id,
            image(
                &item.status,
                item.revised_prompt.as_deref(),
                item.saved_path.as_deref(),
                item.failure.is_some(),
                cwd,
            ),
        ),
        Item::ContextCompaction(item) => (&item.id, notice(COMPACTED)),
        _ => return None,
    };
    Some(ChatItem {
        id: id.clone(),
        at,
        body,
    })
}

/// The item of an `item/completed` notification, at its completion time.
pub fn completed(
    notification: &protocol::ItemCompletedNotification,
    cwd: Option<&str>,
) -> Option<ChatItem> {
    item(&notification.item, notification.completed_at_ms, cwd)
}

/// A `thread/items/list` page asked newest first (`sortDirection: "desc"`),
/// as the chat's page: its items oldest first, and its `nextCursor` as the
/// cursor of the page before it. What waits at the conversation's end —
/// an approval, a call running — is the live feed's (T4c), not a page's.
///
/// Assumption (from the protocol schema's descriptions, not run): a
/// descending page lists its items newest first, and its `nextCursor`
/// continues further back.
pub fn items_page(page: &protocol::ThreadItemsListResponse, cwd: Option<&str>) -> ChatPage {
    let items = page
        .data
        .iter()
        .rev()
        .filter_map(|entry| {
            let at = entry.completed_at_ms.or(entry.started_at_ms).unwrap_or(0);
            item(&entry.item, at, cwd)
        })
        .collect();
    ChatPage {
        items,
        older: page.next_cursor.is_some(),
        cursor: page.next_cursor.clone(),
        question: None,
        open_call: None,
    }
}

// ---------------------------------------------------------------------------
// A rollout's lines
// ---------------------------------------------------------------------------

/// Core's `TurnItem`, as a rollout's `item_completed` carries it
/// (`protocol/src/items.rs` at rust-v0.170.0). Only what the chat shows is
/// read; every other kind, known or added later, is `Other`.
#[derive(Deserialize)]
#[serde(tag = "type")]
enum TurnItem {
    UserMessage {
        id: String,
        #[serde(default)]
        content: Vec<UserInput>,
    },
    AgentMessage {
        id: String,
        #[serde(default)]
        content: Vec<AgentText>,
    },
    Plan {
        id: String,
        text: String,
    },
    CommandExecution {
        id: String,
        command: Vec<String>,
        status: String,
        aggregated_output: Option<String>,
    },
    FileChange {
        id: String,
        #[serde(default)]
        changes: HashMap<String, FileChange>,
        status: Option<String>,
    },
    // `McpToolCallItem` is camelCase: these fields are one word either way.
    McpToolCall {
        id: String,
        server: String,
        tool: String,
        #[serde(default)]
        arguments: Value,
        status: String,
        result: Option<ToolResult>,
        error: Option<ToolError>,
    },
    DynamicToolCall {
        id: String,
        tool: String,
        #[serde(default)]
        arguments: Value,
        status: String,
        content_items: Option<Vec<Value>>,
        success: Option<bool>,
    },
    CollabAgentToolCall {
        id: String,
        tool: String,
        status: String,
        prompt: Option<String>,
    },
    /// The Responses API's hosted search.
    WebSearch {
        id: String,
        query: String,
        action: Option<Value>,
    },
    /// The Responses API's hosted generation.
    ImageGeneration {
        id: String,
        status: String,
        revised_prompt: Option<String>,
        saved_path: Option<String>,
    },
    Extension(Extension),
    ContextCompaction {
        id: String,
    },
    #[serde(other)]
    Other,
}

/// The items an extension owns (`ext/items/src/lib.rs`), tagged by `kind`
/// with camelCase fields: the standalone web search and image generation.
#[derive(Deserialize)]
#[serde(tag = "kind")]
enum Extension {
    #[serde(rename = "web.search")]
    WebSearch {
        id: String,
        query: String,
        action: Option<Value>,
    },
    #[serde(rename = "image_gen.generation", rename_all = "camelCase")]
    ImageGeneration {
        id: String,
        status: String,
        revised_prompt: Option<String>,
        saved_path: Option<String>,
        failure: Option<Value>,
    },
    #[serde(other)]
    Other,
}

/// Core's `UserInput` (`protocol/src/user_input.rs`).
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum UserInput {
    Text {
        text: String,
    },
    Image {},
    LocalImage {},
    #[serde(other)]
    Other,
}

/// Core's `AgentMessageContent`: `{"type": "Text", "text": …}`, its one kind.
#[derive(Deserialize)]
struct AgentText {
    text: Option<String>,
}

/// Core's `FileChange` (`protocol/src/protocol.rs`).
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum FileChange {
    Add {
        content: String,
    },
    Delete {
        content: String,
    },
    Update {
        unified_diff: String,
        move_path: Option<String>,
    },
}

/// MCP's `CallToolResult` (`protocol/src/mcp.rs`), camelCase.
#[derive(Deserialize)]
struct ToolResult {
    #[serde(default)]
    content: Vec<Value>,
    #[serde(rename = "isError")]
    is_error: Option<bool>,
}

#[derive(Deserialize)]
struct ToolError {
    message: String,
}

/// A patch's files, sorted by path, each change's text as the app server
/// gives it (`item_builders.rs`, `format_file_change_diff`).
fn changes(map: HashMap<String, FileChange>) -> Vec<Change> {
    let mut changes: Vec<Change> = map
        .into_iter()
        .map(|(path, change)| {
            let (kind, diff) = match change {
                FileChange::Add { content } => (ChangeKind::Add, content),
                FileChange::Delete { content } => (ChangeKind::Delete, content),
                FileChange::Update {
                    unified_diff,
                    move_path: Some(to),
                } => (
                    ChangeKind::Update,
                    format!("{unified_diff}\n\nMoved to: {to}"),
                ),
                FileChange::Update { unified_diff, .. } => (ChangeKind::Update, unified_diff),
            };
            Change { path, kind, diff }
        })
        .collect();
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    changes
}

/// A rollout's turn item made into what the chat shows, with its id.
fn turn_item(item: TurnItem, cwd: Option<&str>) -> Option<(String, Body)> {
    Some(match item {
        TurnItem::UserMessage { id, content } => {
            let mut texts = Vec::new();
            let mut images = 0;
            for input in content {
                match input {
                    UserInput::Text { text } => texts.push(text),
                    UserInput::Image {} | UserInput::LocalImage {} => images += 1,
                    UserInput::Other => {}
                }
            }
            (id, user(&texts.join("\n\n"), images)?)
        }
        TurnItem::AgentMessage { id, content } => {
            // The app server joins the parts as they are.
            let text: String = content.into_iter().filter_map(|part| part.text).collect();
            (id, assistant(&text)?)
        }
        TurnItem::Plan { id, text } => (id, plan(&text)?),
        TurnItem::CommandExecution {
            id,
            command: words,
            status,
            aggregated_output,
        } => (
            id,
            command(
                tools::script_of_words(&words),
                state_of(&status),
                aggregated_output.as_deref(),
            ),
        ),
        TurnItem::FileChange {
            id,
            changes: map,
            status,
        } => {
            // No status yet is in progress, as the app server reads it.
            let state = status.as_deref().map_or(CallState::Running, state_of);
            (id, patch(&changes(map), state, cwd))
        }
        TurnItem::McpToolCall {
            id,
            server,
            tool,
            arguments,
            status,
            result,
            error,
        } => {
            let (text, failed) = match (error, result) {
                (Some(error), _) => (error.message, true),
                (None, Some(result)) => mcp_outcome(&result.content, result.is_error),
                (None, None) => (String::new(), false),
            };
            let state = match state_of(&status) {
                CallState::Done if failed => CallState::Error,
                state => state,
            };
            (id, mcp(&server, &tool, &arguments, state, &text))
        }
        TurnItem::DynamicToolCall {
            id,
            tool,
            arguments,
            status,
            content_items,
            success,
        } => {
            let text = content_items
                .iter()
                .flatten()
                .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("input_text" | "inputText") => {
                        part.get("text").and_then(Value::as_str).map(str::to_string)
                    }
                    Some("input_image" | "inputImage") => Some("[image]".to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            let state = match state_of(&status) {
                CallState::Done if success == Some(false) => CallState::Error,
                state => state,
            };
            let described = tools::used(&tool, &arguments);
            (id, action(&tool, described, result(state, &text), state))
        }
        TurnItem::CollabAgentToolCall {
            id,
            tool,
            status,
            prompt,
        } if tool == "spawn_agent" => (id, agent(prompt.as_deref(), state_of(&status))),
        TurnItem::WebSearch { id, query, action }
        | TurnItem::Extension(Extension::WebSearch { id, query, action }) => {
            (id, web_search(&query, action.as_ref()))
        }
        TurnItem::ImageGeneration {
            id,
            status,
            revised_prompt,
            saved_path,
        } => (
            id,
            image(
                &status,
                revised_prompt.as_deref(),
                saved_path.as_deref(),
                false,
                cwd,
            ),
        ),
        TurnItem::Extension(Extension::ImageGeneration {
            id,
            status,
            revised_prompt,
            saved_path,
            failure,
        }) => (
            id,
            image(
                &status,
                revised_prompt.as_deref(),
                saved_path.as_deref(),
                failure.is_some_and(|failure| !failure.is_null()),
                cwd,
            ),
        ),
        TurnItem::ContextCompaction { id } => (id, notice(COMPACTED)),
        TurnItem::CollabAgentToolCall { .. }
        | TurnItem::Extension(Extension::Other)
        | TurnItem::Other => return None,
    })
}

/// An `event_msg`'s type, read before the rest of it.
#[derive(Deserialize)]
struct EventType<'a> {
    #[serde(rename = "type", borrow)]
    kind: Cow<'a, str>,
}

#[derive(Deserialize)]
struct ItemCompleted {
    item: TurnItem,
}

// The legacy events, from `protocol/src/protocol.rs` at rust-v0.170.0.

#[derive(Deserialize)]
struct UserMessageEvent {
    message: String,
    images: Option<Vec<Value>>,
    #[serde(default)]
    local_images: Vec<Value>,
}

#[derive(Deserialize)]
struct AgentMessageEvent {
    message: String,
}

#[derive(Deserialize)]
struct ExecCommandEnd {
    call_id: String,
    command: Vec<String>,
    #[serde(default)]
    aggregated_output: String,
    status: String,
}

#[derive(Deserialize)]
struct PatchApplyEnd {
    call_id: String,
    #[serde(default)]
    changes: HashMap<String, FileChange>,
    status: String,
}

#[derive(Deserialize)]
struct McpToolCallEnd {
    call_id: String,
    invocation: McpInvocation,
    /// A Rust `Result`: `{"Ok": CallToolResult}` or `{"Err": message}`.
    result: Value,
}

#[derive(Deserialize)]
struct McpInvocation {
    server: String,
    tool: String,
    arguments: Option<Value>,
}

#[derive(Deserialize)]
struct WebSearchEnd {
    call_id: String,
    query: String,
    action: Option<Value>,
}

#[derive(Deserialize)]
struct ImageGenerationEnd {
    call_id: String,
    status: String,
    revised_prompt: Option<String>,
    saved_path: Option<String>,
    failure: Option<Value>,
}

#[derive(Deserialize)]
struct TurnAborted {
    reason: Option<String>,
}

/// An `event_msg` payload made into what the chat shows, with its id when it
/// has one of its own.
fn event(payload: &str, cwd: Option<&str>) -> Option<(Option<String>, Body)> {
    fn read<'a, T: Deserialize<'a>>(payload: &'a str) -> Option<T> {
        serde_json::from_str(payload).ok()
    }
    let kind: EventType = read(payload)?;
    let (id, body) = match &*kind.kind {
        "item_completed" => {
            let (id, body) = turn_item(read::<ItemCompleted>(payload)?.item, cwd)?;
            (Some(id), body)
        }
        "user_message" => {
            let event: UserMessageEvent = read(payload)?;
            let images = event.images.map_or(0, |list| list.len()) + event.local_images.len();
            (None, user(&event.message, images as u32)?)
        }
        "agent_message" => (
            None,
            assistant(&read::<AgentMessageEvent>(payload)?.message)?,
        ),
        "exec_command_end" => {
            let end: ExecCommandEnd = read(payload)?;
            let body = command(
                tools::script_of_words(&end.command),
                state_of(&end.status),
                Some(&end.aggregated_output),
            );
            (Some(end.call_id), body)
        }
        "patch_apply_end" => {
            let end: PatchApplyEnd = read(payload)?;
            let body = patch(&changes(end.changes), state_of(&end.status), cwd);
            (Some(end.call_id), body)
        }
        "mcp_tool_call_end" => {
            let end: McpToolCallEnd = read(payload)?;
            let (text, failed) = match (end.result.get("Ok"), end.result.get("Err")) {
                (Some(ok), _) => {
                    let result: ToolResult = serde_json::from_value(ok.clone()).ok()?;
                    mcp_outcome(&result.content, result.is_error)
                }
                (None, Some(err)) => (err.as_str().unwrap_or_default().to_string(), true),
                (None, None) => (String::new(), false),
            };
            let state = if failed {
                CallState::Error
            } else {
                CallState::Done
            };
            let call = end.invocation;
            let arguments = call.arguments.unwrap_or(Value::Null);
            let body = mcp(&call.server, &call.tool, &arguments, state, &text);
            (Some(end.call_id), body)
        }
        "web_search_end" => {
            let end: WebSearchEnd = read(payload)?;
            (
                Some(end.call_id),
                web_search(&end.query, end.action.as_ref()),
            )
        }
        "image_generation_end" => {
            let end: ImageGenerationEnd = read(payload)?;
            let body = image(
                &end.status,
                end.revised_prompt.as_deref(),
                end.saved_path.as_deref(),
                end.failure.is_some_and(|failure| !failure.is_null()),
                cwd,
            );
            (Some(end.call_id), body)
        }
        "context_compacted" => (None, notice(COMPACTED)),
        // Replaced by a steer, ended with a review, out of budget: the
        // conversation goes on, nothing to say.
        "turn_aborted" => {
            let aborted: TurnAborted = read(payload)?;
            if aborted.reason.as_deref() != Some("interrupted") {
                return None;
            }
            (None, notice(INTERRUPTED))
        }
        _ => return None,
    };
    Some((id, body))
}

fn time_of(timestamp: Option<&str>) -> i64 {
    timestamp
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.timestamp_millis())
        .unwrap_or(0)
}

struct Entry {
    /// Where its line begins in the (decoded) file: the cursor.
    offset: u64,
    item: ChatItem,
}

/// A rollout's chat items, one span of the file at a time (mod.rs). Every
/// line is one item at most, and each is final as written: a rollout records
/// items once completed, so nothing in it is waiting.
#[derive(Default)]
pub(super) struct Rollout {
    /// The thread's folder: paths show relative to it.
    cwd: Option<String>,
    entries: Vec<Entry>,
    /// Each item's place in `entries`, by id: an item written again — a
    /// message corrected — takes the place of the one before, as the app
    /// server's replay does.
    places: HashMap<String, usize>,
}

impl Rollout {
    fn new(cwd: &str) -> Self {
        Self {
            cwd: (!cwd.is_empty()).then(|| cwd.to_string()),
            ..Self::default()
        }
    }

    fn push(&mut self, offset: u64, item: ChatItem) {
        match self.places.get(&item.id) {
            Some(&at) => self.entries[at].item = item,
            None => {
                self.places.insert(item.id.clone(), self.entries.len());
                self.entries.push(Entry { offset, item });
            }
        }
    }
}

impl Lines for Rollout {
    fn fresh(&self) -> Self {
        Self {
            cwd: self.cwd.clone(),
            ..Self::default()
        }
    }

    fn feed(&mut self, offset: u64, text: &str) {
        let Ok(record) = serde_json::from_str::<Record>(text) else {
            return;
        };
        if record.kind.as_deref() != Some("event_msg") {
            return;
        }
        let Some((id, body)) = record
            .payload
            .and_then(|payload| event(payload.get(), self.cwd.as_deref()))
        else {
            return;
        };
        let item = ChatItem {
            id: id.unwrap_or_else(|| format!("line-{offset}")),
            at: time_of(record.timestamp.as_deref()),
            body,
        };
        self.push(offset, item);
    }

    fn prepend(&mut self, older: Self) {
        let mut joined = older;
        for entry in std::mem::take(&mut self.entries) {
            joined.push(entry.offset, entry.item);
        }
        self.entries = joined.entries;
        self.places = joined.places;
    }

    fn count_before(&self, before: Option<u64>) -> usize {
        match before {
            Some(before) => self.entries.partition_point(|e| e.offset < before),
            None => self.entries.len(),
        }
    }

    fn page(&self, before: Option<u64>, limit: usize) -> (Vec<ChatItem>, Option<u64>, bool) {
        let end = self.count_before(before);
        let start = end.saturating_sub(limit);
        let items = self.entries[start..end]
            .iter()
            .map(|e| e.item.clone())
            .collect();
        let first = (start < end).then(|| self.entries[start].offset);
        (items, first, start > 0)
    }

    fn pending_question(&self) -> Option<PendingQuestion> {
        None
    }

    fn open_call(&self) -> Option<OpenCall> {
        None
    }
}

/// Rollouts kept parsed, apart from Claude Code's transcripts.
static ROLLOUTS: OnceLock<CacheOf<Rollout>> = OnceLock::new();

/// One page of the rollout at `path`, its paths relative to `cwd`; `None`
/// when there is no such file. `slot` holds what was parsed of it so far.
fn page_of(
    path: &Path,
    slot: &mut Option<Span<Rollout>>,
    cwd: &str,
    before: Option<u64>,
    limit: usize,
) -> std::io::Result<Option<ChatPage>> {
    super::page_in(path, slot, before, limit, || Rollout::new(cwd))
}

fn chat_of(
    thread_id: &str,
    cwd: &str,
    before: Option<&str>,
    limit: Option<usize>,
) -> Result<Option<ChatPage>, String> {
    let (before, limit) = super::chat_args(thread_id, before, limit)?;
    let Some(path) = crate::sessions::codex::locate(thread_id) else {
        return Ok(None);
    };
    let cache = ROLLOUTS.get_or_init(CacheOf::default);
    let slot = cache.slot(&path)?;
    // Only this rollout's lock is held across the reads.
    let mut rollout = slot.transcript.lock().map_err(|_| "chat cache poisoned")?;
    page_of(&path, &mut rollout, cwd, before, limit).map_err(|err| err.to_string())
}

/// One page of a Codex thread's conversation, from its rollout: the newest
/// without `before`, else the one before that cursor. `None` while the
/// thread has no rollout. `cwd` is the pane's folder, paths show relative
/// to it.
pub async fn chat_page(
    thread_id: String,
    cwd: String,
    before: Option<String>,
    limit: Option<usize>,
) -> Result<Option<ChatPage>, String> {
    crate::git::off_thread(move || chat_of(&thread_id, &cwd, before.as_deref(), limit)).await
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
