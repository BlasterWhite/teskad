//! Tests of the delegate runner against a fake Codex app-server in the test process, and against
//! a fake Claude Code — in the test process, and on Unix a fake `claude` script on disk.
//!
//! No Codex session was ever recorded (the person's decision, docs/native/agents.md §7): what the
//! fake server says is written by hand **from the protocol schema** of the pinned release (Codex
//! 0.170.0) — `thread/start`, `turn/start`, `turn/interrupt`, the notifications of a turn, the
//! server requests and `serverRequest/resolved` — and, where the schema says nothing, from the
//! docs (`app-server`: a turn starts, its items start and complete, the turn completes) or the
//! source at rust-v0.170.0, as each test says. The shapes are those of
//! crates/hangar-core/tests/fixtures/codex/app-server/thread-and-turn.jsonl and
//! server-requests.jsonl.
//!
//! No Claude Code session was recorded either: what the fake Claude Code says is written by hand
//! **from Claude Code's stream-json docs** (code.claude.com: `headless`; the Agent SDK's
//! TypeScript reference — `SDKSystemMessage`, `SDKAssistantMessage`, `SDKUserMessage`,
//! `SDKResultMessage`, `PermissionResult`, `SDKControlInterruptResponse`), the envelope of the
//! control messages from the Agent SDK's own client, as each test says.

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
    let delegation = begin(host, request, Some(Child::AppServer(connect)))
        .await
        .expect("started");
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
    let delegation = begin(
        &host,
        request(parent, &dir),
        Some(Child::AppServer(connect)),
    )
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

