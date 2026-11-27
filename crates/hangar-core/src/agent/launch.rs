//! One launch, for any agent (docs/native/agents.md §3.4, D4).
//!
//! A [`LaunchRequest`] says what a pane wants started — which agent, on which
//! session, with which model, account, permissions and MCP servers — in words
//! no agent owns; [`build`] turns it into a [`LaunchSpec`]: the command the
//! pane's shell types, and the variables the shell starts with for that
//! agent; [`server`] says which process Hangar starts for the pane first, for
//! a kind that runs attached to a server of its own (Codex, D5). Each kind's
//! spellings live in its own file (`claude.rs`, `codex.rs`); [`build`] and
//! [`server`] are the one place that picks the file by kind.
//!
//! What a pane gets whatever runs in it stays with the PTY (`pty.rs`): its
//! identity and API token, the teskad-bridge mod and the kits' plugin
//! folders, its ports, the build cache. The engine's panes, and the desktop's
//! Claude Code panes through `hangar-engine/src/panes/agents.rs`, type the
//! line [`build`] answers (task T14a); the tests of `claude.rs` hold it to the
//! lines Claude Code was always typed.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use super::{claude, codex, AgentKind, PermissionMode};
use crate::shells::ShellInfo;

/// The variable a pane's shell holds its MCP config file's path in: the line
/// typed names the variable instead of the path, which lies in Hangar's data
/// folder, so holds the person's name, and would stay on screen.
pub const MCP_CONFIG_VAR: &str = "TESKAD_MCP_CONFIG";

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
    /// How the pane's shell reads the line typed into it, for a kind whose
    /// line carries values the builder quotes itself (Codex). Absent, not
    /// known: the line carries only words every shell reads as they are.
    pub shell: Option<ShellSyntax>,
    /// The socket of the server Hangar starts for the pane, for a kind that
    /// runs attached to one (Codex, D5): [`server`] says how to start it, and
    /// the typed line points the pane's program at it. Absent, the kind runs
    /// on its own (Codex's plain TUI, the floor: D6).
    pub server_socket: Option<String>,
    /// The executable of Hangar's own MCP server (`<it> --mcp`), for a kind
    /// that takes its servers as configuration overrides
    /// (`McpInjection::Overrides`): every Codex pane gets Hangar's server
    /// this way (D8). Claude Code reads it from its configuration files, so
    /// its line ignores this.
    pub hangar_mcp: Option<String>,
    /// The launch's other MCP servers, for a kind that takes its servers as
    /// configuration overrides (`McpInjection::Overrides`): each kit's, and,
    /// in a workspace in « custom » MCP mode, each of the agent's own that the
    /// workspace did not choose, turned off — by name, each an object in the
    /// kind's own keys (`mcp_scope::codex_mcp_scope`). Claude Code gets the
    /// same from its `--mcp-config` file ([`LaunchRequest::mcp`]), so its line
    /// ignores this.
    pub mcp_overrides: Vec<(String, serde_json::Value)>,
    /// Folders the agent is to trust for this launch only, for a kind that
    /// asks before working in a folder it does not trust (Codex): a worker's
    /// worktree, when its workspace's own folder is trusted in the person's
    /// configuration — which Hangar reads and never writes (§4.2.1).
    pub trusted: Vec<String>,
}

/// How a shell reads the line typed into it: the quotes it takes, and how it
/// names a variable of its environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellSyntax {
    /// bash, zsh, sh, dash, ksh, and the bash of Git for Windows, MSYS2 and
    /// Cygwin.
    Posix,
    Fish,
    /// PowerShell 7 (`pwsh`) and Windows PowerShell.
    PowerShell,
    Cmd,
}

impl ShellSyntax {
    /// The syntax of a shell by the id `shells.rs` gives it, else by its
    /// program's name; `None` for one Hangar does not type for (Nushell, a
    /// WSL pane) or does not know.
    pub fn of(shell: &ShellInfo) -> Option<ShellSyntax> {
        ShellSyntax::by_name(&shell.id).or_else(|| {
            let program = shell
                .program
                .rsplit(['\\', '/'])
                .next()
                .unwrap_or_default()
                .to_lowercase();
            ShellSyntax::by_name(program.strip_suffix(".exe").unwrap_or(&program))
        })
    }

    fn by_name(name: &str) -> Option<ShellSyntax> {
        Some(match name {
            "bash" | "zsh" | "sh" | "dash" | "ksh" | "git-bash" | "msys2" | "cygwin" => {
                ShellSyntax::Posix
            }
            "fish" => ShellSyntax::Fish,
            "pwsh" | "powershell" => ShellSyntax::PowerShell,
            "cmd" => ShellSyntax::Cmd,
            _ => return None,
        })
    }

