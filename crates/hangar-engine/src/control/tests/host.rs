//! The pane tools over a host with no window (`EngineHost::answer`, through
//! its `HostPort`), down to the launch its terminals are handed: a Codex pane
//! that an agent creates, restarts or queues for starts through core's
//! builder, attached to its own server when that one listens, on the floor
//! otherwise with why in its status (docs/native/agents.md T14a, T16b).

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hangar_core::agent::{AgentKind, launch};
use hangar_core::bridge::bridge_queue;
use hangar_core::pty::{AgentLaunch, FloorReason, ServerState};
use serde_json::{Value, json};

use super::super::fake::lock;
use super::{contains, matches};
use crate::host::{
    AgentStart, Config, EngineHost, Spawn, TermEvent, TermSink, Terminals, write_state_file,
};
use crate::state::{AgentId, AppState, Pane, PaneId, Workspace};
use crate::testdir::scratch;

/// Shells that start at once and keep each agent launch they were handed,
/// with the line core's builder makes of it; a Codex launch that asks to
/// attach gets `server`, as core's PTY would answer it.
struct Launches {
    server: ServerState,
    launched: Mutex<Vec<(PaneId, AgentLaunch, String)>>,
    alive: Mutex<HashSet<PaneId>>,
}

impl Terminals for Launches {
    fn spawn(&self, spawn: Spawn, sink: TermSink) -> Result<(), String> {
        lock(&self.alive).insert(spawn.id.clone());
        sink(TermEvent::Output(spawn.id));
        Ok(())
    }
    fn spawn_agent(
        &self,
        spawn: Spawn,
        launch: AgentLaunch,
        sink: TermSink,
    ) -> Result<AgentStart, String> {
        let line = launch::build(&launch.request)
            .map_err(|error| error.to_string())?
            .command_line();
        let server =
            (launch.request.kind == AgentKind::Codex && launch.attach).then(|| self.server.clone());
        lock(&self.launched).push((spawn.id.clone(), launch, line.clone()));
        self.spawn(spawn, sink)?;
        Ok(AgentStart { line, server })
    }
    fn write(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn alive(&self, pane: &str) -> bool {
        lock(&self.alive).contains(pane)
    }
    fn kill(&self, pane: &str) -> Result<(), String> {
        lock(&self.alive).remove(pane);
        Ok(())
    }
    fn lines(&self, pane: &str, _: usize) -> Option<Vec<String>> {
        self.alive(pane).then(|| vec!["$ ".to_string()])
    }
    fn screen(&self, pane: &str) -> Option<Vec<String>> {
        self.alive(pane).then(|| vec!["$ ".to_string()])
    }
    fn kill_all(&self) {
        lock(&self.alive).clear();
    }
}

fn pane(id: &str, name: &str, agent: AgentId) -> Pane {
    Pane {
        id: id.into(),
        name: name.into(),
        agent,
        ..Pane::default()
    }
}

/// A started host over "Shop": Ava, the Claude Code pane that calls, and
/// Ada, another; Codex's servers end as `server` says.
fn shop_host(server: ServerState) -> (EngineHost, Arc<Launches>) {
    let root = scratch("control-codex");
    let file = root.join("engine").join("state.json");
    let state = AppState {
        workspaces: vec![Workspace {
            id: "shop".into(),
            name: "Shop".into(),
            cwd: root.to_string_lossy().into_owned(),
            panes: vec![
                pane("p1", "Ava", AgentId::Claude),
                pane("p2", "Ada", AgentId::Claude),
            ],
            ..Workspace::default()
        }],
        // « Attach Teskad to Codex » on, as by default (D5).
        ..AppState::default()
    };
    write_state_file(&file, &serde_json::to_value(&state).unwrap()).unwrap();
    let terminals = Arc::new(Launches {
        server,
        launched: Mutex::default(),
        alive: Mutex::default(),
    });
    let host = EngineHost::load(Config {
        core: hangar_core::Host::with_paths(hangar_core::paths::Paths::rooted(&root)),
        terminals: terminals.clone(),
        state_file: file,
    })
    .unwrap();
    host.start();
    (host, terminals)
}

/// An agent's request, from Ava.
async fn from_ava(host: &EngineHost, op: &str, mut args: Value) -> Value {
    args["caller"] = json!({"paneId": "p1", "paneName": "Ava"});
    host.answer(op, args, false)
        .await
        .unwrap_or_else(|refused| panic!("{op} refused: {refused}"))
}

/// The launch pane `id` was started with, and the line it typed, once it
/// was (five seconds at most).
async fn launched(terminals: &Launches, id: &str) -> (AgentLaunch, String) {
    for _ in 0..500 {
        let found = lock(&terminals.launched)
            .iter()
            .find(|(pane, _, _)| pane == id)
            .map(|(_, launch, line)| (launch.clone(), line.clone()));
        if let Some(found) = found {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("pane {id} was never launched");
}

/// The host's word on pane `id`'s server, once it has one.
async fn server_of(host: &EngineHost, id: &str) -> ServerState {
    for _ in 0..500 {
        if let Some(server) = host.agent_server(id) {
            return server;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("pane {id}'s server was never known");
}

/// What is queued for pane `id`: each text and who it is from.
fn queued(id: &str) -> Vec<(String, Option<String>)> {
    bridge_queue(id.to_string())
        .into_iter()
        .map(|queued| (queued.text, queued.from))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_codex_pane_created_with_a_prompt_starts_through_cores_builder_and_says_why_on_the_floor()
{
    let floor = ServerState::Floor {
        reason: FloorReason::Timeout { seconds: 10 },
    };
    let (host, terminals) = shop_host(floor.clone());
    let answer = from_ava(
        &host,
        "pane_create",
        json!({"agent": "codex", "name": "Cody", "prompt": "Port the parser"}),
    )
    .await;
    matches(
        &answer,
        json!({"workspace": "Shop", "pane": {"name": "Cody", "agent": "codex"}}),
    );
    let id = answer["pane"]["id"].as_str().unwrap().to_string();

    let (launch, line) = launched(&terminals, &id).await;
    assert_eq!(launch.request.kind, AgentKind::Codex);
    assert!(launch.attach, "attached when it can be (D5)");
    assert!(line.starts_with("codex"), "{line}");
    // Its server did not listen: the floor (D6), and the pane says why.
    assert_eq!(server_of(&host, &id).await, floor);
    let note = host.status_note(&id).expect("a status note");
    contains(&note, "Codex runs without Teskad attached");
    // The prompt waits in Hangar's queue for the session, from Ava.
    assert_eq!(
        queued(&id),
        [("Port the parser".to_string(), Some("Ava".to_string()))]
    );
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pane_restarted_on_codex_runs_attached_and_takes_queued_messages() {
    let attached = ServerState::Attached {
        socket: "/run/user/1000/hangar/codex-0123456789ab.sock".into(),
    };
    let (host, terminals) = shop_host(attached.clone());
    let answer = from_ava(
        &host,
        "pane_restart",
        json!({"pane": "Ada", "agent": "codex"}),
    )
    .await;
    matches(
        &answer,
        json!({"pane": {"name": "Ada", "agent": "codex"}, "fresh": true}),
    );
    let id = answer["pane"]["id"].as_str().unwrap().to_string();
    assert_ne!(id, "p2", "a restart is a new pane");

    let (launch, _) = launched(&terminals, &id).await;
    assert_eq!(launch.request.kind, AgentKind::Codex);
    assert_eq!(launch.request.resume, None, "a new agent, a new session");
    assert_eq!(server_of(&host, &id).await, attached);
    assert_eq!(host.status_note(&id), None, "attached: nothing to say");

    let sent = from_ava(
        &host,
        "pane_send",
        json!({"pane": "Ada", "text": "Next: the cart", "when_idle": true}),
    )
    .await;
    assert_eq!(sent, json!({"queued": true, "pane": "Ada"}));
    assert_eq!(
        queued(&id),
        [("Next: the cart".to_string(), Some("Ava".to_string()))]
    );
    // Ava, the Claude Code pane, was started as before.
    let (ava, _) = launched(&terminals, "p1").await;
    assert_eq!(ava.request.kind, AgentKind::Claude);
    assert_eq!(host.agent_server("p1"), None);
    host.shutdown().await;
}
