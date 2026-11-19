//! The folder half of the new-workspace form: the names it derives, the
//! agents assigned to panes, the agents of its orchestration, and the draft
//! it hands the store (`WorkspaceDialog.tsx`).
//!
//! Nothing here asks which agent a kind is (docs/native/agents.md D3): the
//! kinds are listed in the pickers' order (`AgentKind::ALL`), named by their
//! row and the dictionary, and the default orchestrator is the engine's rule
//! (D20, `Settings::agent_for`).

use crate::state::WorkspaceDraft;
use crate::state::types::{AgentId, AgentKind, OrchestrationRole, Settings, automatic_agent};

use super::hooks::HooksDraft;

/// The pane counts offered (`LayoutSize`).
pub const LAYOUTS: [u8; 4] = [1, 2, 4, 8];
pub const DEFAULT_LAYOUT: u8 = 4;

/// Grid shape per layout, mirrored from the window's grid so the preview
/// matches reality.
pub fn preview_columns(layout: u8) -> usize {
    match layout {
        1 => 1,
        2 | 4 => 2,
        _ => 4,
    }
}

/// One folder of a `.code-workspace` (`WorkspaceRoot`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub name: String,
    pub path: String,
    /// A workspace file outlives the folders it points at; a missing one is
    /// shown, and cannot be the main one.
    pub exists: bool,
}

/// The last part of a path (the path itself when it has none).
pub fn basename(path: &str) -> String {
    let trimmed = path.trim_end_matches(['\\', '/']);
    match trimmed.rsplit(['\\', '/']).next() {
        Some(last) if !last.is_empty() => last.to_string(),
        _ => path.to_string(),
    }
}

/// `My project.code-workspace` names a workspace `My project`.
pub fn workspace_file_name(path: &str) -> String {
    let name = basename(path);
    let suffix = ".code-workspace";
    match name.len().checked_sub(suffix.len()) {
        Some(at) if name.is_char_boundary(at) && name[at..].eq_ignore_ascii_case(suffix) => {
            name[..at].to_string()
        }
        _ => name,
    }
}

/// The first existing root is the one the terminals open in, when the file
/// has no better idea: terminals need a folder that is really there, so a
/// file whose first entry is missing on this machine falls through to the
/// first one that is not.
pub fn main_root(roots: &[Root]) -> Option<&Root> {
    roots.iter().find(|root| root.exists)
}

// ── the agents on this machine ────────────────────────────────────────

/// The agents found on this machine (`hangar_core::agent::detect`), which
/// the pickers offer; the others are greyed, with the reason.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Installed(Vec<AgentKind>);

impl Installed {
    pub fn of(kinds: impl IntoIterator<Item = AgentKind>) -> Installed {
        Installed(kinds.into_iter().collect())
    }

    /// Every kind: what the form assumes until the machine has answered,
    /// which is what it did before it asked (an orchestrator by default).
    pub fn every() -> Installed {
        Installed::of(AgentKind::ALL)
    }

    pub fn has(&self, kind: AgentKind) -> bool {
        self.0.contains(&kind)
    }

    /// Whether a pane of `agent` starts here: a shell always does.
    pub fn runs(&self, agent: AgentId) -> bool {
        agent.kind().is_none_or(|kind| self.has(kind))
    }

    pub fn any(&self) -> bool {
        !self.0.is_empty()
    }

    /// The kinds that are not here, in the pickers' order: each gets a link
    /// to the setup.
    pub fn missing(&self) -> impl Iterator<Item = AgentKind> + '_ {
        AgentKind::ALL.into_iter().filter(|kind| !self.has(*kind))
    }
}

/// The dictionary id of the name a picker gives `kind`: its own name, with
/// « beta » while its support is one (docs/native/agents.md, decision 5).
/// The dictionary holds it rather than a test on the kind here.
pub fn picker_label_id(kind: AgentKind) -> String {
    format!("create.agent.{}", kind.id())
}

// ── the agents assigned to the panes ──────────────────────────────────

/// One entry per pane: the source of truth the counters and the preview
/// share.
pub fn initial_assignments(layout: u8) -> Vec<AgentId> {
    vec![AgentId::Shell; usize::from(layout)]
}

/// The assignments for another number of panes: cut, or padded with shells.
pub fn resize(assignments: &mut Vec<AgentId>, layout: u8) {
    assignments.resize(usize::from(layout), AgentId::Shell);
}

