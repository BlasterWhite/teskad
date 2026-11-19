//! The agents Hangar runs — Claude Code and Codex — described once, by what
//! they can do (docs/native/agents.md §3).
//!
//! An agent is a *kind* ([`AgentKind`]) with a row of [`AgentInfo`]: its
//! name, its command, the instructions file it reads, and its
//! [`Capabilities`], each a level (caps.rs). [`info`] is the row; [`detect`]
//! says which agents are on this machine; [`launch`] builds the command each
//! kind is started with, from its own file's spellings (`claude.rs`,
//! [`codex`]).
//!
//! **The rule (D3): features read capabilities, never the agent kind.** Only
//! the agent modules (this one and the kind's own driver files) and the state
//! reader that maps persisted strings match on `AgentKind`. A feature written
//! as "if the agent is Claude Code" silently does nothing on a machine with
//! Codex alone; one written as "if it holds approvals" works for both, and
//! for a third agent later. A test in hangar-engine holds the rule.
//!
//! A shell pane has no agent: it is not a kind (the engine's
//! `AgentId::Shell` stays the pane's "nothing to start").

pub mod caps;
mod claude;
pub mod codex;
pub mod codex_attach;
pub mod detect;
pub mod launch;

use serde::{Deserialize, Serialize};

pub use caps::{
    Accounts, AllowedTools, Asks, Briefing, Capabilities, Extensions, Headless, IdleInject, Images,
    Inject, KitPart, Launch, Mcp, McpInjection, PermissionMode, RoleBriefing, RunningInject,
    SessionId, Sessions, StateBriefing, TranscriptFormat, TurnSignal, Usage, UsageSource,
};

/// An agent Hangar can run in a pane, by the id `state.json` and the MCP
/// tools give it (`claude`, `codex`).
///
/// The default is Claude Code: what a file written before Hangar ran more
/// than one agent means wherever it names none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    #[default]
    Claude,
    Codex,
}

impl AgentKind {
    /// Every kind, in the order the pickers list them.
    pub const ALL: [AgentKind; 2] = [AgentKind::Claude, AgentKind::Codex];

    /// The id `state.json` and the MCP tools use.
    pub fn id(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
        }
    }

    /// The kind an id names; `None` for anything else, a shell included.
    pub fn from_id(id: &str) -> Option<AgentKind> {
        AgentKind::ALL.into_iter().find(|kind| kind.id() == id)
    }

    /// This kind's row.
    pub fn info(self) -> &'static AgentInfo {
        info(self)
    }

    /// The arguments after the agent's command that resume `session_id`:
    /// `claude --resume <id>`, `codex resume <id>`.
    pub fn resume_args(self, session_id: &str) -> Vec<String> {
        let verb = match self {
            AgentKind::Claude => "--resume",
            AgentKind::Codex => "resume",
        };
        vec![verb.to_string(), session_id.to_string()]
    }
}

/// What Hangar knows of a kind without asking the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub kind: AgentKind,
    /// Its name, as the person reads it.
    pub label: &'static str,
    /// The command a pane types when the settings name no other.
    pub program: &'static str,
    /// The names its command is looked for by, on PATH and in its
    /// installer's folders (`install::agent`): the binary, and on Windows
    /// the native executable and the shim npm writes.
    pub binaries: &'static [&'static str],
    /// The project instructions file it reads, where Hangar's playbook goes.
    pub instructions_file: &'static str,
    /// What its prompt reads as "a new line, not sent" from a terminal that
    /// does not speak the kitty keyboard protocol: what a pane's Shift+Enter
    /// sends it (hangar-app's `panes/look.rs`).
    ///
    /// - Claude Code: ESC+CR (Alt+Enter), what it reads as a newline.
    /// - Codex: LF, its Ctrl+J, the newline key its own footer offers when the
    ///   terminal reports no enhanced keys (`codex-rs/tui`, `keymap.rs`:
    ///   `editor.insert_newline`, and `footer.rs`, read at the pinned
    ///   0.170.0). Its Alt+Enter is a newline too, but an ESC that reaches it
    ///   on its own is its Esc: the running turn interrupted, or the last
    ///   message taken back to be edited.
    #[serde(skip)]
    pub newline: &'static [u8],
    /// The best the tested version does. A pane's live capabilities may be
    /// lower (the side channel off, a version too old).
    pub caps: Capabilities,
}

/// The row of `kind`.
pub fn info(kind: AgentKind) -> &'static AgentInfo {
    match kind {
        AgentKind::Claude => &CLAUDE,
        AgentKind::Codex => &CODEX,
    }
}

