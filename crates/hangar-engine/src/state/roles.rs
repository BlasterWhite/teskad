//! Orchestration roles (`src/lib/orchestration.ts`). The pin rule of
//! `lib/orchestrationActions.ts` (`pinRefusal`, `canCreateOrchestrator`) is
//! P10's, in `orchestration_ui::actions`, which the command table asks; what
//! it asks of the pane's agent is here ([`lead_refusal`]).
//!
//! A workspace's orchestration is one agent's pane pinned as its
//! orchestrator, which plans on the board and hands each task to a worker pane
//! of its own. How many workers run at once, and how many times a task is
//! tried, are the orchestrator's call: Hangar sets no limit on either. P10
//! (`orchestration_ui`) and P19 (`orchestration`) build on these rather than
//! writing their own.
//!
//! Who may hold a role is read from what its agent can do, never from which
//! agent it is (docs/native/agents.md D3, §5.1, §6.7): an orchestrator of
//! either agent runs workers of either agent.

use hangar_core::agent::{AgentKind, Capabilities, IdleInject, McpInjection, RoleBriefing};

use super::types::{
    AgentId, AppState, CodexModels, OrchestrationSettings, Pane, PaneRole, Settings, Workspace,
};
use crate::live::live_caps;

/// The board label of a task only the person can move forward.
pub const NEEDS_YOU_LABEL: &str = "needs-you";
/// The board label of a delivery that passed its check and review, waiting
/// to land.
pub const READY_TO_LAND_LABEL: &str = "ready-to-land";

/// The permission modes offered for an orchestration's agents; none means
/// the person's own (`ORCHESTRATION_PERMISSION_MODES`).
pub const ORCHESTRATION_PERMISSION_MODES: [&str; 3] = ["auto", "acceptEdits", "bypassPermissions"];

/// What an orchestration pane may use without asking, whatever its mode: the
/// tools of Hangar's own server, each of which checks who calls it
/// (`ORCHESTRATION_ALLOWED_TOOLS`).
pub const ORCHESTRATION_ALLOWED_TOOLS: [&str; 1] = ["mcp__hangar"];

/// `ORCHESTRATION_DEFAULTS`: the `Default` of `OrchestrationSettings`.
pub fn orchestration_defaults() -> OrchestrationSettings {
    OrchestrationSettings::default()
}

/// A workspace's orchestration settings, each one absent or malformed read as
/// its default (`orchestrationOf`): a blank model name, a permission mode not
/// offered. Names and optional texts come back trimmed.
pub fn orchestration_of(ws: &Workspace) -> OrchestrationSettings {
    let defaults = OrchestrationSettings::default();
    let Some(saved) = &ws.orchestration else {
        return defaults;
    };
    let name = |value: &str, fallback: &str| {
        let value = value.trim();
        if value.is_empty() {
            fallback.to_string()
        } else {
            value.to_string()
        }
    };
    let optional = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    OrchestrationSettings {
        auto_merge: saved.auto_merge,
        shared_build_cache: saved.shared_build_cache,
        batch_size: saved
            .batch_size
            .filter(|size| *size >= 1)
            .map(|size| size.min(hangar_core::delivery::MAX_BATCH_SIZE)),
        check_command: optional(&saved.check_command),
        orchestrator_model: name(&saved.orchestrator_model, &defaults.orchestrator_model),
        worker_model: name(&saved.worker_model, &defaults.worker_model),
        reviewer_model: name(&saved.reviewer_model, &defaults.reviewer_model),
        branch: optional(&saved.branch),
        permission_mode: saved
            .permission_mode
            .clone()
            .filter(|mode| ORCHESTRATION_PERMISSION_MODES.contains(&mode.as_str())),
        orchestrator_agent: saved.orchestrator_agent,
        worker_agent: saved.worker_agent,
        reviewer_agent: saved.reviewer_agent,
        codex_models: CodexModels {
            orchestrator: optional(&saved.codex_models.orchestrator),
            worker: optional(&saved.codex_models.worker),
            reviewer: optional(&saved.codex_models.reviewer),
            extra: saved.codex_models.extra.clone(),
        },
    }
}

/// The workspace's orchestrator pane, if it has one (`orchestratorOf`).
pub fn orchestrator_of(ws: &Workspace) -> Option<&Pane> {
    ws.panes
        .iter()
        .find(|pane| pane.role == Some(PaneRole::Orchestrator))
}

/// The workspace's worker panes (`workersOf`).
pub fn workers_of(ws: &Workspace) -> Vec<&Pane> {
    ws.panes
        .iter()
        .filter(|pane| pane.role == Some(PaneRole::Worker))
        .collect()
}

