//! The playbook the panel writes and checks: into the file of each agent the
//! workspace runs, `AGENTS.md` for Codex, `CLAUDE.md` for Claude Code.

use super::*;
use crate::app::Packages;
use crate::navigator::testing::{ctx, model_of, pane, simulate, workspace};
use crate::state::types::{AgentId, AppState, OrchestrationSettings};

const CLAUDE: AgentKind = AgentKind::Claude;
const CODEX: AgentKind = AgentKind::Codex;

fn nothing_installed(_: AgentKind) -> bool {
    false
}

fn both_installed(_: AgentKind) -> bool {
    true
}

fn run_by(panes: Vec<(&str, AgentId)>) -> Workspace {
    let panes = panes
        .into_iter()
        .map(|(id, agent)| pane(id, id, agent))
        .collect();
    workspace("w", "Studio", None, panes)
}

fn orchestrated(
    mut ws: Workspace,
    orchestrator: Option<AgentKind>,
    worker: Option<AgentKind>,
    reviewer: Option<AgentKind>,
) -> Workspace {
    ws.orchestration = Some(OrchestrationSettings {
        orchestrator_agent: orchestrator,
        worker_agent: worker,
        reviewer_agent: reviewer,
        ..OrchestrationSettings::default()
    });
    ws
}

#[test]
fn a_codex_pane_puts_codex_among_the_agents_whatever_is_installed() {
    let ws = run_by(vec![("a", AgentId::Codex), ("s", AgentId::Shell)]);
    assert_eq!(playbook_agents(Some(&ws), nothing_installed), [CODEX]);
    assert_eq!(playbook_agents(Some(&ws), both_installed), [CODEX]);

    let mixed = run_by(vec![("a", AgentId::Codex), ("b", AgentId::Claude)]);
    assert_eq!(
        playbook_agents(Some(&mixed), nothing_installed),
        [CLAUDE, CODEX]
    );
}

#[test]
fn a_claude_only_workspace_stays_claude_only_even_with_codex_installed() {
    let ws = run_by(vec![("a", AgentId::Claude), ("b", AgentId::Claude)]);
    assert_eq!(playbook_agents(Some(&ws), both_installed), [CLAUDE]);
    assert_eq!(playbook_agents(Some(&ws), nothing_installed), [CLAUDE]);
}

#[test]
fn the_agents_the_orchestration_chose_count_with_the_panes() {
    // Shells alone, the reviewer is Codex: its deliveries are read by an agent
    // that reads `AGENTS.md`.
    let ws = orchestrated(run_by(vec![("s", AgentId::Shell)]), None, None, Some(CODEX));
    assert_eq!(playbook_agents(Some(&ws), both_installed), [CODEX]);

    // Claude Code on the panes, Codex for the workers.
    let ws = orchestrated(
        run_by(vec![("a", AgentId::Claude)]),
        Some(CLAUDE),
        Some(CODEX),
        None,
    );
    assert_eq!(
        playbook_agents(Some(&ws), nothing_installed),
        [CLAUDE, CODEX]
    );

    // A role the workspace left open names nothing.
    let ws = orchestrated(run_by(vec![("a", AgentId::Claude)]), None, None, None);
    assert_eq!(playbook_agents(Some(&ws), both_installed), [CLAUDE]);
}

#[test]
fn a_workspace_that_names_no_agent_gets_the_installed_ones() {
    let ws = run_by(vec![("s", AgentId::Shell), ("x", AgentId::Shell)]);
    assert_eq!(playbook_agents(Some(&ws), both_installed), [CLAUDE, CODEX]);
    assert_eq!(playbook_agents(Some(&ws), |kind| kind == CODEX), [CODEX]);
    assert_eq!(playbook_agents(Some(&ws), |kind| kind == CLAUDE), [CLAUDE]);

    let bare = orchestrated(ws, None, None, None);
    assert_eq!(playbook_agents(Some(&bare), |kind| kind == CODEX), [CODEX]);
}

#[test]
fn the_agents_are_never_an_empty_list() {
    let ws = run_by(Vec::new());
    assert_eq!(playbook_agents(Some(&ws), nothing_installed), [CLAUDE]);
    assert_eq!(playbook_agents(None, nothing_installed), [CLAUDE]);
    assert_eq!(playbook_agents(None, |kind| kind == CODEX), [CODEX]);
}

