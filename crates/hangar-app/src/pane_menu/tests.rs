//! The « … » menu of a pane header: what a pane is to the orchestration (the
//! orchestration panels' rule decides), the lines its agent's capabilities
//! give it, and the account block it ends with, which is set off by one
//! separator and one heading, not two.

use hangar_i18n::Locale;

use super::*;
use crate::navigator::testing::pane;
use crate::shell::MenuEntry;
use crate::state::types::AgentId;
use crate::workspace_dialog::testing::Harness;

fn workspace(panes: Vec<Pane>) -> Workspace {
    Workspace {
        panes,
        ..Workspace::default()
    }
}

fn with_role(id: &str, agent: AgentId, role: PaneRole) -> Pane {
    Pane {
        role: Some(role),
        ..pane(id, id, agent)
    }
}

fn part(ws: &Workspace, id: &str) -> Option<Part> {
    let entry = ws.panes.iter().find(|entry| entry.id == id).unwrap();
    part_in_orchestration(ws, entry)
}

#[test]
fn only_a_free_agent_pane_of_either_kind_can_become_the_orchestrator() {
    let ws = workspace(vec![
        pane("claude", "Claude", AgentId::Claude),
        pane("codex", "Codex", AgentId::Codex),
        pane("shell", "Shell", AgentId::Shell),
        with_role("worker", AgentId::Claude, PaneRole::Worker),
        with_role("manager", AgentId::Claude, PaneRole::Manager),
    ]);
    assert_eq!(part(&ws, "claude"), Some(Part::Free));
    assert_eq!(part(&ws, "codex"), Some(Part::Free));
    for other in ["shell", "worker", "manager"] {
        assert_eq!(part(&ws, other), None, "{other}");
    }
}

#[test]
fn one_orchestrator_per_workspace_and_none_in_the_hq() {
    let mut taken = workspace(vec![
        pane("claude", "Claude", AgentId::Claude),
        with_role("lead", AgentId::Claude, PaneRole::Orchestrator),
    ]);
    // The orchestrator offers to let go; the others can no longer be one.
    assert_eq!(part(&taken, "lead"), Some(Part::Orchestrator));
    assert_eq!(part(&taken, "claude"), None);

    taken.panes.pop();
    taken.hq = Some(true);
    assert_eq!(part(&taken, "claude"), None);
}

#[test]
fn the_claude_account_block_comes_with_its_own_separator_and_heading() {
    let harness = Harness::new(Locale::En, "graphite");
    let ctx = harness.ctx();
    // The accounts package starts its block with a separator and its heading:
    // what this menu appends is that block, not that block under a second pair.
    let block = crate::accounts::menu_items(ctx, &"w1".to_string(), Some(&"p1".to_string()));
    assert!(matches!(
        block.as_slice(),
        [MenuEntry::Separator, MenuEntry::Heading(_), ..]
    ));
    let separators = block
        .iter()
        .filter(|entry| matches!(entry, MenuEntry::Separator))
        .count();
    assert_eq!(separators, 1);
    let headings = block
        .iter()
        .filter(|entry| matches!(entry, MenuEntry::Heading(_)))
        .count();
    assert_eq!(headings, 1);
    assert_eq!(set_off(block.clone()).len(), block.len());
}

#[test]
fn a_block_that_comes_without_a_separator_gets_one_and_an_empty_block_none() {
    let heading = || MenuEntry::Heading("Account".into());
    assert!(matches!(
        set_off(vec![heading()]).as_slice(),
        [MenuEntry::Separator, MenuEntry::Heading(_)]
    ));
    assert!(matches!(
        set_off(vec![MenuEntry::Separator, heading()]).as_slice(),
        [MenuEntry::Separator, MenuEntry::Heading(_)]
    ));
    assert!(set_off(Vec::new()).is_empty());
}

#[test]
fn a_pane_that_does_not_run_claude_code_has_no_account_block() {
    let harness = Harness::new(Locale::En, "graphite");
    // `p2` is a shell.
    let block =
        crate::accounts::menu_items(harness.ctx(), &"w1".to_string(), Some(&"p2".to_string()));
    assert!(block.is_empty());
}

mod pin {
    //! The line that pins a pane to its edge of the grid, or lets it go,
    //! picked as the menu opens: from the keyboard, the arrows down to it,
    //! then Enter.

    use iced::keyboard::key::Named;

    use super::*;
    use crate::app::Packages;
    use crate::navigator::testing::{ctx, model_of, simulate, workspace};
    use crate::state::types::AppState;
    use crate::state::{Model, StoreAction};

    const WS: &str = "pin-menu";

