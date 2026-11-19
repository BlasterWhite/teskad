//! A Codex conversation read as chat items. Every fixture here was written by
//! hand (docs/native/agents.md §4.7, decision 1), never recorded: each says
//! where its shape comes from — "from the protocol schema" (Codex 0.170.0) or
//! "from the source" (openai/codex at rust-v0.170.0, with the file).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::*;
use crate::chat::tools::{self, Change, ChangeKind, WebAction};
use crate::chat::SPAN_BYTES;
use crate::sessions::codex::tests::{compress, LEGACY};

/// One conversation of three turns in a paginated rollout, from the source:
/// `history/src/rollout_payload.rs` (the records), `rollout/src/policy.rs`
/// (what a paginated thread keeps), `protocol/src/items.rs` and
/// `ext/items/src/lib.rs` (its items), `protocol/src/protocol.rs` (its
/// events). Its first line says so, and is a line no reader knows.
const PAGINATED: &str = include_str!(
    "../sessions/fixtures/rollout-2026-11-19T16-20-05-019aa0b1-6c2e-7d41-8f3a-1b2c3d4e5f60.jsonl"
);
const PAGINATED_NAME: &str =
    "rollout-2026-11-19T16-20-05-019aa0b1-6c2e-7d41-8f3a-1b2c3d4e5f60.jsonl";

/// Turns 1 and 2 of the same conversation as the app server lists them
/// (`thread/items/list`, newest first), from the protocol schema.
const ITEMS_PAGE: &str = include_str!("fixtures/codex-items-page.json");

const CWD: &str = "/home/dev/shop";

