//! store.test.ts, case by case, then the semantics of each action as
//! store.tsx has them (inventory §1.3), which the TS left to its callers'
//! tests.

use super::*;
use crate::accounts::{AccountLevels, AccountSource, ClaudeAccount, DEFAULT_CHOICE};
use crate::state::layout::{
    Rect, Side, closed_by_preset_in, compute_layout, drawn_tree, leaf_ids, normalize_tree, ratio_at,
};
use crate::state::types::{PanePin, PaneRole, WindowBounds};

fn draft(layout: u8, agents: &[AgentId]) -> WorkspaceDraft {
    WorkspaceDraft {
        name: "w".into(),
        cwd: "/p".into(),
        layout,
        agents: agents.to_vec(),
        ..WorkspaceDraft::default()
    }
}

fn model() -> Store {
    let mut model = Store::default();
    model.hydrate(AppState::default());
    model
}

/// A model holding one workspace of `layout` panes; answers its id.
fn with_workspace(model: &mut Store, layout: u8, agents: &[AgentId]) -> WsId {
    model
        .apply(StoreAction::AddWorkspace(draft(layout, agents)))
        .workspace
        .unwrap()
}

fn ws<'a>(model: &'a Store, id: &str) -> &'a Workspace {
    model.workspace(id).unwrap()
}

fn pane_ids(model: &Store, id: &str) -> Vec<PaneId> {
    ws(model, id).panes.iter().map(|p| p.id.clone()).collect()
}

fn tree_ids(model: &Store, id: &str) -> Vec<PaneId> {
    let w = ws(model, id);
    leaf_ids(&normalize_tree(&w.panes, w.tree.as_ref()))
}

// store.test.ts: makeWorkspace and the Claude account

#[test]
fn keeps_the_account_the_dialog_chose() {
    let mut d = draft(1, &[AgentId::Claude]);
    d.claude_account = Some("travail".into());
    assert_eq!(
        make_workspace(&d, Vec::<String>::new())
            .claude_account
            .as_deref(),
        Some("travail")
    );
}

#[test]
fn leaves_the_field_out_when_the_workspace_follows_the_default() {
    let absent = |account: Option<&str>| {
        let mut d = draft(1, &[AgentId::Claude]);
        d.claude_account = account.map(Into::into);
        let ws = make_workspace(&d, Vec::<String>::new());
        serde_json::to_value(&ws)
            .unwrap()
            .get("claudeAccount")
            .is_none()
    };
    assert!(absent(None));
    assert!(absent(Some("  ")));
    assert!(absent(Some("")));
}

// makeWorkspace

#[test]
fn makes_the_layout_s_panes_with_their_agents_named_against_every_workspace() {
    let ws = make_workspace(
        &draft(3, &[AgentId::Claude, AgentId::Claude]),
        ["Ava", "Leo"],
    );
    let names: Vec<&str> = ws.panes.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Max", "Mia", "Sam"]);
    let agents: Vec<AgentId> = ws.panes.iter().map(|p| p.agent).collect();
    assert_eq!(agents, [AgentId::Claude, AgentId::Claude, AgentId::Shell]);
    assert_eq!(
        ws.tree,
        Some(preset_tree(
            &ws.panes.iter().map(|p| p.id.clone()).collect::<Vec<_>>()
        ))
    );
    assert_eq!(ws.theme_id, None);
    assert!(ws.saved_commands.is_empty());
}

#[test]
fn writes_kits_only_when_some_were_chosen() {
    let mut d = draft(1, &[]);
    d.kits = Some(vec![]);
    let json = serde_json::to_value(make_workspace(&d, Vec::<String>::new())).unwrap();
    assert!(json.get("kits").is_none());
    d.kits = Some(vec![WorkspaceKit {
        id: "k".into(),
        granted: vec![],
    }]);
    assert_eq!(
        make_workspace(&d, Vec::<String>::new()).kits.unwrap().len(),
        1
    );
}

// workspaces

#[test]
fn a_new_workspace_becomes_the_active_one() {
    let mut m = model();
    let first = with_workspace(&mut m, 1, &[]);
    let second = with_workspace(&mut m, 2, &[]);
    assert_eq!(
        m.state().active_workspace_id.as_deref(),
        Some(second.as_str())
    );
    assert_ne!(first, second);
    // Names are unique across workspaces.
    let names: Vec<&str> = m
        .state()
        .workspaces
        .iter()
        .flat_map(|w| &w.panes)
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names, ["Ava", "Max", "Leo"]);
}

#[test]
fn removing_the_active_workspace_activates_the_first_one_left() {
    let mut m = model();
    let a = with_workspace(&mut m, 1, &[]);
    let b = with_workspace(&mut m, 1, &[]);
    let c = with_workspace(&mut m, 1, &[]);
    m.apply(StoreAction::SetActiveWorkspace(Some(b.clone())));
    m.apply(StoreAction::RemoveWorkspace(c.clone()));
    assert_eq!(m.state().active_workspace_id.as_deref(), Some(b.as_str()));
    m.apply(StoreAction::RemoveWorkspace(b));
    assert_eq!(m.state().active_workspace_id.as_deref(), Some(a.as_str()));
    m.apply(StoreAction::RemoveWorkspace(a));
    assert_eq!(m.state().active_workspace_id, None);
}

#[test]
fn an_update_is_a_shallow_merge_and_a_no_op_changes_nothing() {
    let mut m = model();
    let id = with_workspace(&mut m, 1, &[]);
    let revision = m.revision();
    let outcome = m.apply(StoreAction::UpdateWorkspace(
        id.clone(),
        Patch::new(|w: &mut Workspace| {
            w.detached = Some(true);
            w.window_bounds = Some(WindowBounds {
                x: 1.0,
                y: 2.0,
                width: 800.0,
                height: 600.0,
            });
        }),
    ));
    assert!(outcome.changed);
    assert_eq!(m.revision(), revision + 1);
    assert_eq!(ws(&m, &id).detached, Some(true));
    assert_eq!(ws(&m, &id).panes.len(), 1);
    let again = m.apply(StoreAction::UpdateWorkspace(
        id.clone(),
        Patch::new(|w: &mut Workspace| w.detached = Some(true)),
    ));
    assert!(!again.changed);
    assert_eq!(m.revision(), revision + 1);
    assert!(
        !m.apply(StoreAction::UpdateWorkspace("nope".into(), Patch::none()))
            .changed
    );
}

// folders

#[test]
fn folders_are_added_expanded_renamed_moved_and_removed() {
    let mut m = model();
    let a = m.apply(StoreAction::AddFolder("A".into())).folder.unwrap();
    let b = m.apply(StoreAction::AddFolder("B".into())).folder.unwrap();
    assert!(!m.state().folders[0].collapsed);
    m.apply(StoreAction::UpdateFolder(
        a.clone(),
        Patch::new(|f: &mut WorkspaceFolder| {
            f.name = "Work".into();
            f.id = "hijacked".into();
        }),
    ));
    assert_eq!(m.state().folders[0].name, "Work");
    assert_eq!(m.state().folders[0].id, a, "an update never changes the id");
    m.apply(StoreAction::MoveFolder {
        id: b.clone(),
        before: Some(a.clone()),
    });
    let order: Vec<&str> = m.state().folders.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(order, [b.as_str(), a.as_str()]);
    assert!(
        !m.apply(StoreAction::MoveFolder {
            id: b.clone(),
            before: Some(b.clone()),
        })
        .changed
    );

    let w = with_workspace(&mut m, 1, &[]);
    m.apply(StoreAction::MoveWorkspace {
        id: w.clone(),
        folder: Some(a.clone()),
        before: None,
    });
    assert_eq!(ws(&m, &w).folder_id.as_deref(), Some(a.as_str()));
    // Removing a folder sends its workspaces back to the top level.
    m.apply(StoreAction::RemoveFolder(a.clone()));
    assert_eq!(m.state().folders.len(), 1);
    assert_eq!(ws(&m, &w).folder_id, None);
    assert_eq!(m.state().workspaces.len(), 1);
}

#[test]
fn a_workspace_moved_into_a_folder_deleted_meanwhile_lands_at_the_top_level() {
    let mut m = model();
    let a = with_workspace(&mut m, 1, &[]);
    let b = with_workspace(&mut m, 1, &[]);
    m.apply(StoreAction::MoveWorkspace {
        id: b.clone(),
        folder: Some("gone".into()),
        before: Some(a.clone()),
    });
    let order: Vec<&str> = m.state().workspaces.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(order, [b.as_str(), a.as_str()]);
    assert_eq!(ws(&m, &b).folder_id, None);
}

/// A folder « Games » whose account is `account`; answers its id.
fn folder_on(m: &mut Store, account: Option<&str>) -> String {
    let id = m
        .apply(StoreAction::AddFolder("Games".into()))
        .folder
        .unwrap();
    let account = account.map(str::to_string);
    m.apply(StoreAction::UpdateFolder(
        id.clone(),
        Patch::new(move |folder: &mut WorkspaceFolder| folder.claude_account = account.clone()),
    ));
    id
}

/// The account workspace `id` runs on, and the level that decided it, among
/// the accounts « perso » and « pro ».
fn account_of(m: &Store, id: &str) -> (String, AccountSource) {
    let accounts: Vec<ClaudeAccount> = ["main", "perso", "pro"]
        .into_iter()
        .map(|id| ClaudeAccount {
            id: id.into(),
            label: id.into(),
            is_main: id == "main",
            logged_in: true,
            email: None,
            plan: None,
        })
        .collect();
    let resolved = AccountLevels::of(m.state(), ws(m, id), None).resolve(&accounts);
    (resolved.id, resolved.source)
}