/// By capability, never by kind: Codex serves a protocol, Claude Code runs one prompt to its end.
#[test]
fn a_child_runs_by_what_its_kind_can_do_without_a_terminal() {
    assert_eq!(runner_for(AgentKind::Codex), Some(Runner::AppServer));
    assert_eq!(runner_for(AgentKind::Claude), Some(Runner::OneShot));
    // Claude Code enforces each policy itself (agent.rs).
    for policy in PermissionMode::ALL {
        assert_eq!(
            policy_for(AgentKind::Claude, Some(policy)),
            Ok(Some(policy))
        );
    }
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
    // Checkouts byte for byte: Git for Windows turns LF into CRLF by default
    // (core.autocrlf), and the worktree's checkout is the production helper's.
    git(&repo, &["config", "core.autocrlf", "false"]);
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

// ----- a Claude Code child

const SESSION: &str = "8c3f2a1e-6b7d-4e59-9a0c-2d1f5e7b3c64";
const CLAUDE_MODEL: &str = "claude-sonnet-4-5-20250929";
const CLAUDE_TASK: &str = "Fix the off-by-one in the parser.";

/// Claude Code's side of a child's stdio, one JSON message per line: what it is told on stdin,
/// what it says on stdout.
struct FakeClaude {
    told: Lines<BufReader<ReadHalf<DuplexStream>>>,
    say: WriteHalf<DuplexStream>,
}

impl FakeClaude {
    async fn told(&mut self) -> Option<Value> {
        let line = tokio::time::timeout(PATIENCE, self.told.next_line())
            .await
            .expect("Hangar says something in time")
            .expect("readable")?;
        Some(serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}")))
    }

    async fn say(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        // Hangar may be gone already (a test's end): nothing to check then.
        let _ = self.say.write_all(line.as_bytes()).await;
    }

    /// Hangar says nothing for a moment: it does not answer for the person.
    async fn quiet(&mut self) {
        if let Ok(line) =
            tokio::time::timeout(Duration::from_millis(300), self.told.next_line()).await
        {
            panic!("Hangar said {line:?}");
        }
    }

    /// Hangar closed its input: the run is let go of.
    async fn closed(&mut self) {
        while let Some(message) = self.told().await {
            assert_eq!(
                message["request"]["subtype"], "interrupt",
                "only an interruption before the end: {message}"
            );
        }
    }
}

/// A Claude Code child over an in-process stdio, and Claude Code's side of it.
fn claude_child() -> (Child, FakeClaude) {
    let (hangar, theirs) = tokio::io::duplex(1 << 20);
    let (output, input) = tokio::io::split(hangar);
    let (told, say) = tokio::io::split(theirs);
    let open: claude::Open = Box::new(move || {
        Ok::<_, String>(claude::Io {
            output: Box::new(output),
            input: Box::new(input),
            process: None,
        })
    });
    (
        Child::OneShot(open),
        FakeClaude {
            told: BufReader::new(told).lines(),
            say,
        },
    )
}

fn claude_request(parent: &str, cwd: &Path) -> Request {
    Request {
        parent: parent.into(),
        cwd: cwd.to_string_lossy().into_owned(),
        task: CLAUDE_TASK.into(),
        kind: AgentKind::Claude,
        model: Some("sonnet".into()),
        ..Request::default()
    }
}

/// The task, as the child is told it: an `SDKUserMessage`.
fn the_task() -> Value {
    json!({
        "type": "user",
        "message": { "role": "user", "content": CLAUDE_TASK },
        "parent_tool_use_id": null,
    })
}

/// Starts a delegation on a fake Claude Code, which is told the task: the delegation's id and the
/// fake.
async fn claude_started(host: &Host, request: Request) -> (String, FakeClaude) {
    let (child, mut claude) = claude_child();
    let delegation = begin(host, request, Some(child)).await.expect("started");
    assert_eq!(delegation.status, Status::Running);
    assert_eq!(delegation.kind, AgentKind::Claude);
    assert_eq!(claude.told().await, Some(the_task()));
    (delegation.id, claude)
}

// The messages below are written from Claude Code's stream-json docs: `system/init`
// (`SDKSystemMessage`), `assistant` (`SDKAssistantMessage`, its `message` the Messages API's),
// `user` (`SDKUserMessage`, a `tool_result` block), `result` (`SDKResultMessage`).

fn init(cwd: &str) -> Value {
    json!({
        "type": "system", "subtype": "init",
        "uuid": "5e1c7a2b-0d3f-4b6e-8a91-7c2d4f6e8b01", "session_id": SESSION,
        "apiKeySource": "none", "claude_code_version": "2.1.328", "cwd": cwd,
        "tools": ["AskUserQuestion", "Bash", "Edit", "Glob", "Grep", "Read", "Write"],
        "mcp_servers": [], "model": CLAUDE_MODEL, "permissionMode": "default",
        "slash_commands": [], "output_style": "default", "skills": [], "plugins": [],
        "capabilities": ["interrupt_receipt_v1"],
    })
}

/// An `assistant` message `id` with `content`, in the main conversation or a subagent's.
fn assistant(id: &str, content: Value, parent: Option<&str>) -> Value {
    json!({
        "type": "assistant", "uuid": format!("{id}-uuid"), "session_id": SESSION,
        "parent_tool_use_id": parent,
        "message": {
            "id": id, "type": "message", "role": "assistant", "model": CLAUDE_MODEL,
            "content": content, "stop_reason": null, "stop_sequence": null,
            "usage": {
                "input_tokens": 6, "cache_creation_input_tokens": 1200,
                "cache_read_input_tokens": 9800, "output_tokens": 85,
            },
        },
    })
}

/// The result of call `call`, in the main conversation or a subagent's.
fn tool_result(call: &str, is_error: bool, parent: Option<&str>) -> Value {
    json!({
        "type": "user", "uuid": format!("{call}-result"), "session_id": SESSION,
        "parent_tool_use_id": parent,
        "message": { "role": "user", "content": [{
            "type": "tool_result", "tool_use_id": call, "is_error": is_error,
            "content": if is_error { "Permission to use Write has been denied." } else { "Done." },
        }] },
    })
}

fn usage_of_the_run() -> Value {
    json!({
        "usage": {
            "input_tokens": 18, "cache_creation_input_tokens": 1200,
            "cache_read_input_tokens": 29400, "output_tokens": 310,
        },
        "modelUsage": { CLAUDE_MODEL: {
            "inputTokens": 18, "outputTokens": 310, "cacheReadInputTokens": 29400,
            "cacheCreationInputTokens": 1200, "webSearchRequests": 0, "costUSD": 0.0213,
            "contextWindow": 200000,
        } },
    })
}

fn result_success(text: &str) -> Value {
    let mut result = json!({
        "type": "result", "subtype": "success", "uuid": "0b7e3d51-9c2a-4f86-b1d4-6a8e2c5f7d93",
        "session_id": SESSION, "duration_ms": 41250, "duration_api_ms": 38800,
        "is_error": false, "num_turns": 4, "result": text, "stop_reason": "end_turn",
        "total_cost_usd": 0.0213, "permission_denials": [],
    });
    let spent = usage_of_the_run();
    result["usage"] = spent["usage"].clone();
    result["modelUsage"] = spent["modelUsage"].clone();
    result
}

fn result_error(subtype: &str, errors: &[&str]) -> Value {
    json!({
        "type": "result", "subtype": subtype, "uuid": "7f2a9c04-1e6b-4d38-a5c7-3b9e1d0f6a28",
        "session_id": SESSION, "duration_ms": 9100, "duration_api_ms": 8700, "is_error": true,
        "num_turns": 2, "stop_reason": null, "total_cost_usd": 0.004,
        "usage": { "input_tokens": 4, "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0, "output_tokens": 12 },
        "modelUsage": {}, "permission_denials": [], "errors": errors,
    })
}

/// A permission prompt: a `control_request` `can_use_tool`, as the Agent SDK's client reads it
/// (the arguments of its `canUseTool`: tool, input, suggestions, the call's id).
fn can_use_tool(request: &str, tool: &str, input: Value, call: &str) -> Value {
    json!({
        "type": "control_request", "request_id": request,
        "request": {
            "subtype": "can_use_tool", "tool_name": tool, "input": input,
            "permission_suggestions": [], "tool_use_id": call,
        },
    })
}

/// The person's answer to `request`, as Claude Code is told it (`PermissionResult`).
fn answer_to(request: &str, response: Value) -> Value {
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": request, "response": response },
    })
}

