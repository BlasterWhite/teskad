//! Tests of the delegate runner against a fake Codex app-server in the test process.
//!
//! No Codex session was ever recorded (the person's decision, docs/native/agents.md §7): what the
//! fake server says is written by hand **from the protocol schema** of the pinned release (Codex
//! 0.170.0) — `thread/start`, `turn/start`, `turn/interrupt`, the notifications of a turn, the
//! server requests and `serverRequest/resolved` — and, where the schema says nothing, from the
//! docs (`app-server`: a turn starts, its items start and complete, the turn completes) or the
//! source at rust-v0.170.0, as each test says. The shapes are those of
//! crates/hangar-core/tests/fixtures/codex/app-server/thread-and-turn.jsonl and
//! server-requests.jsonl.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{
    AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};
use tokio::sync::oneshot;

use super::*;
use crate::codex_app::Framing;

const PATIENCE: Duration = Duration::from_secs(10);
const THREAD: &str = "019a7c2e-5b1d-7f43-9a61-3c8e2f4d6b10";
const TASK: &str = "Generate the five toolbar icons as PNG files.";

/// The server's side of the child's connection, one JSON message per line (stdio).
struct Fake {
    lines: Lines<BufReader<ReadHalf<DuplexStream>>>,
    write: WriteHalf<DuplexStream>,
}

impl Fake {
    async fn recv(&mut self) -> Option<Value> {
        let line = tokio::time::timeout(PATIENCE, self.lines.next_line())
            .await
            .expect("Hangar says something in time")
            .expect("readable")?;
        Some(serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}")))
    }

    async fn expect(&mut self, method: &str) -> Value {
        let message = self.recv().await.expect("the connection is open");
        assert_eq!(message["method"], method, "{message}");
        message
    }

    async fn send(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        // The client may be gone already (a test's end): nothing to check then.
        let _ = self.write.write_all(line.as_bytes()).await;
    }