/// Claude Code with the hangar-bridge mod: what Hangar has run since 2.0.
static CLAUDE: AgentInfo = AgentInfo {
    kind: AgentKind::Claude,
    label: "Claude Code",
    program: "claude",
    binaries: &["claude.exe", "claude.cmd", "claude"],
    instructions_file: "CLAUDE.md",
    newline: b"\x1b\r",
    caps: Capabilities {
        launch: Launch {
            model: true,
            // Hangar starts it with no effort of its own: the session keeps
            // the person's, which the mod reports.
            effort: false,
            extra_roots: true,
            session_name: true,
            allowed_tools: AllowedTools::Rules,
        },
        turns: TurnSignal::Reported,
        inject: Inject {
            idle: IdleInject::Submit,
            running: RunningInject::Append,
        },
        submits_commands: true,
        briefing: Briefing {
            role: RoleBriefing::SystemPrompt,
            state: StateBriefing::SessionContext,
        },
        approvals: Asks::Held,
        questions: Asks::Held,
        transcript: Some(TranscriptFormat::ClaudeJsonl),
        sessions: Sessions {
            resume: true,
            resume_last: true,
            fork: true,
            id: SessionId::Reported,
        },
        usage: Usage {
            context: UsageSource::Transcript,
            limits: UsageSource::Reported,
        },
        mcp: Mcp {
            injection: McpInjection::ConfigFile,
            strict: true,
        },
        extensions: Extensions {
            mcp_servers: KitPart::PerLaunch,
            skills: KitPart::PerLaunch,
            hooks: KitPart::PerLaunch,
            commands: KitPart::PerLaunch,
            agents: KitPart::PerLaunch,
            output_styles: KitPart::PerLaunch,
        },
        accounts: Accounts::ConfigDir {
            var: "CLAUDE_CONFIG_DIR",
        },
        images: Images {
            generates: false,
            input: true,
        },
        headless: Headless {
            one_shot: true,
            app_server: false,
        },
        permission_modes: &PermissionMode::ALL,
        background_jobs: true,
    },
};