/// Back to running once what it asked is over.
async fn running_again(id: &str) {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while status(id, None).await.unwrap().delegation.status != Status::Running {
        assert!(tokio::time::Instant::now() < deadline, "back to running");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The ask is held again, with no answer yet.
async fn asked_again(id: &str, ask: &str) {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while !(bridge::held_asks(id).iter().any(|held| held.id == ask)
        && !bridge::answer_untaken(id, ask))
    {
        assert!(tokio::time::Instant::now() < deadline, "asked again");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_claude_child_does_its_task_and_hands_its_result_back() {
    let host = host();
    let parent = "p-claude-result";
    let dir = crate::testdir::scratch("claude-result");
    let cwd = dir.to_string_lossy().into_owned();
    let (id, mut claude) = claude_started(&host, claude_request(parent, &dir)).await;
    claude.say(init(&cwd)).await;
    claude
        .say(assistant(
            "msg_01",
            json!([
                { "type": "text", "text": "Reading the parser first." },
                { "type": "tool_use", "id": "toolu_01", "name": "Edit",
                  "input": { "file_path": "src/parser.rs", "old_string": "i <= n", "new_string": "i < n" } },
            ]),
            None,
        ))
        .await;
    claude.say(tool_result("toolu_01", false, None)).await;
    // A write that was refused changed nothing.
    claude
        .say(assistant(
            "msg_02",
            json!([{ "type": "tool_use", "id": "toolu_02", "name": "Write",
                     "input": { "file_path": "/etc/hosts", "content": "x" } }]),
            None,
        ))
        .await;
    claude.say(tool_result("toolu_02", true, None)).await;
    // A subagent's edit is the child's work too; its text is not the child's answer.
    let notes = format!("{cwd}/NOTES.md");
    claude
        .say(assistant(
            "msg_03",
            json!([
                { "type": "text", "text": "Subagent: notes written." },
                { "type": "tool_use", "id": "toolu_03", "name": "Write",
                  "input": { "file_path": notes, "content": "parser notes" } },
            ]),
            Some("toolu_00"),
        ))
        .await;
    claude
        .say(tool_result("toolu_03", false, Some("toolu_00")))
        .await;
    // A request Hangar does not answer is refused rather than left waiting (the Agent SDK's
    // client answers an unknown one with an error).
    claude
        .say(json!({
            "type": "control_request", "request_id": "req-hook-1",
            "request": { "subtype": "hook_callback", "callback_id": "hook_0", "input": {} },
        }))
        .await;
    let refused = claude.told().await.unwrap();
    assert_eq!(refused["type"], "control_response");
    assert_eq!(refused["response"]["subtype"], "error");
    assert_eq!(refused["response"]["request_id"], "req-hook-1");

    let running = status(&id, None).await.unwrap().delegation;
    assert_eq!(running.status, Status::Running);
    assert_eq!(running.session.as_deref(), Some(SESSION));
    assert_eq!(running.model.as_deref(), Some(CLAUDE_MODEL));
    // What its API calls spent so far, each counted once.
    assert_eq!(running.tokens.map(|tokens| tokens.output), Some(3 * 85));

    claude
        .say(assistant(
            "msg_04",
            json!([{ "type": "text", "text": "Fixed it: the loop stopped one past the end." }]),
            None,
        ))
        .await;
    claude
        .say(result_success(
            "Fixed the off-by-one: the loop in src/parser.rs stopped one past the end.",
        ))
        .await;

    let done = settled(&id).await;
    assert_eq!(done.status, Status::Done);
    assert_eq!(
        done.result.as_deref(),
        Some("Fixed the off-by-one: the loop in src/parser.rs stopped one past the end.")
    );
    assert_eq!(
        done.files,
        [
            dir.join("src/parser.rs").to_string_lossy().into_owned(),
            notes
        ]
    );
    assert!(done.images.is_empty());
    assert_eq!(
        done.tokens,
        Some(Tokens {
            input: 18 + 1200 + 29400,
            cached_input: 29400,
            output: 310,
            reasoning_output: 0,
            total: 18 + 1200 + 29400 + 310,
        })
    );
    assert_eq!(done.kind, AgentKind::Claude);
    assert_eq!(done.branch, None);
    claude.closed().await;

    let told = told(parent);
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(
        told[0].starts_with(&format!(
            "Delegation {id} (claude) finished: Fixed the off-by-one"
        )),
        "{}",
        told[0]
    );
    assert!(told[0].contains("src/parser.rs"), "{}", told[0]);
    bridge::bridge_cancel(&host, parent.into(), None);
}

// From Claude Code's stream-json docs: a permission prompt goes to the host (`--permission-prompts
// host`, the Agent SDK's `canUseTool`), whose `PermissionResult` gives the call's input back.
#[tokio::test]
async fn what_a_claude_child_asks_waits_for_the_person_and_takes_their_answer() {
    let host = host();
    let parent = "p-claude-ask";
    let dir = crate::testdir::scratch("claude-ask");
    let (id, mut claude) = claude_started(&host, claude_request(parent, &dir)).await;
    claude.say(init(&dir.to_string_lossy())).await;
    let input =
        json!({ "command": "cargo test -p hangar-core", "description": "Run the core tests" });
    claude
        .say(can_use_tool("req-1", "Bash", input.clone(), "toolu_05"))
        .await;

    let waiting = settled(&id).await;
    assert_eq!(waiting.status, Status::NeedsYou);
    let held = bridge::held_asks(&id);
    assert_eq!(held.len(), 1, "{held:?}");
    let ask = held[0].id.clone();
    assert_eq!(ask, format!("claude-{id}-1"));
    assert_eq!(held[0].tool, "Bash");
    assert_eq!(held[0].tool_use_id.as_deref(), Some("toolu_05"));
    assert_eq!(held[0].input["command"], "cargo test -p hangar-core");
    // Nobody but the person answers.
    claude.quiet().await;

    assert!(bridge::answer_ask(
        &id,
        &ask,
        json!({ "behavior": "allow" })
    ));
    assert_eq!(
        claude.told().await,
        Some(answer_to(
            "req-1",
            json!({ "behavior": "allow", "updatedInput": input })
        ))
    );
    running_again(&id).await;
    assert!(bridge::held_asks(&id).is_empty());

    claude.say(result_success("The core tests pass.")).await;
    assert_eq!(ended(&id).await.status, Status::Done);
    claude.closed().await;
    bridge::bridge_cancel(&host, parent.into(), None);
}

// From Claude Code's stream-json docs: an `AskUserQuestion` is answered by an allow whose
// `updatedInput` carries the answers (the mod's `decisionOf`).
#[tokio::test]
async fn a_question_takes_the_persons_answers() {
    let host = host();
    let parent = "p-claude-question";
    let dir = crate::testdir::scratch("claude-question");
    let (id, mut claude) = claude_started(&host, claude_request(parent, &dir)).await;
    let question = json!({ "questions": [{
        "question": "Which parser should I fix first?", "header": "Parser", "multiSelect": false,
        "options": [
            { "label": "JSON", "description": "The config reader" },
            { "label": "TOML", "description": "The kit manifests" },
        ],
    }] });
    claude
        .say(can_use_tool(
            "req-2",
            "AskUserQuestion",
            question.clone(),
            "toolu_06",
        ))
        .await;
    assert_eq!(settled(&id).await.status, Status::NeedsYou);
    let ask = bridge::held_asks(&id)[0].id.clone();

    // An allow without answers does not answer a question: it is asked again.
    assert!(bridge::answer_ask(
        &id,
        &ask,
        json!({ "behavior": "allow" })
    ));
    claude.quiet().await;
    asked_again(&id, &ask).await;
    assert_eq!(
        status(&id, None).await.unwrap().delegation.status,
        Status::NeedsYou
    );

    let answers = json!({ "Which parser should I fix first?": "TOML" });
    assert!(bridge::answer_ask(
        &id,
        &ask,
        json!({ "behavior": "allow", "answers": answers })
    ));
    let mut answered = question;
    answered["answers"] = answers;
    assert_eq!(
        claude.told().await,
        Some(answer_to(
            "req-2",
            json!({ "behavior": "allow", "updatedInput": answered })
        ))
    );
    running_again(&id).await;
    claude.say(result_success("Fixed the TOML parser.")).await;
    assert_eq!(ended(&id).await.status, Status::Done);
    bridge::bridge_cancel(&host, parent.into(), None);
}

// From Claude Code's stream-json docs: a `PermissionResult` deny with `interrupt` stops the turn,
// which then ends with its result.
#[tokio::test]
async fn a_no_that_stops_ends_the_claude_child() {
    let host = host();
    let parent = "p-claude-stop";
    let dir = crate::testdir::scratch("claude-stop");
    let (id, mut claude) = claude_started(&host, claude_request(parent, &dir)).await;
    claude
        .say(can_use_tool(
            "req-3",
            "Bash",
            json!({ "command": "git push --force" }),
            "toolu_07",
        ))
        .await;
    assert_eq!(settled(&id).await.status, Status::NeedsYou);
    let ask = bridge::held_asks(&id)[0].id.clone();
    assert!(bridge::answer_ask(
        &id,
        &ask,
        json!({ "behavior": "deny", "message": "Never force-push.", "interrupt": true })
    ));
    assert_eq!(
        claude.told().await,
        Some(answer_to(
            "req-3",
            json!({ "behavior": "deny", "message": "Never force-push.", "interrupt": true })
        ))
    );
    claude
        .say(result_error("error_during_execution", &[]))
        .await;
    let stopped = ended(&id).await;
    assert_eq!(stopped.status, Status::Cancelled);
    assert_eq!(
        told(parent),
        [format!(
            "Delegation {id} (claude) was cancelled: its turn was stopped before its end"
        )]
    );
    bridge::bridge_cancel(&host, parent.into(), None);
}

// From the Agent SDK's client: a `control_cancel_request` withdraws a request it had made.
#[tokio::test]
async fn a_prompt_claude_code_withdraws_is_withdrawn_from_the_person() {
    let host = host();
    let parent = "p-claude-withdrawn";
    let dir = crate::testdir::scratch("claude-withdrawn");
    let (id, mut claude) = claude_started(&host, claude_request(parent, &dir)).await;
    claude
        .say(can_use_tool(
            "req-4",
            "WebFetch",
            json!({ "url": "https://example.com", "prompt": "Read it" }),
            "toolu_08",
        ))
        .await;
    assert_eq!(settled(&id).await.status, Status::NeedsYou);
    claude
        .say(json!({ "type": "control_cancel_request", "request_id": "req-4" }))
        .await;
    running_again(&id).await;
    assert!(bridge::held_asks(&id).is_empty());
    claude.quiet().await;
    claude.say(result_success("Done without the page.")).await;
    assert_eq!(ended(&id).await.status, Status::Done);
    bridge::bridge_cancel(&host, parent.into(), None);
}

// From Claude Code's stream-json docs: an interrupt is answered with its receipt
// (`SDKControlInterruptResponse`), then the interrupted turn's `SDKResultMessage`.
#[tokio::test]
async fn cancelling_a_claude_child_interrupts_its_turn_and_withdraws_what_it_asked() {
    let host = host();
    let parent = "p-claude-cancel";
    let dir = crate::testdir::scratch("claude-cancel");
    let (id, mut claude) = claude_started(&host, claude_request(parent, &dir)).await;
    claude
        .say(can_use_tool(
            "req-5",
            "Write",
            json!({ "file_path": "/etc/hangar.toml", "content": "x" }),
            "toolu_09",
        ))
        .await;
    assert_eq!(settled(&id).await.status, Status::NeedsYou);
    assert_eq!(bridge::held_asks(&id).len(), 1);

    let cancelling = tokio::spawn({
        let id = id.clone();
        async move { cancel(&id).await }
    });
    assert_eq!(
        claude.told().await,
        Some(json!({
            "type": "control_request", "request_id": "hangar-interrupt",
            "request": { "subtype": "interrupt" },
        }))
    );
    claude
        .say(json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": "hangar-interrupt",
                "response": { "still_queued": [] } },
        }))
        .await;
    claude
        .say(result_error("error_during_execution", &[]))
        .await;
    let cancelled = tokio::time::timeout(PATIENCE, cancelling)
        .await
        .expect("cancelled in time")
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert!(bridge::held_asks(&id).is_empty());
    claude.closed().await;
    assert_eq!(
        told(parent),
        [format!("Delegation {id} (claude) was cancelled.")]
    );
    assert_eq!(cancel(&id).await.unwrap().status, Status::Cancelled);
    bridge::bridge_cancel(&host, parent.into(), None);
}

// From Claude Code's stream-json docs (`headless`): a failure inside the run is printed as its
// result — a `success` that `is_error`.
#[tokio::test]
async fn a_failed_claude_run_fails_the_delegation_with_its_reason() {
    let host = host();
    let parent = "p-claude-failed";
    let dir = crate::testdir::scratch("claude-failed");
    let (id, mut claude) = claude_started(&host, claude_request(parent, &dir)).await;
    let mut limit = result_success("You've hit your usage limit · resets 4pm (Europe/Paris)");
    limit["is_error"] = json!(true);
    claude.say(limit).await;
    let failed = settled(&id).await;
    assert_eq!(failed.status, Status::Failed);
    assert_eq!(
        failed.error.as_deref(),
        Some("You've hit your usage limit · resets 4pm (Europe/Paris)")
    );
    assert_eq!(
        told(parent),
        [format!(
            "Delegation {id} (claude) failed: You've hit your usage limit · resets 4pm (Europe/Paris)"
        )]
    );
    bridge::bridge_cancel(&host, parent.into(), None);
}

#[tokio::test]
async fn a_claude_child_that_ends_before_its_answer_fails() {
    let host = host();
    let parent = "p-claude-gone";
    let dir = crate::testdir::scratch("claude-gone");
    let (id, claude) = claude_started(&host, claude_request(parent, &dir)).await;
    drop(claude);
    let failed = settled(&id).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(
        failed
            .error
            .as_deref()
            .is_some_and(|error| error.contains("ended before its answer")),
        "{failed:?}"
    );
    bridge::bridge_cancel(&host, parent.into(), None);
}

/// A Claude Code child in a worktree of its own gets its branch back, as a Codex child does.
#[tokio::test]
async fn a_claude_child_in_a_worktree_hands_its_branch_and_files_back() {
    let host = host();
    let parent = "p-claude-worktree";
    let repo = repo("claude-worktree");
    let (id, mut claude) = claude_started(
        &host,
        Request {
            isolation: Isolation::Worktree,
            ..claude_request(parent, &repo)
        },
    )
    .await;
    let running = status(&id, None).await.unwrap().delegation;
    let folder = PathBuf::from(&running.cwd);
    assert!(running
        .branch
        .as_deref()
        .is_some_and(|branch| branch.starts_with(&format!("delegate/{id}-"))));
    std::fs::write(folder.join("PARSER.md"), "made by the child\n").unwrap();
    claude
        .say(result_success("Wrote PARSER.md and committed nothing."))
        .await;
    let done = settled(&id).await;
    assert_eq!(done.status, Status::Done);
    assert_eq!(done.branch, running.branch);
    assert_eq!(
        done.files,
        [folder.join("PARSER.md").to_string_lossy().into_owned()]
    );
    bridge::bridge_cancel(&host, parent.into(), None);
}

/// A fake `claude` on disk, run as Hangar runs Claude Code: the launch as `start` makes it, the
/// process stopped as a real one is.
#[cfg(unix)]
mod fake_claude {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// Plays `script` beside it, a step a line: `<` reads one line of its input into `told`,
    /// `<<` reads its input to its end, `!text` says text on stderr, `exit N` exits, `sleep`
    /// sleeps until stopped, and any other line is printed as it is — a stream-json message.
    const SCRIPT: &str = r#"#!/bin/sh
# A fake `claude` for Hangar's tests: never Anthropic's program.
here=$(dirname "$0")
if [ "$1" = --version ]; then
  echo "2.1.328 (Claude Code)"
  exit 0
fi
printf '%s\n' "$@" > "$here/args"
pwd > "$here/cwd"
echo $$ > "$here/pid"
printf '%s\n' "${HANGAR_PANE_ID-unset}" > "$here/pane"
while IFS= read -r step <&3; do
  case "$step" in
    '<') IFS= read -r line; printf '%s\n' "$line" >> "$here/told" ;;
    '<<') while IFS= read -r line; do printf '%s\n' "$line" >> "$here/told"; done ;;
    '!'*) printf '%s\n' "${step#!}" >&2 ;;
    'exit '*) exit "${step#exit }" ;;
    sleep) exec sleep 600 ;;
    *) printf '%s\n' "$step" ;;
  esac