    /// `value` as one word this shell hands the program exactly as it is;
    /// `None` when it cannot be typed so.
    ///
    /// - The POSIX shells take single quotes literally; a quote inside is
    ///   closed, escaped and reopened.
    /// - fish's single quotes still read `\\` and `\'`.
    /// - PowerShell's single quotes are literal, a quote inside doubled — and
    ///   the typographic single quotes are quotes to it too. A double quote is
    ///   refused: Windows PowerShell and PowerShell 7 pass one to a native
    ///   program differently (`$PSNativeCommandArgumentPassing`), and so is a
    ///   trailing backslash, which the double quotes Windows PowerShell wraps
    ///   a word with would read as escaping the closing one.
    /// - cmd only has double quotes, which cannot hold one, and expands
    ///   `%VAR%` even between them; a trailing backslash would escape the
    ///   closing quote for the C runtime that splits the program's line.
    ///
    /// No shell is typed a control character: a line typed into a terminal
    /// stays one line, and a Ctrl-C or a Tab would be read as keys.
    pub fn quote(self, value: &str) -> Option<String> {
        if value.is_empty() || value.chars().any(char::is_control) {
            return None;
        }
        match self {
            ShellSyntax::Posix => Some(format!("'{}'", value.replace('\'', "'\\''"))),
            ShellSyntax::Fish => Some(format!(
                "'{}'",
                value.replace('\\', "\\\\").replace('\'', "\\'")
            )),
            ShellSyntax::PowerShell => {
                if !windows_native_arg(value) {
                    return None;
                }
                let mut quoted = String::from("'");
                for c in value.chars() {
                    if POWERSHELL_SINGLE_QUOTES.contains(&c) {
                        quoted.push(c);
                    }
                    quoted.push(c);
                }
                quoted.push('\'');
                Some(quoted)
            }
            ShellSyntax::Cmd => {
                (windows_native_arg(value) && !value.contains('%')).then(|| format!("\"{value}\""))
            }
        }
    }

    /// The variable `name` of the shell's environment as one word, which the
    /// shell replaces with the variable's value: what a line types instead of
    /// a value it should not show (a path holding the person's name, a long
    /// text). See [`carries`](Self::carries) for the values that arrive
    /// whole.
    pub fn variable(self, name: &str) -> String {
        match self {
            ShellSyntax::Posix | ShellSyntax::Fish => format!("\"${name}\""),
            ShellSyntax::PowerShell => format!("$env:{name}"),
            // Expanded once, before the line is read: what it holds is not
            // expanded again, `%` included.
            ShellSyntax::Cmd => format!("\"%{name}%\""),
        }
    }

    /// Whether a variable holding `value`, typed as
    /// [`variable`](Self::variable), reaches the program as `value`. The
    /// POSIX shells and fish pass it whole; PowerShell and cmd hand it to the
    /// program through a command line, with the same limits as
    /// [`quote`](Self::quote), less cmd's `%`.
    pub fn carries(self, value: &str) -> bool {
        match self {
            ShellSyntax::Posix | ShellSyntax::Fish => true,
            ShellSyntax::PowerShell | ShellSyntax::Cmd => {
                !value.chars().any(char::is_control) && windows_native_arg(value)
            }
        }
    }
}

/// What PowerShell reads as a single quote: `'` and the typographic ones.
const POWERSHELL_SINGLE_QUOTES: [char; 5] = ['\'', '\u{2018}', '\u{2019}', '\u{201a}', '\u{201b}'];

/// A value a Windows shell can hand a native program whole: not empty (an
/// empty word is dropped), no double quote, no trailing backslash.
fn windows_native_arg(value: &str) -> bool {
    !value.is_empty() && !value.contains('"') && !value.ends_with('\\')
}

/// `value` as one word `shell` hands the program exactly as it is, the shell
/// known or not: an unknown one gets single quotes, which every shell above
/// but cmd reads literally as long as they hold no quote and no backslash.
pub(super) fn quote(shell: Option<ShellSyntax>, value: &str) -> Option<String> {
    match shell {
        Some(shell) => shell.quote(value),
        None => (!value.is_empty()
            && !value.chars().any(char::is_control)
            && !value.contains(['\'', '\\']))
        .then(|| format!("'{value}'")),
    }
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

/// A process Hangar starts for a pane before typing the pane's line: the
/// server the pane's program is attached to (Codex's `app-server`, D5). It
/// runs with the pane's environment and the variables of the pane's
/// [`LaunchSpec::env`], and lives and dies with the pane (task T4b).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSpec {
    /// The kind's command, the very one the pane types (the settings' one,
    /// else the kind's program), so the server and the pane's program are
    /// the same release. Started as one executable, never through a shell.
    pub program: String,
    /// Its arguments, each exactly as the program receives it: no shell
    /// reads them, so nothing is quoted, and nothing is ever on screen.
    pub args: Vec<String>,
    /// The folder it starts in: the pane's.
    pub cwd: String,
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
        AgentKind::Codex => Ok(codex::spec(request)),
    }
}