#[test]
fn the_files_are_each_agents_instructions_file_once() {
    assert_eq!(playbook_files(&[CLAUDE]), ["CLAUDE.md"]);
    assert_eq!(playbook_files(&[CODEX]), ["AGENTS.md"]);
    assert_eq!(playbook_files(&[CLAUDE, CODEX]), ["CLAUDE.md", "AGENTS.md"]);
    assert_eq!(playbook_files(&[CODEX, CODEX]), ["AGENTS.md"]);
}

/// The panel's line is the dictionary's, with the files it will write in the
/// place of the one the text names: in both languages, or the line says
/// `CLAUDE.md` for a Codex workspace.
#[test]
fn the_hint_lists_the_files_in_each_language() {
    use hangar_i18n::{Locale, Translator};

    for locale in [Locale::En, Locale::Fr] {
        let text = Translator::new(locale).t(Key::McpPlaybookHint);
        let both = playbook_hint(&["CLAUDE.md", "AGENTS.md"], &text);
        assert!(
            both.starts_with("CLAUDE.md, AGENTS.md · "),
            "{locale:?}: {both}"
        );
        assert!(both.ends_with(".gitignore"), "{locale:?}: {both}");

        let codex = playbook_hint(&["AGENTS.md"], &text);
        assert!(codex.starts_with("AGENTS.md · "), "{locale:?}: {codex}");
        assert!(!codex.contains("CLAUDE.md"), "{locale:?}: {codex}");
        assert!(codex.contains(".teskad/"), "{locale:?}: {codex}");
    }
    // A text with no separator is kept whole.
    assert_eq!(playbook_hint(&["AGENTS.md"], "x"), "AGENTS.md · x");
}

/// The model of a workspace `w` in `dir`, with `ws`'s panes and
/// orchestration.
fn model_in(dir: &std::path::Path, mut ws: Workspace) -> Model {
    ws.cwd = dir.to_string_lossy().into_owned();
    model_of(AppState {
        workspaces: vec![ws],
        active_workspace_id: Some("w".into()),
        ..AppState::default()
    })
}

fn opened(m: &Model) -> State {
    let mut st = State::default();
    let _ = open(&mut st, m);
    st
}

fn text_of(dir: &std::path::Path, file: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(file)).ok()
}

fn ids(reports: &[Report]) -> Vec<&str> {
    reports.iter().map(|report| report.id.as_str()).collect()
}

/// What the install button runs, with no tool to configure: the playbook, then
/// the list again.
fn installed_reports(panel: &Install) -> Vec<Report> {
    let message = run(Vec::new(), panel.cwd.clone(), true, &panel.agents);
    match message {
        InstallMsg::Installed { reports, .. } => reports,
        other => panic!("expected the end of the install, got {other:?}"),
    }
}

#[test]
fn a_workspace_with_a_codex_pane_gets_agents_md_written_and_checked() {
    let dir = tempfile::tempdir().unwrap();
    let ws = run_by(vec![("a", AgentId::Codex)]);
    let m = model_in(dir.path(), ws);
    let st = opened(&m);
    let panel = st.install.as_ref().unwrap();
    let cwd = || panel.cwd.clone();
    assert_eq!(panel.agents, [CODEX]);

    assert!(!instructions::agent_instructions_status_for(
        cwd(),
        &panel.agents
    ));

    let reports = installed_reports(panel);
    assert_eq!(ids(&reports), ["AGENTS.md", ".gitignore"], "{reports:?}");
    assert!(reports.iter().all(|report| report.ok), "{reports:?}");
    let agents_md = text_of(dir.path(), "AGENTS.md").expect("AGENTS.md written");
    assert!(agents_md.contains("<!-- TESKAD:START"), "{agents_md}");
    assert!(agents_md.contains("board_claim_task"), "{agents_md}");
    assert!(text_of(dir.path(), "CLAUDE.md").is_none(), "Codex alone");

    // The status the panel shows follows the same files.
    assert!(instructions::agent_instructions_status_for(
        cwd(),
        &panel.agents
    ));
    assert!(!instructions::agent_instructions_status_for(
        cwd(),
        &[CLAUDE]
    ));
    assert!(instructions::agent_instructions_status_for(cwd(), &[CODEX]));
}