fn made_in(m: &mut Store, folder: Option<&str>, account: Option<&str>) -> WsId {
    m.apply(StoreAction::AddWorkspace(WorkspaceDraft {
        folder_id: folder.map(str::to_string),
        claude_account: account.map(str::to_string),
        ..draft(1, &[])
    }))
    .workspace
    .unwrap()
}

fn move_to(m: &mut Store, id: &str, folder: Option<&str>) {
    m.apply(StoreAction::MoveWorkspace {
        id: id.to_string(),
        folder: folder.map(str::to_string),
        before: None,
    });
}

#[test]
fn a_workspace_made_in_a_folder_runs_on_its_account_without_copying_it() {
    let mut m = model();
    let games = folder_on(&mut m, Some("perso"));
    let plain = made_in(&mut m, Some(&games), None);
    assert_eq!(ws(&m, &plain).claude_account, None, "nothing copied");
    assert_eq!(
        account_of(&m, &plain),
        ("perso".into(), AccountSource::Folder)
    );
    let chosen = made_in(&mut m, Some(&games), Some("pro"));
    assert_eq!(
        account_of(&m, &chosen),
        ("pro".into(), AccountSource::Workspace)
    );
    // The folder's account changed: the workspaces that follow it follow.
    m.apply(StoreAction::UpdateFolder(
        games.clone(),
        Patch::new(|folder: &mut WorkspaceFolder| folder.claude_account = Some("pro".into())),
    ));
    assert_eq!(
        account_of(&m, &plain),
        ("pro".into(), AccountSource::Folder)
    );
    // A folder without an account leaves the default.
    let loose = folder_on(&mut m, None);
    let w = made_in(&mut m, Some(&loose), None);
    assert_eq!(account_of(&m, &w), ("main".into(), AccountSource::Default));
}

#[test]
fn a_workspace_made_in_a_folder_that_is_gone_lands_at_the_top_level() {
    let mut m = model();
    let w = made_in(&mut m, Some("gone"), None);
    assert_eq!(ws(&m, &w).folder_id, None);
    let games = folder_on(&mut m, None);
    let w = made_in(&mut m, Some(&games), None);
    assert_eq!(ws(&m, &w).folder_id, Some(games));
}

#[test]
fn a_workspace_moved_between_folders_follows_each_unless_it_chose_an_account() {
    let mut m = model();
    let games = folder_on(&mut m, Some("perso"));
    let work = folder_on(&mut m, Some("pro"));
    let free = with_workspace(&mut m, 1, &[]);
    let own = made_in(&mut m, None, Some("perso"));
    move_to(&mut m, &free, Some(&games));
    move_to(&mut m, &own, Some(&work));
    assert_eq!(
        account_of(&m, &free),
        ("perso".into(), AccountSource::Folder)
    );
    assert_eq!(
        account_of(&m, &own),
        ("perso".into(), AccountSource::Workspace)
    );
    move_to(&mut m, &free, Some(&work));
    assert_eq!(account_of(&m, &free), ("pro".into(), AccountSource::Folder));
    // Back at the top level: the default again. Nothing was written.
    move_to(&mut m, &free, None);
    assert_eq!(
        account_of(&m, &free),
        ("main".into(), AccountSource::Default)
    );
    assert_eq!(ws(&m, &free).claude_account, None);
    assert_eq!(ws(&m, &own).claude_account.as_deref(), Some("perso"));
}

#[test]
fn a_workspace_that_chose_the_default_keeps_it_inside_a_folder_that_names_another() {
    let mut m = model();
    let games = folder_on(&mut m, Some("perso"));
    let w = made_in(&mut m, Some(&games), Some(DEFAULT_CHOICE));
    assert_eq!(ws(&m, &w).claude_account.as_deref(), Some(DEFAULT_CHOICE));
    assert_eq!(account_of(&m, &w), ("main".into(), AccountSource::Default));
    m.apply(StoreAction::UpdateSettings(Patch::new(
        |s: &mut Settings| {
            s.claude_account = Some("pro".into());
        },
    )));
    assert_eq!(account_of(&m, &w), ("pro".into(), AccountSource::Default));
    // Moved elsewhere, it still says the same.
    let work = folder_on(&mut m, Some("perso"));
    move_to(&mut m, &w, Some(&work));
    assert_eq!(account_of(&m, &w), ("pro".into(), AccountSource::Default));
}

#[test]
fn moving_a_hosted_workspace_into_a_folder_tells_its_node_nothing() {
    let mut m = model();
    let mut site = theirs("site", &[("p1", "Nia")]);
    site.claude_account = Some("their-own".into());
    sync(&mut m, "n1", &[&site]);
    let games = folder_on(&mut m, Some("perso"));
    let before = ws(&m, "site").clone();
    move_to(&mut m, "site", Some(&games));
    let after = ws(&m, "site");
    assert_eq!(after.folder_id, Some(games));
    assert_eq!(after.claude_account.as_deref(), Some("their-own"));
    assert!(hosted::diff(&before, Some(after), m.respawns()).is_empty());
    // Nor does it read the folder's account: its own is its machine's.
    assert_eq!(AccountLevels::of(m.state(), after, None).folder, None);
}

#[test]
fn a_folders_account_is_written_only_when_set_and_read_back() {
    let mut m = model();
    let plain = folder_on(&mut m, None);
    let games = folder_on(&mut m, Some("perso"));
    let written = serde_json::to_value(m.state()).unwrap();
    let folders = written["folders"].as_array().unwrap();
    let of = |id: &str| folders.iter().find(|f| f["id"] == id).unwrap().clone();
    assert!(of(&plain).get("claudeAccount").is_none());
    assert_eq!(of(&games)["claudeAccount"], "perso");
    assert_eq!(crate::state::read_state(written), *m.state());
}

// presets and tidying

#[test]
fn a_growing_preset_copies_the_agent_of_the_pane_the_person_is_in() {
    let mut m = model();
    let w = with_workspace(&mut m, 2, &[AgentId::Shell, AgentId::Claude]);
    let ids = pane_ids(&m, &w);
    m.apply(StoreAction::ApplyPreset {
        ws: w.clone(),
        count: 4,
        model_pane: Some(ids[1].clone()),
    });
    let panes = &ws(&m, &w).panes;
    assert_eq!(panes.len(), 4);
    assert_eq!(&panes[0].id, &ids[0], "the panes kept keep their ids");
    assert_eq!(panes[2].agent, AgentId::Claude);
    assert_eq!(panes[3].agent, AgentId::Claude);
    assert_eq!(tree_ids(&m, &w), pane_ids(&m, &w));
}

#[test]
fn without_a_focused_pane_a_preset_copies_the_last_pane_kept() {
    let mut m = model();
    let w = with_workspace(&mut m, 1, &[AgentId::Claude]);
    m.apply(StoreAction::ApplyPreset {
        ws: w.clone(),
        count: 2,
        model_pane: None,
    });
    assert_eq!(ws(&m, &w).panes[1].agent, AgentId::Claude);
}

#[test]
fn a_shrinking_preset_closes_the_panes_read_last() {
    let mut m = model();
    let w = with_workspace(&mut m, 3, &[]);
    let ids = pane_ids(&m, &w);
    // The third pane dragged to the front: it is read first now.
    m.apply(StoreAction::MovePane {
        ws: w.clone(),
        drag: ids[2].clone(),
        target: ids[0].clone(),
        zone: DropZone::Center,
    });
    m.apply(StoreAction::ApplyPreset {
        ws: w.clone(),
        count: 1,
        model_pane: None,
    });
    assert_eq!(pane_ids(&m, &w), [ids[2].clone()]);
}

#[test]
fn tidying_keeps_the_ids_and_their_reading_order() {
    let mut m = model();
    let w = with_workspace(&mut m, 3, &[]);
    let ids = pane_ids(&m, &w);
    m.apply(StoreAction::MovePane {
        ws: w.clone(),
        drag: ids[0].clone(),
        target: ids[2].clone(),
        zone: DropZone::Bottom,
    });
    let order = tree_ids(&m, &w);
    m.apply(StoreAction::TidyLayout(w.clone()));
    assert_eq!(tree_ids(&m, &w), order);
    assert_eq!(ws(&m, &w).tree, Some(preset_tree(&order)));
}

// panes

#[test]
fn a_new_pane_copies_the_agent_of_the_pane_it_splits() {
    let mut m = model();
    let w = with_workspace(&mut m, 2, &[AgentId::Shell, AgentId::Claude]);
    let ids = pane_ids(&m, &w);
    let new = m
        .apply(StoreAction::AddPane {
            ws: w.clone(),
            agent: None,
            near: Some(ids[1].clone()),
            dir: Some(SplitDir::Col),
            cwd: Some("/elsewhere".into()),
            name: None,
            init: None,
        })
        .pane
        .unwrap();
    let pane = ws(&m, &w)
        .panes
        .iter()
        .find(|p| p.id == new)
        .unwrap()
        .clone();
    assert_eq!(pane.agent, AgentId::Claude);
    assert_eq!(pane.cwd.as_deref(), Some("/elsewhere"));
    assert_eq!(pane.name, "Leo");
    // Below the pane it split.
    assert_eq!(tree_ids(&m, &w), [ids[0].clone(), ids[1].clone(), new]);
}