fn scratch(tag: &str, name: &str, bytes: &[u8]) -> PathBuf {
    let path = crate::testdir::scratch(&format!("chat-codex-{tag}")).join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn read(
    path: &Path,
    slot: &mut Option<Span<Rollout>>,
    before: Option<u64>,
    limit: usize,
) -> ChatPage {
    page_of(path, slot, CWD, before, limit)
        .unwrap()
        .expect("the file exists")
}

/// Every item of a rollout, oldest first.
fn all_of(text: &str) -> Vec<ChatItem> {
    let mut rollout = Rollout::new(CWD);
    let mut offset = 0;
    for line in text.lines() {
        rollout.feed(offset, line);
        offset += line.len() as u64 + 1;
    }
    rollout.page(None, usize::MAX).0
}

fn ids(items: &[ChatItem]) -> Vec<&str> {
    items.iter().map(|item| item.id.as_str()).collect()
}

/// An action's tool, title, detail, result and state.
fn action_of(item: &ChatItem) -> (&str, &str, Option<&str>, Option<&CallResult>, CallState) {
    match &item.body {
        Body::Action {
            tool,
            title,
            detail,
            result,
            state,
        } => (tool, title, detail.as_deref(), result.as_ref(), *state),
        other => panic!("not an action: {other:?}"),
    }
}

fn protocol_page() -> protocol::ThreadItemsListResponse {
    let doc: Value = serde_json::from_str(ITEMS_PAGE).unwrap();
    serde_json::from_value(doc["response"].clone()).expect("a thread/items/list answer")
}

/// The ids of the fixture's items the chat shows, oldest first: no
/// reasoning, no hologram.
const SHOWN: &[&str] = &[
    "019aa0b1-7d40-7f63-8b5c-3d4e5f607182",
    "call_Hd1rG7tq0uWc3sYx9zKp2LmN",
    "call_P4tchH3ad3rT0ta1x9Yz",
    "call_Mcp9b0aRd7Pr0gr3ssQw2E",
    "msg_0c1d2e3f405162738495a6b7",
    "019aa0b6-0a12-7c34-9d45-6e7f8091a2b3",
    "ws_0a1b2c3d4e5f60718293a4b5",
    "ws_1c2d3e4f5061728394a5b6c7",
    "ig_5e6f708192a3b4c5",
    "019aa0b6-2b34-7d56-8e67-7f8091a2b3c4",
    "019aa0b7-0c45-7e67-9f78-8091a2b3c4d5",
    "call_P4tch2St1cky8x7Wv6Ut5",
    "msg_1d2e3f405162738495a6b7c8",
    "019aa0bd-1b23-7d45-8e56-6f708192a3b4",
    "call_E2eRunF41l3dQ1w2E3r4T5",
];

#[test]
fn a_paginated_rollout_reads_as_the_chat_of_its_items() {
    let items = all_of(PAGINATED);
    let (last, shown) = items.split_last().unwrap();
    assert_eq!(ids(shown), SHOWN);
    assert!(last.id.starts_with("line-"));
    assert_eq!(
        last.body,
        Body::Notice {
            text: "Turn interrupted".into()
        }
    );

    assert_eq!(
        items[0].body,
        Body::User {
            text: "Show the cart total in the header".into(),
            from: Sender::Person,
            images: 1,
        }
    );
    // The record's time, in ms.
    assert_eq!(items[0].at, 1_795_105_210_404);

    let (tool, title, detail, result, state) = action_of(&items[1]);
    assert_eq!(tool, "commandExecution");
    assert_eq!(title, r#"Ran rg -n "total" src/header"#);
    assert_eq!(detail, Some(r#"rg -n "total" src/header"#));
    assert_eq!(
        result,
        Some(&CallResult {
            ok: true,
            text: "src/header/Header.tsx:8:  const total = useCartTotal();\n".into()
        })
    );
    assert_eq!(state, CallState::Done);

    let (tool, title, detail, result, state) = action_of(&items[2]);
    assert_eq!(
        (tool, title),
        ("fileChange", "Edited src/header/Header.tsx")
    );
    assert!(detail.unwrap().contains("+  <CartTotal value={total} />"));
    assert_eq!((result, state), (None, CallState::Done));

    let (tool, title, detail, result, _) = action_of(&items[3]);
    assert_eq!(tool, "mcp__hangar__board_report_progress");
    assert_eq!(title, "Used board_report_progress");
    assert_eq!(
        serde_json::from_str::<Value>(detail.unwrap()).unwrap(),
        json!({ "id": "t-42", "percent": 50, "note": "Header shows the total" })
    );
    assert_eq!(result.map(|r| r.text.as_str()), Some(r#"{"ok":true}"#));

    assert_eq!(
        items[4].body,
        Body::Assistant {
            text: "The header now shows the cart total.".into()
        }
    );
    let Body::User { from, .. } = &items[5].body else {
        panic!("{:?}", items[5])
    };
    assert_eq!(*from, Sender::Hangar);

    let (_, title, detail, _, _) = action_of(&items[6]);
    assert_eq!(title, "Searched the web for sticky header mobile safari");
    assert_eq!(
        detail,
        Some("sticky header mobile safari\nposition sticky ios")
    );
    let (_, title, _, _, _) = action_of(&items[7]);
    assert_eq!(
        title,
        "Fetched https://developer.mozilla.org/en-US/docs/Web/CSS/position"
    );

    let (tool, title, detail, result, state) = action_of(&items[8]);
    assert_eq!((tool, title), ("imageGeneration", "Generated an image"));
    assert_eq!(
        detail,
        Some("A phone-width header with the cart total on the right")
    );
    assert!(result.unwrap().text.ends_with("/ig_5e6f708192a3b4c5.png"));
    assert_eq!(state, CallState::Done);

    let (tool, title, detail, _, _) = action_of(&items[9]);
    assert_eq!((tool, title), ("plan", "Presented a plan"));
    assert_eq!(
        detail,
        Some("1. Make the header sticky\n2. Check it at 360 px")
    );
    assert_eq!(
        items[10].body,
        Body::Notice {
            text: "Conversation compacted".into()
        }
    );

    let (_, title, detail, _, _) = action_of(&items[11]);
    assert_eq!(title, "Edited 2 files");
    let detail = detail.unwrap();
    assert!(detail.starts_with("src/header/Header.test.tsx\nimport { render }"));
    // A deleted file is named, its old content not shown again.
    assert!(detail.ends_with("\n\nsrc/header/old.css"));

    let (_, title, _, result, state) = action_of(&items[14]);
    assert_eq!(title, "Ran pnpm e2e");
    assert_eq!(state, CallState::Error);
    assert_eq!(result.map(|r| r.ok), Some(false));
}

/// Attached or not, a conversation reads the same: the app server's page of
/// turns 1 and 2 gives the very items the rollout does.
#[test]
fn the_protocol_page_and_the_rollout_give_the_same_items() {
    let page = items_page(&protocol_page(), Some(CWD));
    let rollout = all_of(PAGINATED);
    assert_eq!(page.items, rollout[..13]);
    assert!(!page.older);
    assert_eq!(page.cursor, None);
    assert_eq!((page.question, page.open_call), (None, None));
}

#[test]
fn an_item_kind_the_protocol_does_not_know_never_fails_a_page() {
    let mut response = protocol_page();
    let unknown = response
        .data
        .iter()
        .filter(|entry| matches!(entry.item, protocol::ThreadItem::Unrecognized(_)))
        .count();
    assert_eq!(unknown, 1, "the hologram item");
    response.next_cursor = Some("older-1".into());
    let page = items_page(&response, Some(CWD));
    assert_eq!(page.items.len(), 13);
    assert!(page.older);
    assert_eq!(page.cursor.as_deref(), Some("older-1"));
}

/// From the protocol schema: `item/completed`'s params.
#[test]
fn a_completed_notification_gives_its_item_at_its_time() {
    let note: protocol::ItemCompletedNotification = serde_json::from_value(json!({
        "threadId": "thr_1",
        "turnId": "turn_1",
        "completedAtMs": 1_795_191_700_000_i64,
        "item": {
            "type": "commandExecution",
            "id": "call_1",
            "command": "pwsh -Command 'Get-ChildItem src'",
            "commandActions": [],
            "cwd": "C:\\dev\\shop",
            "status": "declined"
        }
    }))
    .unwrap();
    let item = completed(&note, Some("C:\\dev\\shop")).unwrap();
    assert_eq!(item.at, 1_795_191_700_000);
    let (_, title, _, result, state) = action_of(&item);
    assert_eq!(title, "Ran Get-ChildItem src");
    assert_eq!(state, CallState::Error);
    assert_eq!(result.map(|r| r.ok), Some(false));
}

/// From the protocol schema: the items a running or failed call gives.
#[test]
fn protocol_items_say_what_runs_and_what_failed() {
    let one = |value: Value| {
        let item: protocol::ThreadItem = serde_json::from_value(value).unwrap();
        super::item(&item, 7, Some(CWD))
    };
    let running = one(json!({
        "type": "commandExecution", "id": "c1", "command": "npm test",
        "commandActions": [], "cwd": CWD, "status": "inProgress"
    }))
    .unwrap();
    let (_, title, _, result, state) = action_of(&running);
    assert_eq!(
        (title, result, state),
        ("Ran npm test", None, CallState::Running)
    );

    let mcp = one(json!({
        "type": "mcpToolCall", "id": "m1", "server": "github", "tool": "get_issue",
        "arguments": { "number": 7 }, "status": "failed",
        "error": { "message": "not found" }
    }))
    .unwrap();
    let (tool, title, _, result, state) = action_of(&mcp);
    assert_eq!((tool, title), ("mcp__github__get_issue", "Used get_issue"));
    assert_eq!(state, CallState::Error);
    assert_eq!(result.map(|r| r.text.as_str()), Some("not found"));

    let dynamic = one(json!({
        "type": "dynamicToolCall", "id": "d1", "tool": "open_ticket",
        "arguments": {}, "status": "completed", "success": false,
        "contentItems": [{ "type": "inputText", "text": "no such queue" }]
    }))
    .unwrap();
    let (tool, title, detail, result, state) = action_of(&dynamic);
    assert_eq!(
        (tool, title, detail),
        ("open_ticket", "Used open_ticket", None)
    );
    assert_eq!(state, CallState::Error);
    assert_eq!(result.map(|r| r.text.as_str()), Some("no such queue"));

    let spawn = one(json!({
        "type": "collabAgentToolCall", "id": "a1", "tool": "spawnAgent",
        "status": "completed", "senderThreadId": "t", "receiverThreadIds": [],
        "agentsStates": {}, "prompt": "Find where the total is computed"
    }))
    .unwrap();
    assert_eq!(
        action_of(&spawn).1,
        "Ran agent: Find where the total is computed"
    );
    let failed_image = one(json!({
        "type": "imageGeneration", "id": "i1", "status": "completed",
        "failure": { "type": "usageLimitExceeded", "limitId": "images" }
    }))
    .unwrap();
    assert_eq!(action_of(&failed_image).4, CallState::Error);

    // Left out on purpose.
    for skipped in [
        json!({ "type": "reasoning", "id": "r1", "summary": ["thinking"] }),
        json!({ "type": "hookPrompt", "id": "h1", "fragments": [] }),
        json!({ "type": "collabAgentToolCall", "id": "a2", "tool": "wait", "status": "completed",
                "senderThreadId": "t", "receiverThreadIds": [], "agentsStates": {} }),
        json!({ "type": "imageView", "id": "v1", "path": "/x.png" }),
        json!({ "type": "plan", "id": "p1", "text": "" }),
        json!({ "type": "agentMessage", "id": "m2", "text": "  " }),
        json!({ "type": "hologram", "id": "x1" }),
    ] {
        assert_eq!(one(skipped.clone()), None, "{skipped}");
    }
}

/// The legacy history of an older thread (the rollout fixture of
/// sessions/codex.rs, from the source): its prompts and answers, and none of
/// the `response_item`s the model was sent, as the app server replays it. Its
/// second answer has no `agent_message` event — only the model's own
/// `response_item` and the turn's `last_agent_message` — so, as in Codex's
/// replay (`thread_history.rs` reads events only), it does not show.
#[test]
fn a_legacy_rollout_reads_as_codex_replays_it() {
    let items = all_of(LEGACY);
    let said: Vec<(&str, &Body)> = items
        .iter()
        .map(|item| (item.id.split('-').next().unwrap(), &item.body))
        .collect();
    assert_eq!(
        said,
        [
            (
                "line",
                &Body::User {
                    text: "Add a cart total to the checkout page".into(),
                    from: Sender::Person,
                    images: 0
                }
            ),
            (
                "line",
                &Body::Assistant {
                    text: "The cart total now shows under the item list, and `npm test` passes."
                        .into()
                }
            ),
            (
                "line",
                &Body::User {
                    text: "Now add a test for the total".into(),
                    from: Sender::Person,
                    images: 0
                }
            ),
        ]
    );
}

/// From the source: a legacy thread's tool events
/// (`protocol/src/protocol.rs`: `ExecCommandEndEvent`, `PatchApplyEndEvent`,
/// `McpToolCallEndEvent`, `WebSearchEndEvent`, `ImageGenerationEndEvent`,
/// `TurnAbortedEvent`), as `event_msg` records.
#[test]
fn legacy_tool_events_read_as_their_items() {
    let record = |second: u32, payload: Value| {
        json!({ "timestamp": format!("2026-11-18T09:20:{second:02}.000Z"), "type": "event_msg", "payload": payload })
            .to_string()
    };
    let lines = [
        record(
            1,
            json!({ "type": "user_message", "message": "Fix the build", "images": ["data:image/png;base64,AA=="],
            "local_images": ["/home/dev/shop/shot.png"], "text_elements": [] }),
        ),
        record(
            2,
            json!({ "type": "exec_command_end", "call_id": "call_x1", "turn_id": "u1",
            "command": ["bash", "-lc", "cargo build"], "cwd": "file:///home/dev/shop",
            "parsed_cmd": [{ "type": "unknown", "cmd": "cargo build" }], "source": "agent",
            "aggregated_output": "error[E0425]: cannot find value `x`", "exit_code": 101,
            "duration": { "secs": 3, "nanos": 0 }, "status": "failed" }),
        ),
        record(
            3,
            json!({ "type": "patch_apply_end", "call_id": "call_p1", "turn_id": "u1", "stdout": "", "stderr": "",
            "success": true, "status": "completed",
            "changes": { "/home/dev/shop/src/lib.rs": { "type": "update", "unified_diff": "@@ -1 +1 @@\n-x\n+y\n",
                "move_path": "/home/dev/shop/src/main.rs" } } }),
        ),
        record(
            4,
            json!({ "type": "mcp_tool_call_end", "call_id": "call_m1",
            "invocation": { "server": "hangar", "tool": "board_list_tasks", "arguments": null },
            "duration": { "secs": 0, "nanos": 5000000 },
            "result": { "Ok": { "content": [{ "type": "text", "text": "3 tasks" }], "isError": false } } }),
        ),
        record(
            5,
            json!({ "type": "mcp_tool_call_end", "call_id": "call_m2",
            "invocation": { "server": "hangar", "tool": "board_claim_task", "arguments": { "id": "t1" } },
            "duration": { "secs": 0, "nanos": 0 }, "result": { "Err": "task t1 is taken" } }),
        ),
        record(
            6,
            json!({ "type": "web_search_end", "call_id": "ws_1", "query": "",
            "action": { "type": "find_in_page", "url": "https://docs.rs/serde", "pattern": "untagged" } }),
        ),
        record(
            7,
            json!({ "type": "image_generation_end", "call_id": "ig_1", "status": "failed",
            "revised_prompt": "A logo", "result": "",
            "failure": { "type": "usageLimitExceeded", "limitId": "images", "resetsAt": null } }),
        ),
        record(8, json!({ "type": "context_compacted" })),
        record(
            9,
            json!({ "type": "turn_aborted", "turn_id": "u1", "reason": "replaced" }),
        ),
        record(
            10,
            json!({ "type": "turn_aborted", "turn_id": "u2", "reason": "interrupted" }),
        ),
    ];
    let items = all_of(&lines.join("\n"));
    assert_eq!(items.len(), 9, "{items:#?}");

    let Body::User { text, images, .. } = &items[0].body else {
        panic!("{:?}", items[0])
    };
    assert_eq!((text.as_str(), *images), ("Fix the build", 2));

    let (tool, title, _, result, state) = action_of(&items[1]);
    assert_eq!(
        (tool, title, state),
        ("commandExecution", "Ran cargo build", CallState::Error)
    );
    assert!(result.unwrap().text.starts_with("error[E0425]"));
    assert_eq!(items[1].id, "call_x1");

    let (_, title, detail, _, state) = action_of(&items[2]);
    assert_eq!((title, state), ("Edited src/lib.rs", CallState::Done));
    assert!(detail
        .unwrap()
        .ends_with("Moved to: /home/dev/shop/src/main.rs"));

    let (tool, title, detail, result, state) = action_of(&items[3]);
    assert_eq!(
        (tool, title, detail),
        (
            "mcp__hangar__board_list_tasks",
            "Used board_list_tasks",
            None
        )
    );
    assert_eq!(
        (result.map(|r| r.text.as_str()), state),
        (Some("3 tasks"), CallState::Done)
    );
    let (_, _, _, result, state) = action_of(&items[4]);
    assert_eq!(
        (result.map(|r| r.text.as_str()), state),
        (Some("task t1 is taken"), CallState::Error)
    );

    assert_eq!(
        action_of(&items[5]).1,
        "Searched https://docs.rs/serde for untagged"
    );
    let (_, title, detail, _, state) = action_of(&items[6]);
    assert_eq!(
        (title, detail, state),
        ("Generated an image", Some("A logo"), CallState::Error)
    );
    assert_eq!(
        items[7].body,
        Body::Notice {
            text: "Conversation compacted".into()
        }
    );
    assert_eq!(
        items[8].body,
        Body::Notice {
            text: "Turn interrupted".into()
        }
    );
}

/// A line that is not a record, a record nobody knows, an item kind a later
/// Codex adds, an extension nobody knows, an item whose fields do not read:
/// each is skipped, and the page goes on.
#[test]
fn what_a_rollout_line_does_not_say_is_skipped() {
    let completed = |item: Value| {
        json!({ "timestamp": "2026-11-19T10:00:00.000Z", "type": "event_msg",
                "payload": { "type": "item_completed", "thread_id": "t", "turn_id": "u", "item": item,
                             "completed_at_ms": 1 } })
        .to_string()
    };
    let lines = [
        "not json at all".to_string(),
        json!({ "fixture": "a line of a kind no reader knows" }).to_string(),
        json!({ "timestamp": "2026-11-19T10:00:00.000Z", "type": "realtime_item", "payload": {} })
            .to_string(),
        completed(json!({ "type": "Hologram", "id": "h1" })),
        completed(
            json!({ "type": "Extension", "kind": "clock.sleep", "id": "s1", "durationMs": 1000 }),
        ),
        completed(json!({ "type": "CommandExecution", "id": "c1", "status": "completed" })),
        completed(
            json!({ "type": "AgentMessage", "id": "a1", "content": [{ "type": "Text", "text": "Still here." }] }),
        ),
    ];
    let items = all_of(&lines.join("\n"));
    assert_eq!(ids(&items), ["a1"]);
}

/// An item written again — a message corrected — takes the place of the one
/// before, even when the two lie in spans read apart.
#[test]
fn an_item_written_again_replaces_the_one_before() {
    let message = |text: &str| {
        json!({ "timestamp": "2026-11-19T10:00:00.000Z", "type": "event_msg",
                "payload": { "type": "item_completed", "thread_id": "t", "turn_id": "u",
                             "item": { "type": "AgentMessage", "id": "a1", "content": [{ "type": "Text", "text": text }] } } })
        .to_string()
    };
    let mut older = Rollout::new(CWD);
    older.feed(0, &message("First draft."));
    let mut newer = older.fresh();
    newer.feed(100, &message("Corrected."));
    newer.prepend(older);
    let (items, first, more) = newer.page(None, 10);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].body,
        Body::Assistant {
            text: "Corrected.".into()
        }
    );
    assert_eq!((first, more), (Some(0), false));
}

/// A rollout longer than a span is read from its end, and further back as
/// the cursor asks: the turns are those of the fixture, with a long tool
/// output between turns 1 and 2 as Codex keeps them (`response_item`s).
#[test]
fn a_long_rollout_is_read_from_its_end_and_paged_back() {
    let filler = json!({ "timestamp": "2026-11-19T16:25:00.000Z", "type": "response_item",
        "payload": { "type": "function_call_output", "call_id": "call_long", "output": "y".repeat(8 * 1024) } })
    .to_string();
    let lines: Vec<&str> = PAGINATED.lines().collect();
    let cut = lines
        .iter()
        .position(|line| line.contains("\"turn_complete\""))
        .unwrap()
        + 1;
    let mut text = lines[..cut].join("\n");
    while (text.len() as u64) < 3 * SPAN_BYTES {
        text.push('\n');
        text.push_str(&filler);
    }
    text.push('\n');
    text.push_str(&lines[cut..].join("\n"));
    text.push('\n');
    let path = scratch("long", PAGINATED_NAME, text.as_bytes());
    let mut slot = None;

    let newest = read(&path, &mut slot, None, 3);
    assert_eq!(newest.items.len(), 3);
    assert_eq!(newest.items[1].id, "call_E2eRunF41l3dQ1w2E3r4T5");
    assert!(newest.older);
    let span = slot.as_ref().unwrap();
    assert!(span.start > 0, "only the end was read");

    let mut seen = newest.items.clone();
    let mut cursor = newest.cursor;
    while let Some(before) = cursor {
        let page = read(&path, &mut slot, Some(before.parse().unwrap()), 4);
        assert!(!page.items.is_empty());
        cursor = page.cursor;
        let mut older = page.items;
        older.extend(seen);
        seen = older;
    }
    assert_eq!(slot.as_ref().unwrap().start, 0);
    assert_eq!(seen, all_of(&text));
    assert_eq!(seen.len(), all_of(PAGINATED).len());

    // A turn appended later is read alone, from where the last look ended.
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    std::io::Write::write_all(
        &mut file,
        format!(
            "{}\n",
            json!({ "timestamp": "2026-11-19T16:45:00.000Z", "type": "event_msg",
                    "payload": { "type": "user_message", "message": "Try again", "local_images": [] } })
        )
        .as_bytes(),
    )
    .unwrap();
    let after = read(&path, &mut slot, None, 2);
    assert_eq!(
        after.items[1].body,
        Body::User {
            text: "Try again".into(),
            from: Sender::Person,
            images: 0
        }
    );
}

/// A compressed rollout, in two zstd frames as one appended after
/// compression would be, reads as its plain twin, and pages back alike.
#[test]
fn a_compressed_rollout_reads_as_its_plain_twin() {
    let cut = PAGINATED[..PAGINATED.len() / 2].rfind('\n').unwrap() + 1;
    let mut bytes = compress(&PAGINATED[..cut]);
    bytes.extend(compress(&PAGINATED[cut..]));
    let path = scratch("zst", &format!("{PAGINATED_NAME}.zst"), &bytes);
    let mut slot = None;

    let newest = read(&path, &mut slot, None, 5);
    let everything = all_of(PAGINATED);
    assert_eq!(newest.items, everything[everything.len() - 5..]);
    assert!(newest.older);
    let back = read(
        &path,
        &mut slot,
        newest.cursor.map(|c| c.parse().unwrap()),
        100,
    );
    assert_eq!(back.items, everything[..everything.len() - 5]);
    assert!(!back.older);
    assert_eq!(back.cursor, None);
}

#[test]
fn a_missing_rollout_is_no_page_and_a_bad_thread_id_an_error() {
    let path = crate::testdir::scratch("chat-codex-missing").join("rollout-gone.jsonl");
    let mut slot = None;
    assert_eq!(page_of(&path, &mut slot, CWD, None, 10).unwrap(), None);
    assert!(chat_of("../../etc/passwd", CWD, None, None).is_err());
    assert!(chat_of("019aa0b1", CWD, Some("not-a-cursor"), None).is_err());
}

#[test]
fn a_command_reads_as_its_script() {
    let words = |list: &[&str]| list.iter().map(|w| w.to_string()).collect::<Vec<_>>();
    assert_eq!(
        tools::script_of_words(&words(&["bash", "-lc", "npm test"])),
        "npm test"
    );
    assert_eq!(
        tools::script_of_words(&words(&[
            "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
            "-NoProfile",
            "-Command",
            "Get-ChildItem"
        ])),
        "Get-ChildItem"
    );
    assert_eq!(
        tools::script_of_words(&words(&["git", "status"])),
        "git status"
    );
    assert_eq!(
        tools::script_of_words(&words(&["echo", "it's done"])),
        r#"echo 'it'"'"'s done'"#
    );
    assert_eq!(
        tools::script_of_line(r#"/bin/zsh -lc 'echo "a b" && ls'"#),
        r#"echo "a b" && ls"#
    );
    assert_eq!(
        tools::script_of_line(r#"bash -lc "printf \"%s\" \$HOME""#),
        r#"printf "%s" $HOME"#
    );
    // Not a shell around a script, or quotes left open: as it came.
    assert_eq!(
        tools::script_of_line("git commit -m 'a b'"),
        "git commit -m 'a b'"
    );
    assert_eq!(tools::script_of_line("bash -lc 'open"), "bash -lc 'open");
    assert_eq!(
        tools::command("npm test\nnpm run lint").title,
        "Ran npm test"
    );
}

#[test]
fn a_patch_reads_as_claude_codes_writes_and_edits() {
    let change = |path: &str, kind| Change {
        path: path.into(),
        kind,
        diff: "+x".into(),
    };
    let title = |changes: &[Change]| tools::patch(changes, Some(CWD)).title;
    assert_eq!(
        title(&[change("/home/dev/shop/a.rs", ChangeKind::Add)]),
        "Wrote a.rs"
    );
    assert_eq!(
        title(&[change("/home/dev/shop/a.rs", ChangeKind::Update)]),
        "Edited a.rs"
    );
    assert_eq!(
        title(&[change("/elsewhere/a.rs", ChangeKind::Delete)]),
        "Deleted /elsewhere/a.rs"
    );
    assert_eq!(
        tools::patch(
            &[change("/home/dev/shop/a.rs", ChangeKind::Delete)],
            Some(CWD)
        )
        .detail,
        None
    );
    assert_eq!(
        title(&[
            change("/home/dev/shop/a.rs", ChangeKind::Add),
            change("/home/dev/shop/b.rs", ChangeKind::Update)
        ]),
        "Edited 2 files"
    );
}

#[test]
fn a_web_search_reads_as_claude_codes_search_or_fetch() {
    let title = |query: &str, action: WebAction| tools::web_search(query, &action).title;
    assert_eq!(
        title("rust zstd", WebAction::Search { queries: vec![] }),
        "Searched the web for rust zstd"
    );
    assert_eq!(title("", WebAction::Other), "Searched the web");
    assert_eq!(
        title(
            "",
            WebAction::OpenPage {
                url: Some("https://example.com".into())
            }
        ),
        "Fetched https://example.com"
    );
    assert_eq!(
        title(
            "",
            WebAction::FindInPage {
                url: None,
                pattern: Some("total".into())
            }
        ),
        "Searched the page for total"
    );
}