/// The server Hangar starts for the pane before typing [`build`]'s line, for
/// a kind that runs attached to one; `None` when the pane's program runs on
/// its own. The line [`build`] gives points at this server exactly when
/// there is one.
pub fn server(request: &LaunchRequest) -> Option<ServerSpec> {
    match request.kind {
        AgentKind::Claude => None,
        AgentKind::Codex => codex::server(request),
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

    /// Both kinds start; a kind Hangar could not start would say so by name.
    #[test]
    fn every_kind_has_a_launch_and_a_refusal_names_the_kind() {
        for kind in AgentKind::ALL {
            let request = LaunchRequest {
                kind,
                ..Default::default()
            };
            assert_eq!(build(&request).unwrap().program, kind.info().program);
        }
        assert_eq!(
            LaunchError::Unsupported(AgentKind::Codex).to_string(),
            "Hangar cannot start Codex yet"
        );
    }

    /// Only a kind that runs attached, and only when given a socket, has a
    /// server.
    #[test]
    fn a_server_is_started_only_for_an_attached_kind_given_a_socket() {
        let socket = Some("/run/hangar/p1.sock".to_string());
        for (kind, socket, expected) in [
            (AgentKind::Claude, socket.clone(), false),
            (AgentKind::Codex, socket, true),
            (AgentKind::Codex, None, false),
        ] {
            let request = LaunchRequest {
                kind,
                server_socket: socket,
                shell: Some(ShellSyntax::Posix),
                ..Default::default()
            };
            assert_eq!(server(&request).is_some(), expected, "{kind:?}");
        }
    }

    fn shell(id: &str, program: &str) -> ShellInfo {
        ShellInfo {
            id: id.into(),
            label: id.into(),
            program: program.into(),
            args: Vec::new(),
        }
    }

    #[test]
    fn a_shell_is_known_by_its_id_else_by_its_program() {
        for (id, program, syntax) in [
            ("bash", "/bin/bash", Some(ShellSyntax::Posix)),
            ("zsh", "/bin/zsh", Some(ShellSyntax::Posix)),
            (
                "git-bash",
                "C:\\Program Files\\Git\\bin\\bash.exe",
                Some(ShellSyntax::Posix),
            ),
            ("fish", "/usr/bin/fish", Some(ShellSyntax::Fish)),
            (
                "pwsh",
                "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
                Some(ShellSyntax::PowerShell),
            ),
            (
                "powershell",
                "powershell.exe",
                Some(ShellSyntax::PowerShell),
            ),
            (
                "cmd",
                "C:\\Windows\\System32\\cmd.exe",
                Some(ShellSyntax::Cmd),
            ),
            ("custom", "/usr/local/bin/ZSH", Some(ShellSyntax::Posix)),
            (
                "custom",
                "C:\\tools\\PWSH.EXE",
                Some(ShellSyntax::PowerShell),
            ),
            ("nu", "/usr/bin/nu", None),
            ("wsl", "C:\\Windows\\System32\\wsl.exe", None),
            ("custom", "/opt/xonsh", None),
        ] {
            assert_eq!(
                ShellSyntax::of(&shell(id, program)),
                syntax,
                "{id} {program}"
            );
        }
    }

    // Quoting, shell by shell.

    #[test]
    fn bash_and_zsh_take_anything_in_single_quotes() {
        let posix = ShellSyntax::of(&shell("zsh", "/bin/zsh")).unwrap();
        assert_eq!(posix, ShellSyntax::of(&shell("bash", "/bin/bash")).unwrap());
        assert_eq!(posix.quote("a b"), Some("'a b'".into()));
        assert_eq!(posix.quote("it's"), Some("'it'\\''s'".into()));
        assert_eq!(
            posix.quote("$HOME `x` \"y\" \\z !1 %P%"),
            Some("'$HOME `x` \"y\" \\z !1 %P%'".into())
        );
        assert_eq!(posix.variable("TESKAD_X"), "\"$TESKAD_X\"");
        assert!(posix.carries("x=\"a\nb\"\\"));
    }

    #[test]
    fn fish_escapes_backslashes_and_quotes_in_single_quotes() {
        let fish = ShellSyntax::Fish;
        assert_eq!(fish.quote("a b"), Some("'a b'".into()));
        assert_eq!(fish.quote("it's"), Some("'it\\'s'".into()));
        assert_eq!(fish.quote("C:\\x\\\\y"), Some("'C:\\\\x\\\\\\\\y'".into()));
        assert_eq!(fish.quote("$HOME \"y\""), Some("'$HOME \"y\"'".into()));
        assert_eq!(fish.variable("TESKAD_X"), "\"$TESKAD_X\"");
        assert!(fish.carries("x=\"a\nb\"\\"));
    }

    #[test]
    fn powershell_doubles_its_single_quotes_and_refuses_double_ones() {
        let pwsh = ShellSyntax::PowerShell;
        assert_eq!(pwsh.quote("a b"), Some("'a b'".into()));
        assert_eq!(pwsh.quote("it's"), Some("'it''s'".into()));
        assert_eq!(
            pwsh.quote("l\u{2019}été"),
            Some("'l\u{2019}\u{2019}été'".into())
        );
        assert_eq!(
            pwsh.quote("$env:HOME C:\\x"),
            Some("'$env:HOME C:\\x'".into())
        );
        assert_eq!(pwsh.quote("a \"b\""), None);
        assert_eq!(pwsh.quote("C:\\dir\\"), None);
        assert_eq!(pwsh.variable("TESKAD_X"), "$env:TESKAD_X");
        assert!(pwsh.carries("x='C:\\a b'"));
        assert!(!pwsh.carries("x=\"a\""));
        assert!(!pwsh.carries("x='a\nb'"));
    }

    #[test]
    fn cmd_takes_double_quotes_without_a_quote_or_a_percent_inside() {
        let cmd = ShellSyntax::Cmd;
        assert_eq!(cmd.quote("a b & c"), Some("\"a b & c\"".into()));
        assert_eq!(
            cmd.quote("C:\\Users\\it's"),
            Some("\"C:\\Users\\it's\"".into())
        );
        assert_eq!(cmd.quote("a \"b\""), None);
        assert_eq!(cmd.quote("100%"), None);
        assert_eq!(cmd.quote("C:\\dir\\"), None);
        assert_eq!(cmd.variable("TESKAD_X"), "\"%TESKAD_X%\"");
        // Expanded once: a `%` in the value stays as it is.
        assert!(cmd.carries("x='C:\\100%'"));
        assert!(!cmd.carries("x=\"a\""));
        assert!(!cmd.carries("x='a\rb'"));
    }

    /// A shell whose syntax is not known gets single quotes, holding neither
    /// a quote nor a backslash.
    #[test]
    fn an_unknown_shell_gets_plain_single_quotes_or_nothing() {
        assert_eq!(quote(None, "a b"), Some("'a b'".into()));
        assert_eq!(quote(None, "it's"), None);
        assert_eq!(quote(None, "C:\\x"), None);
    }

    /// No shell is typed an empty word or a control character: Enter,
    /// Ctrl-C or Tab would be keys.
    #[test]
    fn no_shell_is_typed_a_control_character() {
        for syntax in [
            Some(ShellSyntax::Posix),
            Some(ShellSyntax::Fish),
            Some(ShellSyntax::PowerShell),
            Some(ShellSyntax::Cmd),
            None,
        ] {
            for value in ["", "a\nb", "a\rb", "a\tb", "a\u{3}b", "a\u{1b}[2J"] {
                assert_eq!(quote(syntax, value), None, "{syntax:?} {value:?}");
            }
        }
    }

    /// The POSIX quoting, read back by a real shell.
    #[cfg(unix)]
    #[test]
    fn sh_reads_a_quoted_word_back_as_it_was() {
        let values = [
            "plain",
            "a b",
            "it's",
            "''",
            "$HOME `id` $(id) \"q\" \\ \\\\ * ? [a] ~ ! # ; & | < > { } %P%",
            "C:\\Users\\Ann Lee\\",
            "l’été ünïcode",
        ];
        for value in values {
            let word = ShellSyntax::Posix.quote(value).unwrap();
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("printf '%s' {word}"))
                .output()
                .expect("sh runs");
            assert!(output.status.success(), "{value:?}: {output:?}");
            assert_eq!(String::from_utf8(output.stdout).unwrap(), value, "{word}");
        }
    }

    #[test]
    fn the_mcp_variable_is_set_with_a_file_and_removed_without_one() {
        let mut request = LaunchRequest::default();
        assert_eq!(mcp_env(&request), (MCP_CONFIG_VAR.to_string(), None));
        request.mcp = Some(McpRequest {
            file: "/data/mcp/ws-1.json".into(),
            arg: "\"$TESKAD_MCP_CONFIG\"".into(),
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
