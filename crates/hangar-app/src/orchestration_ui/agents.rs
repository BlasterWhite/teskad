//! Which agent each part of a workspace's orchestration runs, as the toolbar
//! and the panel say it (docs/native/agents.md D11, D20): the pinned
//! orchestrator's own agent, else the one a new orchestrator would start; its
//! workers' as the workspace chose, else as the settings' default, else the
//! orchestrator's (`Settings::agent_for`, the rule the workers are started
//! by). Codex is named with its « beta » (`accounts::agent_label`).

use hangar_i18n::{Key, Translator, args};

use crate::state::Model;
use crate::state::roles::{orchestration_of, orchestrator_of};
use crate::state::types::{AgentKind, OrchestrationRole, OrchestrationSettings, Workspace};

/// The agents of a workspace's orchestration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Agents {
    pub orchestrator: AgentKind,
    pub workers: AgentKind,
}

/// The agents `ws`'s orchestration runs, as it is saved.
pub fn of(m: &Model, ws: &Workspace) -> Agents {
    with(m, ws, &orchestration_of(ws))
}

/// The agents `ws`'s orchestration would run with `orchestration`: what the
/// panel's draft stands for.
pub fn with(m: &Model, ws: &Workspace, orchestration: &OrchestrationSettings) -> Agents {
    let installed = |kind| crate::shell::setup::installed(m, kind);
    let settings = m.settings();
    let orchestrator = orchestrator_of(ws)
        .and_then(|pane| pane.agent.kind())
        .unwrap_or_else(|| {
            settings.agent_for(
                OrchestrationRole::Orchestrator,
                Some(orchestration),
                None,
                installed,
            )
        });
    let workers = settings.agent_for(
        OrchestrationRole::Worker,
        Some(orchestration),
        Some(orchestrator),
        installed,
    );
    Agents {
        orchestrator,
        workers,
    }
}

impl Agents {
    /// « Orchestrator: Claude Code · workers: Codex (beta) ».
    pub fn sentence(self, t: Translator) -> String {
        t.t_with(
            Key::OrchStatusAgents,
            &args!(
                "orchestrator" => crate::accounts::agent_label(t, self.orchestrator),
                "workers" => crate::accounts::agent_label(t, self.workers),
            ),
        )
        .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use hangar_i18n::Locale;

    use super::*;
    use crate::navigator::testing::{model_of, pane, workspace};
    use crate::state::types::{AgentId, AppState, PaneRole};

    fn shop(orchestrator: Option<AgentId>, chosen: OrchestrationSettings) -> Model {
        let mut ws = workspace("w", "Shop", None, Vec::new());
        if let Some(agent) = orchestrator {
            let mut lead = pane("lead", "Leo", agent);
            lead.role = Some(PaneRole::Orchestrator);
            ws.panes.push(lead);
        }
        ws.orchestration = Some(chosen);
        let mut model = model_of(AppState {
            workspaces: vec![ws],
            ..AppState::default()
        });
        model.ui.claude_installed = true;
        model.ui.codex_installed = true;
        model
    }

    fn agents(model: &Model) -> Agents {
        of(model, model.workspace("w").unwrap())
    }

    #[test]
    fn the_workers_run_the_pinned_orchestrator_s_agent_unless_the_workspace_chose() {
        let model = shop(Some(AgentId::Codex), OrchestrationSettings::default());
        assert_eq!(
            agents(&model),
            Agents {
                orchestrator: AgentKind::Codex,
                workers: AgentKind::Codex,
            }
        );
        let chosen = OrchestrationSettings {
            worker_agent: Some(AgentKind::Claude),
            ..OrchestrationSettings::default()
        };
        let model = shop(Some(AgentId::Codex), chosen);
        assert_eq!(agents(&model).workers, AgentKind::Claude);
    }

    #[test]
    fn without_an_orchestrator_it_names_the_one_a_new_orchestrator_would_start() {
        // Both installed, nothing chosen: Claude Code, as it always was.
        let model = shop(None, OrchestrationSettings::default());
        assert_eq!(agents(&model).orchestrator, AgentKind::Claude);
        let chosen = OrchestrationSettings {
            orchestrator_agent: Some(AgentKind::Codex),
            ..OrchestrationSettings::default()
        };
        let model = shop(None, chosen);
        assert_eq!(
            agents(&model),
            Agents {
                orchestrator: AgentKind::Codex,
                workers: AgentKind::Codex,
            }
        );
    }

    #[test]
    fn codex_is_said_with_its_beta_in_both_languages() {
        let agents = Agents {
            orchestrator: AgentKind::Claude,
            workers: AgentKind::Codex,
        };
        assert_eq!(
            agents.sentence(Translator::new(Locale::En)),
            "Orchestrator: Claude Code · workers: Codex (beta)"
        );
        assert!(
            agents
                .sentence(Translator::new(Locale::Fr))
                .contains("Codex (bêta)")
        );
    }
}