done 3< "$here/script"
"#;

    struct FakeBin {
        /// The folder `claude` is in, with what it was given.
        bin: PathBuf,
    }

    impl FakeBin {
        fn new(steps: &[String]) -> FakeBin {
            let bin = crate::testdir::scratch("fake-claude");
            let program = bin.join("claude");
            std::fs::write(&program, SCRIPT).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            let script: String = steps.iter().map(|step| format!("{step}\n")).collect();
            std::fs::write(bin.join("script"), script).unwrap();
            // A process another test forks while the script is being written can hold it open
            // for a moment ("text file busy"): run it once first.
            for _ in 0..50 {
                match std::process::Command::new(&program)
                    .arg("--version")
                    .output()
                {
                    Err(err) if err.raw_os_error() == Some(libc::ETXTBSY) => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    ran => {
                        assert!(ran.unwrap().status.success());
                        break;
                    }
                }
            }
            FakeBin { bin }
        }

        fn program(&self) -> String {
            self.bin.join("claude").to_string_lossy().into_owned()
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.bin.join(name)).unwrap_or_default()
        }

        fn args(&self) -> Vec<String> {
            self.read("args").lines().map(str::to_string).collect()
        }

        fn told(&self) -> Vec<Value> {
            self.read("told")
                .lines()
                .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}")))
                .collect()
        }
    }

    fn step(message: Value) -> String {
        message.to_string()
    }

    fn on(fake: &FakeBin, parent: &str, work: &Path) -> Request {
        Request {
            command: Some(fake.program()),
            permission_mode: Some(PermissionMode::Auto),
            ..claude_request(parent, work)
        }
    }

    fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        let at = args.iter().position(|arg| arg == flag)?;
        args.get(at + 1).map(String::as_str)
    }

    #[tokio::test]
    async fn a_fake_claude_runs_on_the_launch_settings_and_hands_its_result_back() {
        let host = host();
        let parent = "p-fake-claude-result";
        let work = crate::testdir::scratch("fake-claude-work");
        let cwd = work.to_string_lossy().into_owned();
        let fake = FakeBin::new(&[
            "<".into(),
            step(init(&cwd)),
            step(assistant(
                "msg_01",
                json!([{ "type": "tool_use", "id": "toolu_01", "name": "Write",
                         "input": { "file_path": "src/parser.rs", "content": "fn main() {}" } }]),
                None,
            )),
            step(tool_result("toolu_01", false, None)),
            step(result_success("Rewrote src/parser.rs.")),
            "<<".into(),
        ]);
        let delegation = start(&host, on(&fake, parent, &work)).await.unwrap();
        let done = settled(&delegation.id).await;
        assert_eq!(done.status, Status::Done, "{done:?}");
        assert_eq!(done.result.as_deref(), Some("Rewrote src/parser.rs."));
        assert_eq!(
            done.files,
            [work.join("src/parser.rs").to_string_lossy().into_owned()]
        );
        assert_eq!(done.session.as_deref(), Some(SESSION));

        let args = fake.args();
        assert_eq!(
            &args[..8],
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
        assert!(after(&args, "--append-system-prompt")
            .is_some_and(|role| role.contains("Nobody watches this session")));
        assert_eq!(after(&args, "--model"), Some("sonnet"));
        assert_eq!(after(&args, "--permission-mode"), Some("auto"));
        assert_eq!(
            std::fs::canonicalize(fake.read("cwd").trim()).unwrap(),
            work.canonicalize().unwrap()
        );
        // Never the identity of a pane Hangar was started from.
        assert_eq!(fake.read("pane").trim(), "unset");
        // The task, and nothing after the result: its input closed.
        assert_eq!(fake.told(), [the_task()]);
        bridge::bridge_cancel(&host, parent.into(), None);
    }

    #[tokio::test]
    async fn a_fake_claudes_prompt_waits_for_the_person() {
        let host = host();
        let parent = "p-fake-claude-ask";
        let work = crate::testdir::scratch("fake-claude-ask");
        let input = json!({ "command": "cargo build", "description": "Build" });
        let fake = FakeBin::new(&[
            "<".into(),
            step(init(&work.to_string_lossy())),
            step(can_use_tool("req-1", "Bash", input.clone(), "toolu_02")),
            "<".into(),
            step(result_success("Built.")),
            "<<".into(),
        ]);
        let delegation = start(&host, on(&fake, parent, &work)).await.unwrap();
        let id = delegation.id;
        assert_eq!(settled(&id).await.status, Status::NeedsYou);
        let ask = bridge::held_asks(&id)[0].id.clone();
        assert!(bridge::answer_ask(
            &id,
            &ask,
            json!({ "behavior": "allow" })
        ));
        assert_eq!(ended(&id).await.status, Status::Done);
        assert_eq!(
            fake.told(),
            [
                the_task(),
                answer_to(
                    "req-1",
                    json!({ "behavior": "allow", "updatedInput": input })
                )
            ]
        );
        bridge::bridge_cancel(&host, parent.into(), None);
    }

    /// One that does not stop when interrupted, nor when its input closes, is stopped.
    #[tokio::test]
    async fn cancelling_stops_a_fake_claude_that_does_not_stop_by_itself() {
        let host = host();
        let parent = "p-fake-claude-cancel";
        let work = crate::testdir::scratch("fake-claude-cancel");
        let fake = FakeBin::new(&[
            "<".into(),
            step(init(&work.to_string_lossy())),
            "sleep".into(),
        ]);
        let delegation = start(&host, on(&fake, parent, &work)).await.unwrap();
        let id = delegation.id;
        let deadline = tokio::time::Instant::now() + PATIENCE;
        while status(&id, None)
            .await
            .unwrap()
            .delegation
            .session
            .is_none()
        {
            assert!(tokio::time::Instant::now() < deadline, "started");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let pid: libc::pid_t = fake.read("pid").trim().parse().unwrap();
        let cancelled = tokio::time::timeout(Duration::from_secs(30), cancel(&id))
            .await
            .expect("cancelled in time")
            .unwrap();
        assert_eq!(cancelled.status, Status::Cancelled);
        // SAFETY: signal 0 only asks whether the process is there.
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "the process went");
        assert_eq!(
            told(parent),
            [format!("Delegation {id} (claude) was cancelled.")]
        );
        bridge::bridge_cancel(&host, parent.into(), None);
    }

    #[tokio::test]
    async fn a_fake_claude_that_dies_fails_with_what_it_said() {
        let host = host();
        let parent = "p-fake-claude-dies";
        let work = crate::testdir::scratch("fake-claude-dies");
        let fake = FakeBin::new(&[
            "<".into(),
            "!Error: Invalid API key · Please run /login".into(),
            "exit 1".into(),
        ]);
        let delegation = start(&host, on(&fake, parent, &work)).await.unwrap();
        let failed = settled(&delegation.id).await;
        assert_eq!(failed.status, Status::Failed);
        let error = failed.error.unwrap_or_default();
        assert!(error.contains("ended before its answer"), "{error}");
        assert!(error.contains("Invalid API key"), "{error}");
        bridge::bridge_cancel(&host, parent.into(), None);
    }

    #[tokio::test]
    async fn a_claude_that_is_not_there_fails_the_delegation() {
        let host = host();
        let parent = "p-fake-claude-missing";
        let work = crate::testdir::scratch("fake-claude-missing");
        let missing = Request {
            command: Some(work.join("no-claude").to_string_lossy().into_owned()),
            ..claude_request(parent, &work)
        };
        let delegation = start(&host, missing).await.unwrap();
        let failed = settled(&delegation.id).await;
        assert_eq!(failed.status, Status::Failed);
        assert!(
            failed
                .error
                .as_deref()
                .is_some_and(|error| error.contains("no-claude")),
            "{failed:?}"
        );
        bridge::bridge_cancel(&host, parent.into(), None);
    }
}
