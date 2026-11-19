//! One launch, for any agent (docs/native/agents.md §3.4, D4).
//!
//! A [`LaunchRequest`] says what a pane wants started — which agent, on which
//! session, with which model, account, permissions and MCP servers — in words
//! no agent owns; [`build`] turns it into a [`LaunchSpec`]: the command the
//! pane's shell types, and the variables the shell starts with for that
//! agent. Each kind's spellings live in its own file (`claude.rs`, and
//! `codex.rs` from task T4a); [`build`] is the one place that picks the file
//! by kind.
//!
//! What a pane gets whatever runs in it stays with the PTY (`pty.rs`): its
//! identity and API token, the hangar-bridge mod and the kits' plugin
//! folders, its ports, the build cache. The engine and the desktop still type
//! their own copy of Claude Code's line (`hangar-engine/src/panes/agents.rs`)
//! until they call [`build`] (task T14a); the tests of `claude.rs` hold this
//! one to the very same lines.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use super::{claude, AgentKind, PermissionMode};

/// The variable a pane's shell holds its MCP config file's path in: the line
/// typed names the variable instead of the path, which lies in Hangar's data
/// folder, so holds the person's name, and would stay on screen.
pub const MCP_CONFIG_VAR: &str = "HANGAR_MCP_CONFIG";

/// What a pane asks of its agent's launch, whatever the agent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchRequest {
    pub kind: AgentKind,
    /// The kind's launch command from the settings, when customised: an
    /// alias, or a wrapper carrying flags of its own. A blank one is not
    /// customised.
    pub command: Option<String>,
    /// The folder the agent starts in: the pane's, else its workspace's.
    pub cwd: String,
    /// The folders a multi-root workspace has beyond `cwd`.
    pub extra_roots: Vec<String>,
    /// The session to resume; none, or an empty id, starts a new one.
    pub resume: Option<String>,
    /// The session's name, the one other sessions reach it by
    /// (`session_name` of the pane names), for a kind that takes one
    /// (`Launch::session_name`).
    pub name: Option<String>,
    /// The model it starts on, an alias or an id: an orchestrator's or a
    /// worker's, from the pane. Absent, the agent's own default.
    pub model: Option<String>,
    /// The reasoning effort, for a kind that takes one at launch
    /// (`Launch::effort`). Absent, the agent's own.
    pub effort: Option<String>,
    /// The workspace's orchestration permission policy, for an orchestrator's
    /// or a worker's pane. Absent, the person's own.
    pub permission_mode: Option<PermissionMode>,
    /// Tools let through without asking, on top of the person's own rules,
    /// as far as the kind can say so (`Launch::allowed_tools`): Hangar's own
    /// for an orchestration pane.
    pub allowed_tools: Vec<String>,
    /// Hangar's choice of MCP servers for this launch. Absent, the agent
    /// starts with its own servers, as it always did.
    pub mcp: Option<McpRequest>,
    /// The folder of the account's profile the agent runs on, prepared by
    /// the accounts module (`accounts::for_launch`). Absent, the main
    /// account: the environment Hangar has.
    pub account_dir: Option<PathBuf>,
    /// The pane's role (orchestrator, worker, project manager) as its brief,
    /// for a kind that takes it at launch (`RoleBriefing::DeveloperInstructions`).
    /// Claude Code gets it from the mod, at the end of its system prompt, so
    /// its line carries none.
    pub role: Option<String>,
}

/// The MCP servers a launch starts with, as Hangar chose them for the pane
/// (`mcp_scope::mcp_scope_write`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpRequest {
    /// The config file written for this launch.
    pub file: String,
    /// `file` as the pane's shell is to read it: the variable that holds its
    /// path ([`MCP_CONFIG_VAR`]), or the path quoted for a shell whose syntax
    /// is not known.
    pub arg: String,
    /// Its servers are the only ones (a workspace in « custom » MCP mode).
    /// False: they come on top of the agent's own — how an orchestration pane
    /// gets Hangar's server where nothing else would give it one, a worker's
    /// worktree for one.
    pub strict: bool,
}

/// What starts an agent in a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    /// What the pane's shell runs: the settings' launch command as typed, or
    /// the kind's program. It may carry words of its own, so it is typed, not
    /// looked up.
    pub program: String,
    /// The arguments after it, each a word as the pane's shell reads it:
    /// quoted where its value needs to be, never holding what would end the
    /// quoting.
    pub args: Vec<String>,
    /// The folder it starts in.
    pub cwd: String,
    /// The variables the pane's shell starts with for this agent, in the
    /// order they are applied: set to the value, or, `None`, removed from
    /// what the shell would inherit.
    pub env: Vec<(String, Option<OsString>)>,
}

impl LaunchSpec {
    /// The line typed into the pane's shell, without the Enter. The arguments
    /// come after the command rather than in place of it, so a custom
    /// launcher resumes a session as well as the bare program does.
    pub fn command_line(&self) -> String {
        let mut line = self.program.clone();
        for arg in &self.args {
            line.push(' ');
            line.push_str(arg);
        }
        line
    }
}

/// Why a request could not be made into a launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchError {
    /// Hangar does not start this kind yet.
    Unsupported(AgentKind),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LaunchError::Unsupported(kind) => {
                write!(f, "Hangar cannot start {} yet", kind.info().label)
            }
        }
    }
}

