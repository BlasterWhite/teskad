//! `state.json` read, hydrated and written back (inventory §1.4), against
//! made-up files shaped like real ones: one as the 1.12 app writes it, one
//! hand-edited and partly written by a newer version, one of the old app with
//! the agents it ran, and one with the fields Codex brought.

use serde_json::{Value, json};

use super::types::{
    AgentId, AgentKind, CodexModels, OrchestrationRole, PaneRole, SplitDir, SplitNode,
};
use super::{
    Patch, Settings, Store, StoreAction, Workspace, layout, read_checked, read_state_keeping,
};

const TYPICAL: &str = include_str!("fixtures/typical.json");
const HAND_EDITED: &str = include_str!("fixtures/hand_edited.json");
/// A file of the old app with a Codex, a Gemini CLI and an OpenCode pane,
/// and a launch command for each agent.
const RETIRED_AGENTS: &str = include_str!("fixtures/retired_agents.json");
/// A file with Codex panes, accounts, agents and models (docs/native/agents.md
/// T13), and a few of them this version cannot read: an agent it does not
/// know, a model that is not text, explicit `null`s.
const CODEX: &str = include_str!("fixtures/codex.json");

fn load(file: &str) -> (Store, Value) {
    let raw: Value = serde_json::from_str(file).unwrap();
    let (state, kept) = read_state_keeping(raw.clone());
    let mut model = Store::default();
    model.hydrate_read(state, kept);
    (model, raw)
}

/// The keys of 1.x this version reads as nothing and leaves out on save:
/// its limits on the workers (`OrchestrationSettings`).
const LIMITS_OF_1X: [&str; 2] = ["maxWorkers", "maxAttempts"];

/// The file as this version writes it back: as it was, but for the limits.
fn without_the_limits(mut raw: Value) -> Value {
    for ws in raw["workspaces"].as_array_mut().into_iter().flatten() {
        if let Some(orchestration) = ws.get_mut("orchestration").and_then(Value::as_object_mut) {
            for key in LIMITS_OF_1X {
                orchestration.remove(key);
            }
        }
    }
    raw
}

/// JSON equality as JavaScript reads it: `14` and `14.0` are one number.
fn same_meaning(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_meaning(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, value)| b.get(key).is_some_and(|other| same_meaning(value, other)))
        }
        _ => a == b,
    }
}

/// The first difference between two documents, for a readable failure.
fn difference(a: &Value, b: &Value, at: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for key in x.keys().chain(y.keys()) {
                let path = format!("{at}.{key}");
                match (x.get(key), y.get(key)) {
                    (Some(p), Some(q)) => {
                        if let Some(found) = difference(p, q, &path) {
                            return Some(found);
                        }
                    }
                    (p, q) => return Some(format!("{path}: {p:?} vs {q:?}")),
                }
            }
            None
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
            .iter()
            .zip(y)
            .enumerate()
            .find_map(|(i, (p, q))| difference(p, q, &format!("{at}[{i}]"))),
        _ if same_meaning(a, b) => None,
        _ => Some(format!("{at}: {a} vs {b}")),
    }
}

#[test]
fn a_file_the_app_wrote_comes_back_as_it_was() {
    let (model, raw) = load(TYPICAL);
    let written = model.to_json();
    assert_eq!(difference(&written, &without_the_limits(raw), ""), None);

    let state = model.state();
    assert_eq!(state.workspaces.len(), 3);
    let hangar = &state.workspaces[0];
    assert_eq!(hangar.panes[1].role, Some(PaneRole::Worker));
    assert_eq!(hangar.panes[1].port_base, Some(4100));
    assert_eq!(hangar.claude_account.as_deref(), Some("travail"));
    assert_eq!(state.workspaces[1].detached, Some(true));
    assert_eq!(state.workspaces[2].tree, None);
    assert_eq!(state.settings.locale, Some(hangar_i18n::Locale::Fr));
    assert_eq!(state.settings.keybindings["view.stats"], json!([]));
}