/// Gives the first free pane (a plain shell) to `agent`.
pub fn add_agent(assignments: &mut [AgentId], agent: AgentId) {
    if let Some(slot) = assignments.iter_mut().find(|slot| **slot == AgentId::Shell) {
        *slot = agent;
    }
}

/// Takes the last pane of `agent` back to a shell.
pub fn remove_agent(assignments: &mut [AgentId], agent: AgentId) {
    if let Some(slot) = assignments.iter_mut().rev().find(|slot| **slot == agent) {
        *slot = AgentId::Shell;
    }
}

/// What a pane of the preview can run, in the order a click goes through
/// them: a shell, then each agent of this machine in the pickers' order.
pub fn pane_choices(installed: &Installed) -> Vec<AgentId> {
    std::iter::once(AgentId::Shell)
        .chain(
            AgentKind::ALL
                .into_iter()
                .filter(|kind| installed.has(*kind))
                .map(AgentId::from),
        )
        .collect()
}

/// Clicking a preview cell cycles that single pane through the choices
/// ([`pane_choices`]): each agent installed, then back to a shell. A pane of
/// an agent that is not (any more) here goes to the first choice after the
/// shell.
pub fn cycle_cell(assignments: &mut [AgentId], index: usize, installed: &Installed) {
    let Some(slot) = assignments.get_mut(index) else {
        return;
    };
    let choices = pane_choices(installed);
    let at = choices.iter().position(|agent| agent == slot).unwrap_or(0);
    *slot = choices[(at + 1) % choices.len()];
}

pub fn count_of(assignments: &[AgentId], agent: AgentId) -> usize {
    assignments
        .iter()
        .filter(|assigned| **assigned == agent)
        .count()
}

// ── what the workspace starts with ────────────────────────────────────

/// What a workspace made from a folder starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// The default: one pane of the orchestrator's agent
    /// ([`orchestrator_of`]), the workspace's orchestrator, ready to be
    /// talked to.
    Orchestrator,
    /// The default when no agent is installed: one plain shell, since an
    /// orchestrator is an agent.
    Shell,
    /// The panes chosen under « More options », made exactly as they are
    /// chosen there.
    Custom,
}

/// The panes picked by hand win; else an orchestrator, when an agent can
/// run it.
pub fn start_of(custom: bool, orchestrator_runs: bool) -> Start {
    match (custom, orchestrator_runs) {
        (true, _) => Start::Custom,
        (false, true) => Start::Orchestrator,
        (false, false) => Start::Shell,
    }
}

/// The agent the orchestrator runs (D20): the one picked in the form, else
/// the settings' default, else Claude Code when it is installed and Codex
/// when it alone is (`Settings::agent_for`). An agent that is not installed
/// is not offered: a default naming one gives way to the rule. `None` when
/// no agent is installed: the workspace starts with a shell.
pub fn orchestrator_of(
    picked: Option<AgentKind>,
    settings: &Settings,
    installed: &Installed,
) -> Option<AgentKind> {
    if let Some(picked) = picked.filter(|kind| installed.has(*kind)) {
        return Some(picked);
    }
    if !installed.any() {
        return None;
    }
    let here = |kind: AgentKind| installed.has(kind);
    let default = settings.agent_for(OrchestrationRole::Orchestrator, None, None, here);
    Some(if here(default) {
        default
    } else {
        automatic_agent(here)
    })
}

/// What the workers run, as the form offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workers {
    /// « Same as the orchestrator ».
    Same,
    Agent(AgentKind),
}

/// The workers' choice the form shows: the one picked, else the settings'
/// default for workers, else « Same as the orchestrator ». An agent that is
/// not installed is not offered: the orchestrator's then.
pub fn workers_of(picked: Option<Workers>, settings: &Settings, installed: &Installed) -> Workers {
    let chosen = picked.unwrap_or_else(|| {
        settings
            .default_agent(OrchestrationRole::Worker)
            .map_or(Workers::Same, Workers::Agent)
    });
    match chosen {
        Workers::Agent(kind) if !installed.has(kind) => Workers::Same,
        chosen => chosen,
    }
}

/// The agents a new workspace's orchestration is written with (T13's
/// `orchestratorAgent` and `workerAgent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Agents {
    pub orchestrator: Option<AgentKind>,
    pub workers: Option<AgentKind>,
}