    fn two_shells() -> Model {
        model_of(AppState {
            workspaces: vec![workspace(
                WS,
                "Shop",
                None,
                vec![
                    pane("leo", "Leo", AgentId::Shell),
                    pane("term", "Term", AgentId::Shell),
                ],
            )],
            active_workspace_id: Some(WS.into()),
            ..AppState::default()
        })
    }

    /// What picking the line of `pane`'s menu labelled `label` sends.
    fn pick(m: &Model, pane: &str, label: &str) -> Vec<Message> {
        let pkg = Packages::default();
        let ctx = ctx(m, &pkg);
        let lines = entries(ctx, &pane.to_string(), Folded::default());
        // Every line here takes the highlight: the arrows count them all.
        let at = lines
            .iter()
            .filter(|entry| matches!(entry, menu::Entry::Item(_)))
            .position(|entry| matches!(entry, menu::Entry::Item(item) if item.label() == label))
            .unwrap_or_else(|| panic!("{pane}'s menu has « {label} »"));
        let mut ui = simulate(menu::menu(ctx.theme, lines).from_keyboard(true));
        for _ in 0..at {
            ui.tap_key(Named::ArrowDown);
        }
        ui.tap_key(Named::Enter);
        ui.into_messages().collect()
    }

    /// The pin a picked line asks for, once the menu has closed.
    fn pinned(said: &[Message]) -> Option<(String, String, bool)> {
        said.iter().find_map(|message| match message {
            Message::PaneMenu(Msg::Pick(_, action)) => match action.as_ref() {
                Message::Panes(crate::panes::Msg::Pin(ws, pane, on)) => {
                    Some((ws.clone(), pane.clone(), *on))
                }
                _ => None,
            },
            _ => None,
        })
    }

    #[test]
    fn a_pane_is_pinned_from_its_menu_and_let_go_from_the_same_line() {
        let mut m = two_shells();
        let said = pick(&m, "term", "Pin in place");
        assert_eq!(pinned(&said), Some((WS.into(), "term".into(), true)));

        // Pinned, the same line lets it go.
        m.apply(StoreAction::PinPane {
            ws: WS.into(),
            pane: "term".into(),
            edge: None,
            grid: None,
        });
        assert!(m.workspace(WS).unwrap().pin_of("term").is_some());
        let said = pick(&m, "term", "Unpin this pane");
        assert_eq!(pinned(&said), Some((WS.into(), "term".into(), false)));
        // The other pane is not pinned: its menu offers to pin it.
        let said = pick(&m, "leo", "Pin in place");
        assert_eq!(pinned(&said), Some((WS.into(), "leo".into(), true)));
    }

    #[test]
    fn the_pin_line_leads_with_the_pin_and_names_the_command() {
        let m = two_shells();
        let pkg = Packages::default();
        let ctx = ctx(&m, &pkg);
        let lines = entries(ctx, &"term".to_string(), Folded::default());
        let line = lines
            .iter()
            .find_map(|entry| match entry {
                menu::Entry::Item(item) if item.label() == "Pin in place" => Some(item),
                _ => None,
            })
            .expect("the pin line");
        assert_eq!(line.lead(), Some(Icon::Pin));
        // In French too.
        let ctx = Ctx {
            i18n: hangar_i18n::Translator::new(Locale::Fr),
            ..ctx
        };
        let lines = entries(ctx, &"term".to_string(), Folded::default());
        assert!(lines.iter().any(
            |entry| matches!(entry, menu::Entry::Item(item) if item.label() == "Épingler à sa place")
        ));
    }
}

mod by_agent {
    //! The lines a pane's agent gives its menu: its sessions, a pane of the
    //! other agent beside it, its models; greyed with why when the agent
    //! they need is not on this computer.

    use super::*;
    use crate::app::Packages;
    use crate::navigator::testing::{ctx, model_of, workspace};
    use crate::orchestration_ui::models;
    use crate::state::types::{AgentKind, AppState};
    use crate::state::{Model, StoreAction};

    const WS: &str = "agents-menu";

    fn two_agents() -> Model {
        let mut model = model_of(AppState {
            workspaces: vec![workspace(
                WS,
                "Shop",
                None,
                vec![
                    pane("ava", "Ava", AgentId::Claude),
                    pane("cody", "Cody", AgentId::Codex),
                ],
            )],
            active_workspace_id: Some(WS.into()),
            ..AppState::default()
        });
        model.ui.claude_installed = true;
        model
    }

    /// The menu of `pane`, as data: each line's label, and why it is greyed.
    fn lines(m: &Model, pane: &str) -> Vec<(String, Option<String>)> {
        let pkg = Packages::default();
        entries(ctx(m, &pkg), &pane.to_string(), Folded::default())
            .iter()
            .filter_map(|entry| match entry {
                menu::Entry::Item(item) => {
                    Some((item.label().to_owned(), item.refusal().map(str::to_owned)))
                }
                menu::Entry::Heading(heading) => Some((heading.clone(), None)),
                menu::Entry::Separator => None,
            })
            .collect()
    }