/// Hangar runs Claude Code and Codex: the Codex pane of the old app is Codex
/// again, its session and model kept, and the panes of the agents it dropped
/// open as shells where they were. The first save writes them so, with the
/// launch commands of Claude Code and Codex and no other.
#[test]
fn the_panes_of_the_retired_agents_open_as_shells_and_are_saved_as_shells() {
    let (model, raw) = load(RETIRED_AGENTS);
    let ws = &model.state().workspaces[0];
    let panes: Vec<(&str, AgentId, Option<&str>, Option<&str>)> = ws
        .panes
        .iter()
        .map(|pane| {
            (
                pane.name.as_str(),
                pane.agent,
                pane.cwd.as_deref(),
                pane.retired_agent,
            )
        })
        .collect();
    assert_eq!(
        panes,
        [
            ("Ava", AgentId::Claude, None, None),
            ("Bob", AgentId::Codex, Some("/home/dev/code/shop/api"), None),
            ("Cleo", AgentId::Shell, None, Some("Gemini CLI")),
            (
                "Dan",
                AgentId::Shell,
                Some("/home/dev/code/shop/web"),
                Some("OpenCode")
            ),
        ]
    );
    // Each keeps its place in the layout, and what was not its agent's.
    let ids: Vec<String> = ws.panes.iter().map(|pane| pane.id.clone()).collect();
    assert_eq!(layout::leaf_ids(ws.tree.as_ref().unwrap()), ids);
    assert_eq!(ws.panes[1].shell_id.as_deref(), Some("zsh"));
    assert_eq!(ws.panes[1].title.as_deref(), Some("api"));
    assert_eq!(
        (
            ws.panes[1].session_id.as_deref(),
            ws.panes[1].model.as_deref()
        ),
        (
            Some("rollout-2026-10-30T09-12-44-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b"),
            Some("gpt-5.5")
        )
    );
    assert_eq!(
        (
            ws.panes[2].session_id.as_deref(),
            ws.panes[2].model.as_deref()
        ),
        (None, None)
    );
    let commands = &model.settings().agent_commands;
    assert_eq!(commands.of(AgentKind::Claude), Some("claude --verbose"));
    assert_eq!(commands.of(AgentKind::Codex), Some("codex --full-auto"));

    let written = model.to_json();
    let agents: Vec<&str> = written["workspaces"][0]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pane| pane["agent"].as_str().unwrap())
        .collect();
    assert_eq!(agents, ["claude", "codex", "shell", "shell"]);
    assert_eq!(
        written["settings"]["agentCommands"],
        json!({"claude": "claude --verbose", "codex": "codex --full-auto"})
    );
    assert_eq!(
        written["workspaces"][0]["panes"][1], raw["workspaces"][0]["panes"][1],
        "the Codex pane is written as it was"
    );
    // Nothing else moved.
    assert_eq!(
        written["workspaces"][0]["tree"],
        raw["workspaces"][0]["tree"]
    );
    assert_eq!(
        written["workspaces"][0]["panes"][3]["cwd"],
        raw["workspaces"][0]["panes"][3]["cwd"]
    );
    // And read back, a shell is a shell, with nothing left to say about it.
    let (again, _) = read_state_keeping(written);
    assert!(
        again.workspaces[0]
            .panes
            .iter()
            .all(|pane| pane.retired_agent.is_none())
    );
}

/// The Hangar account is gone. A file that holds settings of it, in the
/// settings or beside them, loads like any other: nothing is refused or
/// dropped around them, and they change nothing this version reads. As any
/// key it does not know, they are written back as they were.
///
/// The account kept its own preference outside `state.json`
/// (`hangar-account.json`), so these key names stand for any it could hold.
#[test]
fn a_file_with_settings_of_the_removed_hangar_account_loads_and_ignores_them() {
    let base: Value = serde_json::from_str(TYPICAL).unwrap();
    let mut raw = base.clone();
    let account = json!({"sync": true, "provider": "github", "lastSync": 1794000000000_i64});
    raw["hangarAccount"] = account.clone();
    raw["settings"]["hangarAccount"] = account.clone();
    raw["settings"]["hangarAccountSync"] = json!(true);

    let (without, kept_without) = read_checked(base).unwrap();
    let (mut with, kept_with) = read_checked(raw.clone()).unwrap();
    assert_eq!(kept_with.len(), kept_without.len());

    // Held only among the unknown keys; the rest of the state is the same.
    assert_eq!(with.extra.remove("hangarAccount"), Some(account.clone()));
    assert_eq!(with.settings.extra.remove("hangarAccount"), Some(account));
    assert_eq!(
        with.settings.extra.remove("hangarAccountSync"),
        Some(json!(true))
    );
    assert_eq!(with, without);

    let (model, _) = load(&raw.to_string());
    assert_eq!(
        difference(&model.to_json(), &without_the_limits(raw), ""),
        None
    );
}

