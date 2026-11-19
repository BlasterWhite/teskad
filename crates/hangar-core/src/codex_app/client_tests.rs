//! Tests of the client (client.rs) against a fake app-server in the test process.
//!
//! No Codex session was ever recorded (the person's decision, docs/native/agents.md §7): the
//! exchanges the fake server plays are written by hand from the pinned protocol schema and, where
//! the schema says nothing, OpenAI's source at the pinned tag, in
//! crates/hangar-core/tests/fixtures/codex/app-server/. Each says so on its first line. A line
//! `> {…}` is what Hangar must send (compared as JSON), `< …` what Codex sends (sent as it is,
//! even when it is not JSON).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadHalf, WriteHalf,
};
use tokio::task::JoinHandle;

use super::client::{
    Cause, Client, Endpoint, Error, Event, Events, Framing, Incoming, Options, Report, Unhandled,
    INVALID_REQUEST,
};
use super::gate::{Refusal, Verdict, CODEX_PINNED};
use super::generated::*;
use super::websocket::{self, Received};

const FIXTURES: &[(&str, &str)] = &[
    (
        "initialize.jsonl",
        include_str!("../../tests/fixtures/codex/app-server/initialize.jsonl"),
    ),
    (
        "thread-and-turn.jsonl",
        include_str!("../../tests/fixtures/codex/app-server/thread-and-turn.jsonl"),
    ),
    (
        "server-requests.jsonl",
        include_str!("../../tests/fixtures/codex/app-server/server-requests.jsonl"),
    ),
    (
        "lenient.jsonl",
        include_str!("../../tests/fixtures/codex/app-server/lenient.jsonl"),
    ),
    (
        "steer.jsonl",
        include_str!("../../tests/fixtures/codex/app-server/steer.jsonl"),
    ),
    (
        "attached.jsonl",
        include_str!("../../tests/fixtures/codex/app-server/attached.jsonl"),
    ),
];

const THREAD: &str = "019a7c2e-5b1d-7f43-9a61-3c8e2f4d6b10";

/// The longest a test waits for one thing.
const PATIENCE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
enum Step {
    /// What Hangar must send.
    Hangar(Value),
    /// What Codex sends, as it is.
    Codex(String),
}

fn fixture(name: &str) -> &'static str {
    FIXTURES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no fixture {name}"))
        .1
}

fn steps(name: &str) -> Vec<Step> {
    fixture(name)
        .lines()
        .filter_map(|line| {
            if let Some(json) = line.strip_prefix("> ") {
                let value =
                    serde_json::from_str(json).unwrap_or_else(|e| panic!("{name}: {e}: {json}"));
                Some(Step::Hangar(value))
            } else if let Some(text) = line.strip_prefix("< ") {
                Some(Step::Codex(text.to_string()))
            } else {
                assert!(line.is_empty() || line.starts_with('#'), "{name}: {line}");
                None
            }
        })
        .collect()
}

/// What Codex sends in `name`, as JSON (the lines that are JSON).
fn codex_messages(name: &str) -> Vec<Value> {
    steps(name)
        .into_iter()
        .filter_map(|step| match step {
            Step::Codex(text) => serde_json::from_str(&text).ok(),
            Step::Hangar(_) => None,
        })
        .collect()
}