impl std::error::Error for LaunchError {}

/// The launch `request` asks for, in its kind's own spellings.
pub fn build(request: &LaunchRequest) -> Result<LaunchSpec, LaunchError> {
    match request.kind {
        AgentKind::Claude => Ok(claude::spec(request)),
        // Task T4a: `codex::spec` — the pane's app-server with its `-c`
        // settings, and the `codex --remote` line typed into the pane
        // (§4.2.1).
        AgentKind::Codex => Err(LaunchError::Unsupported(AgentKind::Codex)),
    }
}

/// The command the pane types for `kind`: the one the settings hold, or the
/// kind's program. Trimmed, and an empty override reads as "not customised"
/// rather than as a pane that types nothing: clearing the field in the
/// settings is how the person goes back to the default.
pub fn program(kind: AgentKind, custom: Option<&str>) -> String {
    custom
        .map(js_trim)
        .filter(|command| !command.is_empty())
        .unwrap_or(kind.info().program)
        .to_string()
}

/// The session to resume, when there is one.
pub(super) fn resume(request: &LaunchRequest) -> Option<&str> {
    request.resume.as_deref().filter(|id| !id.is_empty())
}

/// The variable naming the MCP config file: set when the launch has one,
/// removed otherwise, so a Hangar started inside a pane never hands its own
/// down.
pub(super) fn mcp_env(request: &LaunchRequest) -> (String, Option<OsString>) {
    let file = request
        .mcp
        .as_ref()
        .map(|mcp| mcp.file.as_str())
        .filter(|file| !file.is_empty());
    (MCP_CONFIG_VAR.to_string(), file.map(OsString::from))
}

/// JavaScript's `String.prototype.trim`: it also strips the byte order mark.
pub(super) fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// A value safe to put between double quotes in a command line.
///
/// Extra roots are read out of a `.code-workspace` file, which can arrive
/// with a cloned repository: nobody types them. A newline would run whatever
/// follows it as its own command, and a quote, backtick or `$` would end the
/// quoting and let the rest be read as shell syntax. None of them belong in
/// a path, a name or a model, so a value carrying one is dropped rather than
/// escaped.
pub(super) fn is_quotable(value: &str) -> bool {
    !value.contains(['"', '`', '$', '\r', '\n'])
}

/// JavaScript's `\w`.
pub(super) fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Whether the whole text is one or more characters of the set (`^[...]+$`).
pub(super) fn all_of(text: &str, allowed: impl Fn(char) -> bool) -> bool {
    !text.is_empty() && text.chars().all(allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_the_program_then_each_argument() {
        let spec = LaunchSpec {
            program: "ccr code".into(),
            args: vec!["--resume".into(), "abc".into(), "\"/x y\"".into()],
            cwd: "/w".into(),
            env: Vec::new(),
        };
        assert_eq!(spec.command_line(), "ccr code --resume abc \"/x y\"");
        let bare = LaunchSpec {
            args: Vec::new(),
            ..spec
        };
        assert_eq!(bare.command_line(), "ccr code");
    }

    #[test]
    fn the_program_is_the_settings_one_trimmed_else_the_kinds() {
        assert_eq!(program(AgentKind::Claude, None), "claude");
        assert_eq!(program(AgentKind::Claude, Some("  ccr code  ")), "ccr code");
        assert_eq!(program(AgentKind::Claude, Some("\u{feff} \t")), "claude");
        assert_eq!(program(AgentKind::Codex, None), "codex");
        assert_eq!(program(AgentKind::Codex, Some("")), "codex");
    }

    /// Until task T4a writes Codex's launch, asking for one says so instead
    /// of typing anything.
    #[test]
    fn a_kind_without_a_launch_is_refused_with_its_name() {
        let request = LaunchRequest {
            kind: AgentKind::Codex,
            ..Default::default()
        };
        let error = build(&request).unwrap_err();
        assert_eq!(error, LaunchError::Unsupported(AgentKind::Codex));
        assert_eq!(error.to_string(), "Hangar cannot start Codex yet");
    }

    #[test]
    fn the_mcp_variable_is_set_with_a_file_and_removed_without_one() {
        let mut request = LaunchRequest::default();
        assert_eq!(mcp_env(&request), (MCP_CONFIG_VAR.to_string(), None));
        request.mcp = Some(McpRequest {
            file: "/data/mcp/ws-1.json".into(),
            arg: "\"$HANGAR_MCP_CONFIG\"".into(),
            strict: true,
        });
        assert_eq!(
            mcp_env(&request),
            (
                MCP_CONFIG_VAR.to_string(),
                Some(OsString::from("/data/mcp/ws-1.json"))
            )
        );
        request.mcp = Some(McpRequest::default());
        assert_eq!(mcp_env(&request), (MCP_CONFIG_VAR.to_string(), None));
    }

    #[test]
    fn an_empty_session_is_none() {
        let mut request = LaunchRequest::default();
        assert_eq!(resume(&request), None);
        request.resume = Some(String::new());
        assert_eq!(resume(&request), None);
        request.resume = Some("abc".into());
        assert_eq!(resume(&request), Some("abc"));
    }
}