/// A file with everything Codex brought is written back as it was, what this
/// version cannot read of it included, and reads as the person set it.
#[test]
fn a_file_with_codex_in_it_comes_back_as_it_was() {
    let (model, raw) = load(CODEX);
    assert_eq!(difference(&model.to_json(), &raw, ""), None);

    let state = model.state();
    let shop = &state.workspaces[0];
    let bob = &shop.panes[1];
    assert_eq!(bob.agent, AgentId::Codex);
    assert_eq!(bob.agent.kind(), Some(AgentKind::Codex));
    assert_eq!(bob.retired_agent, None);
    assert_eq!(bob.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(bob.account(AgentKind::Codex), Some("team"));
    assert_eq!(bob.account(AgentKind::Claude), None);
    assert_eq!(shop.panes[0].account(AgentKind::Claude), Some("pro"));
    assert_eq!(shop.account(AgentKind::Claude), Some("@default"));
    assert_eq!(shop.account(AgentKind::Codex), Some("team"));
    assert_eq!(state.folders[0].account(AgentKind::Codex), Some("perso"));

    let orchestration = shop.orchestration.as_ref().unwrap();
    assert_eq!(
        [
            OrchestrationRole::Orchestrator,
            OrchestrationRole::Worker,
            OrchestrationRole::Reviewer
        ]
        .map(|role| orchestration.agent(role)),
        [
            Some(AgentKind::Claude),
            Some(AgentKind::Codex),
            Some(AgentKind::Codex)
        ]
    );
    let worker = OrchestrationRole::Worker;
    assert_eq!(
        orchestration.model(worker, AgentKind::Codex),
        Some("gpt-5.5")
    );
    assert_eq!(
        orchestration.model(worker, AgentKind::Claude),
        Some("sonnet")
    );
    assert_eq!(
        orchestration.model(OrchestrationRole::Orchestrator, AgentKind::Codex),
        None,
        "Codex's own default"
    );
    assert_eq!(orchestration.codex_models.extra["planner"], json!("o9"));

    let settings = &state.settings;
    assert!(!settings.codex_attach);
    assert_eq!(settings.account(AgentKind::Codex), Some("team"));
    assert_eq!(settings.account(AgentKind::Claude), Some("main"));
    assert_eq!(settings.default_orchestrator_agent, Some(AgentKind::Codex));
    assert_eq!(settings.default_worker_agent, Some(AgentKind::Claude));
    assert_eq!(
        settings.agent_commands.of(AgentKind::Codex),
        Some("codex --search")
    );

    // What it cannot read reads as unset, and stays in the file.
    let lab = &state.workspaces[1];
    assert_eq!(lab.panes[0].agent, AgentId::Codex);
    assert_eq!(lab.panes[0].codex_account, None);
    let orchestration = lab.orchestration.as_ref().unwrap();
    assert_eq!(orchestration.orchestrator_agent, None);
    assert_eq!(orchestration.worker_agent, None);
    assert_eq!(orchestration.codex_models, CodexModels::default());
    assert_eq!(settings.default_reviewer_agent, None);
}

/// A value this version could not read stays until the app writes that
/// field: then the app's value is the one written.
#[test]
fn an_agent_this_version_does_not_know_is_kept_until_one_is_chosen() {
    let (mut model, raw) = load(CODEX);
    assert_eq!(raw["settings"]["defaultReviewerAgent"], json!("aider"));
    model.apply(StoreAction::UpdateSettings(Patch::new(
        |s: &mut Settings| s.default_reviewer_agent = Some(AgentKind::Codex),
    )));
    let lab = raw["workspaces"][1]["id"].as_str().unwrap().to_string();
    model.apply(StoreAction::UpdateWorkspace(
        lab,
        Patch::new(|ws: &mut Workspace| {
            if let Some(orchestration) = &mut ws.orchestration {
                orchestration.orchestrator_agent = Some(AgentKind::Claude);
            }
        }),
    ));
    let written = model.to_json();
    assert_eq!(written["settings"]["defaultReviewerAgent"], json!("codex"));
    let orchestration = &written["workspaces"][1]["orchestration"];
    assert_eq!(orchestration["orchestratorAgent"], json!("claude"));
    // The rest of it is as it was.
    assert_eq!(orchestration["codexModels"], json!({"worker": 5}));
    assert_eq!(orchestration["workerAgent"], Value::Null);
}

#[test]
fn a_file_the_app_wrote_survives_a_change_with_only_that_change() {
    let (mut model, raw) = load(TYPICAL);
    model.apply(StoreAction::UpdateSettings(Patch::new(
        |s: &mut Settings| s.font_size = 15.0,
    )));
    let mut raw = without_the_limits(raw);
    raw["settings"]["fontSize"] = json!(15);
    assert_eq!(difference(&model.to_json(), &raw, ""), None);
}

#[test]
fn the_chat_view_switch_writes_only_its_flag_and_leaves_no_trace_when_off() {
    let (mut model, raw) = load(TYPICAL);
    let id = model.state().workspaces[0].id.clone();
    let raw = without_the_limits(raw);
    model.apply(StoreAction::SetChatView {
        ws: id.clone(),
        on: true,
    });
    let mut on = raw.clone();
    on["workspaces"][0]["chatView"] = json!(true);
    assert_eq!(difference(&model.to_json(), &on, ""), None);
    model.apply(StoreAction::SetChatView { ws: id, on: false });
    assert_eq!(difference(&model.to_json(), &raw, ""), None);
}

/// Hangar sets no limit on the workers any more (owner's decision,
/// 2026-11-04): a file of 1.x that holds `maxWorkers` and `maxAttempts` loads
/// with every other orchestration setting it holds, and the first save leaves
/// the two out, so 1.x reads its own defaults for them.
#[test]
fn the_limits_1x_kept_load_as_nothing_and_are_left_out_on_save() {
    let (model, raw) = load(TYPICAL);
    let read = model.state().workspaces[0].orchestration.clone().unwrap();
    assert!(!read.auto_merge);
    assert_eq!(read.check_command.as_deref(), Some("cargo test"));
    assert_eq!(read.branch.as_deref(), Some("develop"));
    assert_eq!(read.permission_mode.as_deref(), Some("acceptEdits"));

    let written = model.to_json();
    let saved = written["workspaces"][0]["orchestration"]
        .as_object()
        .unwrap();
    let before = raw["workspaces"][0]["orchestration"].as_object().unwrap();
    for key in LIMITS_OF_1X {
        assert!(before.contains_key(key), "{key}");
        assert!(!saved.contains_key(key), "{key}");
    }
    assert_eq!(saved.len(), before.len() - LIMITS_OF_1X.len());
    // A hand-written one that never read goes too, rather than coming back.
    let (model, raw) = load(HAND_EDITED);
    assert_eq!(
        raw["workspaces"][1]["orchestration"]["maxWorkers"],
        json!("many")
    );
    let saved = &model.to_json()["workspaces"][1]["orchestration"];
    assert_eq!(saved["autoMerge"], json!(false));
    assert!(saved.get("maxWorkers").is_none(), "{saved}");
}

#[test]
fn a_hand_edited_file_loads_whole() {
    let (model, _) = load(HAND_EDITED);
    let state = model.state();
    // The string among the workspaces is not one; both others are there.
    assert_eq!(state.workspaces.len(), 2);
    let api = &state.workspaces[0];
    // Its pane that is not an object is not a pane; the others are.
    assert_eq!(api.panes.len(), 2);
    assert_eq!(api.panes[0].role, None, "an unknown role reads as none");
    assert_eq!(api.panes[0].port_base, None);
    assert_eq!(
        api.panes[1].agent,
        AgentId::Shell,
        "an unknown agent reads as the shell"
    );
    assert_eq!(api.detached, None);
    assert_eq!(api.window_bounds, None);
    // A mode it does not know reads as inheriting, as `workspaceMcp` read it.
    assert_eq!(
        api.mcp.as_ref().map(|mcp| mcp.mode),
        Some(super::types::McpMode::Inherit)
    );
    // Hydration: unique names, and a folder that is gone is the top level.
    let names: Vec<&str> = state
        .workspaces
        .iter()
        .flat_map(|ws: &Workspace| &ws.panes)
        .map(|pane| pane.name.as_str())
        .collect();
    assert_eq!(names, ["Ava", "Max", "Mia", "Leo"]);
    assert_eq!(api.folder_id, None);
    // The legacy tree, by pane index, reads as ids.
    let legacy = &state.workspaces[1];
    assert_eq!(
        legacy.tree,
        Some(SplitNode::Split {
            dir: SplitDir::Col,
            ratio: 0.7,
            a: Box::new(layout::leaf("p-two")),
            b: Box::new(layout::leaf("p-one")),
        })
    );
    assert!(!legacy.orchestration.as_ref().unwrap().auto_merge);
    assert_eq!(state.active_workspace_id, None);
    let settings = &state.settings;
    assert_eq!(settings.font_size, 13.0);
    assert_eq!(settings.locale, None);
    assert_eq!(
        settings.scale_factor(),
        1.3,
        "a valid number, clamped where read"
    );
    assert_eq!(settings.saved_commands.len(), 1);
    assert!(!settings.saved_commands[0].auto_run);
    assert!(!state.folders[0].collapsed);
}

#[test]
fn what_could_not_be_read_is_written_back_as_it_was() {
    let (model, raw) = load(HAND_EDITED);
    let written = model.to_json();
    let api = &written["workspaces"][0];
    let raw_api = &raw["workspaces"][0];
    for key in [
        "detached",
        "windowBounds",
        "mcp",
        "claudeAccount",
        "futureWorkspaceField",
    ] {
        assert_eq!(api[key], raw_api[key], "workspace.{key}");
    }
    let ava = &api["panes"][0];
    for key in ["role", "taskId", "portBase", "futurePaneField"] {
        assert_eq!(ava[key], raw_api["panes"][0][key], "pane.{key}");
    }
    assert!(
        ava.get("taskId").is_some_and(Value::is_null),
        "an explicit null stays"
    );
    assert_eq!(api["panes"][1]["agent"], json!("aider"));
    assert_eq!(api["panes"][2], json!(3), "kept, at the end of its list");
    assert_eq!(written["workspaces"][2], json!("not a workspace"));
    assert_eq!(written["folders"][0]["collapsed"], json!("yes"));
    assert_eq!(written["activeWorkspaceId"], json!(42));
    assert_eq!(written["version"], json!(3));
    let settings = &written["settings"];
    for key in [
        "locale",
        "fontSize",
        "cursorStyle",
        "experiments",
        "futureSetting",
        "keybindings",
    ] {
        assert_eq!(settings[key], raw["settings"][key], "settings.{key}");
    }
    assert_eq!(settings["uiScale"], json!(300.0));
    assert_eq!(settings["savedCommands"][0]["autoRun"], json!("sometimes"));
    assert_eq!(settings["savedCommands"][1], json!("junk"));

    // And it reads back the same.
    let (again, kept) = read_state_keeping(written);
    let mut reread = Store::default();
    reread.hydrate_read(again, kept);
    assert_eq!(reread.state(), model.state());
    assert_eq!(reread.to_json(), model.to_json());
}

#[test]
fn a_field_the_app_writes_is_the_app_s_from_then_on() {
    let (mut model, raw) = load(HAND_EDITED);
    model.apply(StoreAction::UpdateSettings(Patch::new(
        |s: &mut Settings| s.font_size = 16.0,
    )));
    model.apply(StoreAction::UpdateWorkspace(
        "w-api".into(),
        Patch::new(|ws: &mut Workspace| ws.detached = Some(true)),
    ));
    let written = model.to_json();
    assert_eq!(written["settings"]["fontSize"], json!(16.0));
    assert_eq!(written["workspaces"][0]["detached"], json!(true));
    // The rest is still as it was.
    assert_eq!(
        written["settings"]["cursorStyle"],
        raw["settings"]["cursorStyle"]
    );
    assert_eq!(
        written["workspaces"][0]["windowBounds"],
        json!("left screen")
    );

    // Saved, then back to the default, the field stays the app's: no "big"
    // again. The save is what forgets the raw value for good.
    assert_eq!(model.take_save().into_json(), written);
    model.apply(StoreAction::UpdateSettings(Patch::new(
        |s: &mut Settings| s.font_size = 13.0,
    )));
    assert_eq!(model.to_json()["settings"]["fontSize"], json!(13.0));
    assert_eq!(
        model.take_save().into_json()["settings"]["fontSize"],
        json!(13.0)
    );
}

#[test]
fn what_a_workspace_held_goes_with_it() {
    let (mut model, _) = load(HAND_EDITED);
    model.apply(StoreAction::RemoveWorkspace("w-api".into()));
    let written = model.to_json();
    let workspaces = written["workspaces"].as_array().unwrap();
    assert_eq!(
        workspaces.len(),
        2,
        "the legacy one, and the string kept at the top level"
    );
    assert_eq!(workspaces[0]["id"], json!("w-legacy"));
    assert!(!written.to_string().contains("left screen"));
}

#[test]
fn a_respawned_pane_keeps_what_it_held() {
    let (mut model, _) = load(HAND_EDITED);
    let new = model
        .apply(StoreAction::RespawnPane {
            ws: "w-api".into(),
            pane: "p-ava".into(),
            patch: Patch::none(),
        })
        .pane
        .unwrap();
    let written = model.to_json();
    let pane = &written["workspaces"][0]["panes"][0];
    assert_eq!(pane["id"], json!(new));
    assert_eq!(pane["role"], json!("reviewer"));
    assert_eq!(pane["portBase"], json!("4100"));
}