#[test]
fn an_explicit_agent_wins_and_a_free_name_is_used() {
    let mut m = model();
    let w = with_workspace(&mut m, 1, &[AgentId::Claude]);
    let new = m
        .apply(StoreAction::AddPane {
            ws: w.clone(),
            agent: Some(AgentId::Shell),
            near: None,
            dir: None,
            cwd: None,
            name: Some("  Zed ".into()),
            init: Some(Patch::new(|p: &mut Pane| {
                p.role = Some(PaneRole::Worker);
                p.id = "hijacked".into();
            })),
        })
        .pane
        .unwrap();
    let pane = ws(&m, &w).panes.last().unwrap().clone();
    assert_eq!(pane.id, new, "init never changes the id");
    assert_eq!(pane.agent, AgentId::Shell);
    assert_eq!(pane.name, "Zed");
    assert_eq!(pane.role, Some(PaneRole::Worker));
    // A name already taken is not.
    let taken = m
        .apply(StoreAction::AddPane {
            ws: w.clone(),
            agent: None,
            near: None,
            dir: None,
            cwd: None,
            name: Some("Ava".into()),
            init: None,
        })
        .pane
        .unwrap();
    let pane = ws(&m, &w).panes.iter().find(|p| p.id == taken).unwrap();
    assert_eq!(pane.name, "Max");
}

#[test]
fn a_full_workspace_takes_no_new_pane_and_answers_no_id() {
    let mut m = model();
    let w = with_workspace(&mut m, 16, &[]);
    let outcome = m.apply(StoreAction::AddPane {
        ws: w.clone(),
        agent: None,
        near: None,
        dir: None,
        cwd: None,
        name: None,
        init: None,
    });
    assert_eq!(outcome.pane, None);
    assert!(!outcome.changed);
    assert_eq!(ws(&m, &w).panes.len(), 16);
    assert_eq!(
        m.apply(StoreAction::AddPane {
            ws: "nope".into(),
            agent: None,
            near: None,
            dir: None,
            cwd: None,
            name: None,
            init: None,
        })
        .pane,
        None
    );
}

#[test]
fn the_last_pane_stays_and_a_closed_pane_s_space_goes_to_its_sibling() {
    let mut m = model();
    let w = with_workspace(&mut m, 2, &[]);
    let ids = pane_ids(&m, &w);
    m.apply(StoreAction::ClosePane {
        ws: w.clone(),
        pane: ids[0].clone(),
    });
    assert_eq!(pane_ids(&m, &w), [ids[1].clone()]);
    assert_eq!(ws(&m, &w).tree, Some(leaf(ids[1].clone())));
    let outcome = m.apply(StoreAction::ClosePane {
        ws: w.clone(),
        pane: ids[1].clone(),
    });
    assert!(!outcome.changed);
    assert_eq!(pane_ids(&m, &w).len(), 1);
}

#[test]
fn a_pane_dropped_on_itself_moves_nothing() {
    let mut m = model();
    let w = with_workspace(&mut m, 2, &[]);
    let ids = pane_ids(&m, &w);
    let outcome = m.apply(StoreAction::MovePane {
        ws: w.clone(),
        drag: ids[0].clone(),
        target: ids[0].clone(),
        zone: DropZone::Left,
    });
    assert!(!outcome.changed);
    m.apply(StoreAction::MovePane {
        ws: w.clone(),
        drag: ids[0].clone(),
        target: ids[1].clone(),
        zone: DropZone::Center,
    });
    assert_eq!(tree_ids(&m, &w), [ids[1].clone(), ids[0].clone()]);
}

#[test]
fn a_respawn_gives_a_new_id_in_the_same_place_and_is_announced() {
    let mut m = model();
    let w = with_workspace(&mut m, 2, &[AgentId::Claude, AgentId::Claude]);
    let ids = pane_ids(&m, &w);
    m.apply(StoreAction::UpdatePane {
        ws: w.clone(),
        pane: ids[0].clone(),
        patch: Patch::new(|p: &mut Pane| p.session_id = Some("s1".into())),
    });
    let name = ws(&m, &w).panes[0].name.clone();
    let new = m
        .apply(StoreAction::RespawnPane {
            ws: w.clone(),
            pane: ids[0].clone(),
            patch: Patch::none(),
        })
        .pane
        .unwrap();
    assert_ne!(new, ids[0]);
    let pane = &ws(&m, &w).panes[0];
    assert_eq!(pane.id, new);
    assert_eq!(pane.name, name);
    assert_eq!(pane.session_id.as_deref(), Some("s1"));
    assert_eq!(tree_ids(&m, &w), [new.clone(), ids[1].clone()]);
    // A reset: a fresh conversation.
    let fresh = m
        .apply(StoreAction::RespawnPane {
            ws: w.clone(),
            pane: new.clone(),
            patch: Patch::new(|p: &mut Pane| p.session_id = None),
        })
        .pane
        .unwrap();
    assert_eq!(
        m.take_respawns(),
        [
            Respawn {
                ws: w.clone(),
                from: ids[0].clone(),
                to: new.clone(),
                keeps_session: true,
            },
            Respawn {
                ws: w.clone(),
                from: new.clone(),
                to: fresh,
                keeps_session: false,
            },
        ]
    );
    // Gone, or respawned already: nothing to do, no id.
    assert_eq!(
        m.apply(StoreAction::RespawnPane {
            ws: w.clone(),
            pane: ids[0].clone(),
            patch: Patch::none(),
        })
        .pane,
        None
    );
    assert!(m.take_respawns().is_empty());
}

// settings

#[test]
fn resetting_the_settings_keeps_what_the_defaults_do_not_name() {
    let mut m = model();
    m.apply(StoreAction::UpdateSettings(Patch::new(
        |s: &mut Settings| {
            s.font_size = 18.0;
            s.projects_folder = Some("/code".into());
            s.extra.insert("future".into(), serde_json::json!(1));
        },
    )));
    assert_eq!(m.settings().font_size, 18.0);
    m.apply(StoreAction::ResetSettings);
    assert_eq!(m.settings().font_size, 13.0);
    assert_eq!(m.settings().projects_folder.as_deref(), Some("/code"));
    assert_eq!(m.settings().extra["future"], serde_json::json!(1));
}

// saved commands

fn command(label: &str) -> SavedCommandDraft {
    SavedCommandDraft {
        label: label.into(),
        command: format!("echo {label}"),
        ..SavedCommandDraft::default()
    }
}

#[test]
fn saved_commands_live_in_either_scope_and_keep_their_id_when_they_move() {
    let mut m = model();
    let w = with_workspace(&mut m, 1, &[]);
    let local = m
        .apply(StoreAction::AddSavedCommand {
            scope: CommandScope::Workspace,
            ws: w.clone(),
            draft: command("build"),
        })
        .command
        .unwrap();
    let global = m
        .apply(StoreAction::AddSavedCommand {
            scope: CommandScope::Global,
            ws: w.clone(),
            draft: command("test"),
        })
        .command
        .unwrap();
    assert_eq!(ws(&m, &w).saved_commands[0].id, local);
    assert_eq!(m.settings().saved_commands[0].id, global);

    m.apply(StoreAction::UpdateSavedCommand {
        scope: CommandScope::Workspace,
        ws: w.clone(),
        id: local.clone(),
        patch: Patch::new(|c: &mut SavedCommand| {
            c.auto_run = true;
            c.id = "hijacked".into();
        }),
    });
    assert!(ws(&m, &w).saved_commands[0].auto_run);
    assert_eq!(ws(&m, &w).saved_commands[0].id, local);

    m.apply(StoreAction::MoveSavedCommand {
        from: CommandScope::Workspace,
        to: CommandScope::Global,
        ws: w.clone(),
        id: local.clone(),
    });
    assert!(ws(&m, &w).saved_commands.is_empty());
    let ids: Vec<&str> = m
        .settings()
        .saved_commands
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, [global.as_str(), local.as_str()], "it lands last");

    // Nowhere to take it: a workspace that is gone.
    assert!(
        !m.apply(StoreAction::MoveSavedCommand {
            from: CommandScope::Global,
            to: CommandScope::Workspace,
            ws: "gone".into(),
            id: local.clone(),
        })
        .changed
    );

    m.apply(StoreAction::RemoveSavedCommand {
        scope: CommandScope::Global,
        ws: w.clone(),
        id: global,
    });
    assert_eq!(m.settings().saved_commands.len(), 1);
}

// hydration

#[test]
fn hydration_names_every_pane_uniquely_and_drops_dangling_folder_ids() {
    let pane = |id: &str, name: &str| Pane {
        id: id.into(),
        name: name.into(),
        ..Pane::default()
    };
    let state = AppState {
        workspaces: vec![
            Workspace {
                id: "one".into(),
                panes: vec![pane("a", "Ava"), pane("b", "")],
                folder_id: Some("gone".into()),
                ..Workspace::default()
            },
            Workspace {
                id: "two".into(),
                panes: vec![pane("c", "Ava")],
                folder_id: Some("f".into()),
                ..Workspace::default()
            },
        ],
        folders: vec![WorkspaceFolder {
            id: "f".into(),
            ..WorkspaceFolder::default()
        }],
        active_workspace_id: Some("two".into()),
        ..AppState::default()
    };
    let mut m = Store::default();
    m.hydrate(state);
    assert_eq!(m.revision(), 0, "hydrating is not a change to save");
    let names: Vec<&str> = m
        .state()
        .workspaces
        .iter()
        .flat_map(|w| &w.panes)
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names, ["Ava", "Max", "Leo"]);
    assert_eq!(m.workspace("one").unwrap().folder_id, None);
    assert_eq!(m.workspace("two").unwrap().folder_id.as_deref(), Some("f"));
}