/// What the workspace keeps of the form's choice of agents, when it starts
/// with an orchestrator: the orchestrator's agent, the one its pane runs;
/// the workers', none for « Same as the orchestrator » — which is what none
/// reads as (`Settings::agent_for`), unless the settings name a default for
/// workers, in which case the orchestrator's agent is written instead.
///
/// `None` for a workspace that starts with a shell or with panes picked by
/// hand: it chose no orchestrator, and reads the defaults.
pub fn agents_of(
    start: Start,
    orchestrator: Option<AgentKind>,
    workers: Workers,
    settings: &Settings,
) -> Option<Agents> {
    let orchestrator = orchestrator.filter(|_| start == Start::Orchestrator)?;
    let workers = match workers {
        Workers::Agent(kind) => Some(kind),
        Workers::Same => settings
            .default_agent(OrchestrationRole::Worker)
            .map(|_| orchestrator),
    };
    Some(Agents {
        orchestrator: Some(orchestrator),
        workers,
    })
}

// ── the draft ─────────────────────────────────────────────────────────

pub struct FolderForm<'a> {
    pub name: &'a str,
    pub cwd: &'a str,
    pub roots: &'a [Root],
    pub start: Start,
    /// The orchestrator's agent ([`orchestrator_of`]), read for
    /// [`Start::Orchestrator`] only.
    pub orchestrator: Option<AgentKind>,
    /// The panes of « More options », read for [`Start::Custom`] only.
    pub layout: u8,
    pub assignments: &'a [AgentId],
    pub shell_id: Option<&'a str>,
    pub kits: &'a [crate::state::types::WorkspaceKit],
    pub claude_account: Option<&'a str>,
}

/// What `addWorkspace` is given, or `None` without a folder.
pub fn draft_of(form: &FolderForm) -> Option<WorkspaceDraft> {
    if form.cwd.is_empty() {
        return None;
    }
    let (layout, agents) = match form.start {
        Start::Orchestrator => (
            1,
            vec![form.orchestrator.map_or(AgentId::Shell, AgentId::from)],
        ),
        Start::Shell => (1, vec![AgentId::Shell]),
        Start::Custom => (form.layout, form.assignments.to_vec()),
    };
    Some(WorkspaceDraft {
        name: match form.name.trim() {
            "" => basename(form.cwd),
            name => name.to_string(),
        },
        cwd: form.cwd.to_string(),
        // `cwd` is the root the terminals get; the others only ride along for
        // the agents that can read outside it. Missing folders are never
        // passed on.
        extra_roots: form
            .roots
            .iter()
            .filter(|root| root.exists && root.path != form.cwd)
            .map(|root| root.path.clone())
            .collect(),
        layout,
        agents,
        shell_id: form.shell_id.map(str::to_string),
        folder_id: None,
        // Written only when set: absent rather than empty.
        kits: (!form.kits.is_empty()).then(|| form.kits.to_vec()),
        claude_account: form.claude_account.map(str::to_string),
    })
}