    async fn reply(&mut self, request: &Value, result: Value) {
        self.send(json!({ "id": request["id"], "result": result }))
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

    /// Hangar says nothing for a moment: it does not answer for the person.
    async fn quiet(&mut self) {
        if let Ok(line) =
            tokio::time::timeout(Duration::from_millis(300), self.lines.next_line()).await
        {
            panic!("Hangar said {line:?}");
        }
    }

    /// Hangar closed its side: the child's server is let go of.
    async fn closed(&mut self) {
        loop {
            match self.recv().await {
                None => return,
                Some(message) => assert!(
                    message.get("method").is_none() || message["method"] == "turn/interrupt",
                    "only an interruption before the end: {message}"
                ),
            }
        }
    }
}

/// A connection to a fake server, past its handshake (initialize.jsonl), and the fake once the
/// child connected.
fn connector() -> (Connect, oneshot::Receiver<Fake>) {
    let (tx, rx) = oneshot::channel();
    let connect: Connect = Box::new(move |options| {
        Box::pin(async move {
            let (hangar, codex) = tokio::io::duplex(1 << 20);
            let (read, write) = tokio::io::split(hangar);
            let (codex_read, codex_write) = tokio::io::split(codex);
            let mut fake = Fake {
                lines: BufReader::new(codex_read).lines(),
                write: codex_write,
            };
            let handshake = async {
                let init = fake.expect("initialize").await;
                assert_eq!(init["params"]["clientInfo"]["name"], "Hangar");
                let opted = &init["params"]["capabilities"]["optOutNotificationMethods"];
                assert!(opted
                    .as_array()
                    .is_some_and(|methods| methods.contains(&json!("item/agentMessage/delta"))));
                fake.reply(
                    &init,
                    json!({
                        "userAgent": "Hangar/0.170.0 (Ubuntu 24.4.0; x86_64) xterm-256color (Hangar; test)",
                        "codexHome": "/home/ada/.codex",
                        "platformFamily": "unix",
                        "platformOs": "linux",
                    }),
                )
                .await;
                fake.expect("initialized").await;
            };
            let (connected, ()) = tokio::join!(
                Client::connect_over(read, write, Framing::Lines, options),
                handshake
            );
            let _ = tx.send(fake);
            connected
        })
    });
    (connect, rx)
}

fn host() -> Host {
    Host::with_paths(crate::paths::Paths::rooted(&std::env::temp_dir()))
}

fn request(parent: &str, cwd: &Path) -> Request {
    Request {
        parent: parent.into(),
        cwd: cwd.to_string_lossy().into_owned(),
        task: TASK.into(),
        kind: AgentKind::Codex,
        images: true,
        ..Request::default()
    }
}

/// `thread/start`'s answer (ThreadStartResponse), in `cwd`.
fn thread_started(cwd: &str) -> Value {
    json!({
        "approvalPolicy": "on-request",
        "approvalsReviewer": "user",
        "cwd": cwd,
        "model": "gpt-5.5-codex",
        "modelProvider": "openai",
        "sandbox": { "type": "workspaceWrite", "writableRoots": [cwd], "networkAccess": false },
        "thread": {
            "id": THREAD, "sessionId": THREAD, "cliVersion": "0.170.0",
            "createdAt": 1795050000, "updatedAt": 1795050000, "cwd": cwd,
            "ephemeral": false, "modelProvider": "openai", "preview": "", "projectId": null,
            "source": "appServer", "status": { "type": "idle" }, "turns": [],
        },
    })
}

fn turn(id: &str, status: &str) -> Value {
    json!({ "id": id, "items": [], "status": status })
}

/// Starts a delegation on the fake server and plays its thread and turn up to the turn's start:
/// the delegation's id and the fake.
async fn started(host: &Host, request: Request) -> (String, Fake) {
    let cwd = request.cwd.clone();
    let (connect, fake) = connector();
    let delegation = begin(host, request, Some(connect)).await.expect("started");
    assert_eq!(delegation.status, Status::Running);
    let mut fake = tokio::time::timeout(PATIENCE, fake)
        .await
        .expect("the child connects in time")
        .expect("connected");
    let start = fake.expect("thread/start").await;
    let cwd = if delegation.isolation == Isolation::Worktree {
        delegation.cwd.clone()
    } else {
        cwd
    };
    assert_eq!(start["params"], json!({ "cwd": cwd }));
    fake.reply(&start, thread_started(&cwd)).await;
    let start = fake.expect("turn/start").await;
    assert_eq!(
        start["params"],
        json!({ "threadId": THREAD, "input": [{ "type": "text", "text": TASK }] })
    );
    fake.reply(&start, json!({ "turn": turn("turn-1", "inProgress") }))
        .await;
    fake.notify(
        "turn/started",
        json!({ "threadId": THREAD, "turn": turn("turn-1", "inProgress") }),
    )
    .await;
    (delegation.id, fake)
}

async fn completed(fake: &mut Fake, item: Value) {
    fake.notify(
        "item/completed",
        json!({ "threadId": THREAD, "turnId": "turn-1", "completedAtMs": 1795050004200i64, "item": item }),
    )
    .await;
}

async fn turn_completed(fake: &mut Fake, turn: Value) {
    fake.notify(
        "turn/completed",
        json!({ "threadId": THREAD, "turn": turn }),
    )
    .await;
}

/// The delegation once it no longer runs.
async fn settled(id: &str) -> Delegation {
    let polled = status(id, Some(PATIENCE)).await.expect("known");
    assert!(!polled.waited_out, "{polled:?}");
    polled.delegation
}

/// The delegation once it ended (past a `needs_you`).
async fn ended(id: &str) -> Delegation {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        let delegation = status(id, Some(Duration::from_millis(200)))
            .await
            .expect("known")
            .delegation;
        if delegation.status.is_over() {
            return delegation;
        }
        assert!(tokio::time::Instant::now() < deadline, "{delegation:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// What was queued for the caller, from Hangar.
fn told(parent: &str) -> Vec<String> {
    bridge::bridge_queue(parent.into())
        .into_iter()
        .map(|queued| {
            assert_eq!(queued.from.as_deref(), Some("hangar"));
            queued.text
        })
        .collect()
}

// ----- a child's work, and its result

// From the protocol schema (Codex 0.170.0): thread/start, turn/start, turn/started, item/started,
// item/completed (agentMessage, fileChange, imageGeneration), thread/tokenUsage/updated,
// turn/completed; their order from the docs (app-server).
#[tokio::test]
async fn a_child_does_its_task_and_hands_its_result_back() {
    let host = host();
    let parent = "p-delegate-result";
    let dir = crate::testdir::scratch("delegate-result");
    let (id, mut fake) = started(&host, request(parent, &dir)).await;
    let running = status(&id, None).await.unwrap();
    assert_eq!(running.delegation.status, Status::Running);
    assert_eq!(running.delegation.session.as_deref(), Some(THREAD));
    assert_eq!(running.delegation.model.as_deref(), Some("gpt-5.5-codex"));

    let patch = json!({
        "type": "fileChange", "id": "item-2", "status": "inProgress",
        "changes": [{ "path": "src/icons.rs", "kind": { "type": "update" }, "diff": "@@ -1 +1 @@\n-a\n+b\n" }],
    });
    fake.notify(
        "item/started",
        json!({ "threadId": THREAD, "turnId": "turn-1", "startedAtMs": 1795050002000i64, "item": patch }),
    )
    .await;
    let mut applied = patch.clone();
    applied["status"] = json!("completed");
    completed(&mut fake, applied).await;
    // A declined patch changed nothing.
    completed(
        &mut fake,
        json!({
            "type": "fileChange", "id": "item-3", "status": "declined",
            "changes": [{ "path": "/etc/hosts", "kind": { "type": "add" }, "diff": "x" }],
        }),
    )
    .await;
    // D15: only an image completed without a failure counts.
    completed(
        &mut fake,
        json!({
            "type": "imageGeneration", "id": "item-4", "status": "completed",
            "revisedPrompt": "A gear icon, flat, 64 px",
            "savedPath": "/home/ada/.codex/generated_images/019a7c2e/item-4.png",
        }),
    )
    .await;
    completed(
        &mut fake,
        json!({
            "type": "imageGeneration", "id": "item-5", "status": "failed",
            "failure": { "type": "usageLimitExceeded", "limitId": "image_generation" },
            "savedPath": "/home/ada/.codex/generated_images/019a7c2e/item-5.png",
        }),
    )
    .await;
    for (item, text, phase) in [
        ("item-6", "Drawing the icons now.", "commentary"),
        (
            "item-7",
            "Made the five icons; the gear is item-4.png.",
            "final_answer",
        ),
        ("item-8", "One more remark.", "commentary"),
    ] {
        completed(
            &mut fake,
            json!({ "type": "agentMessage", "id": item, "text": text, "phase": phase }),
        )
        .await;
    }
    fake.notify(
        "thread/tokenUsage/updated",
        json!({
            "threadId": THREAD, "turnId": "turn-1",
            "tokenUsage": {
                "total": { "inputTokens": 5210, "cachedInputTokens": 3072, "outputTokens": 41, "reasoningOutputTokens": 12, "totalTokens": 5251 },
                "last": { "inputTokens": 5210, "cachedInputTokens": 3072, "outputTokens": 41, "reasoningOutputTokens": 12, "totalTokens": 5251 },
                "modelContextWindow": 272000,
            },
        }),
    )
    .await;
    turn_completed(
        &mut fake,
        json!({ "id": "turn-1", "items": [], "status": "completed", "startedAt": 1795050001, "completedAt": 1795050004, "durationMs": 3150 }),
    )
    .await;

    let done = settled(&id).await;
    assert_eq!(done.status, Status::Done);
    assert_eq!(
        done.result.as_deref(),
        Some("Made the five icons; the gear is item-4.png.")
    );
    assert_eq!(
        done.files,
        [dir.join("src/icons.rs").to_string_lossy().into_owned()]
    );
    assert_eq!(
        done.images,
        ["/home/ada/.codex/generated_images/019a7c2e/item-4.png"]
    );
    assert_eq!(
        done.tokens,
        Some(Tokens {
            input: 5210,
            cached_input: 3072,
            output: 41,
            reasoning_output: 12,
            total: 5251,
        })
    );
    assert_eq!(done.parent, parent);
    assert_eq!(done.kind, AgentKind::Codex);
    assert_eq!(done.branch, None);
    assert!(done.ended.is_some());
    fake.closed().await;

    let told = told(parent);
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(
        told[0].starts_with(&format!(
            "Delegation {id} (codex) finished: Made the five icons"
        )),
        "{}",
        told[0]
    );
    assert!(told[0].contains("item-4.png"), "{}", told[0]);
    assert!(list().iter().any(|listed| listed.id == id));
    bridge::bridge_cancel(&host, parent.into(), None);
}

// ----- what the child asks the person

// From the protocol schema (Codex 0.170.0): item/commandExecution/requestApproval, its response,
// serverRequest/resolved; from the source at rust-v0.170.0 (app-server/src/outgoing_message.rs):
// the server numbers its requests from 0, and an answer resolves one.
#[tokio::test]
async fn what_the_child_asks_waits_for_the_person_and_takes_their_answer() {
    let host = host();
    let parent = "p-delegate-ask";
    let dir = crate::testdir::scratch("delegate-ask");
    let cwd = dir.to_string_lossy().into_owned();
    let (id, mut fake) = started(&host, request(parent, &dir)).await;
    fake.ask(
        0,
        "item/commandExecution/requestApproval",
        json!({
            "threadId": THREAD, "turnId": "turn-1", "itemId": "item-3", "startedAtMs": 1795050010000i64,
            "command": "cargo test -p hangar-core", "cwd": cwd,
            "reason": "Run the tests outside the sandbox",
            "commandActions": [{ "type": "unknown", "command": "cargo test -p hangar-core" }],
        }),
    )
    .await;

    let waiting = settled(&id).await;
    assert_eq!(waiting.status, Status::NeedsYou);
    let held = bridge::held_asks(&id);
    assert_eq!(held.len(), 1, "{held:?}");
    let ask = held[0].id.clone();
    assert_eq!(ask, format!("codex-{id}-0"));
    assert_eq!(held[0].tool, "Bash");
    assert_eq!(held[0].input["command"], "cargo test -p hangar-core");
    // Nobody but the person answers.
    fake.quiet().await;

    assert!(bridge::answer_ask(
        &id,
        &ask,
        json!({ "behavior": "allow" })
    ));
    assert_eq!(
        fake.recv().await.unwrap(),
        json!({ "id": 0, "result": { "decision": "accept" } })
    );
    fake.notify(
        "serverRequest/resolved",
        json!({ "threadId": THREAD, "requestId": 0 }),
    )
    .await;
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while status(&id, None).await.unwrap().delegation.status != Status::Running {
        assert!(tokio::time::Instant::now() < deadline, "back to running");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(bridge::held_asks(&id).is_empty());

    turn_completed(&mut fake, turn("turn-1", "completed")).await;
    assert_eq!(ended(&id).await.status, Status::Done);
    bridge::bridge_cancel(&host, parent.into(), None);
}

// ----- cancelled, failed

// From the protocol schema (Codex 0.170.0): turn/interrupt and its empty answer (steer.jsonl).
#[tokio::test]
async fn cancelling_interrupts_the_turn_and_withdraws_what_it_asked() {
    let host = host();
    let parent = "p-delegate-cancel";
    let dir = crate::testdir::scratch("delegate-cancel");
    let (id, mut fake) = started(&host, request(parent, &dir)).await;
    fake.ask(
        0,
        "item/fileChange/requestApproval",
        json!({
            "threadId": THREAD, "turnId": "turn-1", "itemId": "item-4", "startedAtMs": 1795050020000i64,
            "reason": "Write outside the workspace", "grantRoot": "/etc/hangar",
        }),
    )
    .await;
    assert_eq!(settled(&id).await.status, Status::NeedsYou);
    assert_eq!(bridge::held_asks(&id).len(), 1);

    let cancelling = tokio::spawn({
        let id = id.clone();
        async move { cancel(&id).await }
    });
    let interrupt = fake.expect("turn/interrupt").await;
    assert_eq!(
        interrupt["params"],
        json!({ "threadId": THREAD, "turnId": "turn-1" })
    );
    fake.reply(&interrupt, json!({})).await;
    let cancelled = tokio::time::timeout(PATIENCE, cancelling)
        .await
        .expect("cancelled in time")
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert!(bridge::held_asks(&id).is_empty());
    fake.closed().await;
    assert_eq!(
        told(parent),
        [format!("Delegation {id} (codex) was cancelled.")]
    );
    // Over: cancelling again leaves it as it is.
    assert_eq!(cancel(&id).await.unwrap().status, Status::Cancelled);
    bridge::bridge_cancel(&host, parent.into(), None);
}

// From the protocol schema (Codex 0.170.0): a turn completed as `failed`, with its `TurnError`.
#[tokio::test]
async fn a_failed_turn_fails_the_delegation_with_codexs_reason() {
    let host = host();
    let parent = "p-delegate-failed";
    let dir = crate::testdir::scratch("delegate-failed");
    let (id, mut fake) = started(&host, request(parent, &dir)).await;
    turn_completed(
        &mut fake,
        json!({
            "id": "turn-1", "items": [], "status": "failed",
            "error": { "message": "You've hit your usage limit." },
        }),
    )
    .await;
    let failed = settled(&id).await;
    assert_eq!(failed.status, Status::Failed);
    assert_eq!(
        failed.error.as_deref(),
        Some("You've hit your usage limit.")
    );
    assert_eq!(
        told(parent),
        [format!(
            "Delegation {id} (codex) failed: You've hit your usage limit."
        )]
    );
    bridge::bridge_cancel(&host, parent.into(), None);
}

#[tokio::test]
async fn a_server_that_goes_away_fails_the_delegation() {
    let host = host();
    let parent = "p-delegate-gone";
    let dir = crate::testdir::scratch("delegate-gone");
    let (id, fake) = started(&host, request(parent, &dir)).await;
    drop(fake);
    let failed = settled(&id).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(
        failed
            .error
            .as_deref()
            .is_some_and(|error| error.contains("closed")),
        "{failed:?}"
    );
    bridge::bridge_cancel(&host, parent.into(), None);
}

#[tokio::test]
async fn a_server_that_cannot_start_fails_the_delegation() {
    let host = host();
    let parent = "p-delegate-nostart";
    let dir = crate::testdir::scratch("delegate-nostart");
    let connect: Connect = Box::new(|_| {
        Box::pin(async { Err(Error::Start("codex: No such file or directory".into())) })
    });
    let delegation = begin(&host, request(parent, &dir), Some(connect))
        .await
        .unwrap();
    let failed = settled(&delegation.id).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(failed
        .error
        .as_deref()
        .is_some_and(|error| error.contains("No such file")));
    bridge::bridge_cancel(&host, parent.into(), None);
}

// ----- waiting

#[tokio::test]
async fn a_wait_runs_out_with_the_child_still_running() {
    let host = host();
    let parent = "p-delegate-wait";
    let dir = crate::testdir::scratch("delegate-wait");
    let (id, mut fake) = started(&host, request(parent, &dir)).await;
    let polled = status(&id, Some(Duration::from_millis(200))).await.unwrap();
    assert!(polled.waited_out);
    assert_eq!(polled.delegation.status, Status::Running);
    // Without a wait, nothing ran out.
    assert!(!status(&id, None).await.unwrap().waited_out);
    turn_completed(&mut fake, turn("turn-1", "completed")).await;
    assert_eq!(settled(&id).await.status, Status::Done);
    bridge::bridge_cancel(&host, parent.into(), None);
}

#[test]
fn a_wait_never_outlives_an_mcp_call() {
    assert_eq!(capped(Some(Duration::from_secs(600))), WAIT_MAX);
    assert_eq!(WAIT_MAX, Duration::from_secs(45));
    assert_eq!(capped(Some(Duration::from_secs(3))), Duration::from_secs(3));
    assert_eq!(capped(None), Duration::ZERO);
}

#[tokio::test]
async fn an_unknown_delegation_is_none() {
    assert!(status("d-0", None).await.is_none());
    assert!(status("p1", None).await.is_none());
    assert!(cancel("d-0").await.is_err());
}

// ----- what is refused

#[tokio::test]
async fn a_delegation_needs_a_task_a_caller_and_a_folder() {
    let host = host();
    let dir = crate::testdir::scratch("delegate-refused");
    let blank = Request {
        task: "  \n ".into(),
        ..request("p-delegate-refused", &dir)
    };
    assert!(start(&host, blank).await.is_err());
    let orphan = Request {
        parent: String::new(),
        ..request("p-delegate-refused", &dir)
    };
    assert!(start(&host, orphan).await.is_err());
    let nowhere = request("p-delegate-refused", &dir.join("missing"));
    assert!(start(&host, nowhere).await.is_err());
}

/// Claude Code has no protocol server to run a child on: its one-shot children are T11b's.
#[tokio::test]
async fn a_kind_without_a_headless_server_is_refused_by_capability() {
    let host = host();
    let dir = crate::testdir::scratch("delegate-claude");
    let claude = Request {
        kind: AgentKind::Claude,
        ..request("p-delegate-claude", &dir)
    };
    let refused = start(&host, claude).await.unwrap_err();
    assert!(refused.contains("Claude Code"), "{refused}");
}

#[test]
fn a_policy_the_kind_does_not_enforce_steps_down_to_a_stricter_one() {
    use PermissionMode::*;
    assert_eq!(step_down(Auto, &[AcceptEdits, Auto]), Some(Auto));
    assert_eq!(
        step_down(BypassPermissions, &[AcceptEdits, Auto]),
        Some(Auto)
    );
    assert_eq!(
        step_down(Auto, &[AcceptEdits, BypassPermissions]),
        Some(AcceptEdits)
    );
    assert_eq!(step_down(AcceptEdits, &[Auto, BypassPermissions]), None);
    // Codex enforces each of them (agent.rs).
    for policy in PermissionMode::ALL {
        assert_eq!(policy_for(AgentKind::Codex, Some(policy)), Ok(Some(policy)));
    }
    assert_eq!(policy_for(AgentKind::Codex, None), Ok(None));
}

#[test]
fn the_caller_reads_how_its_delegation_ended() {
    let delegation = Delegation {
        id: "d-3".into(),
        parent: "p1".into(),
        kind: AgentKind::Codex,
        model: Some("gpt-5.5-codex".into()),
        status: Status::Done,
        title: "Generate the icons".into(),
        started: 1,
        ended: Some(2),
        tokens: None,
        isolation: Isolation::Worktree,
        cwd: "/work/hangar-worktrees/delegate-d-3-abcdef".into(),
        session: Some(THREAD.into()),
        result: Some("Made five icons.".into()),
        files: vec!["/work/hangar-worktrees/delegate-d-3-abcdef/icons/gear.png".into()],
        images: vec!["/home/ada/.codex/generated_images/s/c.png".into()],
        branch: Some("delegate/d-3-abcdef".into()),
        error: None,
    };
    assert_eq!(
        event_text(&delegation, None),
        "Delegation d-3 (codex) finished: Made five icons.\n\n\
         Files changed: /work/hangar-worktrees/delegate-d-3-abcdef/icons/gear.png\n\n\
         Images: /home/ada/.codex/generated_images/s/c.png\n\n\
         Its work is on branch delegate/d-3-abcdef, in /work/hangar-worktrees/delegate-d-3-abcdef."
    );
    let cancelled = Delegation {
        status: Status::Cancelled,
        result: None,
        files: Vec::new(),
        images: Vec::new(),
        branch: None,
        ..delegation.clone()
    };
    assert_eq!(
        event_text(&cancelled, Some(true)),
        "Delegation d-3 (codex) was cancelled.\n\nIts worktree was removed: it held no commits."
    );
    let long = Delegation {
        result: Some("x".repeat(EVENT_RESULT_CHARS + 10)),
        files: Vec::new(),
        images: Vec::new(),
        branch: None,
        ..delegation
    };
    let text = event_text(&long, None);
    assert!(text.ends_with('…'));
    assert!(text.chars().count() < EVENT_RESULT_CHARS + 60);
}

#[test]
fn a_delegation_reads_as_the_tools_say_it() {
    assert_eq!(serde_json::to_value(Status::NeedsYou).unwrap(), "needs_you");
    assert_eq!(
        serde_json::to_value(Status::Cancelled).unwrap(),
        "cancelled"
    );
    assert_eq!(
        serde_json::to_value(Isolation::Worktree).unwrap(),
        "worktree"
    );
    assert_eq!(title_of("\n  Draw the icon.\nThen more."), "Draw the icon.");
}

// ----- a worktree of its own

fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Hangar test",
            "-c",
            "user.email=test@hangar.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// `<scratch>/repo` with one commit, and an edit not committed.
fn repo(tag: &str) -> PathBuf {
    let repo = crate::testdir::scratch(tag).join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("README.md"), "Hangar\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::write(repo.join("README.md"), "Hangar, not committed\n").unwrap();
    repo
}

fn branches(repo: &Path) -> String {
    git(repo, &["branch", "--list", "delegate/*"])
}

#[tokio::test]
async fn a_worktree_child_works_on_a_branch_of_its_own_and_goes_when_cancelled_empty() {
    let host = host();
    let parent = "p-delegate-worktree";
    let repo = repo("delegate-worktree");
    let worktree_request = Request {
        isolation: Isolation::Worktree,
        ..request(parent, &repo)
    };
    let (id, mut fake) = started(&host, worktree_request).await;
    let running = status(&id, None).await.unwrap().delegation;
    let branch = running.branch.clone().expect("a branch");
    assert!(branch.starts_with(&format!("delegate/{id}-")), "{branch}");
    let folder = PathBuf::from(&running.cwd);
    assert_eq!(
        std::fs::canonicalize(folder.parent().unwrap()).unwrap(),
        std::fs::canonicalize(repo.parent().unwrap())
            .unwrap()
            .join("repo-worktrees")
    );
    // Made from the last commit: what was not committed is not there.
    assert_eq!(
        std::fs::read_to_string(folder.join("README.md")).unwrap(),
        "Hangar\n"
    );
    assert!(branches(&repo).contains(&branch));

    let cancelling = tokio::spawn({
        let id = id.clone();
        async move { cancel(&id).await }
    });
    let interrupt = fake.expect("turn/interrupt").await;
    fake.reply(&interrupt, json!({})).await;
    let cancelled = tokio::time::timeout(PATIENCE, cancelling)
        .await
        .expect("cancelled in time")
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert_eq!(cancelled.branch, None);
    assert!(!folder.exists(), "the worktree went");
    assert!(branches(&repo).is_empty(), "its branch went");
    assert_eq!(
        told(parent),
        [format!(
            "Delegation {id} (codex) was cancelled.\n\nIts worktree was removed: it held no commits."
        )]
    );
    bridge::bridge_cancel(&host, parent.into(), None);
}

#[tokio::test]
async fn a_cancelled_worktree_that_holds_commits_stays_with_its_branch() {
    let host = host();
    let parent = "p-delegate-commits";
    let repo = repo("delegate-commits");
    let worktree_request = Request {
        isolation: Isolation::Worktree,
        ..request(parent, &repo)
    };
    let (id, mut fake) = started(&host, worktree_request).await;
    let running = status(&id, None).await.unwrap().delegation;
    let folder = PathBuf::from(&running.cwd);
    // What the child would have done with its shell.
    std::fs::create_dir_all(folder.join("icons")).unwrap();
    std::fs::write(folder.join("icons/gear.svg"), "<svg/>").unwrap();
    git(&folder, &["add", "icons/gear.svg"]);
    git(&folder, &["commit", "-q", "-m", "icons"]);

    let cancelling = tokio::spawn({
        let id = id.clone();
        async move { cancel(&id).await }
    });
    let interrupt = fake.expect("turn/interrupt").await;
    fake.reply(&interrupt, json!({})).await;
    let cancelled = tokio::time::timeout(PATIENCE, cancelling)
        .await
        .expect("cancelled in time")
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert_eq!(cancelled.branch, running.branch);
    assert!(
        folder.join("icons/gear.svg").is_file(),
        "the worktree stays"
    );
    assert_eq!(
        cancelled.files,
        [folder.join("icons/gear.svg").to_string_lossy().into_owned()]
    );
    let told = told(parent);
    assert!(
        told[0].contains(&format!("on branch {}", running.branch.unwrap())),
        "{told:?}"
    );
    bridge::bridge_cancel(&host, parent.into(), None);
}

/// A child that ends by itself leaves its worktree to the caller, its changes listed.
#[tokio::test]
async fn a_finished_worktree_child_hands_its_branch_and_files_back() {
    let host = host();
    let parent = "p-delegate-wt-done";
    let repo = repo("delegate-wt-done");
    let worktree_request = Request {
        isolation: Isolation::Worktree,
        ..request(parent, &repo)
    };
    let (id, mut fake) = started(&host, worktree_request).await;
    let running = status(&id, None).await.unwrap().delegation;
    let folder = PathBuf::from(&running.cwd);
    std::fs::write(folder.join("NOTES.md"), "made by the child\n").unwrap();
    completed(
        &mut fake,
        json!({ "type": "agentMessage", "id": "item-2", "text": "Wrote NOTES.md.", "phase": "final_answer" }),
    )
    .await;
    turn_completed(&mut fake, turn("turn-1", "completed")).await;
    let done = settled(&id).await;
    assert_eq!(done.status, Status::Done);
    assert_eq!(done.branch, running.branch);
    assert_eq!(
        done.files,
        [folder.join("NOTES.md").to_string_lossy().into_owned()]
    );
    assert!(folder.is_dir(), "left to the caller");
    // Unlocked: the caller's tools can take it from here.
    let listed = git(&repo, &["worktree", "list", "--porcelain"]);
    assert!(!listed.contains("locked"), "{listed}");
    bridge::bridge_cancel(&host, parent.into(), None);
}

#[tokio::test]
async fn a_worktree_needs_a_repository_with_a_commit() {
    let host = host();
    let dir = crate::testdir::scratch("delegate-norepo");
    let refused = start(
        &host,
        Request {
            isolation: Isolation::Worktree,
            ..request("p-delegate-norepo", &dir)
        },
    )
    .await
    .unwrap_err();
    assert!(refused.contains("shared"), "{refused}");
}