// View panes: what a preview shows, a leaf of the grid beside the terminals.

/// Every leaf of the grid, terminals and views, in reading order.
fn grid_ids(model: &Store, id: &str) -> Vec<PaneId> {
    crate::state::layout::grid_order(ws(model, id))
}

fn add_view(model: &mut Store, w: &str, id: &str, near: Option<&str>) -> Option<PaneId> {
    model
        .apply(StoreAction::AddView {
            ws: w.into(),
            id: id.into(),
            near: near.map(Into::into),
            dir: None,
        })
        .pane
}

#[test]
fn a_view_opens_beside_the_pane_it_is_near_and_keeps_the_terminals_terminals() {
    let mut m = model();
    let w = with_workspace(&mut m, 2, &[]);
    let ids = pane_ids(&m, &w);
    let (a, b) = (ids[0].clone(), ids[1].clone());
    assert_eq!(add_view(&mut m, &w, "v1", Some(&a)), Some("v1".into()));
    assert_eq!(grid_ids(&m, &w), [a.clone(), "v1".into(), b.clone()]);
    // `panes` is still the terminals alone, as 1.x and the agents read it.
    assert_eq!(pane_ids(&m, &w), [a.clone(), b.clone()]);
    assert_eq!(tree_ids(&m, &w), [a, b]);
    // The same entry is never two panes.
    assert_eq!(add_view(&mut m, &w, "v1", None), None);
    assert_eq!(ws(&m, &w).view_panes.len(), 1);
}

#[test]
fn views_count_toward_the_most_panes_a_workspace_holds() {
    let mut m = model();
    let w = with_workspace(&mut m, 14, &[]);
    assert!(add_view(&mut m, &w, "v1", None).is_some());
    assert!(add_view(&mut m, &w, "v2", None).is_some());
    assert_eq!(ws(&m, &w).pane_count(), MAX_PANES);
    assert_eq!(add_view(&mut m, &w, "v3", None), None);
    let added = m.apply(StoreAction::AddPane {
        ws: w.clone(),
        agent: None,
        near: None,
        dir: None,
        cwd: None,
        name: None,
        init: None,
    });
    assert_eq!(added.pane, None, "a terminal finds no room either");
}

#[test]
fn a_view_closes_giving_its_room_to_its_sibling_and_the_last_may_go() {
    let mut m = model();
    let w = with_workspace(&mut m, 1, &[]);
    let a = pane_ids(&m, &w)[0].clone();
    add_view(&mut m, &w, "v1", Some(&a));
    assert!(
        m.apply(StoreAction::CloseView {
            ws: w.clone(),
            id: "v1".into(),
        })
        .changed
    );
    assert!(ws(&m, &w).view_panes.is_empty());
    assert_eq!(ws(&m, &w).tree, Some(leaf(a.clone())));
    // A terminal is not a view: CloseView leaves it.
    assert!(
        !m.apply(StoreAction::CloseView {
            ws: w.clone(),
            id: a.clone(),
        })
        .changed
    );
    // And the last terminal still stays, whatever views it has beside it.
    add_view(&mut m, &w, "v2", None);
    assert!(
        !m.apply(StoreAction::ClosePane {
            ws: w.clone(),
            pane: a,
        })
        .changed
    );
}

#[test]
fn closing_a_terminal_keeps_the_views_where_they_are() {
    let mut m = model();
    let w = with_workspace(&mut m, 3, &[]);
    let ids = pane_ids(&m, &w);
    add_view(&mut m, &w, "v1", Some(&ids[2]));
    m.apply(StoreAction::ClosePane {
        ws: w.clone(),
        pane: ids[0].clone(),
    });
    assert_eq!(
        grid_ids(&m, &w),
        [ids[1].clone(), ids[2].clone(), "v1".into()]
    );
}

#[test]
fn a_view_moves_and_swaps_with_a_terminal_like_any_pane() {
    let mut m = model();
    let w = with_workspace(&mut m, 2, &[]);
    let ids = pane_ids(&m, &w);
    add_view(&mut m, &w, "v1", Some(&ids[1]));
    m.apply(StoreAction::MovePane {
        ws: w.clone(),
        drag: "v1".into(),
        target: ids[0].clone(),
        zone: DropZone::Center,
    });
    assert_eq!(
        grid_ids(&m, &w),
        ["v1".into(), ids[1].clone(), ids[0].clone()]
    );
    m.apply(StoreAction::MovePane {
        ws: w.clone(),
        drag: ids[0].clone(),
        target: "v1".into(),
        zone: DropZone::Left,
    });
    assert_eq!(
        grid_ids(&m, &w),
        [ids[0].clone(), "v1".into(), ids[1].clone()]
    );
}

#[test]
fn tidying_and_arranging_lay_the_views_out_with_the_terminals() {
    let mut m = model();
    let w = with_workspace(&mut m, 3, &[]);
    let ids = pane_ids(&m, &w);
    add_view(&mut m, &w, "v1", Some(&ids[0]));
    let before = grid_ids(&m, &w);
    m.apply(StoreAction::TidyLayout(w.clone()));
    assert_eq!(ws(&m, &w).tree, Some(preset_tree(&before)));
    // The orchestrator leads; the view is one of the others.
    let lead = ids[2].clone();
    m.apply(StoreAction::UpdatePane {
        ws: w.clone(),
        pane: lead.clone(),
        patch: Patch::new(|p: &mut Pane| p.role = Some(PaneRole::Orchestrator)),
    });
    m.apply(StoreAction::ArrangePanes(w.clone()));
    let order = grid_ids(&m, &w);
    assert_eq!(order[0], lead);
    assert!(order.contains(&"v1".to_string()));
    assert_eq!(order.len(), 4);
}

#[test]
fn a_preset_counts_terminals_and_keeps_every_view() {
    let mut m = model();
    let w = with_workspace(&mut m, 4, &[]);
    let ids = pane_ids(&m, &w);
    add_view(&mut m, &w, "v1", Some(&ids[0]));
    m.apply(StoreAction::ApplyPreset {
        ws: w.clone(),
        count: 2,
        model_pane: None,
    });
    assert_eq!(pane_ids(&m, &w), [ids[0].clone(), ids[1].clone()]);
    assert_eq!(
        grid_ids(&m, &w),
        [ids[0].clone(), "v1".into(), ids[1].clone()]
    );
    // Growing: the terminals have what the views leave them.
    for n in 2..MAX_PANES {
        add_view(&mut m, &w, &format!("v{n}"), None);
    }
    let views = ws(&m, &w).view_panes.len();
    assert_eq!(views + 2, MAX_PANES);
    m.apply(StoreAction::ApplyPreset {
        ws: w.clone(),
        count: 8,
        model_pane: None,
    });
    assert_eq!(ws(&m, &w).panes.len(), 2, "no room for more terminals");
    assert_eq!(ws(&m, &w).view_panes.len(), views);
}

#[test]
fn a_respawn_keeps_the_views_in_the_tree() {
    let mut m = model();
    let w = with_workspace(&mut m, 2, &[]);
    let ids = pane_ids(&m, &w);
    add_view(&mut m, &w, "v1", Some(&ids[0]));
    let to = m
        .apply(StoreAction::RespawnPane {
            ws: w.clone(),
            pane: ids[0].clone(),
            patch: Patch::none(),
        })
        .pane
        .unwrap();
    assert_eq!(grid_ids(&m, &w), [to, "v1".into(), ids[1].clone()]);
}