#[test]
fn a_workspace_running_both_agents_gets_both_files() {
    let dir = tempfile::tempdir().unwrap();
    let ws = run_by(vec![("a", AgentId::Claude), ("b", AgentId::Codex)]);
    let m = model_in(dir.path(), ws);
    let st = opened(&m);
    let panel = st.install.as_ref().unwrap();
    assert_eq!(panel.agents, [CLAUDE, CODEX]);

    // One file is not enough for the status: each agent reads its own.
    std::fs::write(dir.path().join("CLAUDE.md"), "# Projet\n").unwrap();
    assert!(!instructions::agent_instructions_status_for(
        panel.cwd.clone(),
        &panel.agents
    ));

    let reports = installed_reports(panel);
    assert_eq!(ids(&reports), ["CLAUDE.md", "AGENTS.md", ".gitignore"]);
    assert!(
        text_of(dir.path(), "CLAUDE.md")
            .unwrap()
            .starts_with("# Projet")
    );
    assert!(
        text_of(dir.path(), "AGENTS.md")
            .unwrap()
            .contains("TESKAD:START")
    );
    assert!(instructions::agent_instructions_status_for(
        panel.cwd.clone(),
        &panel.agents
    ));
}

#[test]
fn a_claude_only_workspace_is_written_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let ws = run_by(vec![("a", AgentId::Claude)]);
    // Codex is installed: the workspace does not run it, so it is not asked
    // for.
    let mut m = model_in(dir.path(), ws);
    m.ui.codex_installed = true;
    let st = opened(&m);
    let panel = st.install.as_ref().unwrap();
    assert_eq!(panel.agents, [CLAUDE]);

    let reports = installed_reports(panel);
    assert_eq!(ids(&reports), ["CLAUDE.md", ".gitignore"], "{reports:?}");
    assert!(
        text_of(dir.path(), "CLAUDE.md")
            .unwrap()
            .contains("TESKAD:START")
    );
    assert!(text_of(dir.path(), "AGENTS.md").is_none());

    // What the callers that predate Codex ask for gives the same files.
    let before = tempfile::tempdir().unwrap();
    let cwd = before.path().to_string_lossy().into_owned();
    let old: Vec<String> = instructions::write_agent_instructions(cwd.clone())
        .iter()
        .map(|written| file_name(&written.path))
        .collect();
    assert_eq!(old, ids(&reports));
    assert_eq!(
        text_of(before.path(), "CLAUDE.md"),
        text_of(dir.path(), "CLAUDE.md")
    );
    assert!(instructions::agent_instructions_status(cwd));
}

#[test]
fn opening_on_a_workspace_with_no_agent_takes_the_installed_ones() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = model_in(dir.path(), run_by(vec![("s", AgentId::Shell)]));
    m.ui.claude_installed = false;
    m.ui.codex_installed = true;
    let st = opened(&m);
    assert_eq!(st.install.as_ref().unwrap().agents, [CODEX]);

    m.ui.claude_installed = true;
    let st = opened(&m);
    assert_eq!(st.install.as_ref().unwrap().agents, [CLAUDE, CODEX]);
}

/// Whether `words` is anywhere in the panel `st` draws on `m`.
fn draws(st: &State, m: &Model, words: &str) -> bool {
    let pkg = Packages::default();
    simulate(view(st, ctx(m, &pkg))).find(words).is_ok()
}

#[test]
fn the_panel_lists_the_files_it_will_write() {
    let dir = tempfile::tempdir().unwrap();

    let codex = model_in(dir.path(), run_by(vec![("a", AgentId::Codex)]));
    assert!(draws(
        &opened(&codex),
        &codex,
        "AGENTS.md · adds .teskad/ to .gitignore"
    ));

    let both = model_in(
        dir.path(),
        run_by(vec![("a", AgentId::Claude), ("b", AgentId::Codex)]),
    );
    assert!(draws(
        &opened(&both),
        &both,
        "CLAUDE.md, AGENTS.md · adds .teskad/ to .gitignore"
    ));

    let claude = model_in(dir.path(), run_by(vec![("a", AgentId::Claude)]));
    assert!(draws(
        &opened(&claude),
        &claude,
        "CLAUDE.md · adds .teskad/ to .gitignore"
    ));
}