/// The fake app-server: the server's side of a stream, in either framing.
struct Fake<R, W> {
    read: BufReader<R>,
    write: W,
    framing: Framing,
    frames: websocket::Reader,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Fake<R, W> {
    fn new(read: R, write: W, framing: Framing) -> Self {
        Fake {
            read: BufReader::new(read),
            write,
            framing,
            frames: websocket::Reader::default(),
        }
    }

    /// Takes the client's WebSocket upgrade, as Codex's socket does.
    async fn accept_upgrade(&mut self) -> String {
        let mut request = String::new();
        loop {
            let mut line = String::new();
            let n = tokio::time::timeout(PATIENCE, self.read.read_line(&mut line))
                .await
                .expect("the upgrade in time")
                .unwrap();
            assert!(n > 0, "the client went before its upgrade");
            request.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        self.write
            .write_all(
                b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
                  Sec-WebSocket-Accept: not-checked\r\n\r\n",
            )
            .await
            .unwrap();
        self.write.flush().await.unwrap();
        request
    }

    /// The client's next message; `None` once it closed its side.
    async fn recv(&mut self) -> Option<Value> {
        tokio::time::timeout(PATIENCE, self.recv_now())
            .await
            .expect("a message from the client in time")
    }

    async fn recv_now(&mut self) -> Option<Value> {
        match self.framing {
            Framing::Lines => {
                let mut line = String::new();
                if self.read.read_line(&mut line).await.unwrap() == 0 {
                    return None;
                }
                assert!(line.ends_with('\n'), "one message per line: {line}");
                Some(serde_json::from_str(&line).expect("the client sends JSON"))
            }
            Framing::WebSocket => match self.frames.next(&mut self.read).await.unwrap() {
                Received::Text(bytes) => Some(serde_json::from_slice(&bytes).expect("JSON")),
                Received::Closed => None,
                other => panic!("the client sent {other:?}"),
            },
        }
    }

    /// The client's next WebSocket frame as it is: its opcode and unmasked payload. Checks that
    /// a client's frame is masked.
    async fn raw_frame(&mut self) -> (u8, Vec<u8>) {
        use tokio::io::AsyncReadExt;
        let read = async {
            let mut head = [0u8; 2];
            self.read.read_exact(&mut head).await.unwrap();
            assert_ne!(head[1] & 0x80, 0, "a client's frame is masked");
            let len = usize::from(head[1] & 0x7f);
            assert!(len < 126, "a short frame");
            let mut mask = [0u8; 4];
            self.read.read_exact(&mut mask).await.unwrap();
            let mut payload = vec![0u8; len];
            self.read.read_exact(&mut payload).await.unwrap();
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
            (head[0] & 0x0f, payload)
        };
        tokio::time::timeout(PATIENCE, read)
            .await
            .expect("a frame in time")
    }

    /// The client's next WebSocket message or close, as the reader sees it.
    async fn recv_frame(&mut self) -> Received {
        tokio::time::timeout(PATIENCE, self.frames.next(&mut self.read))
            .await
            .expect("a frame in time")
            .unwrap()
    }

    async fn send(&mut self, text: &str) {
        match self.framing {
            Framing::Lines => {
                self.write.write_all(text.as_bytes()).await.unwrap();
                self.write.write_all(b"\n").await.unwrap();
            }
            Framing::WebSocket => {
                let frame = websocket::server_frame(websocket::TEXT, true, text.as_bytes());
                self.write.write_all(&frame).await.unwrap();
            }
        }
        self.write.flush().await.unwrap();
    }

    /// Plays a fixture: sends Codex's lines, checks Hangar's.
    async fn play(&mut self, name: &str) {
        for step in steps(name) {
            match step {
                Step::Hangar(expected) => {
                    let got = self.recv().await.unwrap_or_else(|| {
                        panic!("{name}: the client closed; expected {expected}")
                    });
                    assert_eq!(got, expected, "{name}: what Hangar sent");
                }
                Step::Codex(text) => self.send(&text).await,
            }
        }
    }
}

type DuplexFake = Fake<ReadHalf<tokio::io::DuplexStream>, WriteHalf<tokio::io::DuplexStream>>;

/// The reports a client made.
#[derive(Clone, Default)]
struct Reports(Arc<Mutex<Vec<String>>>);

impl Reports {
    fn report(&self) -> Report {
        let all = Arc::clone(&self.0);
        Report::new(move |message| all.lock().unwrap().push(message.to_string()))
    }

    fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

fn options(reports: &Reports) -> Options {
    Options {
        client_version: "test".into(),
        request_timeout: PATIENCE,
        initialize_timeout: PATIENCE,
        shutdown_grace: Duration::from_millis(500),
        report: reports.report(),
        ..Options::default()
    }
}

/// A client past the handshake of initialize.jsonl, over lines, and the fake server playing
/// `then` after it (its task gives the fake back).
async fn start(
    options: Options,
    then: &'static [&'static str],
) -> (Client, Events, JoinHandle<DuplexFake>) {
    let (client_side, server_side) = tokio::io::duplex(1 << 20);
    let (server_read, server_write) = tokio::io::split(server_side);
    let mut fake = Fake::new(server_read, server_write, Framing::Lines);
    let server = tokio::spawn(async move {
        fake.play("initialize.jsonl").await;
        for name in then {
            fake.play(name).await;
        }
        fake
    });
    let (read, write) = tokio::io::split(client_side);
    let (client, events) = Client::connect_over(read, write, Framing::Lines, options)
        .await
        .expect("connected");
    (client, events, server)
}

async fn next(events: &mut Events) -> Event {
    tokio::time::timeout(PATIENCE, events.next())
        .await
        .expect("an event in time")
        .expect("the stream is open")
}

async fn finished<T>(server: JoinHandle<T>) -> T {
    tokio::time::timeout(PATIENCE, server)
        .await
        .expect("the fake server played its fixtures in time")
        .expect("the fake server's checks passed")
}

fn method(notification: &ServerNotification) -> &str {
    notification.method()
}

#[test]
fn every_fixture_says_it_was_written_from_the_pinned_schema() {
    let first = format!("# from the protocol schema (Codex {CODEX_PINNED})");
    for (name, text) in FIXTURES {
        let line = text.lines().next().unwrap_or("");
        assert!(line.starts_with(&first), "{name}: {line}");
        assert!(line.contains("not recorded"), "{name}: {line}");
        assert!(!steps(name).is_empty(), "{name}");
    }
}

/// The fixtures' messages are what the pinned schema reads: none falls to `Unrecognized` but
/// those lenient.jsonl and attached.jsonl are about.
#[test]
fn the_fixtures_messages_read_as_the_pinned_types() {
    for name in [
        "initialize.jsonl",
        "thread-and-turn.jsonl",
        "server-requests.jsonl",
        "steer.jsonl",
    ] {
        for message in codex_messages(name) {
            let has_id = message.get("id").is_some();
            match message.get("method") {
                Some(_) if has_id => {
                    let request: ServerRequest = serde_json::from_value(message.clone()).unwrap();
                    assert!(
                        !matches!(request, ServerRequest::Unrecognized(_)),
                        "{name}: {message}"
                    );
                }
                Some(_) => {
                    let notification: ServerNotification =
                        serde_json::from_value(message.clone()).unwrap();
                    assert!(
                        !matches!(notification, ServerNotification::Unrecognized(_)),
                        "{name}: {message}"
                    );
                }
                None => {}
            }
        }
    }
    let answer = &codex_messages("initialize.jsonl")[0]["result"];
    serde_json::from_value::<InitializeResponse>(answer.clone()).unwrap();
}

#[tokio::test]
async fn the_handshake_is_initialize_then_initialized() {
    let reports = Reports::default();
    let (client, mut events, server) = start(options(&reports), &[]).await;
    let mut fake = finished(server).await;

    let server_info = client.server();
    assert_eq!(
        server_info.verdict,
        Verdict::Supported(semver::Version::new(0, 170, 0))
    );
    assert_eq!(server_info.info.codex_home, "/home/ada/.codex");
    assert_eq!(server_info.info.platform_os, "linux");
    assert!(reports.all().is_empty(), "{:?}", reports.all());

    assert_eq!(client.shutdown().await, None, "nothing was spawned");
    assert_eq!(fake.recv().await, None, "Hangar closed its side");
    match next(&mut events).await {
        Event::Closed(closed) => assert_eq!(closed.cause, Cause::Shutdown),
        other => panic!("{other:?}"),
    }
    assert!(events.next().await.is_none());
    // Again: nothing more happens.
    assert_eq!(client.shutdown().await, None);
    assert_eq!(client.closed().map(|c| c.cause), Some(Cause::Shutdown));
}

#[tokio::test]
async fn a_thread_and_a_turn_come_in_the_order_codex_sends_them() {
    let reports = Reports::default();
    let (client, mut events, server) = start(options(&reports), &["thread-and-turn.jsonl"]).await;

    let started = client
        .request::<requests::ThreadStart>(ThreadStartParams {
            cwd: Some("/work/hangar".into()),
            developer_instructions: Some("You are a worker of the workspace \"hangar\".".into()),
            ..ThreadStartParams::default()
        })
        .await
        .unwrap();
    assert_eq!(started.thread.id, THREAD);
    assert_eq!(started.model, "gpt-5.5-codex");
    assert_eq!(started.approval_policy, AskForApproval::OnRequest);

    let turn = client
        .request::<requests::TurnStart>(TurnStartParams {
            thread_id: THREAD.into(),
            input: vec![UserInput::Text(TextUserInput {
                text: "Summarise README.md in one sentence.".into(),
                text_elements: None,
            })],
            ..TurnStartParams::default()
        })
        .await
        .unwrap()
        .turn;
    assert_eq!(turn.id, "turn-1");
    assert_eq!(turn.status, TurnStatus::InProgress);

    let mut seen = Vec::new();
    let mut text = String::new();
    let mut usage = None;
    loop {
        let Event::Notification(notification) = next(&mut events).await else {
            panic!("only notifications in this turn");
        };
        seen.push(method(&notification).to_string());
        match notification {
            ServerNotification::ItemAgentMessageDelta(delta) => text.push_str(&delta.delta),
            ServerNotification::ItemCompleted(completed) => {
                if let ThreadItem::AgentMessage(message) = completed.item {
                    assert_eq!(message.text, text, "the deltas make the message");
                }
            }
            ServerNotification::ThreadTokenUsageUpdated(updated) => {
                usage = Some(updated.token_usage)
            }
            ServerNotification::ThreadStatusChanged(changed) => {
                if changed.status == ThreadStatus::Idle {
                    break;
                }
                assert!(
                    matches!(changed.status, ThreadStatus::Active(_)),
                    "{changed:?}"
                );
            }
            ServerNotification::TurnCompleted(completed) => {
                assert_eq!(completed.turn.status, TurnStatus::Completed);
            }
            _ => {}
        }
    }
    assert_eq!(
        seen,
        [
            "thread/started",
            "turn/started",
            "thread/status/changed",
            "item/started",
            "item/completed",
            "item/started",
            "item/agentMessage/delta",
            "item/agentMessage/delta",
            "item/completed",
            "thread/tokenUsage/updated",
            "turn/completed",
            "thread/status/changed",
        ]
    );
    assert_eq!(
        text,
        "Hangar runs coding agents in real terminals, side by side."
    );
    let usage = usage.expect("the turn's usage");
    assert_eq!(usage.total.total_tokens, 5251);
    assert_eq!(usage.model_context_window, Some(272000));
    finished(server).await;
    assert!(reports.all().is_empty(), "{:?}", reports.all());
}

#[tokio::test]
async fn server_requests_are_answered_with_the_response_their_method_takes() {
    let reports = Reports::default();
    let (_client, mut events, server) = start(options(&reports), &["server-requests.jsonl"]).await;

    let mut asked = Vec::new();
    let mut resolved = Vec::new();
    while resolved.len() < 5 {
        match next(&mut events).await {
            Event::Request(incoming) => {
                asked.push((incoming.id().clone(), incoming.method()));
                match incoming {
                    Incoming::CommandExecutionApproval(ask) => {
                        assert_eq!(
                            ask.params().command.as_deref(),
                            Some("cargo test -p hangar-core")
                        );
                        ask.respond(CommandExecutionRequestApprovalResponse {
                            decision: CommandExecutionApprovalDecision::AcceptForSession,
                        })
                        .unwrap();
                    }
                    Incoming::FileChangeApproval(ask) => {
                        assert_eq!(ask.params().grant_root.as_deref(), Some("/etc/hangar"));
                        ask.respond(FileChangeRequestApprovalResponse {
                            decision: FileChangeApprovalDecision::Decline,
                        })
                        .unwrap();
                    }
                    Incoming::PermissionsApproval(ask) => {
                        let network = ask.params().permissions.network.clone();
                        assert_eq!(network.and_then(|n| n.enabled), Some(true));
                        ask.respond(PermissionsRequestApprovalResponse {
                            permissions: GrantedPermissionProfile {
                                network: Some(AdditionalNetworkPermissions {
                                    enabled: Some(true),
                                }),
                                ..GrantedPermissionProfile::default()
                            },
                            scope: Some(PermissionGrantScope::Turn),
                            strict_auto_review: None,
                        })
                        .unwrap();
                    }
                    Incoming::UserInput(ask) => {
                        let question = &ask.params().questions[0];
                        assert_eq!(question.id, "target");
                        let labels: Vec<&str> = question
                            .options
                            .iter()
                            .flatten()
                            .map(|o| o.label.as_str())
                            .collect();
                        assert_eq!(labels, ["main", "release"]);
                        ask.respond(ToolRequestUserInputResponse {
                            answers: BTreeMap::from([(
                                "target".to_string(),
                                ToolRequestUserInputAnswer {
                                    answers: vec!["main".into()],
                                },
                            )]),
                        })
                        .unwrap();
                    }
                    Incoming::McpElicitation(ask) => {
                        let McpServerElicitationRequestParams::Form(form) = ask.params() else {
                            panic!("a form: {:?}", ask.params());
                        };
                        assert_eq!(form.server_name, "github");
                        assert_eq!(form.message, "Which repository?");
                        ask.respond(McpServerElicitationRequestResponse {
                            meta: None,
                            action: McpServerElicitationAction::Accept,
                            content: Some(json!({ "repo": "hangar-ai/hangar" })),
                        })
                        .unwrap();
                    }
                }
            }
            Event::Notification(ServerNotification::ServerRequestResolved(done)) => {
                assert_eq!(done.thread_id, THREAD);
                resolved.push(done.request_id);
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(
        asked,
        [
            (
                RequestId::Integer(0),
                "item/commandExecution/requestApproval"
            ),
            (RequestId::Integer(1), "item/fileChange/requestApproval"),
            (RequestId::Integer(2), "item/permissions/requestApproval"),
            (RequestId::Integer(3), "item/tool/requestUserInput"),
            (RequestId::Integer(4), "mcpServer/elicitation/request"),
        ]
    );
    let asked_ids: Vec<RequestId> = asked.into_iter().map(|(id, _)| id).collect();
    assert_eq!(resolved, asked_ids);
    // The fake checked each answer on the wire.
    finished(server).await;
    assert!(reports.all().is_empty(), "{:?}", reports.all());
}

#[tokio::test]
async fn what_hangar_cannot_read_is_dropped_reported_once_and_never_stops_the_stream() {
    let reports = Reports::default();
    let (_client, mut events, server) = start(options(&reports), &["lenient.jsonl"]).await;

    // Unknown fields of a newer Codex are ignored.
    match next(&mut events).await {
        Event::Notification(ServerNotification::TurnStarted(started)) => {
            assert_eq!(started.turn.id, "turn-2");
        }
        other => panic!("{other:?}"),
    }
    match next(&mut events).await {
        Event::Request(Incoming::FileChangeApproval(ask)) => {
            assert_eq!(ask.id(), &RequestId::Integer(9));
            ask.respond(FileChangeRequestApprovalResponse {
                decision: FileChangeApprovalDecision::Cancel,
            })
            .unwrap();
        }
        other => panic!("{other:?}"),
    }
    match next(&mut events).await {
        Event::Notification(ServerNotification::Warning(warning)) => {
            assert_eq!(warning.message, "The model was rerouted");
        }
        other => panic!("{other:?}"),
    }
    finished(server).await;

    let reports = reports.all();
    let expected = [
        "Codex 0.170.0 sends `thread/name/updated`, a notification Hangar does not read: ignored",
        "could not read a `turn/completed` notification from Codex 0.170.0: dropped",
        "dropped a message from Codex 0.170.0 that is not a JSON-RPC object: not JSON at all",
        "Codex 0.170.0 asks `item/tool/call`, a request Hangar does not handle: refused",
        "could not read a `item/commandExecution/requestApproval` request from Codex 0.170.0: refused",
    ];
    assert_eq!(reports, expected, "each once, in order");
}

#[tokio::test]
async fn beside_the_tui_hangar_leaves_what_it_does_not_answer_to_it() {
    let reports = Reports::default();
    let options = Options {
        unhandled: Unhandled::Leave,
        ..options(&reports)
    };
    let (client, mut events, server) = start(options, &["attached.jsonl"]).await;

    let held = match next(&mut events).await {
        Event::Request(Incoming::CommandExecutionApproval(ask)) => ask,
        other => panic!("{other:?}"),
    };
    assert_eq!(held.id(), &RequestId::Integer(13));
    match next(&mut events).await {
        Event::Notification(ServerNotification::ServerRequestResolved(done)) => {
            // The TUI answered first: Hangar lets its ask go, without an answer.
            assert_eq!(&done.request_id, held.id());
            drop(held);
        }
        other => panic!("{other:?}"),
    }
    // The fake's next message from Hangar must be this request: no refusal went before it.
    let loaded = client
        .request::<requests::ThreadLoadedList>(ThreadLoadedListParams::default())
        .await
        .unwrap();
    assert_eq!(loaded.data, [THREAD]);
    finished(server).await;
    assert_eq!(
        reports.all(),
        [
            "Codex 0.170.0 asks `item/tool/call`, a request Hangar does not handle: left to the other client",
            "could not read a `item/fileChange/requestApproval` request from Codex 0.170.0: left to the other client",
        ]
    );
}

#[tokio::test]
async fn steering_a_turn_that_ended_is_refused_by_codex() {
    let reports = Reports::default();
    let (client, mut events, server) = start(options(&reports), &["steer.jsonl"]).await;

    let steered = client
        .request::<requests::TurnSteer>(TurnSteerParams {
            thread_id: THREAD.into(),
            expected_turn_id: "turn-1".into(),
            input: vec![UserInput::Text(TextUserInput {
                text: "[Hangar] From agent \"Leo\": also run the linter.".into(),
                text_elements: None,
            })],
            client_user_message_id: None,
        })
        .await
        .unwrap();
    assert_eq!(steered.turn_id, "turn-1");
    client
        .request::<requests::TurnInterrupt>(TurnInterruptParams {
            thread_id: THREAD.into(),
            turn_id: "turn-1".into(),
        })
        .await
        .unwrap();
    match next(&mut events).await {
        Event::Notification(ServerNotification::TurnCompleted(done)) => {
            assert_eq!(done.turn.status, TurnStatus::Interrupted);
        }
        other => panic!("{other:?}"),
    }
    let refused = client
        .request::<requests::TurnSteer>(TurnSteerParams {
            thread_id: THREAD.into(),
            expected_turn_id: "turn-1".into(),
            input: vec![UserInput::Text(TextUserInput {
                text: "And the formatter.".into(),
                text_elements: None,
            })],
            client_user_message_id: None,
        })
        .await
        .unwrap_err();
    match &refused {
        Error::Rpc { method, error } => {
            assert_eq!(*method, "turn/steer");
            assert_eq!(error.code, INVALID_REQUEST);
            assert_eq!(error.message, "no active turn to steer");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        refused.to_string(),
        "Codex refused `turn/steer`: no active turn to steer (code -32600)"
    );
    finished(server).await;
}

#[tokio::test]
async fn a_request_codex_does_not_answer_times_out_and_its_late_answer_is_dropped() {
    let reports = Reports::default();
    let (client, mut events, server) = start(options(&reports), &[]).await;
    let mut fake = finished(server).await;

    let waited = client
        .request_within::<requests::ThreadLoadedList>(
            ThreadLoadedListParams::default(),
            Duration::from_millis(100),
        )
        .await
        .unwrap_err();
    match &waited {
        Error::Timeout { method, after } => {
            assert_eq!(*method, "thread/loaded/list");
            assert_eq!(*after, Duration::from_millis(100));
        }
        other => panic!("{other:?}"),
    }
    let asked = fake.recv().await.unwrap();
    assert_eq!(asked["id"], 1);
    fake.send(r#"{"id":1,"result":{"data":[]}}"#).await;
    fake.send(r#"{"method":"warning","params":{"message":"late"}}"#)
        .await;
    match next(&mut events).await {
        Event::Notification(ServerNotification::Warning(w)) => assert_eq!(w.message, "late"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        reports.all(),
        ["dropped an answer from Codex to a request no longer waiting (id 1)"]
    );

    // The connection goes on.
    let next_request = tokio::spawn(async move {
        let answer = client
            .request::<requests::ThreadLoadedList>(ThreadLoadedListParams::default())
            .await;
        (client, answer)
    });
    let asked = fake.recv().await.unwrap();
    assert_eq!(asked["id"], 2);
    fake.send(r#"{"id":2,"result":{"data":["t"]}}"#).await;
    let (_client, answer) = next_request.await.unwrap();
    assert_eq!(answer.unwrap().data, ["t"]);
}

#[tokio::test]
async fn when_codex_goes_waiting_requests_fail_and_the_events_end() {
    let reports = Reports::default();
    let (client, mut events, server) = start(options(&reports), &[]).await;
    let mut fake = finished(server).await;
    let client = Arc::new(client);

    let waiting = {
        let client = Arc::clone(&client);
        tokio::spawn(async move {
            client
                .request::<requests::ModelList>(ModelListParams::default())
                .await
        })
    };
    assert_eq!(fake.recv().await.unwrap()["method"], "model/list");
    drop(fake);

    match waiting.await.unwrap().unwrap_err() {
        Error::Closed(closed) => assert_eq!(closed.cause, Cause::ServerClosed),
        other => panic!("{other:?}"),
    }
    match next(&mut events).await {
        Event::Closed(closed) => {
            assert_eq!(closed.cause, Cause::ServerClosed);
            assert_eq!(
                closed.to_string(),
                "Codex's app-server closed the connection"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(events.next().await.is_none());
    // Nothing is sent any more.
    let after = client
        .request::<requests::ModelList>(ModelListParams::default())
        .await
        .unwrap_err();
    assert!(matches!(after, Error::Closed(_)), "{after:?}");
}

#[tokio::test]
async fn a_request_hangar_drops_is_refused_rather_than_left_waiting() {
    let reports = Reports::default();
    let (_client, mut events, server) = start(options(&reports), &[]).await;
    let mut fake = finished(server).await;
    let ask = &codex_messages("server-requests.jsonl")[0];
    fake.send(&ask.to_string()).await;
    match next(&mut events).await {
        Event::Request(incoming) => drop(incoming),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        fake.recv().await.unwrap(),
        json!({
            "id": 0,
            "error": {
                "code": -32603,
                "message": "Hangar dropped `item/commandExecution/requestApproval` without an answer"
            }
        })
    );
}

/// initialize.jsonl's answer, from another release.
fn initialize_answer_from(user_agent: &str) -> String {
    let mut answer = codex_messages("initialize.jsonl")[0].clone();
    answer["result"]["userAgent"] = json!(user_agent);
    answer.to_string()
}

#[tokio::test]
async fn an_older_codex_is_refused_at_the_handshake() {
    let (client_side, server_side) = tokio::io::duplex(1 << 16);
    let (server_read, server_write) = tokio::io::split(server_side);
    let mut fake = Fake::new(server_read, server_write, Framing::Lines);
    let server = tokio::spawn(async move {
        let initialize = fake.recv().await.unwrap();
        assert_eq!(initialize["method"], "initialize");
        fake.send(&initialize_answer_from(
            "codex-tui/0.156.1 (Ubuntu 24.4.0; x86_64) xterm-256color (codex-tui; 0.156.1)",
        ))
        .await;
        // No `initialized`: Hangar leaves.
        assert_eq!(fake.recv().await, None);
    });
    let (read, write) = tokio::io::split(client_side);
    let refused = Client::connect_over(read, write, Framing::Lines, options(&Reports::default()))
        .await
        .unwrap_err();
    match &refused {
        Error::Unsupported(Refusal::TooOld { found, .. }) => {
            assert_eq!(found, &semver::Version::new(0, 156, 1));
        }
        other => panic!("{other:?}"),
    }
    assert!(
        refused.to_string().contains("0.157.0 or newer"),
        "{refused}"
    );
    finished(server).await;
}

#[tokio::test]
async fn a_newer_codex_is_spoken_to_and_said_once() {
    let reports = Reports::default();
    let (client_side, server_side) = tokio::io::duplex(1 << 16);
    let (server_read, server_write) = tokio::io::split(server_side);
    let mut fake = Fake::new(server_read, server_write, Framing::Lines);
    let server = tokio::spawn(async move {
        fake.recv().await.unwrap();
        fake.send(&initialize_answer_from(
            "codex-tui/0.175.2 (Mac OS 15.5.0; arm64) iTerm.app/3.5.14 (codex-tui; 0.175.2)",
        ))
        .await;
        assert_eq!(
            fake.recv().await.unwrap(),
            json!({ "method": "initialized" })
        );
        fake
    });
    let (read, write) = tokio::io::split(client_side);
    let (client, _events) = Client::connect_over(read, write, Framing::Lines, options(&reports))
        .await
        .unwrap();
    assert_eq!(
        client.server().verdict,
        Verdict::Newer(semver::Version::new(0, 175, 2))
    );
    assert_eq!(
        reports.all(),
        [
            "Codex 0.175.2 is newer than 0.170.0, the release Hangar's protocol types come from: \
          what changed is read leniently"
        ]
    );
    finished(server).await;
}

/// The WebSocket a pane's socket speaks, here as it comes through `codex app-server proxy`'s
/// stdio: the upgrade, the handshake, a ping answered, the close at shutdown.
#[tokio::test]
async fn a_pane_socket_is_spoken_to_over_websocket() {
    let reports = Reports::default();
    let (client_side, server_side) = tokio::io::duplex(1 << 20);
    let (server_read, server_write) = tokio::io::split(server_side);
    let mut fake = Fake::new(server_read, server_write, Framing::WebSocket);
    let server = tokio::spawn(async move {
        let upgrade = fake.accept_upgrade().await;
        assert!(upgrade.starts_with("GET / HTTP/1.1\r\n"), "{upgrade}");
        fake.play("initialize.jsonl").await;
        let ping = websocket::server_frame(0x9, true, b"are you there");
        fake.write.write_all(&ping).await.unwrap();
        fake.write.flush().await.unwrap();
        // The pong carries the ping's payload back.
        assert_eq!(fake.raw_frame().await, (0xA, b"are you there".to_vec()));
        fake.send(r#"{"method":"warning","params":{"message":"over a socket"}}"#)
            .await;
        fake
    });
    let (read, write) = tokio::io::split(client_side);
    let (client, mut events) =
        Client::connect_over(read, write, Framing::WebSocket, options(&reports))
            .await
            .unwrap();
    let mut fake = finished(server).await;
    match next(&mut events).await {
        Event::Notification(ServerNotification::Warning(w)) => {
            assert_eq!(w.message, "over a socket")
        }
        other => panic!("{other:?}"),
    }
    client.shutdown().await;
    // A close frame, then the end of the stream.
    assert_eq!(fake.recv_frame().await, Received::Closed);
    assert!(reports.all().is_empty(), "{:?}", reports.all());
}

/// A pane's server socket (`--listen unix://<socket>`), connected to directly.
#[cfg(unix)]
#[tokio::test]
async fn a_pane_socket_is_reached_by_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("codex-pane.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, write) = tokio::io::split(stream);
        let mut fake = Fake::new(read, write, Framing::WebSocket);
        fake.accept_upgrade().await;
        fake.play("initialize.jsonl").await;
        fake.play("steer.jsonl").await;
        fake
    });
    let reports = Reports::default();
    let (client, _events) = Client::connect(Endpoint::Socket(path), options(&reports))
        .await
        .unwrap();
    let steered = client
        .request::<requests::TurnSteer>(TurnSteerParams {
            thread_id: THREAD.into(),
            expected_turn_id: "turn-1".into(),
            input: vec![UserInput::Text(TextUserInput {
                text: "[Hangar] From agent \"Leo\": also run the linter.".into(),
                text_elements: None,
            })],
            client_user_message_id: None,
        })
        .await
        .unwrap();
    assert_eq!(steered.turn_id, "turn-1");
    client
        .request::<requests::TurnInterrupt>(TurnInterruptParams {
            thread_id: THREAD.into(),
            turn_id: "turn-1".into(),
        })
        .await
        .unwrap();
    let refused = client
        .request::<requests::TurnSteer>(TurnSteerParams {
            thread_id: THREAD.into(),
            expected_turn_id: "turn-1".into(),
            input: vec![UserInput::Text(TextUserInput {
                text: "And the formatter.".into(),
                text_elements: None,
            })],
            client_user_message_id: None,
        })
        .await;
    assert!(matches!(refused, Err(Error::Rpc { .. })), "{refused:?}");
    let mut fake = finished(server).await;
    client.shutdown().await;
    assert_eq!(fake.recv_frame().await, Received::Closed);
}

#[cfg(unix)]
#[tokio::test]
async fn a_socket_nobody_listens_on_is_a_start_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("gone.sock");
    let error = Client::connect(Endpoint::Socket(missing), options(&Reports::default()))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Start(_)), "{error:?}");
    assert!(error.to_string().contains("gone.sock"), "{error}");
}

/// A shell script standing in for `codex app-server --listen stdio://`: it answers the handshake
/// with initialize.jsonl's answer, then runs `rest`.
#[cfg(unix)]
fn fake_app_server(rest: &str) -> tokio::process::Command {
    let answer = codex_messages("initialize.jsonl")[0].to_string();
    assert!(!answer.contains('\''), "quoted for sh as it is");
    let script =
        format!("read -r initialize; printf '%s\\n' '{answer}'; read -r initialized; {rest}");
    let mut command = tokio::process::Command::new("sh");
    command.arg("-c").arg(script);
    command
}

#[cfg(unix)]
#[tokio::test]
async fn a_spawned_server_stops_by_itself_when_its_input_ends() {
    // It answers one request, then waits for the end of its input, as Codex does.
    let command = fake_app_server(
        r#"read -r request; printf '%s\n' '{"id":1,"result":{"data":["t"]}}'; cat > /dev/null; exit 0"#,
    );
    let reports = Reports::default();
    let (client, mut events) = Client::connect(Endpoint::Stdio(command), options(&reports))
        .await
        .unwrap();
    let loaded = client
        .request::<requests::ThreadLoadedList>(ThreadLoadedListParams::default())
        .await
        .unwrap();
    assert_eq!(loaded.data, ["t"]);
    let status = client.shutdown().await.expect("a spawned server's status");
    assert!(status.success(), "it went by itself: {status:?}");
    match next(&mut events).await {
        Event::Closed(closed) => assert_eq!(closed.cause, Cause::Shutdown),
        other => panic!("{other:?}"),
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_spawned_server_that_stays_is_killed_after_the_grace() {
    // It ignores the end of its input.
    let command = fake_app_server("exec sleep 30");
    let reports = Reports::default();
    let (client, _events) = Client::connect(Endpoint::Stdio(command), options(&reports))
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let status = client.shutdown().await.expect("a spawned server's status");
    assert!(!status.success(), "killed: {status:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[cfg(unix)]
#[tokio::test]
async fn a_server_that_dies_at_start_says_why() {
    let mut command = tokio::process::Command::new("sh");
    command
        .arg("-c")
        .arg("echo \"error: unexpected argument '--listen' found\" >&2; exit 2");
    let error = Client::connect(Endpoint::Stdio(command), options(&Reports::default()))
        .await
        .unwrap_err();
    let text = error.to_string();
    assert!(matches!(error, Error::Closed(_)), "{error:?}");
    assert_eq!(
        text,
        "Codex's app-server closed the connection (it said: error: unexpected argument \
         '--listen' found)"
    );
}

#[tokio::test]
async fn a_command_that_does_not_exist_is_a_start_error() {
    let command = super::client::app_server_command("/nonexistent/hangar-test/codex");
    let error = Client::connect(Endpoint::Stdio(command), options(&Reports::default()))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Start(_)), "{error:?}");
    assert!(
        error.to_string().contains("/nonexistent/hangar-test/codex"),
        "{error}"
    );
}

#[test]
fn the_commands_are_codex_own() {
    let stdio = super::client::app_server_command("codex");
    let args: Vec<_> = stdio.as_std().get_args().collect();
    assert_eq!(args, ["app-server", "--listen", "stdio://"]);
    let proxy = super::client::proxy_command("codex", std::path::Path::new("/run/hangar/p.sock"));
    let args: Vec<_> = proxy.as_std().get_args().collect();
    assert_eq!(
        args,
        ["app-server", "proxy", "--sock", "/run/hangar/p.sock"]
    );
}