#[test]
fn a_workspace_with_views_is_written_so_that_1x_reads_its_terminals() {
    let mut m = model();
    let w = with_workspace(&mut m, 2, &[]);
    let before = serde_json::to_value(ws(&m, &w)).unwrap();
    assert!(before.get("viewPanes").is_none(), "absent while empty");
    let ids = pane_ids(&m, &w);
    add_view(&mut m, &w, "p18f-1", Some(&ids[0]));
    let written = serde_json::to_value(ws(&m, &w)).unwrap();
    assert_eq!(
        written["viewPanes"],
        serde_json::json!([{ "id": "p18f-1" }])
    );
    let panes: Vec<&str> = written["panes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pane| pane["id"].as_str().unwrap())
        .collect();
    assert_eq!(panes, [ids[0].as_str(), ids[1].as_str()]);
    // 1.x's normalizeTree: the leaves it knows, in their order, the rest
    // dropped; a tree it can always read.
    let tree: SplitNode = serde_json::from_value(written["tree"].clone()).unwrap();
    let known: Vec<PaneId> = leaf_ids(&tree)
        .into_iter()
        .filter(|id| ids.contains(id))
        .collect();
    assert_eq!(known, ids);
    assert_eq!(
        leaf_ids(&normalize_tree(&ws(&m, &w).panes, Some(&tree))),
        ids
    );
    // And this version reads its own file back whole.
    let read: Workspace = serde_json::from_value(written).unwrap();
    assert_eq!(read, *ws(&m, &w));
}

// what `apply` costs

#[test]
fn applying_writes_the_state_in_place_when_nothing_else_holds_it() {
    let mut m = model();
    let id = with_workspace(&mut m, 2, &[]);
    let at = Arc::as_ptr(&m.state);
    let outcome = m.apply(StoreAction::UpdateWorkspace(
        id.clone(),
        Patch::new(|w: &mut Workspace| w.name = "renamed".into()),
    ));
    assert!(outcome.changed, "the change is still seen");
    assert_eq!(ws(&m, &id).name, "renamed");
    assert!(
        std::ptr::eq(at, Arc::as_ptr(&m.state)),
        "no copy of the state was made for a change nothing else saw"
    );
    let unchanged = m.apply(StoreAction::SetActiveWorkspace(Some(id)));
    assert!(!unchanged.changed);
    assert!(std::ptr::eq(at, Arc::as_ptr(&m.state)));
}

#[test]
fn a_published_snapshot_keeps_the_state_it_was_taken_with() {
    let mut m = model();
    let id = with_workspace(&mut m, 1, &[]);
    let published = m.state_arc();
    // Nothing written: the snapshot and the model still share it.
    let outcome = m.apply(StoreAction::UpdateWorkspace(id.clone(), Patch::none()));
    assert!(!outcome.changed);
    assert!(Arc::ptr_eq(&published, &m.state));
    // Written: copied once, the snapshot left as it was.
    let outcome = m.apply(StoreAction::UpdateWorkspace(
        id.clone(),
        Patch::new(|w: &mut Workspace| w.name = "renamed".into()),
    ));
    assert!(outcome.changed);
    assert!(!Arc::ptr_eq(&published, &m.state));
    assert_eq!(published.workspaces[0].name, "w");
    assert_eq!(ws(&m, &id).name, "renamed");
    // The next write goes in place: only the model holds its copy now.
    let at = Arc::as_ptr(&m.state);
    m.apply(StoreAction::UpdateWorkspace(
        id,
        Patch::new(|w: &mut Workspace| w.name = "again".into()),
    ));
    assert!(std::ptr::eq(at, Arc::as_ptr(&m.state)));
}

#[test]
fn a_removed_workspace_is_handed_over_for_its_check_to_go() {
    let mut m = model();
    let id = with_workspace(&mut m, 1, &[]);
    m.apply(StoreAction::RemoveWorkspace("nope".into()));
    assert!(
        m.take_removed().is_empty(),
        "nothing removed, nothing to clean"
    );
    m.apply(StoreAction::RemoveWorkspace(id.clone()));
    assert_eq!(
        m.take_removed(),
        [Removed {
            ws: id,
            cwd: "/p".into(),
            node: None,
        }]
    );
    assert!(m.take_removed().is_empty(), "handed over once");
}

// Hosted workspaces: the mirrors of what a node runs itself (hosted.rs).

/// A workspace as a node lists it.
fn theirs(id: &str, panes: &[(&str, &str)]) -> Workspace {
    let panes: Vec<Pane> = panes
        .iter()
        .map(|(id, name)| Pane {
            id: (*id).into(),
            name: (*name).into(),
            agent: AgentId::Claude,
            ..Pane::default()
        })
        .collect();
    let ids: Vec<PaneId> = panes.iter().map(|pane| pane.id.clone()).collect();
    Workspace {
        id: id.into(),
        name: format!("Space {id}"),
        cwd: format!("/srv/{id}"),
        tree: Some(preset_tree(&ids)),
        panes,
        ..Workspace::default()
    }
}

fn sync(model: &mut Store, node: &str, workspaces: &[&Workspace]) -> StoreOutcome {
    sync_keeping(model, node, workspaces, &[])
}

fn sync_keeping(
    model: &mut Store,
    node: &str,
    workspaces: &[&Workspace],
    keep: &[&str],
) -> StoreOutcome {
    model.apply(StoreAction::SyncHosted {
        node: node.into(),
        workspaces: workspaces.iter().map(|ws| (*ws).clone()).collect(),
        keep: keep.iter().map(|id| (*id).to_string()).collect(),
    })
}

#[test]
fn a_node_s_workspaces_are_mirrored_after_this_hangar_s_own() {
    let mut m = model();
    let local = with_workspace(&mut m, 1, &[]);
    let site = theirs("site", &[("p1", "Nia"), ("p2", "Bo")]);
    assert!(sync(&mut m, "n1", &[&site]).changed);
    let ids: Vec<&str> = m.state().workspaces.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(ids, [local.as_str(), "site"]);
    let mirror = ws(&m, "site");
    assert_eq!(mirror.node.as_deref(), Some("n1"));
    assert_eq!(mirror.hosted, Some(true));
    assert_eq!(mirror.panes, site.panes);
    assert_eq!(tree_ids(&m, "site"), ["p1", "p2"]);
    assert_eq!(
        m.state().active_workspace_id.as_deref(),
        Some(local.as_str()),
        "what the person looks at stays"
    );
    let written = serde_json::to_value(mirror).unwrap();
    assert_eq!(written["hosted"], true);
    let own = serde_json::to_value(ws(&m, &local)).unwrap();
    assert!(own.get("hosted").is_none(), "absent for those it runs");
}

#[test]
fn a_sync_keeps_the_mirror_s_folder_tree_and_chat_view() {
    let mut m = model();
    let mut site = theirs("site", &[("p1", "Nia"), ("p2", "Bo")]);
    sync(&mut m, "n1", &[&site]);
    let folder = m
        .apply(StoreAction::AddFolder("Servers".into()))
        .folder
        .unwrap();
    m.apply(StoreAction::MoveWorkspace {
        id: "site".into(),
        folder: Some(folder.clone()),
        before: None,
    });
    m.apply(StoreAction::MovePane {
        ws: "site".into(),
        drag: "p1".into(),
        target: "p2".into(),
        zone: DropZone::Bottom,
    });
    m.apply(StoreAction::SetChatView {
        ws: "site".into(),
        on: true,
    });
    m.apply(StoreAction::AddView {
        ws: "site".into(),
        id: "view".into(),
        near: Some("p2".into()),
        dir: None,
    });
    let tree = ws(&m, "site").tree.clone();

    site.name = "Shop".into();
    site.panes[0].session_id = Some("s-1".into());
    site.tree = Some(preset_tree(&["p2".to_string(), "p1".to_string()]));
    assert!(sync(&mut m, "n1", &[&site]).changed);
    let mirror = ws(&m, "site");
    assert_eq!(mirror.name, "Shop");
    assert_eq!(mirror.panes[0].session_id.as_deref(), Some("s-1"));
    assert_eq!(mirror.folder_id, Some(folder));
    assert_eq!(mirror.chat_view, Some(true));
    assert_eq!(mirror.tree, tree);
    assert!(mirror.has_view("view"));
}

#[test]
fn a_sync_that_says_nothing_new_leaves_the_revision_alone() {
    let mut m = model();
    let site = theirs("site", &[("p1", "Nia")]);
    sync(&mut m, "n1", &[&site]);
    m.apply(StoreAction::SetChatView {
        ws: "site".into(),
        on: true,
    });
    let revision = m.revision();
    assert!(!sync(&mut m, "n1", &[&site]).changed);
    // The node's own layout and presentation are not the mirror's.
    let moved = Workspace {
        theme_id: Some("paper".into()),
        folder_id: Some("f".into()),
        ..site.clone()
    };
    assert!(!sync(&mut m, "n1", &[&moved]).changed);
    assert_eq!(m.revision(), revision);
}

#[test]
fn a_pane_the_node_respawned_keeps_its_place_and_is_announced() {
    let mut m = model();
    let mut site = theirs("site", &[("p1", "Nia"), ("p2", "Bo"), ("p3", "Cy")]);
    site.panes[1].session_id = Some("s-1".into());
    sync(&mut m, "n1", &[&site]);
    // The same pane under a new id, by its name; and a new conversation.
    site.panes[1].id = "p2b".into();
    site.panes[1].session_id = None;
    sync(&mut m, "n1", &[&site]);
    assert_eq!(pane_ids(&m, "site"), ["p1", "p2b", "p3"]);
    assert_eq!(tree_ids(&m, "site"), ["p1", "p2b", "p3"]);
    assert_eq!(
        m.take_respawns(),
        [Respawn {
            ws: "site".into(),
            from: "p2".into(),
            to: "p2b".into(),
            keeps_session: false,
        }]
    );
}

#[test]
fn a_mirror_goes_when_its_node_no_longer_has_it_and_nothing_is_cleaned_up() {
    let mut m = model();
    let local = with_workspace(&mut m, 1, &[]);
    let site = theirs("site", &[("p1", "Nia")]);
    let docs = theirs("docs", &[("p2", "Bo")]);
    sync(&mut m, "n1", &[&site, &docs]);
    m.apply(StoreAction::SetActiveWorkspace(Some("docs".into())));
    // Another node's listing says nothing of this one's.
    assert!(!sync(&mut m, "n2", &[]).changed);
    assert!(sync(&mut m, "n1", &[&site]).changed);
    assert!(m.workspace("docs").is_none());
    assert!(m.workspace("site").is_some());
    assert_eq!(
        m.state().active_workspace_id.as_deref(),
        Some(local.as_str()),
        "the active one gone, the first one left is"
    );
    assert!(
        m.take_removed().is_empty(),
        "the node removed it: its worktrees are the node's"
    );
    sync(&mut m, "n1", &[]);
    assert_eq!(m.state().workspaces.len(), 1);
    assert!(m.take_removed().is_empty());
}

#[test]
fn a_workspace_being_handed_over_is_kept_until_the_node_lists_it() {
    let mut m = model();
    // One this Hangar drives on the node, and the same once it said the
    // node would run it, before the node lists it.
    let mut driven = theirs("site", &[("p1", "Nia")]);
    driven.node = Some("n1".into());
    let mut pending = theirs("docs", &[("p2", "Bo")]);
    pending.node = Some("n1".into());
    pending.hosted = Some(true);
    pending.folder_id = None;
    m.apply(StoreAction::PutWorkspace(Box::new(driven.clone())));
    m.apply(StoreAction::PutWorkspace(Box::new(pending.clone())));

    assert!(!sync_keeping(&mut m, "n1", &[], &["docs"]).changed);
    assert_eq!(*ws(&m, "site"), driven, "driven from here: not the node's");
    assert_eq!(*ws(&m, "docs"), pending);

    // The node lists both: each is its mirror from now on, as it stood.
    let site = theirs("site", &[("p1", "Nia")]);
    let docs = theirs("docs", &[("p2", "Bo")]);
    sync_keeping(&mut m, "n1", &[&site, &docs], &["docs"]);
    assert!(ws(&m, "site").is_hosted());
    assert_eq!(*ws(&m, "docs"), pending);
    assert_eq!(m.state().workspaces.len(), 2, "no second copy");
    // Without the keep, one the node does not list goes.
    sync(&mut m, "n1", &[&site]);
    assert!(m.workspace("docs").is_none());
}

#[test]
fn an_id_another_workspace_holds_is_not_mirrored() {
    let mut m = model();
    let local = with_workspace(&mut m, 1, &[]);
    let before = ws(&m, &local).clone();
    let clash = theirs(&local, &[("p1", "Nia")]);
    assert!(!sync(&mut m, "n1", &[&clash, &clash]).changed);
    assert_eq!(*ws(&m, &local), before);
    assert_eq!(m.state().workspaces.len(), 1);
}

#[test]
fn a_workspace_is_put_whole_with_its_ids_or_in_place_of_its_namesake() {
    let mut m = model();
    let local = with_workspace(&mut m, 1, &[]);
    let site = theirs("site", &[("p1", "Nia"), ("p2", "Bo")]);
    let put = m.apply(StoreAction::PutWorkspace(Box::new(site.clone())));
    assert_eq!(put.workspace.as_deref(), Some("site"));
    assert!(put.changed);
    assert_eq!(*ws(&m, "site"), site);
    assert_eq!(
        m.state().active_workspace_id.as_deref(),
        Some(local.as_str())
    );
    assert!(
        !m.apply(StoreAction::PutWorkspace(Box::new(site.clone())))
            .changed
    );
    let renamed = Workspace {
        name: "Shop".into(),
        ..site
    };
    assert!(
        m.apply(StoreAction::PutWorkspace(Box::new(renamed.clone())))
            .changed
    );
    assert_eq!(m.state().workspaces.len(), 2);
    assert_eq!(*ws(&m, "site"), renamed);
    assert_eq!(
        m.apply(StoreAction::PutWorkspace(Box::default())).workspace,
        None,
        "a workspace needs an id"
    );
}

#[test]
fn a_pane_is_put_with_its_id_once_and_named_apart() {
    let mut m = model();
    let w = with_workspace(&mut m, 1, &[AgentId::Claude]);
    let first = pane_ids(&m, &w)[0].clone();
    let other = with_workspace(&mut m, 1, &[]);
    let put = |m: &mut Store, ws: &str, id: &str, name: &str| {
        m.apply(StoreAction::PutPane {
            ws: ws.into(),
            pane: Box::new(Pane {
                id: id.into(),
                name: name.into(),
                role: Some(PaneRole::Worker),
                ..Pane::default()
            }),
            near: Some(first.clone()),
            dir: Some(SplitDir::Col),
        })
    };
    assert_eq!(
        put(&mut m, &w, "desk-1", "Zed").pane.as_deref(),
        Some("desk-1")
    );
    let pane = &ws(&m, &w).panes[1];
    assert_eq!((pane.id.as_str(), pane.name.as_str()), ("desk-1", "Zed"));
    assert_eq!(pane.role, Some(PaneRole::Worker));
    assert_eq!(
        pane.agent,
        AgentId::Shell,
        "as it was put, not as its neighbour"
    );
    assert_eq!(tree_ids(&m, &w), [first.clone(), "desk-1".to_string()]);
    assert!(matches!(
        ws(&m, &w).tree,
        Some(SplitNode::Split {
            dir: SplitDir::Col,
            ..
        })
    ));
    // Again, or under an id any workspace has: nothing.
    let again = put(&mut m, &w, "desk-1", "Zed");
    assert_eq!((again.pane, again.changed), (None, false));
    assert_eq!(put(&mut m, &other, "desk-1", "Yan").pane, None);
    assert_eq!(put(&mut m, &w, "", "Yan").pane, None);
    // A name in use, here or in another workspace, is not taken twice.
    let taken = ws(&m, &other).panes[0].name.clone();
    put(&mut m, &w, "desk-2", &taken);
    let name = ws(&m, &w).panes[2].name.clone();
    assert!(!name.is_empty() && name != taken && name != "Zed", "{name}");
}

#[test]
fn a_pane_is_respawned_as_the_id_given_like_any_respawn() {
    let mut m = model();
    let w = with_workspace(&mut m, 2, &[AgentId::Claude, AgentId::Claude]);
    let ids = pane_ids(&m, &w);
    let respawn = |m: &mut Store, pane: &str, to: &str| {
        m.apply(StoreAction::RespawnPaneAs {
            ws: w.clone(),
            pane: pane.into(),
            to: to.into(),
            patch: Patch::new(|p: &mut Pane| p.agent = AgentId::Shell),
        })
    };
    assert_eq!(
        respawn(&mut m, &ids[0], "desk-1").pane.as_deref(),
        Some("desk-1")
    );
    assert_eq!(pane_ids(&m, &w), ["desk-1".to_string(), ids[1].clone()]);
    assert_eq!(tree_ids(&m, &w), ["desk-1".to_string(), ids[1].clone()]);
    assert_eq!(ws(&m, &w).panes[0].agent, AgentId::Shell);
    assert_eq!(
        m.take_respawns(),
        [Respawn {
            ws: w.clone(),
            from: ids[0].clone(),
            to: "desk-1".into(),
            keeps_session: true,
        }]
    );
    // Again (the pane no longer goes by its old id), onto an id in use,
    // or onto none: nothing.
    for (pane, to) in [
        (ids[0].as_str(), "desk-1"),
        (ids[1].as_str(), "desk-1"),
        (ids[1].as_str(), ""),
    ] {
        let again = respawn(&mut m, pane, to);
        assert_eq!(
            (again.pane, again.changed),
            (None, false),
            "{pane} as {to:?}"
        );
    }
    assert!(m.take_respawns().is_empty());
}

#[test]
fn hydration_leaves_the_names_of_a_node_s_panes_alone() {
    let pane = |id: &str, name: &str| Pane {
        id: id.into(),
        name: name.into(),
        ..Pane::default()
    };
    let state = AppState {
        workspaces: vec![
            Workspace {
                id: "mirror".into(),
                panes: vec![pane("a", "Ava"), pane("b", "")],
                node: Some("n1".into()),
                hosted: Some(true),
                ..Workspace::default()
            },
            Workspace {
                id: "own".into(),
                panes: vec![pane("c", "Ava"), pane("d", "Ava")],
                ..Workspace::default()
            },
        ],
        ..AppState::default()
    };
    let mut m = Store::default();
    m.hydrate(state);
    let names =
        |id: &str| -> Vec<String> { ws(&m, id).panes.iter().map(|p| p.name.clone()).collect() };
    assert_eq!(names("mirror"), ["Ava", ""], "the node names its panes");
    assert_eq!(
        names("own"),
        ["Ava", "Max"],
        "its own are told apart as ever"
    );
}

// Pinned panes (`Workspace::pins`, layout.rs): a pinned pane keeps its edge
// and its size however many panes open, close or move beside it.

const GRID: (f32, f32) = (1600.0, 900.0);

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// A workspace of an orchestrator and a shell beside it, laid out as Hangar
/// lays an agent's workspace out; answers it and the two panes.
fn orchestrated(m: &mut Store) -> (WsId, PaneId, PaneId) {
    let w = with_workspace(m, 2, &[AgentId::Claude, AgentId::Shell]);
    let ids = pane_ids(m, &w);
    m.apply(StoreAction::UpdatePane {
        ws: w.clone(),
        pane: ids[0].clone(),
        patch: PanePatch::new(|pane: &mut Pane| pane.role = Some(PaneRole::Orchestrator)),
    });
    m.apply(StoreAction::ArrangePanes(w.clone()));
    (w, ids[0].clone(), ids[1].clone())
}

/// A worker opened as the orchestrator opens one (worker.rs, then
/// host/writes.rs): beside `near`, then the workspace laid out again.
fn open_worker(m: &mut Store, w: &str, near: &str) -> PaneId {
    let id = m
        .apply(StoreAction::AddPane {
            ws: w.into(),
            agent: Some(AgentId::Claude),
            near: Some(near.into()),
            dir: None,
            cwd: None,
            name: None,
            init: Some(PanePatch::new(|pane: &mut Pane| {
                pane.role = Some(PaneRole::Worker)
            })),
        })
        .pane
        .unwrap();
    m.apply(StoreAction::ArrangePanes(w.into()));
    id
}

fn close_worker(m: &mut Store, w: &str, pane: &str) {
    m.apply(StoreAction::ClosePane {
        ws: w.into(),
        pane: pane.into(),
    });
    m.apply(StoreAction::ArrangePanes(w.into()));
}

fn pin(
    m: &mut Store,
    w: &str,
    pane: &str,
    edge: Option<PinEdge>,
    grid: Option<(f32, f32)>,
) -> bool {
    m.apply(StoreAction::PinPane {
        ws: w.into(),
        pane: pane.into(),
        edge,
        grid,
    })
    .changed
}

/// Where pane `id` lies, the grid's size unknown (`grid_tree`).
fn laid(m: &Store, w: &str, id: &str) -> Rect {
    compute_layout(&grid_tree(ws(m, w))).rects[id]
}

/// Where pane `id` is drawn in a grid of `GRID` pixels (`drawn_tree`).
fn drawn(m: &Store, w: &str, id: &str) -> Rect {
    compute_layout(&drawn_tree(ws(m, w), Some(GRID))).rects[id]
}

#[test]
fn a_pinned_terminal_keeps_its_place_and_size_as_one_to_eight_workers_open_and_close() {
    let mut m = model();
    let (w, leo, term) = orchestrated(&mut m);
    // The terminal on the right of the orchestrator: pinned there, at the
    // pixels it has.
    assert!(pin(&mut m, &w, &term, None, Some(GRID)));
    let pinned = ws(&m, &w).pin_of(&term).cloned().unwrap();
    assert_eq!(pinned.edge, PinEdge::Right);
    assert!(close(pinned.px.unwrap(), 640.0), "{pinned:?}");
    assert!(close(pinned.fraction.unwrap(), 0.4));
    let (on_screen, laid_out) = (drawn(&m, &w, &term), laid(&m, &w, &term));
    assert!(close(on_screen.width * 1600.0, 640.0));
    assert!(close(on_screen.left + on_screen.width, 1.0));

    let mut workers: Vec<PaneId> = Vec::new();
    for n in 1..=8 {
        let near = workers.last().cloned().unwrap_or_else(|| leo.clone());
        workers.push(open_worker(&mut m, &w, &near));
        assert_eq!(drawn(&m, &w, &term), on_screen, "{n} workers");
        assert_eq!(laid(&m, &w, &term), laid_out, "{n} workers");
        // None of them in its room; the orchestrator leads the others.
        for id in workers.iter().chain([&leo]) {
            let rect = laid(&m, &w, id);
            assert!(rect.left + rect.width <= laid_out.left + 1e-9, "{n}: {id}");
        }
        assert_eq!(laid(&m, &w, &leo).left, 0.0);
        assert_eq!(laid(&m, &w, &leo).height, 1.0);
    }
    while let Some(worker) = workers.pop() {
        close_worker(&mut m, &w, &worker);
        assert_eq!(drawn(&m, &w, &term), on_screen, "{} workers", workers.len());
        assert_eq!(laid(&m, &w, &term), laid_out);
    }
    assert_eq!(pane_ids(&m, &w), [leo, term]);
}

#[test]
fn a_pane_split_off_a_pinned_one_lands_beside_it_out_of_its_room() {
    let mut m = model();
    let (w, leo, term) = orchestrated(&mut m);
    pin(&mut m, &w, &term, None, None);
    let before = laid(&m, &w, &term);
    let new = m
        .apply(StoreAction::AddPane {
            ws: w.clone(),
            agent: None,
            near: Some(term.clone()),
            dir: None,
            cwd: None,
            name: None,
            init: None,
        })
        .pane
        .unwrap();
    assert_eq!(laid(&m, &w, &term), before);
    let rect = laid(&m, &w, &new);
    assert!(close(rect.left + rect.width, before.left), "{rect:?}");
    // It runs what the pane split runs: a shell.
    let agent = ws(&m, &w).panes.iter().find(|p| p.id == new).unwrap().agent;
    assert_eq!(agent, AgentId::Shell);
    assert_eq!(grid_ids(&m, &w), [leo, new, term]);
}

#[test]
fn a_preset_counts_the_unpinned_terminals_and_never_closes_a_pinned_one() {
    let mut m = model();
    let (w, leo, term) = orchestrated(&mut m);
    pin(&mut m, &w, &term, None, Some(GRID));
    let (on_screen, laid_out) = (drawn(&m, &w, &term), laid(&m, &w, &term));
    m.apply(StoreAction::ApplyPreset {
        ws: w.clone(),
        count: 4,
        model_pane: None,
    });
    let panes = &ws(&m, &w).panes;
    assert_eq!(panes.len(), 5, "four and the pinned one");
    assert!(panes.iter().any(|pane| pane.id == term));
    assert_eq!(drawn(&m, &w, &term), on_screen);
    assert_eq!(laid(&m, &w, &term), laid_out);
    // The four in an even grid in the room it leaves.
    for pane in panes.iter().filter(|pane| pane.id != term) {
        let rect = laid(&m, &w, &pane.id);
        assert!(close(rect.width, laid_out.left / 2.0), "{rect:?}");
        assert!(close(rect.height, 0.5));
    }
    // What the toolbar names before shrinking is what goes, never the pin.
    let named: Vec<PaneId> = closed_by_preset_in(ws(&m, &w), 1)
        .iter()
        .map(|pane| pane.id.clone())
        .collect();
    assert_eq!(named.len(), 3);
    assert!(!named.contains(&term) && !named.contains(&leo));
    m.apply(StoreAction::ApplyPreset {
        ws: w.clone(),
        count: 1,
        model_pane: None,
    });
    assert_eq!(pane_ids(&m, &w), [leo.clone(), term.clone()]);
    assert_eq!(laid(&m, &w, &term), laid_out);
    // Tidying works on the others alone too.
    m.apply(StoreAction::TidyLayout(w.clone()));
    assert_eq!(laid(&m, &w, &term), laid_out);
    assert_eq!(laid(&m, &w, &leo).width, laid_out.left);
}

#[test]
fn dragging_a_pinned_pane_s_divider_sets_the_size_it_keeps() {
    let mut m = model();
    let (w, leo, term) = orchestrated(&mut m);
    pin(&mut m, &w, &term, None, Some(GRID));
    // The grid's drag as it is sent today: the tree, its ratio moved.
    let tree = with_ratio(&grid_tree(ws(&m, &w)), &[], 0.5);
    assert!(m.apply(StoreAction::SetTree(w.clone(), tree)).changed);
    let pinned = ws(&m, &w).pin_of(&term).cloned().unwrap();
    assert!(close(pinned.fraction.unwrap(), 0.5));
    assert!(close(pinned.px.unwrap(), 800.0), "{pinned:?}");
    assert!(close(laid(&m, &w, &term).width, 0.5));
    // With the grid's size: the pixels themselves.
    m.apply(StoreAction::ResizeSplit {
        ws: w.clone(),
        path: Vec::new(),
        ratio: 0.75,
        grid: Some((1200.0, 900.0)),
    });
    let pinned = ws(&m, &w).pin_of(&term).cloned().unwrap();
    assert!(close(pinned.px.unwrap(), 300.0));
    assert!(close(drawn(&m, &w, &term).width * 1600.0, 300.0));
    // And it stays that size as workers come.
    let worker = open_worker(&mut m, &w, &leo);
    assert!(close(drawn(&m, &w, &term).width * 1600.0, 300.0));
    // A divider between the others sets no pin.
    let before = ws(&m, &w).pins.clone();
    m.apply(StoreAction::ResizeSplit {
        ws: w.clone(),
        path: vec![Side::A],
        ratio: 0.3,
        grid: Some(GRID),
    });
    assert_eq!(ws(&m, &w).pins, before);
    assert_eq!(ratio_at(&grid_tree(ws(&m, &w)), &[Side::A]), 0.3);
    assert!(laid(&m, &w, &worker).width > laid(&m, &w, &leo).width);
    // Nor a tree that moves only them.
    let tree = with_ratio(&grid_tree(ws(&m, &w)), &[Side::A], 0.4);
    m.apply(StoreAction::SetTree(w.clone(), tree));
    assert_eq!(ws(&m, &w).pins, before);
}

#[test]
fn two_pins_on_two_edges_each_keep_theirs_while_workers_come_and_go() {
    let mut m = model();
    let (w, leo, term) = orchestrated(&mut m);
    let first = open_worker(&mut m, &w, &leo);
    pin(&mut m, &w, &term, None, Some(GRID));
    // The orchestrator pinned too, on the left: the first pin outermost.
    assert!(pin(&mut m, &w, &leo, None, Some(GRID)));
    let pins = &ws(&m, &w).pins;
    assert_eq!(
        pins.iter()
            .map(|pin| (pin.id.as_str(), pin.edge))
            .collect::<Vec<_>>(),
        [
            (term.as_str(), PinEdge::Right),
            (leo.as_str(), PinEdge::Left)
        ]
    );
    let rects = |m: &Store| {
        [
            drawn(m, &w, &term),
            drawn(m, &w, &leo),
            laid(m, &w, &term),
            laid(m, &w, &leo),
        ]
    };
    let before = rects(&m);
    let mut workers = vec![first];
    for _ in 0..6 {
        let near = workers.last().unwrap().clone();
        workers.push(open_worker(&mut m, &w, &near));
        assert_eq!(rects(&m), before);
    }
    // With the orchestrator pinned, the others are an even grid between.
    for worker in &workers {
        let rect = laid(&m, &w, worker);
        assert!(rect.left >= before[3].width - 1e-9);
        assert!(rect.left + rect.width <= before[2].left + 1e-9);
    }
    while workers.len() > 1 {
        let worker = workers.pop().unwrap();
        close_worker(&mut m, &w, &worker);
        assert_eq!(rects(&m), before);
    }
}

#[test]
fn two_pins_on_one_edge_stand_side_by_side_and_the_inner_one_keeps_its_size() {
    let mut m = model();
    let (w, leo, term) = orchestrated(&mut m);
    let worker = open_worker(&mut m, &w, &leo);
    pin(&mut m, &w, &term, None, None);
    let outer = laid(&m, &w, &term);
    // The worker beside it, on the right of what is left: inside it.
    assert!(pin(&mut m, &w, &worker, None, None));
    assert_eq!(
        ws(&m, &w).pin_of(&worker).map(|pin| pin.edge),
        Some(PinEdge::Right)
    );
    let inner = laid(&m, &w, &worker);
    assert_eq!(laid(&m, &w, &term), outer);
    assert!(close(inner.left + inner.width, outer.left));
    assert_eq!(
        grid_ids(&m, &w),
        [leo.clone(), worker.clone(), term.clone()]
    );
    // The outer one closed by hand: it goes like any pane, and the other
    // comes against the edge at the size it had.
    assert!(
        m.apply(StoreAction::ClosePane {
            ws: w.clone(),
            pane: term.clone(),
        })
        .changed
    );
    assert_eq!(pane_ids(&m, &w), [leo.clone(), worker.clone()]);
    let pins: Vec<&str> = ws(&m, &w).pins.iter().map(|pin| pin.id.as_str()).collect();
    assert_eq!(pins, [worker.as_str()]);
    let now = laid(&m, &w, &worker);
    assert!(close(now.width, inner.width), "{now:?} {inner:?}");
    assert!(close(now.left + now.width, 1.0));
}

#[test]
fn unpinning_leaves_the_pane_where_it_is_until_the_next_layout() {
    let mut m = model();
    let (w, leo, term) = orchestrated(&mut m);
    pin(&mut m, &w, &term, None, Some(GRID));
    let worker = open_worker(&mut m, &w, &leo);
    let before = laid(&m, &w, &term);
    assert!(
        m.apply(StoreAction::UnpinPane {
            ws: w.clone(),
            pane: term.clone(),
        })
        .changed
    );
    assert!(ws(&m, &w).pins.is_empty());
    assert_eq!(laid(&m, &w, &term), before, "where it was");
    // Not pinned: nothing to undo.
    assert!(
        !m.apply(StoreAction::UnpinPane {
            ws: w.clone(),
            pane: term.clone(),
        })
        .changed
    );
    // Laid out with the others from then on.
    m.apply(StoreAction::ArrangePanes(w.clone()));
    assert_eq!(grid_ids(&m, &w), [leo, worker, term.clone()]);
    assert_ne!(laid(&m, &w, &term), before);
}

#[test]
fn pinning_again_changes_nothing_unless_another_edge_is_named() {
    let mut m = model();
    let (w, leo, term) = orchestrated(&mut m);
    assert!(pin(&mut m, &w, &term, None, None));
    assert!(!pin(&mut m, &w, &term, None, Some(GRID)));
    assert!(!pin(&mut m, &w, &term, Some(PinEdge::Right), None));
    assert!(pin(&mut m, &w, &term, Some(PinEdge::Bottom), None));
    assert_eq!(ws(&m, &w).pins.len(), 1);
    let rect = laid(&m, &w, &term);
    assert!(close(rect.top + rect.height, 1.0) && close(rect.width, 1.0));
    // A pane that is not in the workspace is not pinned.
    assert!(!pin(&mut m, &w, "gone", None, None));
    assert_eq!(laid(&m, &w, &leo).top, 0.0);
}

#[test]
fn a_restarted_pane_keeps_its_pin_under_its_new_id() {
    let mut m = model();
    let (w, _leo, term) = orchestrated(&mut m);
    pin(&mut m, &w, &term, None, Some(GRID));
    let pinned = ws(&m, &w).pin_of(&term).cloned().unwrap();
    let before = laid(&m, &w, &term);
    let to = m
        .apply(StoreAction::RespawnPane {
            ws: w.clone(),
            pane: term.clone(),
            patch: PanePatch::none(),
        })
        .pane
        .unwrap();
    assert_eq!(
        ws(&m, &w).pin_of(&to),
        Some(&PanePin {
            id: to.clone(),
            ..pinned
        })
    );
    assert_eq!(ws(&m, &w).pin_of(&term), None);
    assert_eq!(laid(&m, &w, &to), before);
}

#[test]
fn a_pinned_pane_is_neither_moved_nor_dropped_on_and_the_others_still_move() {
    let mut m = model();
    let (w, leo, term) = orchestrated(&mut m);
    let worker = open_worker(&mut m, &w, &leo);
    pin(&mut m, &w, &term, None, None);
    let tree = ws(&m, &w).tree.clone();
    let moved = |m: &mut Store, drag: &str, target: &str, zone| {
        m.apply(StoreAction::MovePane {
            ws: w.clone(),
            drag: drag.into(),
            target: target.into(),
            zone,
        })
        .changed
    };
    assert!(!moved(&mut m, &term, &leo, DropZone::Left));
    assert!(!moved(&mut m, &leo, &term, DropZone::Center));
    assert!(!moved(&mut m, &worker, &term, DropZone::Bottom));
    assert_eq!(ws(&m, &w).tree, tree);
    let before = laid(&m, &w, &term);
    assert!(moved(&mut m, &leo, &worker, DropZone::Center));
    assert_eq!(grid_ids(&m, &w), [worker, leo, term.clone()]);
    assert_eq!(laid(&m, &w, &term), before);
}

#[test]
fn a_view_pins_like_a_terminal_and_its_pin_goes_with_it() {
    let mut m = model();
    let (w, leo, _term) = orchestrated(&mut m);
    add_view(&mut m, &w, "k", Some(leo.as_str()));
    assert!(pin(&mut m, &w, "k", Some(PinEdge::Left), None));
    let before = laid(&m, &w, "k");
    assert_eq!((before.left, before.top, before.height), (0.0, 0.0, 1.0));
    open_worker(&mut m, &w, &leo);
    assert_eq!(laid(&m, &w, "k"), before);
    m.apply(StoreAction::CloseView {
        ws: w.clone(),
        id: "k".into(),
    });
    assert!(ws(&m, &w).pins.is_empty());
    assert_eq!(laid(&m, &w, &leo).left, 0.0);
}

#[test]
fn pins_are_saved_with_the_workspace_and_read_back_as_they_were() {
    let mut m = model();
    let (w, leo, term) = orchestrated(&mut m);
    open_worker(&mut m, &w, &leo);
    pin(&mut m, &w, &term, None, Some(GRID));
    let json = m.to_json();
    let written = &json["workspaces"][0];
    let pins = written["pins"].as_array().unwrap();
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0]["id"], serde_json::json!(term));
    assert_eq!(pins[0]["edge"], "right");
    assert!(pins[0]["px"].as_f64().is_some_and(|px| px > 0.0));
    assert!(pins[0]["fraction"].as_f64().is_some());
    // The tree saved has it on its edge: a Hangar that does not know pins
    // draws it where it is.
    let tree: SplitNode = serde_json::from_value(written["tree"].clone()).unwrap();
    assert_eq!(compute_layout(&tree).rects[&term], laid(&m, &w, &term));
    // Read back from the file: the same pins, the same grid.
    let (state, kept) = crate::state::read_state_keeping(json);
    assert!(kept.is_empty(), "{kept:?}");
    let mut again = Store::default();
    again.hydrate_read(state, kept);
    assert_eq!(ws(&again, &w).pins, ws(&m, &w).pins);
    assert_eq!(grid_tree(ws(&again, &w)), grid_tree(ws(&m, &w)));
    assert_eq!(drawn(&again, &w, &term), drawn(&m, &w, &term));
    // A workspace without pins writes no key.
    let plain = with_workspace(&mut m, 2, &[]);
    let json = m.to_json();
    let plain = json["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ws| ws["id"] == serde_json::json!(plain))
        .unwrap();
    assert!(plain.get("pins").is_none());
}