/// Codex attached to the app-server Hangar starts for its pane (§4.1, D5).
static CODEX: AgentInfo = AgentInfo {
    kind: AgentKind::Codex,
    label: "Codex",
    program: "codex",
    binaries: &["codex.exe", "codex.cmd", "codex"],
    instructions_file: "AGENTS.md",
    newline: b"\n",
    caps: Capabilities {
        launch: Launch {
            model: true,
            effort: true,
            // As writable roots of the sandbox: the `--remote` TUI refuses
            // `--add-dir`.
            extra_roots: true,
            session_name: false,
            allowed_tools: AllowedTools::HangarTools,
        },
        turns: TurnSignal::Reported,
        inject: Inject {
            idle: IdleInject::Submit,
            running: RunningInject::Steer,
        },
        submits_commands: false,
        briefing: Briefing {
            role: RoleBriefing::DeveloperInstructions,
            state: StateBriefing::SessionContext,
        },
        approvals: Asks::Held,
        questions: Asks::Held,
        transcript: Some(TranscriptFormat::CodexRollout),
        sessions: Sessions {
            resume: true,
            resume_last: true,
            fork: true,
            id: SessionId::Reported,
        },
        usage: Usage {
            context: UsageSource::Protocol,
            limits: UsageSource::Protocol,
        },
        mcp: Mcp {
            injection: McpInjection::Overrides,
            // The servers a « custom » workspace did not choose are turned
            // off one by one (`mcp_servers.<name>.enabled=false`).
            strict: true,
        },
        extensions: Extensions {
            mcp_servers: KitPart::PerLaunch,
            // Codex has no per-launch plugin folder: a kit's skills and
            // hooks reach it as a Codex plugin the person adds once.
            skills: KitPart::Installed,
            hooks: KitPart::Installed,
            commands: KitPart::None,
            agents: KitPart::None,
            output_styles: KitPart::None,
        },
        accounts: Accounts::ConfigDir { var: "CODEX_HOME" },
        images: Images {
            generates: true,
            input: true,
        },
        headless: Headless {
            one_shot: true,
            app_server: true,
        },
        // Codex has no "edits only": `acceptEdits` is its workspace-write
        // sandbox asking before escalations (§4.2.1).
        permission_modes: &PermissionMode::ALL,
        background_jobs: true,
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_is_written_and_read_by_its_persisted_id() {
        for kind in AgentKind::ALL {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.id()));
            assert_eq!(serde_json::from_str::<AgentKind>(&json).unwrap(), kind);
            assert_eq!(AgentKind::from_id(kind.id()), Some(kind));
        }
        assert_eq!(AgentKind::from_id("codex"), Some(AgentKind::Codex));
        assert_eq!(AgentKind::from_id("claude"), Some(AgentKind::Claude));
    }

    #[test]
    fn a_shell_or_a_retired_agent_is_no_kind() {
        for id in ["shell", "gemini", "opencode", "Claude", ""] {
            assert_eq!(AgentKind::from_id(id), None, "{id:?}");
        }
        assert!(serde_json::from_str::<AgentKind>("\"shell\"").is_err());
    }

    /// What `state.json` written before Codex came back means.
    #[test]
    fn no_kind_named_reads_as_claude_code() {
        assert_eq!(AgentKind::default(), AgentKind::Claude);
    }

    #[test]
    fn each_kind_resumes_its_own_way() {
        assert_eq!(AgentKind::Claude.resume_args("abc"), ["--resume", "abc"]);
        assert_eq!(AgentKind::Codex.resume_args("abc"), ["resume", "abc"]);
    }

    #[test]
    fn each_kind_has_its_own_row() {
        for kind in AgentKind::ALL {
            assert_eq!(info(kind).kind, kind);
            assert_eq!(kind.info(), info(kind));
        }
        assert_ne!(info(AgentKind::Claude), info(AgentKind::Codex));
    }

    /// The row says what Hangar does for Claude Code today: its name in the
    /// pickers, `claude` typed when the settings name nothing, the names
    /// `sessions::claude_installed` looks for, the playbook in `CLAUDE.md`.
    #[test]
    fn claude_code_reads_as_it_runs_today() {
        let claude = info(AgentKind::Claude);
        assert_eq!(claude.label, "Claude Code");
        assert_eq!(claude.program, "claude");
        assert_eq!(claude.binaries, ["claude.exe", "claude.cmd", "claude"]);
        assert_eq!(claude.instructions_file, "CLAUDE.md");
        assert_eq!(claude.newline, b"\x1b\r");

        let caps = claude.caps;
        assert_eq!(caps.turns, TurnSignal::Reported);
        assert_eq!(
            caps.inject,
            Inject {
                idle: IdleInject::Submit,
                running: RunningInject::Append,
            }
        );
        assert!(caps.submits_commands);
        assert_eq!(caps.briefing.role, RoleBriefing::SystemPrompt);
        assert_eq!(caps.briefing.state, StateBriefing::SessionContext);
        assert_eq!(caps.approvals, Asks::Held);
        assert_eq!(caps.questions, Asks::Held);
        assert_eq!(caps.transcript, Some(TranscriptFormat::ClaudeJsonl));
        assert_eq!(caps.usage.context, UsageSource::Transcript);
        assert_eq!(caps.usage.limits, UsageSource::Reported);
        assert_eq!(
            caps.mcp,
            Mcp {
                injection: McpInjection::ConfigFile,
                strict: true,
            }
        );
        assert_eq!(
            caps.accounts,
            Accounts::ConfigDir {
                var: "CLAUDE_CONFIG_DIR"
            }
        );
        assert!(caps.launch.session_name);
        assert_eq!(caps.launch.allowed_tools, AllowedTools::Rules);
        assert!(!caps.images.generates);
        assert!(caps.headless.one_shot && !caps.headless.app_server);
    }

    /// Codex attached (§4.1): the same channels as Claude Code with its mod,
    /// over the app-server protocol instead.
    #[test]
    fn codex_reads_as_the_design_drives_it() {
        let codex = info(AgentKind::Codex);
        assert_eq!(codex.label, "Codex");
        assert_eq!(codex.program, "codex");
        assert_eq!(codex.binaries, ["codex.exe", "codex.cmd", "codex"]);
        assert_eq!(codex.instructions_file, "AGENTS.md");
        assert_eq!(codex.newline, b"\n");

        let caps = codex.caps;
        assert_eq!(caps.turns, TurnSignal::Reported);
        assert_eq!(caps.inject.idle, IdleInject::Submit);
        assert_eq!(caps.inject.running, RunningInject::Steer);
        // Its slash commands run in its own window, not from a turn.
        assert!(!caps.submits_commands);
        assert_eq!(caps.briefing.role, RoleBriefing::DeveloperInstructions);
        assert_eq!(caps.approvals, Asks::Held);
        assert_eq!(caps.transcript, Some(TranscriptFormat::CodexRollout));
        assert_eq!(caps.sessions.id, SessionId::Reported);
        assert_eq!(caps.usage.context, UsageSource::Protocol);
        assert_eq!(caps.usage.limits, UsageSource::Protocol);
        assert_eq!(caps.mcp.injection, McpInjection::Overrides);
        assert_eq!(caps.accounts, Accounts::ConfigDir { var: "CODEX_HOME" });
        assert!(caps.images.generates);
        assert!(caps.headless.one_shot && caps.headless.app_server);
        assert!(caps.launch.effort);
        assert!(!caps.launch.session_name);
        assert_eq!(caps.launch.allowed_tools, AllowedTools::HangarTools);
    }

    /// What a kit brings to Codex (§6.4): its servers at every launch, its
    /// skills and hooks once added, the rest Claude Code's only.
    #[test]
    fn a_kit_reaches_codex_in_part() {
        let codex = info(AgentKind::Codex).caps.extensions;
        assert_eq!(codex.mcp_servers, KitPart::PerLaunch);
        assert_eq!(codex.skills, KitPart::Installed);
        assert_eq!(codex.hooks, KitPart::Installed);
        assert_eq!(codex.commands, KitPart::None);
        assert_eq!(codex.agents, KitPart::None);
        assert_eq!(codex.output_styles, KitPart::None);

        let claude = info(AgentKind::Claude).caps.extensions;
        for part in [
            claude.mcp_servers,
            claude.skills,
            claude.hooks,
            claude.commands,
            claude.agents,
            claude.output_styles,
        ] {
            assert_eq!(part, KitPart::PerLaunch);
        }
    }

    /// Hangar alone with either kind (§6.9): each one can be resumed, takes
    /// Hangar's MCP server and events, and holds the person's answers, so
    /// orchestration, the chat view and Mobile have what they read.
    #[test]
    fn either_kind_alone_has_what_hangar_needs() {
        for kind in AgentKind::ALL {
            let caps = info(kind).caps;
            assert!(caps.sessions.resume, "{kind:?}");
            assert!(caps.transcript.is_some(), "{kind:?}");
            assert_eq!(caps.turns, TurnSignal::Reported, "{kind:?}");
            assert_eq!(caps.inject.idle, IdleInject::Submit, "{kind:?}");
            assert_ne!(caps.inject.running, RunningInject::None, "{kind:?}");
            assert_eq!(caps.approvals, Asks::Held, "{kind:?}");
            assert!(caps.mcp.strict, "{kind:?}");
            assert!(caps.headless.one_shot, "{kind:?}");
            assert!(caps.background_jobs, "{kind:?}");
            assert_eq!(caps.permission_modes, PermissionMode::ALL, "{kind:?}");
        }
    }

    #[test]
    fn the_program_is_one_of_the_names_looked_for() {
        for kind in AgentKind::ALL {
            let row = info(kind);
            assert!(row.binaries.contains(&row.program), "{kind:?}");
            assert!(
                row.binaries
                    .iter()
                    .all(|name| name.starts_with(row.program)),
                "{kind:?}"
            );
        }
    }

    /// The ids the orchestration settings hold (`ORCHESTRATION_PERMISSION_MODES`).
    #[test]
    fn permission_modes_keep_their_persisted_ids() {
        let ids: Vec<&str> = PermissionMode::ALL.iter().map(|m| m.id()).collect();
        assert_eq!(ids, ["acceptEdits", "auto", "bypassPermissions"]);
        for mode in PermissionMode::ALL {
            assert_eq!(PermissionMode::from_id(mode.id()), Some(mode));
        }
        assert_eq!(PermissionMode::from_id("plan"), None);
        assert_eq!(PermissionMode::from_id("yolo"), None);
    }

    /// The pickers, the setup sheet and the node's info read the rows as
    /// JSON.
    #[test]
    fn a_row_is_written_as_json_the_pickers_read() {
        let row = serde_json::to_value(info(AgentKind::Codex)).unwrap();
        assert_eq!(row["kind"], "codex");
        assert_eq!(row["label"], "Codex");
        assert_eq!(row["instructionsFile"], "AGENTS.md");
        assert_eq!(row["caps"]["turns"], "reported");
        assert_eq!(row["caps"]["inject"]["running"], "steer");
        assert_eq!(row["caps"]["transcript"], "codexRollout");
        assert_eq!(row["caps"]["accounts"]["configDir"]["var"], "CODEX_HOME");
        assert_eq!(row["caps"]["permissionModes"][0], "acceptEdits");
    }
}