/// The hooks of the new workspace, when it has any.
pub fn hooks_of(draft: &HooksDraft) -> Option<crate::state::types::WorktreeHooks> {
    draft.clean()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str, path: &str, exists: bool) -> Root {
        Root {
            name: name.into(),
            path: path.into(),
            exists,
        }
    }

    #[test]
    fn a_path_is_named_by_its_last_part() {
        assert_eq!(basename("/home/me/shop"), "shop");
        assert_eq!(basename("/home/me/shop/"), "shop");
        assert_eq!(basename("C:\\dev\\shop\\"), "shop");
        assert_eq!(basename("/"), "/");
        assert_eq!(basename("shop"), "shop");
    }

    #[test]
    fn a_workspace_file_names_its_workspace_without_the_extension() {
        assert_eq!(
            workspace_file_name("/x/My project.code-workspace"),
            "My project"
        );
        assert_eq!(workspace_file_name("C:\\x\\App.CODE-WORKSPACE"), "App");
        assert_eq!(workspace_file_name("/x/notes.txt"), "notes.txt");
    }

    #[test]
    fn the_main_root_is_the_first_one_that_exists() {
        let roots = [
            root("gone", "/gone", false),
            root("api", "/api", true),
            root("web", "/web", true),
        ];
        assert_eq!(
            main_root(&roots).map(|root| root.path.as_str()),
            Some("/api")
        );
        assert_eq!(main_root(&[root("gone", "/gone", false)]), None);
        assert_eq!(main_root(&[]), None);
    }

    #[test]
    fn the_preview_has_the_columns_of_the_window_grid() {
        let columns: Vec<_> = LAYOUTS
            .iter()
            .map(|layout| preview_columns(*layout))
            .collect();
        assert_eq!(columns, [1, 2, 2, 4]);
    }

    #[test]
    fn a_new_layout_cuts_or_pads_with_shells() {
        let mut assignments = initial_assignments(4);
        assert_eq!(assignments, [AgentId::Shell; 4]);
        add_agent(&mut assignments, AgentId::Claude);
        add_agent(&mut assignments, AgentId::Claude);
        resize(&mut assignments, 2);
        assert_eq!(assignments, [AgentId::Claude, AgentId::Claude]);
        resize(&mut assignments, 8);
        assert_eq!(assignments.len(), 8);
        assert_eq!(count_of(&assignments, AgentId::Shell), 6);
        assert_eq!(count_of(&assignments, AgentId::Claude), 2);
        resize(&mut assignments, 1);
        assert_eq!(assignments, [AgentId::Claude]);
    }

    #[test]
    fn an_agent_takes_the_first_free_pane_and_gives_back_the_last_it_had() {
        let mut assignments = initial_assignments(4);
        add_agent(&mut assignments, AgentId::Claude);
        add_agent(&mut assignments, AgentId::Claude);
        add_agent(&mut assignments, AgentId::Claude);
        assert_eq!(
            assignments,
            [
                AgentId::Claude,
                AgentId::Claude,
                AgentId::Claude,
                AgentId::Shell
            ]
        );
        remove_agent(&mut assignments, AgentId::Claude);
        assert_eq!(
            assignments,
            [
                AgentId::Claude,
                AgentId::Claude,
                AgentId::Shell,
                AgentId::Shell
            ]
        );
        cycle_cell(&mut assignments, 3, &Installed::every());
        remove_agent(&mut assignments, AgentId::Claude);
        assert_eq!(
            assignments,
            [
                AgentId::Claude,
                AgentId::Claude,
                AgentId::Shell,
                AgentId::Shell
            ]
        );
        // Every pane taken: nothing to take it, no change.
        let mut full = vec![AgentId::Claude; 2];
        add_agent(&mut full, AgentId::Claude);
        assert_eq!(full, [AgentId::Claude; 2]);
        // Nothing to take back: no change either.
        let mut empty = initial_assignments(2);
        remove_agent(&mut empty, AgentId::Claude);
        assert_eq!(empty, [AgentId::Shell; 2]);
    }

    /// The cell's agent after each of `clicks` clicks on a shell.
    fn cycled(installed: &Installed, clicks: usize) -> Vec<AgentId> {
        let mut assignments = initial_assignments(2);
        let mut seen = Vec::new();
        for _ in 0..clicks {
            cycle_cell(&mut assignments, 0, installed);
            seen.push(assignments[0]);
        }
        assert_eq!(assignments[1], AgentId::Shell, "the other cell is left");
        seen
    }

    #[test]
    fn a_cell_cycles_through_the_agents_installed_in_the_order_of_the_pickers() {
        assert_eq!(
            cycled(&Installed::every(), 3),
            [AgentId::Claude, AgentId::Codex, AgentId::Shell]
        );
        assert_eq!(
            cycled(&Installed::of([AgentKind::Codex]), 2),
            [AgentId::Codex, AgentId::Shell]
        );
        assert_eq!(
            cycled(&Installed::of([AgentKind::Claude]), 2),
            [AgentId::Claude, AgentId::Shell]
        );
        // Nothing to run but shells: a click changes nothing.
        assert_eq!(cycled(&Installed::default(), 1), [AgentId::Shell]);
        // A pane of an agent gone from the machine moves on to one that is.
        let mut assignments = vec![AgentId::Claude];
        cycle_cell(&mut assignments, 0, &Installed::of([AgentKind::Codex]));
        assert_eq!(assignments, [AgentId::Codex]);
        // Out of the grid: nothing.
        cycle_cell(&mut assignments, 9, &Installed::every());
        assert_eq!(assignments, [AgentId::Codex]);
    }

    #[test]
    fn what_is_installed_is_what_runs_and_the_rest_is_missing() {
        let codex = Installed::of([AgentKind::Codex]);
        assert!(codex.any());
        assert!(codex.runs(AgentId::Codex));
        assert!(codex.runs(AgentId::Shell));
        assert!(!codex.runs(AgentId::Claude));
        assert_eq!(codex.missing().collect::<Vec<_>>(), [AgentKind::Claude]);
        let none = Installed::default();
        assert!(!none.any());
        assert!(none.runs(AgentId::Shell));
        assert_eq!(none.missing().collect::<Vec<_>>(), AgentKind::ALL);
        assert_eq!(Installed::every().missing().count(), 0);
        assert_eq!(
            pane_choices(&Installed::every()),
            [AgentId::Shell, AgentId::Claude, AgentId::Codex]
        );
    }

    #[test]
    fn every_kind_has_its_picker_name_in_both_languages() {
        use hangar_i18n::{Locale, Translator};
        for kind in AgentKind::ALL {
            let id = picker_label_id(kind);
            for locale in Locale::ALL {
                let name = Translator::new(locale).t(id.as_str());
                assert_ne!(name, id, "{locale:?} {id}");
                assert!(name.starts_with(kind.info().label), "{locale:?} {name}");
            }
        }
        let en = Translator::new(Locale::En);
        assert_eq!(
            en.t(picker_label_id(AgentKind::Claude).as_str()),
            "Claude Code"
        );
        assert_eq!(
            en.t(picker_label_id(AgentKind::Codex).as_str()),
            "Codex (beta)"
        );
    }

    fn installed(kinds: &[AgentKind]) -> Installed {
        Installed::of(kinds.iter().copied())
    }

    /// D20: nobody chose, so Claude Code when both are installed, else
    /// whichever is; nothing without an agent.
    #[test]
    fn the_orchestrator_is_claude_code_when_both_are_there_else_the_one_that_is() {
        use AgentKind::{Claude, Codex};
        let settings = Settings::default();
        assert_eq!(
            orchestrator_of(None, &settings, &installed(&[Claude, Codex])),
            Some(Claude)
        );
        assert_eq!(
            orchestrator_of(None, &settings, &installed(&[Codex])),
            Some(Codex)
        );
        assert_eq!(
            orchestrator_of(None, &settings, &installed(&[Claude])),
            Some(Claude)
        );
        assert_eq!(orchestrator_of(None, &settings, &installed(&[])), None);
        assert_eq!(
            orchestrator_of(Some(Codex), &settings, &installed(&[])),
            None
        );
    }

    #[test]
    fn the_orchestrator_picked_wins_then_the_settings_default_while_installed() {
        use AgentKind::{Claude, Codex};
        let both = installed(&[Claude, Codex]);
        let mut settings = Settings::default();
        assert_eq!(orchestrator_of(Some(Codex), &settings, &both), Some(Codex));
        // Picked, then gone from the machine: the rule again.
        assert_eq!(
            orchestrator_of(Some(Codex), &settings, &installed(&[Claude])),
            Some(Claude)
        );
        settings.default_orchestrator_agent = Some(Codex);
        assert_eq!(orchestrator_of(None, &settings, &both), Some(Codex));
        assert_eq!(
            orchestrator_of(Some(Claude), &settings, &both),
            Some(Claude)
        );
        // A default the machine lacks is not offered: the rule.
        assert_eq!(
            orchestrator_of(None, &settings, &installed(&[Claude])),
            Some(Claude)
        );
    }

    #[test]
    fn the_workers_follow_the_orchestrator_unless_picked_or_set_in_the_settings() {
        use AgentKind::{Claude, Codex};
        let both = installed(&[Claude, Codex]);
        let mut settings = Settings::default();
        assert_eq!(workers_of(None, &settings, &both), Workers::Same);
        assert_eq!(
            workers_of(Some(Workers::Agent(Codex)), &settings, &both),
            Workers::Agent(Codex)
        );
        settings.default_worker_agent = Some(Codex);
        assert_eq!(workers_of(None, &settings, &both), Workers::Agent(Codex));
        assert_eq!(
            workers_of(Some(Workers::Same), &settings, &both),
            Workers::Same
        );
        // Codex is not here: the orchestrator's agent, whatever was set.
        assert_eq!(
            workers_of(None, &settings, &installed(&[Claude])),
            Workers::Same
        );
    }

    #[test]
    fn the_workspace_keeps_its_orchestrators_agent_and_the_workers_choice() {
        use AgentKind::{Claude, Codex};
        let mut settings = Settings::default();
        let kept =
            |start, workers, settings: &Settings| agents_of(start, Some(Codex), workers, settings);
        assert_eq!(
            kept(Start::Orchestrator, Workers::Same, &settings),
            Some(Agents {
                orchestrator: Some(Codex),
                workers: None,
            })
        );
        assert_eq!(
            kept(Start::Orchestrator, Workers::Agent(Claude), &settings),
            Some(Agents {
                orchestrator: Some(Codex),
                workers: Some(Claude),
            })
        );
        // « Same » over a default for workers is written out, or the
        // default would be read instead.
        settings.default_worker_agent = Some(Claude);
        assert_eq!(
            kept(Start::Orchestrator, Workers::Same, &settings),
            Some(Agents {
                orchestrator: Some(Codex),
                workers: Some(Codex),
            })
        );
        // No orchestrator made: nothing chosen.
        assert_eq!(kept(Start::Custom, Workers::Same, &settings), None);
        assert_eq!(kept(Start::Shell, Workers::Same, &settings), None);
        assert_eq!(
            agents_of(Start::Orchestrator, None, Workers::Same, &settings),
            None
        );
    }

    fn form<'a>(name: &'a str, cwd: &'a str, roots: &'a [Root]) -> FolderForm<'a> {
        FolderForm {
            name,
            cwd,
            roots,
            start: Start::Custom,
            orchestrator: Some(AgentKind::Claude),
            layout: 4,
            assignments: &[],
            shell_id: None,
            kits: &[],
            claude_account: None,
        }
    }

    #[test]
    fn panes_picked_by_hand_win_else_an_orchestrator_when_claude_code_runs() {
        assert_eq!(start_of(false, true), Start::Orchestrator);
        assert_eq!(start_of(false, false), Start::Shell);
        assert_eq!(start_of(true, true), Start::Custom);
        assert_eq!(start_of(true, false), Start::Custom);
    }

    #[test]
    fn the_default_is_one_claude_code_pane_and_its_fallback_one_shell() {
        let assignments = [
            AgentId::Shell,
            AgentId::Claude,
            AgentId::Shell,
            AgentId::Shell,
        ];
        let mut fields = form("", "/x", &[]);
        fields.assignments = &assignments;
        fields.start = Start::Orchestrator;
        let draft = draft_of(&fields).expect("a draft");
        assert_eq!((draft.layout, draft.agents), (1, vec![AgentId::Claude]));
        // The orchestrator's agent is the one chosen.
        fields.orchestrator = Some(AgentKind::Codex);
        let draft = draft_of(&fields).expect("a draft");
        assert_eq!((draft.layout, draft.agents), (1, vec![AgentId::Codex]));
        fields.start = Start::Shell;
        let draft = draft_of(&fields).expect("a draft");
        assert_eq!((draft.layout, draft.agents), (1, vec![AgentId::Shell]));
        // « More options » makes what is chosen there, as before.
        fields.start = Start::Custom;
        let draft = draft_of(&fields).expect("a draft");
        assert_eq!((draft.layout, draft.agents), (4, assignments.to_vec()));
    }

    #[test]
    fn without_a_folder_there_is_nothing_to_create() {
        assert!(draft_of(&form("x", "", &[])).is_none());
    }

    #[test]
    fn the_name_defaults_to_the_folders_and_blanks_are_not_a_name() {
        let draft = draft_of(&form("  ", "/home/me/shop", &[])).expect("a draft");
        assert_eq!(draft.name, "shop");
        let draft = draft_of(&form(" The shop ", "/home/me/shop", &[])).expect("a draft");
        assert_eq!(draft.name, "The shop");
    }

    #[test]
    fn only_the_other_existing_roots_ride_along() {
        let roots = [
            root("api", "/api", true),
            root("web", "/web", true),
            root("gone", "/gone", false),
        ];
        let draft = draft_of(&form("", "/api", &roots)).expect("a draft");
        assert_eq!(draft.extra_roots, ["/web"]);
        assert_eq!(draft.cwd, "/api");
    }

    #[test]
    fn kits_and_account_are_written_only_when_set() {
        let mut fields = form("", "/x", &[]);
        let draft = draft_of(&fields).expect("a draft");
        assert!(draft.kits.is_none());
        assert!(draft.claude_account.is_none());
        let kits = [crate::state::types::WorkspaceKit {
            id: "kit-editor".into(),
            granted: vec![],
        }];
        fields.kits = &kits;
        fields.claude_account = Some("perso");
        fields.shell_id = Some("bash");
        let draft = draft_of(&fields).expect("a draft");
        assert_eq!(draft.kits.as_ref().map(Vec::len), Some(1));
        assert_eq!(draft.claude_account.as_deref(), Some("perso"));
        assert_eq!(draft.shell_id.as_deref(), Some("bash"));
    }
}
