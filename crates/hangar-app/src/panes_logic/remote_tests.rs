//! An agent pane of a workspace on a node, against a fake node: its session
//! hunt and its context gauge read the node's transcripts, of its agent.

use hangar_core::agent::AgentKind;
use iced::futures::StreamExt as _;
use iced_runtime::Action;
use tokio::net::TcpListener;

use super::*;
use crate::nodes::remote;
use crate::nodes::testing::{answer, listen, read_request};

const NOW: EpochMs = 5_000_000;

/// The messages a task gives, run to its end.
async fn outputs(task: Task<Message>) -> Vec<Message> {
    let Some(stream) = iced_runtime::task::into_stream(task) else {
        return Vec::new();
    };
    stream
        .filter_map(|action| async move {
            match action {
                Action::Output(message) => Some(message),
                _ => None,
            }
        })
        .collect()
        .await
}

/// A fake node answering each request with the next body, in order; the
/// request lines it was asked, once they are all answered.
fn serve(listener: TcpListener, bodies: Vec<String>) -> tokio::task::JoinHandle<Vec<String>> {
    tokio::spawn(async move {
        let mut lines = Vec::new();
        for body in bodies {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await;
            lines.push(request.line().to_owned());
            answer(&mut stream, 200, &body).await;
        }
        lines
    })
}

/// A model with one workspace on node `node`, its Claude Code pane mounted,
/// on screen and running.
fn remote_model(node: &str) -> Model {
    model_on(Some(node))
}

fn model_on(node: Option<&str>) -> Model {
    model_running(node, AgentId::Claude)
}

/// [`model_on`], its pane running `agent`.
fn model_running(node: Option<&str>, agent: AgentId) -> Model {
    let mut m = Model::default();
    m.hydrate(AppState {
        workspaces: vec![Workspace {
            id: "w1".into(),
            name: "remote".into(),
            cwd: "/home/hangar/demo".into(),
            node: node.map(str::to_owned),
            panes: vec![Pane {
                id: "p1".into(),
                name: "Ava".into(),
                agent,
                session_id: Some("s-old".into()),
                ..Pane::default()
            }],
            ..Workspace::default()
        }],
        active_workspace_id: Some("w1".into()),
        ..AppState::default()
    });
    m.live
        .windows
        .insert(window::Id::unique(), WindowKind::Main);
    m.live.panes_logic.mount_pane("p1", "w1", "Ava", NOW);
    m.live
        .panes_logic
        .status
        .insert("p1".into(), PaneStatus::Running);
    m
}