    fn labels(m: &Model, pane: &str) -> Vec<String> {
        lines(m, pane).into_iter().map(|(label, _)| label).collect()
    }

    /// What picking the line labelled `label` of `pane`'s menu does, once the
    /// menu has closed.
    fn picked(m: &Model, pane: &str, label: &str) -> Message {
        let pkg = Packages::default();
        let lines = entries(ctx(m, &pkg), &pane.to_string(), Folded::default());
        let sent = lines
            .iter()
            .find_map(|entry| match entry {
                menu::Entry::Item(item) if item.label() == label => item.selects().cloned(),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{pane}'s menu picks « {label} »"));
        match sent {
            Message::PaneMenu(Msg::Pick(from, action)) => {
                assert_eq!(from, pane);
                *action
            }
            other => other,
        }
    }

    #[test]
    fn either_agent_resumes_a_session_and_a_shell_has_none() {
        let mut m = two_agents();
        m.apply(StoreAction::AddPane {
            ws: WS.into(),
            agent: Some(AgentId::Shell),
            near: None,
            dir: None,
            cwd: None,
            name: Some("Sam".into()),
            init: None,
        });
        let resume = "Pick the session to resume";
        assert!(labels(&m, "ava").iter().any(|label| label == resume));
        assert!(labels(&m, "cody").iter().any(|label| label == resume));
        let shell = m.workspace(WS).unwrap().panes[2].id.clone();
        assert!(!labels(&m, &shell).iter().any(|label| label == resume));
    }

    #[test]
    fn a_pane_of_the_other_agent_opens_beside_it_greyed_when_not_installed() {
        let m = two_agents();
        // Claude Code is on this computer; Codex is not.
        let codex = lines(&m, "ava")
            .into_iter()
            .find(|(label, _)| label == "New Codex pane (beta)")
            .expect("a Claude Code pane offers a Codex one");
        assert_eq!(
            codex.1.as_deref(),
            Some("Codex is not installed on this computer")
        );
        assert!(
            !labels(&m, "ava")
                .iter()
                .any(|l| l == "New Claude Code pane")
        );

        let new = picked(&m, "cody", "New Claude Code pane");
        let mut m = m;
        let Message::PaneMenu(msg @ Msg::NewPane { .. }) = new else {
            panic!("not a new pane");
        };
        drop(step(&mut State::default(), &mut m, msg));
        let panes = &m.workspace(WS).unwrap().panes;
        assert_eq!(panes.len(), 3);
        assert_eq!(panes[2].agent, AgentId::Claude);
    }

    #[test]
    fn the_model_lines_offer_each_agent_s_own_with_the_one_it_runs_ticked() {
        let mut m = two_agents();
        let ava = labels(&m, "ava");
        for wanted in [
            "Model · Claude Code",
            "Claude Code's default",
            "opus",
            "sonnet",
            "haiku",
        ] {
            assert!(ava.iter().any(|label| label == wanted), "{wanted}: {ava:?}");
        }
        // Codex lists its own: none here until its server has said.
        let cody = labels(&m, "cody");
        assert!(
            cody.iter().any(|label| label == "Model · Codex"),
            "{cody:?}"
        );
        assert!(cody.iter().any(|label| label == "Codex's default"));
        assert!(!cody.iter().any(|label| label == "opus"));
        models::answered(&mut m, AgentKind::Codex, Ok(vec!["gpt-b".into()]));
        assert!(labels(&m, "cody").iter().any(|label| label == "gpt-b"));

        // Another model restarts the pane with it, on its conversation.
        let Message::Intent(Intent::ReplacePane { ws, pane, patch }) = picked(&m, "ava", "sonnet")
        else {
            panic!("a restart");
        };
        assert_eq!((ws.as_str(), pane.as_str()), (WS, "ava"));
        let mut ava = m.workspace(WS).unwrap().panes[0].clone();
        patch.apply(&mut ava);
        assert_eq!(ava.model.as_deref(), Some("sonnet"));

        // The one it runs is ticked, and kept: picking it only closes the menu.
        m.apply(StoreAction::UpdatePane {
            ws: WS.into(),
            pane: "ava".into(),
            patch: Patch::new(|pane: &mut Pane| pane.model = Some("sonnet".into())),
        });
        assert!(matches!(
            picked(&m, "ava", "sonnet"),
            Message::PaneMenu(Msg::Close)
        ));
    }
}