/// The manager pane, wherever it is (`managerOf`): one for the whole of
/// Hangar, following every workspace from the HQ, neither an orchestrator nor
/// a worker of the workspace it sits in.
pub fn manager_of(state: &AppState) -> Option<(&Workspace, &Pane)> {
    state.workspaces.iter().find_map(|ws| {
        ws.panes
            .iter()
            .find(|pane| pane.role == Some(PaneRole::Manager))
            .map(|pane| (ws, pane))
    })
}

/// The HQ workspace, the manager's, once Hangar has created it (`hqOf`).
pub fn hq_of(state: &AppState) -> Option<&Workspace> {
    state.workspaces.iter().find(|ws| ws.hq == Some(true))
}

/* ── Who can hold a role ───────────────────────────────────────── */

/// Why a pane's agent cannot hold an orchestration role, as data: the
/// pickers word it in the person's language, the tools in a model's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleRefusal {
    /// A plain shell: no agent to give the role to.
    NoAgent,
    /// What its agent can do leaves it out (`briefing`, `inject`, `mcp`):
    /// it could not be handed a role, take Hangar's messages or reach
    /// Hangar's tools.
    Unable { label: &'static str },
    /// The link Hangar hands its agent a brief through is turned off in the
    /// settings: `switch` is that setting, as « Settings › Sessions » names
    /// it.
    SwitchedOff { label: &'static str, switch: String },
}

impl RoleRefusal {
    /// The reason, in a model's words: what the tools answer.
    pub fn reason(&self) -> String {
        match self {
            RoleRefusal::NoAgent => {
                "it runs a plain shell, and a role is given to an agent".to_string()
            }
            RoleRefusal::Unable { label } => format!(
                "Hangar cannot hand {label} a role: it could not brief it, message it or give it Hangar's tools"
            ),
            RoleRefusal::SwitchedOff { label, switch } => format!(
                "« {switch} » is turned off (Settings › Sessions), and that is how Hangar hands {label} its brief and its messages"
            ),
        }
    }
}

/// Whether an agent that can do `caps` can hold an orchestration role at
/// all: be handed its role and the workspace's state (`briefing`), take
/// Hangar's messages (`inject`) and call Hangar's tools (`mcp`), §6.7. Every
/// level of each does it, in its own way; a level added later decides here
/// whether it still does.
pub fn takes_a_role(caps: &Capabilities) -> bool {
    let briefed = match caps.briefing.role {
        RoleBriefing::SystemPrompt
        | RoleBriefing::DeveloperInstructions
        | RoleBriefing::FirstMessage => true,
    };
    let told = match caps.inject.idle {
        IdleInject::Submit | IdleInject::Type => true,
    };
    let tooled = match caps.mcp.injection {
        McpInjection::ConfigFile | McpInjection::Overrides => true,
    };
    briefed && told && tooled
}

/// Why a pane running `agent` cannot be pinned as its workspace's
/// orchestrator (or be the project manager), or `None` when it can: any
/// agent whose capabilities let it hold a role (§6.7). The other rules of a
/// pin — one orchestrator per workspace, never a worker, never in the HQ —
/// are the pin's own (`orchestration_ui::actions`).
pub fn lead_refusal(agent: AgentId) -> Option<RoleRefusal> {
    let Some(kind) = agent.kind() else {
        return Some(RoleRefusal::NoAgent);
    };
    let info = kind.info();
    (!takes_a_role(&info.caps)).then_some(RoleRefusal::Unable { label: info.label })
}

/// The agents a pane can be pinned on, for a sentence: « Claude Code or
/// Codex ».
pub fn lead_labels() -> String {
    AgentKind::ALL
        .into_iter()
        .filter(|kind| lead_refusal((*kind).into()).is_none())
        .map(|kind| kind.info().label)
        .collect::<Vec<_>>()
        .join(" or ")
}

/// Whether the settings let the side channel of `kind`'s panes run: Hangar's
/// client of the server a kind runs attached to (« Attach Hangar to Codex »,
/// `codex_attach`, D5) for a kind Hangar reaches through a server of its own
/// (`headless.app_server`), the hangar-bridge mod (`claude_bridge`) for any
/// other — the launch's own rule (`host/launch.rs`, `side_channel`).
pub fn side_channel_on(settings: &Settings, kind: AgentKind) -> bool {
    if kind.info().caps.headless.app_server {
        settings.codex_attach
    } else {
        settings.claude_bridge
    }
}

/// The setting of « Settings › Sessions » that turns `kind`'s side channel
/// on, as the person reads it: « Claude Code bridge », « Attach Hangar to
/// Codex ».
pub fn side_channel_switch(kind: AgentKind) -> String {
    let info = kind.info();
    if info.caps.headless.app_server {
        format!("Attach Hangar to {}", info.label)
    } else {
        format!("{} bridge", info.label)
    }
}

/// Why a worker cannot run `kind`'s agent with the settings in force, or
/// `None` when it can (§5.1). Its brief, and every message Hangar has for it
/// after, wait in Hangar's queue for the pane: what the agent can do with
/// the settings in force (`live::live_caps`, its side channel on or off)
/// must take a queued message by itself (`inject.idle` `Submit`). On the
/// floor nothing types the queue into the agent, so the brief would never
/// arrive: the worker is refused, and the refusal names the setting.
pub fn worker_refusal(kind: AgentKind, settings: &Settings) -> Option<RoleRefusal> {
    let info = kind.info();
    if !takes_a_role(&info.caps) {
        return Some(RoleRefusal::Unable { label: info.label });
    }
    let caps = live_caps(&info.caps, side_channel_on(settings, kind));
    match caps.inject.idle {
        IdleInject::Submit => None,
        IdleInject::Type => Some(RoleRefusal::SwitchedOff {
            label: info.label,
            switch: side_channel_switch(kind),
        }),
    }
}

#[cfg(test)]
mod tests {
    //! The cases of projects.test.ts for the manager role, and of
    //! orchestrationForm.test.ts for what `orchestrationOf` reads back.

    use super::*;
    use crate::state::types::AgentId;

    fn pane(id: &str, agent: AgentId, role: Option<PaneRole>) -> Pane {
        Pane {
            id: id.into(),
            name: id.into(),
            agent,
            role,
            ..Pane::default()
        }
    }
    fn claude(id: &str) -> Pane {
        pane(id, AgentId::Claude, None)
    }
    fn ws(panes: Vec<Pane>) -> Workspace {
        Workspace {
            id: "w".into(),
            panes,
            ..Workspace::default()
        }
    }

    #[test]
    fn the_manager_is_neither_an_orchestrator_nor_a_worker() {
        let home = ws(vec![pane("m", AgentId::Claude, Some(PaneRole::Manager))]);
        assert!(orchestrator_of(&home).is_none());
        assert!(workers_of(&home).is_empty());
        assert_eq!(orchestration_of(&home), orchestration_defaults());
        let mixed = ws(vec![
            pane("m2", AgentId::Claude, Some(PaneRole::Manager)),
            pane("o2", AgentId::Claude, Some(PaneRole::Orchestrator)),
        ]);
        assert_eq!(orchestrator_of(&mixed).map(|p| p.id.as_str()), Some("o2"));
        assert!(workers_of(&mixed).is_empty());
        let state = AppState {
            workspaces: vec![ws(vec![claude("a")]), mixed.clone()],
            ..AppState::default()
        };
        assert_eq!(manager_of(&state).map(|(_, p)| p.id.as_str()), Some("m2"));
    }

    #[test]
    fn a_file_without_the_build_cache_field_reads_false() {
        let saved: OrchestrationSettings = serde_json::from_str(r#"{"autoMerge": false}"#).unwrap();
        assert!(!saved.shared_build_cache);
        assert!(!saved.auto_merge);
        let mut w = ws(vec![]);
        w.orchestration = Some(saved);
        assert!(!orchestration_of(&w).shared_build_cache);

        w.orchestration = Some(OrchestrationSettings {
            shared_build_cache: true,
            ..OrchestrationSettings::default()
        });
        assert!(orchestration_of(&w).shared_build_cache);
        let json = serde_json::to_value(orchestration_of(&w)).unwrap();
        assert_eq!(json["sharedBuildCache"], true);
    }

    #[test]
    fn reads_back_what_was_stored_and_defaults_what_is_off() {
        let stored = OrchestrationSettings {
            auto_merge: false,
            check_command: Some("make check".into()),
            branch: Some("develop".into()),
            ..OrchestrationSettings::default()
        };
        let mut w = ws(vec![]);
        w.orchestration = Some(stored.clone());
        assert_eq!(orchestration_of(&w), stored);

        w.orchestration = Some(OrchestrationSettings {
            worker_model: "  ".into(),
            reviewer_model: " haiku ".into(),
            check_command: Some("   ".into()),
            permission_mode: Some("yolo".into()),
            ..OrchestrationSettings::default()
        });
        let read = orchestration_of(&w);
        assert_eq!(read.worker_model, "sonnet");
        assert_eq!(read.reviewer_model, "haiku");
        assert_eq!(read.check_command, None);
        assert_eq!(read.permission_mode, None);
    }

    /// The agents chosen and Codex's models read back as stored, the names
    /// trimmed and a blank one as none: Codex's own default.
    #[test]
    fn reads_back_the_agents_and_codex_s_models() {
        use super::super::types::AgentKind;

        let stored = OrchestrationSettings {
            orchestrator_agent: Some(AgentKind::Codex),
            worker_agent: Some(AgentKind::Claude),
            reviewer_agent: Some(AgentKind::Codex),
            codex_models: CodexModels {
                orchestrator: Some(" gpt-5.5 ".into()),
                worker: Some("  ".into()),
                reviewer: None,
                extra: serde_json::from_value(serde_json::json!({"planner": "o9"})).unwrap(),
            },
            ..OrchestrationSettings::default()
        };
        let mut w = ws(vec![]);
        w.orchestration = Some(stored.clone());
        let read = orchestration_of(&w);
        assert_eq!(
            (
                read.orchestrator_agent,
                read.worker_agent,
                read.reviewer_agent
            ),
            (
                Some(AgentKind::Codex),
                Some(AgentKind::Claude),
                Some(AgentKind::Codex)
            )
        );
        assert_eq!(
            read.codex_models,
            CodexModels {
                orchestrator: Some("gpt-5.5".into()),
                worker: None,
                reviewer: None,
                extra: stored.codex_models.extra.clone(),
            }
        );
    }

    /// Either agent can be pinned as an orchestrator, by what it can do; a
    /// shell cannot (§6.7).
    #[test]
    fn a_pane_of_either_agent_can_lead_and_a_shell_cannot() {
        assert_eq!(lead_refusal(AgentId::Claude), None);
        assert_eq!(lead_refusal(AgentId::Codex), None);
        assert_eq!(lead_refusal(AgentId::Shell), Some(RoleRefusal::NoAgent));
        assert_eq!(lead_labels(), "Claude Code or Codex");
        for kind in AgentKind::ALL {
            assert!(takes_a_role(&kind.info().caps), "{kind:?}");
        }
    }

    /// A worker of an agent starts only while the link its brief comes
    /// through is on: the Claude Code bridge for Claude Code, « Attach Hangar
    /// to Codex » for Codex; each switch leaves the other agent alone.
    #[test]
    fn a_worker_needs_the_link_its_brief_comes_through() {
        let on = Settings::default();
        assert!(on.claude_bridge && on.codex_attach);
        assert_eq!(worker_refusal(AgentKind::Claude, &on), None);
        assert_eq!(worker_refusal(AgentKind::Codex, &on), None);

        let no_bridge = Settings {
            claude_bridge: false,
            ..Settings::default()
        };
        assert_eq!(
            worker_refusal(AgentKind::Claude, &no_bridge),
            Some(RoleRefusal::SwitchedOff {
                label: "Claude Code",
                switch: "Claude Code bridge".into(),
            })
        );
        assert_eq!(worker_refusal(AgentKind::Codex, &no_bridge), None);

        let no_attach = Settings {
            codex_attach: false,
            ..Settings::default()
        };
        let refused = worker_refusal(AgentKind::Codex, &no_attach).expect("refused");
        assert_eq!(
            refused,
            RoleRefusal::SwitchedOff {
                label: "Codex",
                switch: "Attach Hangar to Codex".into(),
            }
        );
        assert!(
            refused
                .reason()
                .starts_with("« Attach Hangar to Codex » is turned off (Settings › Sessions)"),
            "{}",
            refused.reason()
        );
        assert_eq!(worker_refusal(AgentKind::Claude, &no_attach), None);
    }

    /// The switches are named as « Settings › Sessions » names them.
    #[test]
    fn names_each_agents_link_as_the_settings_do() {
        assert_eq!(side_channel_switch(AgentKind::Claude), "Claude Code bridge");
        assert_eq!(
            side_channel_switch(AgentKind::Codex),
            "Attach Hangar to Codex"
        );
        let no_attach = Settings {
            codex_attach: false,
            ..Settings::default()
        };
        assert!(side_channel_on(&no_attach, AgentKind::Claude));
        assert!(!side_channel_on(&no_attach, AgentKind::Codex));
        let no_bridge = Settings {
            claude_bridge: false,
            ..Settings::default()
        };
        assert!(!side_channel_on(&no_bridge, AgentKind::Claude));
        assert!(side_channel_on(&no_bridge, AgentKind::Codex));
    }
}