#[test]
fn a_mirror_s_pins_are_its_own_and_follow_what_its_node_says() {
    let mut m = model();
    let mut site = theirs("site", &[("p1", "Nia"), ("p2", "Bo")]);
    sync(&mut m, "n1", &[&site]);
    let before = ws(&m, "site").clone();
    assert!(pin(&mut m, "site", "p2", None, Some(GRID)));
    let after = ws(&m, "site").clone();
    assert_eq!(after.pin_of("p2").map(|pin| pin.edge), Some(PinEdge::Right));
    assert!(
        hosted::diff(&before, Some(&after), &[]).is_empty(),
        "the node hears nothing of it"
    );
    let pinned = laid(&m, "site", "p2");
    // The node opens a pane and restarts the pinned one: the pin follows,
    // and the new pane lands out of its room.
    site.panes.push(Pane {
        id: "p3".into(),
        name: "Cy".into(),
        agent: AgentId::Claude,
        ..Pane::default()
    });
    site.panes[1].id = "p2b".into();
    sync(&mut m, "n1", &[&site]);
    let mirror = ws(&m, "site");
    assert_eq!(
        mirror.pin_of("p2b").map(|pin| pin.edge),
        Some(PinEdge::Right)
    );
    assert_eq!(laid(&m, "site", "p2b"), pinned);
    let new = laid(&m, "site", "p3");
    assert!(new.left + new.width <= pinned.left + 1e-9, "{new:?}");
    // The node closes it: its pin goes.
    site.panes.remove(1);
    sync(&mut m, "n1", &[&site]);
    assert!(ws(&m, "site").pins.is_empty());
    assert_eq!(grid_ids(&m, "site"), ["p1", "p3"]);
}
