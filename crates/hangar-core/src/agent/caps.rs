//! What an agent can do, as levels (docs/native/agents.md §3.2, D2).
//!
//! A capability is a level rather than a yes or no, so a feature can say
//! honestly how much of it works — "turn end only", "keys checked on
//! screen" — instead of pretending. The levels here are the static ones of
//! [`AgentInfo::caps`](super::AgentInfo): what the tested version of each kind
//! does at its best (Claude Code with the teskad-bridge mod, Codex attached to
//! its app-server), so what Hangar may *offer*. A pane's live capabilities,
//! which drop to the floor when the side channel is off or could not attach,
//! are the engine's (`hangar-engine/src/live.rs`).
//!
//! Features read these, never the agent kind (D3).

use serde::Serialize;

/// Everything a kind can do, one level per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// What its launch line can carry.
    pub launch: Launch,
    /// How Hangar learns a turn started and ended.
    pub turns: TurnSignal,
    /// How Hangar's events reach the session.
    pub inject: Inject,
    /// The agent's own slash commands (`/model`, `/effort`, `/plan`) run
    /// from what its side channel submits, as they do from its prompt:
    /// Claude Code's mod submits the text as typed. Codex's side channel
    /// starts a turn with it, where a command would be words to the model,
    /// so it refuses the person's (`codex_attach.rs`): they are typed in
    /// its own window. Read with the side channel speaking; the floor types
    /// into the prompt, where either agent runs them.
    pub submits_commands: bool,
    /// How a role and the workspace's state are handed to it.
    pub briefing: Briefing,
    /// Who answers the agent's permission requests, and from where.
    pub approvals: Asks,
    /// Who answers the agent's structured questions (Claude Code's
    /// `AskUserQuestion`, Codex's `requestUserInput`).
    pub questions: Asks,
    /// The history it writes, read for the chat view, the statistics, the
    /// context gauge and the session list. `None`: the terminal is all
    /// there is.
    pub transcript: Option<TranscriptFormat>,
    /// What can be resumed, and how Hangar learns the session's id.
    pub sessions: Sessions,
    /// Where the context gauge and the plan limits come from.
    pub usage: Usage,
    /// How Hangar's MCP server and the workspace's choice of servers reach it.
    pub mcp: Mcp,
    /// Which parts of a kit it can load, and how.
    pub extensions: Extensions,
    /// How one machine holds several of its accounts.
    pub accounts: Accounts,
    /// What it does with images.
    pub images: Images,
    /// How it runs without a terminal.
    pub headless: Headless,
    /// The orchestration permission policies it enforces itself; a policy
    /// not listed is not offered for it.
    pub permission_modes: &'static [PermissionMode],
    /// It runs commands that outlive a turn (`pty/jobs.rs`).
    pub background_jobs: bool,
}

/// What a launch line can carry beyond the command and the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Launch {
    /// A model per pane (Claude Code `--model`, Codex `-c model=…`).
    pub model: bool,
    /// A reasoning effort per pane (Codex `-c model_reasoning_effort=…`).
    pub effort: bool,
    /// The other folders of a multi-root workspace (Claude Code `--add-dir`,
    /// Codex `sandbox_workspace_write.writable_roots`).
    pub extra_roots: bool,
    /// A session name other sessions reach it by (Claude Code `--name`).
    pub session_name: bool,
    /// Which tools can be let through without asking.
    pub allowed_tools: AllowedTools,
}

/// Which tools a launch can let through without a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AllowedTools {
    /// Any rule (Claude Code `--allowedTools`).
    Rules,
    /// Only Hangar's own MCP tools (Codex, through the server's default
    /// approval mode).
    HangarTools,
    None,
}

/// How Hangar learns what the session is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnSignal {
    /// Start and end, live, from a side channel.
    Reported,
    /// The end only (Codex's `notify`).
    EndOnly,
    /// Guessed from the output: bursts, silences, the bell, OSC 9 / 777.
    Inferred,
}

/// How a message reaches the session, idle and mid-turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inject {
    pub idle: IdleInject,
    pub running: RunningInject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum IdleInject {
    /// Submitted through the side channel, nothing typed.
    Submit,
    /// Pasted into the composer, then Enter.
    Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RunningInject {
    /// Appended to the running turn (the Claude mod's `session.append`).
    Append,
    /// Steers the running turn (Codex `turn/steer`).
    Steer,
    /// Waits for the turn to end.
    AfterTurn,
    /// Cannot reach a running turn at all.
    None,
}

/// How a pane's role and the workspace's state reach the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Briefing {
    pub role: RoleBriefing,
    pub state: StateBriefing,
}