#[tokio::test]
async fn a_remote_launch_starts_the_hunt_with_what_the_node_holds() {
    let (listener, client) = listen().await;
    let node = "node-hunt-start";
    remote::publish(node, Some(client));
    let fake = serve(
        listener,
        vec![r#"{"sessions":[{"id":"s-before","label":"","modifiedMs":1}]}"#.into()],
    );
    let m = remote_model(node);
    let messages = outputs(watch_remote(&m, "p1", Some("s-old"))).await;
    let [
        Message::PanesLogic(Msg::WatchSession {
            pane, cwd, known, ..
        }),
    ] = &messages[..]
    else {
        panic!("one WatchSession, got {messages:?}");
    };
    assert_eq!(pane, "p1");
    assert_eq!(cwd, "/home/hangar/demo");
    let mut known: Vec<&str> = known.iter().map(String::as_str).collect();
    known.sort_unstable();
    assert_eq!(
        known,
        ["s-before", "s-old"],
        "the node's, and the one resumed"
    );
    assert_eq!(
        fake.await.unwrap(),
        ["GET /node/sessions?cwd=%2Fhome%2Fhangar%2Fdemo HTTP/1.1"]
    );
    remote::publish(node, None);
}

/// A Codex pane of a remote workspace hunts its own agent's sessions on the
/// node: Codex's rollouts, asked by kind.
#[tokio::test]
async fn a_remote_codex_launch_asks_the_node_for_codex_sessions() {
    let (listener, client) = listen().await;
    let node = "node-hunt-codex";
    remote::publish(node, Some(client));
    let fake = serve(
        listener,
        vec![r#"{"sessions":[{"id":"0199a5c4","label":"","modifiedMs":1}]}"#.into()],
    );
    let m = model_running(Some(node), AgentId::Codex);
    let messages = outputs(watch_remote(&m, "p1", None)).await;
    let [Message::PanesLogic(Msg::WatchSession { agent, known, .. })] = &messages[..] else {
        panic!("one WatchSession, got {messages:?}");
    };
    assert_eq!(*agent, AgentId::Codex);
    assert!(known.contains("0199a5c4"), "{known:?}");
    assert_eq!(
        fake.await.unwrap(),
        ["GET /node/sessions?cwd=%2Fhome%2Fhangar%2Fdemo&kind=codex HTTP/1.1"]
    );
    remote::publish(node, None);
}

#[test]
fn a_local_launch_is_left_to_the_controller() {
    let m = model_on(None);
    assert_eq!(watch_remote(&m, "p1", None).units(), 0);
}

#[tokio::test]
async fn the_hunt_and_the_gauge_of_a_remote_pane_ask_its_node() {
    let (listener, client) = listen().await;
    let node = "node-hunt-round";
    remote::publish(node, Some(client));
    let mut m = remote_model(node);
    // A hunt due, the pane having printed since.
    let mut watch = SessionWatch::for_agent(
        AgentId::Claude,
        "/home/hangar/demo".into(),
        HashSet::new(),
        0,
    )
    .expect("Claude Code resumes its sessions");
    watch.note_output(0);
    m.live.panes_logic.watches.insert("p1".into(), watch);
    let mut st = State {
        // The branches are not this test's.
        last_branch_poll: NOW,
        ..State::default()
    };
    // The node's clock is 60 s behind this one: the transcript it touched a
    // second ago is fresh here too.
    let node_now = NOW - 60_000;
    let fake = serve(
        listener,
        vec![
            format!(
                r#"{{"sessions":[{{"id":"s-new","label":"","modifiedMs":{}}}],"now":{node_now}}}"#,
                node_now - 1000
            ),
            r#"{"usage":{"usedTokens":50000,"model":"claude-opus-5-5"}}"#.into(),
        ],
    );
    let task = slow(&mut st, &mut m, NOW);
    let messages = outputs(task).await;
    let mut lines = fake.await.unwrap();
    lines.sort();
    assert_eq!(
        lines,
        [
            "GET /node/context?session=s-old&cwd=%2Fhome%2Fhangar%2Fdemo HTTP/1.1",
            "GET /node/sessions?cwd=%2Fhome%2Fhangar%2Fdemo HTTP/1.1",
        ]
    );
    let sessions = messages.iter().find_map(|message| match message {
        Message::PanesLogic(Msg::Sessions { sessions, .. }) => Some(sessions.clone()),
        _ => None,
    });
    let sessions = sessions.flatten().expect("the node's listing");
    assert_eq!(sessions[0].id, "s-new");
    assert!(
        NOW.saturating_sub(sessions[0].modified_ms) < session_watch::FRESH_MS,
        "read on this clock"
    );
    let usage = messages.iter().find_map(|message| match message {
        Message::PanesLogic(Msg::Context { usage, .. }) => Some(usage.clone()),
        _ => None,
    });
    assert_eq!(
        usage.flatten().map(|reading| reading.usage.used_tokens),
        Some(50000)
    );
    remote::publish(node, None);
}

/// A Codex pane of a remote workspace has its context read on the node as
/// Codex's, from its rollout. The node's answer names no window: the gauge
/// knows none for a Codex model, and shows nothing rather than a guess.
#[tokio::test]
async fn the_gauge_of_a_remote_codex_pane_asks_its_node_for_codex_s() {
    let (listener, client) = listen().await;
    let node = "node-gauge-codex";
    remote::publish(node, Some(client));
    let mut m = model_running(Some(node), AgentId::Codex);
    let mut st = State {
        last_branch_poll: NOW,
        ..State::default()
    };
    let fake = serve(
        listener,
        vec![
            r#"{"usage":{"usedTokens":68000,"model":"gpt-5.5-codex","contextWindow":272000}}"#
                .into(),
        ],
    );
    let messages = outputs(slow(&mut st, &mut m, NOW)).await;
    assert_eq!(
        fake.await.unwrap(),
        ["GET /node/context?session=s-old&cwd=%2Fhome%2Fhangar%2Fdemo&kind=codex HTTP/1.1"]
    );
    let Some((kind, usage)) = messages.iter().find_map(|message| match message {
        Message::PanesLogic(Msg::Context { kind, usage, .. }) => Some((*kind, usage.clone())),
        _ => None,
    }) else {
        panic!("a Context, got {messages:?}");
    };
    assert_eq!(kind, AgentKind::Codex);
    let usage = usage.expect("the node's reading");
    assert_eq!(usage.usage.used_tokens, 68000);
    context_read(&mut m, "p1", kind, Some(usage));
    assert_eq!(m.live.panes_logic.insight("p1").context, None);
    remote::publish(node, None);
}

#[tokio::test]
async fn a_node_away_keeps_the_gauge_and_finds_no_session() {
    let node = "node-away";
    remote::publish(node, None);
    assert!(
        remote_transcripts::hunt_listing(node.into(), AgentKind::Claude, "/w".into())
            .await
            .is_none()
    );
    assert!(
        remote_transcripts::context_usage(node.into(), AgentKind::Claude, "s".into(), "/w".into())
            .await
            .is_err()
    );
    // Its read gives no `Context`: the last reading stands.
    let mut m = remote_model(node);
    let mut st = State {
        last_branch_poll: NOW,
        ..State::default()
    };
    let messages = outputs(slow(&mut st, &mut m, NOW)).await;
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, Message::PanesLogic(Msg::Context { .. }))),
        "{messages:?}"
    );
}