/// Where the role (orchestrator, worker, project manager) goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RoleBriefing {
    /// At the end of the system prompt (Claude Code).
    SystemPrompt,
    /// Developer instructions of every thread (Codex).
    DeveloperInstructions,
    /// A labelled first message.
    FirstMessage,
}

/// Where the state (board, plan, latest events) goes at each start, resume
/// and compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StateBriefing {
    /// Context added to the session by the side channel.
    SessionContext,
    /// With the first message.
    FirstMessage,
}

/// How a request the agent makes of the person (a permission, a question)
/// can be answered from Hangar: the chat view, Mobile, Discord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Asks {
    /// Held open by the side channel until Hangar answers; the first answer
    /// wins against the terminal's own dialog.
    Held,
    /// Server requests of a headless run, answered over the protocol.
    Protocol,
    /// Keys typed into the agent's dialog, checked on screen.
    Keys,
    /// Shown, answered at the PC only.
    None,
}

/// The format of the history an agent writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptFormat {
    /// `~/.claude/projects/<slug>/<session>.jsonl`.
    ClaudeJsonl,
    /// `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-…-<uuid>.jsonl`.
    CodexRollout,
}

/// What can be started again, and how the session's id is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sessions {
    /// Resume a session by its id.
    pub resume: bool,
    /// Resume the latest session of the folder.
    pub resume_last: bool,
    /// Start a new session from an old one.
    pub fork: bool,
    pub id: SessionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionId {
    /// Said by the side channel when the session starts.
    Reported,
    /// Found by watching the agent's history folder for a new file.
    Hunted,
}

/// Where the context gauge and the plan limits are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub context: UsageSource,
    pub limits: UsageSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageSource {
    /// Reported by the side channel.
    Reported,
    /// Read from the history files.
    Transcript,
    /// Notifications of the app-server protocol, exact.
    Protocol,
    /// Not known: hidden.
    None,
}

/// How MCP servers reach a launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mcp {
    pub injection: McpInjection,
    /// The workspace's « custom » MCP mode can keep every other server out.
    pub strict: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum McpInjection {
    /// A file Hangar writes per launch (Claude Code `--mcp-config`).
    ConfigFile,
    /// Configuration overrides on the command line (Codex `-c
    /// mcp_servers.…`), never written to the agent's own configuration.
    Overrides,
}

/// The parts of a kit an agent can load (docs/native/agents.md §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Extensions {
    pub mcp_servers: KitPart,
    pub skills: KitPart,
    pub hooks: KitPart,
    pub commands: KitPart,
    pub agents: KitPart,
    pub output_styles: KitPart,
}

/// How one part of a kit reaches an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KitPart {
    /// Loaded with each launch, nothing installed.
    PerLaunch,
    /// Installed once into the agent's own configuration, with the
    /// person's consent (« Add to Codex »).
    Installed,
    /// Not loaded: listed as another agent's only.
    None,
}

/// How several accounts of one agent live on one machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Accounts {
    /// One configuration folder per account, named by an environment
    /// variable at launch. What each account shares with the main one is
    /// the accounts module's (`accounts/share.rs` for Claude Code).
    ConfigDir { var: &'static str },
    /// The person's one account.
    Single,
}

/// What an agent does with images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Images {
    /// It makes images of its own (Codex `image_gen`).
    pub generates: bool,
    /// It takes images in (a paste, a path).
    pub input: bool,
}

/// How an agent runs with no terminal: the delivery reviewer, `delegate`
/// children, probes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Headless {
    /// One prompt in, one answer out (`claude -p`, `codex exec`).
    pub one_shot: bool,
    /// A long-lived protocol server Hangar is the client of (`codex
    /// app-server`).
    pub app_server: bool,
}

/// The orchestration permission policies of a workspace
/// (`OrchestrationSettings::permission_mode`); none set is the person's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    AcceptEdits,
    Auto,
    BypassPermissions,
}

impl PermissionMode {
    pub const ALL: [PermissionMode; 3] = [
        PermissionMode::AcceptEdits,
        PermissionMode::Auto,
        PermissionMode::BypassPermissions,
    ];

    /// The id `state.json` holds.
    pub fn id(self) -> &'static str {
        match self {
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Auto => "auto",
            PermissionMode::BypassPermissions => "bypassPermissions",
        }
    }

    pub fn from_id(id: &str) -> Option<PermissionMode> {
        PermissionMode::ALL.into_iter().find(|mode| mode.id() == id)
    }
}
