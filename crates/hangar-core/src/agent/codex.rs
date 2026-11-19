//! Codex's launch: the pane's own `codex app-server`, and the line the
//! pane's shell types (docs/native/agents.md §4.1, §4.2.1–4.2.2, D5, D6, D8).
//!
//! **Attached** (`LaunchRequest::server_socket`, D5). Hangar spawns
//! `<codex> app-server --listen unix://<socket>` with every setting of the
//! launch as a `-c` argument — arguments of a process it spawns, never typed,
//! never on screen — and the pane types `<codex> --remote unix://<socket>`.
//! The TUI still owns the settings of the threads it starts: it sends the
//! model, the effort, the approval policy, the reviewer and the sandbox mode
//! its own configuration resolves with each `thread/start`
//! (`tui/src/app_server_session.rs`: `ThreadParamsMode::Remote` is
//! `ThreadSettingsOwner::Client`), so those go on its line too, and both
//! sides agree. The rest it leaves to the server, so it goes there alone:
//! the developer instructions (sent by the TUI only with the
//! `terminal_visualization_instructions` feature, off by default:
//! `tui/src/terminal_visualization_instructions.rs`), the MCP servers, the
//! writable roots ("--add-dir is not supported with --remote. Configure
//! additional workspace roots on the server": `tui/src/startup_orchestration.rs`),
//! the folders' trust (which the TUI reads from the server with `config/read`:
//! `tui/src/config_update.rs`) and the features.
//!
//! **The floor** (no socket, D6). The plain `codex`, every setting on its
//! line. What is long or holds a path — Hangar's executable, the role, the
//! writable roots, the trusted folders — travels in a `HANGAR_CODEX_*`
//! variable the pane's shell starts with and expands, as `$HANGAR_MCP_CONFIG`
//! does for Claude Code: the line names the variable, never the value. A
//! shell that cannot hand the program a variable's value whole (PowerShell
//! and cmd, for a value holding a double quote) goes without that setting
//! rather than with a broken one.
//!
//! The person's `~/.codex/config.toml` is never written: `-c` is a layer of
//! its own, above the person's files, merged into what Codex reads and never
//! saved (`utils/cli/src/config_override.rs`, `config/src/overrides.rs`,
//! `config/src/loader/mod.rs`).
//!
//! Every flag, key and value below was read in `openai/codex` at the release
//! [`CHECKED_AT`] names (paths under `codex-rs/`, as each comment says); a
//! test holds it to the pin the protocol types were generated from, so moving
//! the pin means reading them again. Nothing here was run against a Codex
//! (§4.0): what the code relies on beyond the source says so.

use std::collections::HashSet;
use std::ffi::OsString;

use super::launch::{
    js_trim, program, quote, resume, LaunchRequest, LaunchSpec, ServerSpec, ShellSyntax,
    MCP_CONFIG_VAR,
};
use super::{AgentKind, PermissionMode};

/// The Codex release whose source the flags and keys of this file were read
/// in: the pin of the protocol types (`scripts/codex-protocol/subset.json`).
pub const CHECKED_AT: &str = "0.170.0";

// Flags (`cli/src/main.rs` unless said).

/// `-c key=value`, repeatable, accepted before or after a subcommand
/// (`utils/cli/src/config_override.rs`, `cli/src/config_override_args.rs`).
const CONFIG_FLAG: &str = "-c";
/// `codex app-server`, the protocol's server.
const APP_SERVER: &str = "app-server";
/// `codex app-server --listen <URL>` (`AppServerCommand`).
const LISTEN_FLAG: &str = "--listen";
/// The TUI's `--remote <ADDR>`, kept by `codex resume` too
/// (`InteractiveRemoteOptions`, `ResumeCommand`).
const REMOTE_FLAG: &str = "--remote";
/// A Unix socket at a path, for `--listen` and `--remote`
/// (`app-server-transport/src/transport/mod.rs`, `from_listen_url`).
const UNIX_SCHEME: &str = "unix://";

// Configuration keys (`config/src/config_toml.rs`, `ConfigToml`, unless
// said). `-c` splits a key on every dot and reads no quotes
// (`config/src/overrides.rs`, `apply_toml_override`), so a key here never
// holds a path: a path goes in the value.

const MODEL: &str = "model";
const EFFORT: &str = "model_reasoning_effort";
const APPROVAL_POLICY: &str = "approval_policy";
const SANDBOX_MODE: &str = "sandbox_mode";
const APPROVALS_REVIEWER: &str = "approvals_reviewer";
/// "Developer instructions inserted as a `developer` role message".
const DEVELOPER_INSTRUCTIONS: &str = "developer_instructions";
/// `SandboxWorkspaceWrite::writable_roots` (`config/src/types.rs`). An
/// array: it replaces the person's own list for the launch, arrays not being
/// merged (`config/src/merge.rs`).
const WRITABLE_ROOTS: &str = "sandbox_workspace_write.writable_roots";
/// `projects`, a table by folder of `ProjectConfig { trust_level }`, merged
/// with the person's own, folder by folder. Given whole, `{'<folder>' =
/// {trust_level = 'trusted'}}`, since a folder cannot be part of the key.
const PROJECTS: &str = "projects";
const TRUST_LEVEL: &str = "trust_level";
/// The image generation feature, stable and on by default
/// (`features/src/lib.rs`): on whatever the person's configuration says.
const IMAGE_GENERATION: &str = "features.image_generation";
/// Hangar's MCP server, `[mcp_servers.hangar]` (`RawMcpServerConfig`,
/// `config/src/mcp_types.rs`), given whole as one inline table.
const HANGAR_SERVER: &str = "mcp_servers.hangar";

// Values.

/// `AskForApproval` (`protocol/src/protocol.rs`), kebab-case.
const ON_REQUEST: &str = "on-request";
const NEVER: &str = "never";
/// `SandboxMode` (`protocol/src/config_types.rs`), kebab-case.
const WORKSPACE_WRITE: &str = "workspace-write";
const DANGER_FULL_ACCESS: &str = "danger-full-access";
/// `ApprovalsReviewer::AutoReview` (`protocol/src/config_types.rs`): the
/// automatic reviewer of `--approve-for-me`.
const AUTO_REVIEW: &str = "auto_review";
/// `TrustLevel::Trusted` (`protocol/src/config_types.rs`), lowercase.
const TRUSTED: &str = "trusted";
/// `AppToolApproval::Approve` (`config/src/mcp_types.rs`): a tool of the
/// server runs without a question (`core/src/mcp_tool_call.rs`,
/// `requires_mcp_tool_approval_for_mode`). Without it, a tool that does not
/// say it is read-only asks every time.
const APPROVE: &str = "approve";

// Hangar's MCP server.

/// `hangar-ai --mcp` (`hangar-app/src/main.rs`).
const MCP_ARG: &str = "--mcp";
/// What `hangar-ai --mcp` reads from its environment to know its pane and
/// reach the app (`mcp.rs`, `endpoint.rs`). Codex starts a stdio server from
/// a cleared environment plus a short list (`rmcp-client/src/utils.rs`,
/// `DEFAULT_ENV_VARS`), passing on only the variables `env_vars` names; one
/// that is not set is skipped.
const MCP_ENV_VARS: [&str; 10] = [
    "HANGAR_PANE_ID",
    "HANGAR_PANE_NAME",
    "HANGAR_AGENT",
    "HANGAR_WORKSPACE",
    "HANGAR_API_URL",
    "HANGAR_API_TOKEN",
    "HANGAR_BRIDGE_URL",
    "HANGAR_BRIDGE_TOKEN",
    crate::mcp::BOARD_RELAY,
    "HANGAR_ENDPOINT_FILE",
];
/// How long one of Hangar's tools may run, in seconds: some wait on the app
/// (a worker started, a delivery). Codex's own default is 60 (docs
/// `config-reference`); the field is read as a float (`config/src/mcp_types.rs`,
/// `option_duration_secs`).
const MCP_TOOL_TIMEOUT_SEC: &str = "120.0";
/// The rules of Claude Code's `--allowedTools` that let all of Hangar's
/// tools through: what Codex can say of one server (`AllowedTools::HangarTools`).
const HANGAR_TOOLS_RULES: [&str; 2] = ["mcp__hangar", "mcp__hangar__*"];

// The floor's variables: each holds a whole `key=value`, the line types
// `-c <the variable>`.

/// Hangar's MCP server: its executable's path holds the person's name.
pub const MCP_VAR: &str = "HANGAR_CODEX_MCP";
/// The role: a long text.
pub const ROLE_VAR: &str = "HANGAR_CODEX_ROLE";
/// The writable roots: paths.
pub const ROOTS_VAR: &str = "HANGAR_CODEX_ROOTS";
/// The trusted folders: paths.
pub const TRUST_VAR: &str = "HANGAR_CODEX_TRUST";
/// Every one, in the order the line types them.
const VARIABLES: [&str; 4] = [MCP_VAR, ROLE_VAR, ROOTS_VAR, TRUST_VAR];

/// One `-c` of a launch.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Setting {
    /// The argument after `-c`: the key, `=`, the value.
    arg: String,
    typed: Typed,
}

/// How a typed line carries a setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Typed {
    /// As it is, on every line: the TUI sends it with each thread itself,
    /// attached or not. Plain words only.
    Client,
    /// As it is, on the floor's line: attached, the server's alone.
    Floor,
    /// Through this variable, on the floor's line: attached, the server's
    /// alone.
    Variable(&'static str),
}

/// The line the pane's shell types: `codex --remote <socket>` when the pane
/// is attached to its server ([`server`]), the plain `codex` with every
/// setting otherwise; `resume <id>` last.
pub(super) fn spec(request: &LaunchRequest) -> LaunchSpec {
    let mut args: Vec<String> = Vec::new();
    let mut variables: Vec<(&'static str, String)> = Vec::new();
    match remote(request) {
        Some(address) => {
            args.push(REMOTE_FLAG.into());
            args.push(address);
            for setting in settings(request) {
                if setting.typed == Typed::Client {
                    args.push(CONFIG_FLAG.into());
                    args.push(setting.arg);
                }
            }
        }
        None => {
            for setting in settings(request) {
                match setting.typed {
                    Typed::Client | Typed::Floor => {
                        args.push(CONFIG_FLAG.into());
                        args.push(setting.arg);
                    }
                    Typed::Variable(name) => {
                        let Some(shell) = request.shell.filter(|shell| shell.carries(&setting.arg))
                        else {
                            continue;
                        };
                        args.push(CONFIG_FLAG.into());
                        args.push(shell.variable(name));
                        variables.push((name, setting.arg));
                    }
                }
            }
        }
    }
    // A thread id is a UUID; a session name can be anything, and is quoted.
    if let Some(id) = resume(request).and_then(|id| word(request.shell, id)) {
        args.extend(AgentKind::Codex.resume_args(&id));
    }
    LaunchSpec {
        program: program(AgentKind::Codex, request.command.as_deref()),
        args,
        cwd: request.cwd.clone(),
        env: env(request, &variables),
    }
}

/// The pane's `codex app-server`, listening on the pane's socket with every
/// setting of the launch; `None` for a pane on the floor — no socket, or one
/// its shell could not type.
pub(super) fn server(request: &LaunchRequest) -> Option<ServerSpec> {
    remote(request)?;
    let socket = request.server_socket.as_deref()?;
    let mut args = vec![
        APP_SERVER.to_string(),
        LISTEN_FLAG.to_string(),
        format!("{UNIX_SCHEME}{socket}"),
    ];
    for setting in settings(request) {
        args.push(CONFIG_FLAG.into());
        args.push(setting.arg);
    }
    Some(ServerSpec {
        program: program(AgentKind::Codex, request.command.as_deref()),
        args,
        cwd: request.cwd.clone(),
    })
}

/// The `--remote` address as the pane's shell types it; `None` without a
/// socket, or with one the shell cannot be typed: the pane then runs the
/// floor, and no server is started for it.
fn remote(request: &LaunchRequest) -> Option<String> {
    let socket = request
        .server_socket
        .as_deref()
        .filter(|socket| !socket.is_empty())?;
    word(request.shell, &format!("{UNIX_SCHEME}{socket}"))
}

/// Every setting of the launch, in the order they are given.
fn settings(request: &LaunchRequest) -> Vec<Setting> {
    let mut settings = Vec::new();
    if let Some(server) = hangar_server(request) {
        settings.push(Setting {
            arg: format!("{HANGAR_SERVER}={server}"),
            typed: Typed::Variable(MCP_VAR),
        });
    }
    // Assumption (from source, not run): the TUI attached with `--remote`
    // leaves the developer instructions to the server (module header).
    if let Some(role) = request
        .role
        .as_deref()
        .filter(|role| !js_trim(role).is_empty())
    {
        settings.push(Setting {
            arg: format!("{DEVELOPER_INSTRUCTIONS}={}", toml_string(role)),
            typed: Typed::Variable(ROLE_VAR),
        });
    }
    if let Some(mode) = request.permission_mode {
        settings.extend(permissions(mode).iter().map(|(key, value)| Setting {
            arg: format!("{key}={value}"),
            typed: Typed::Client,
        }));
    }
    let roots: Vec<String> = request
        .extra_roots
        .iter()
        .filter(|root| !root.is_empty())
        .map(|root| toml_string(root))
        .collect();
    if !roots.is_empty() {
        settings.push(Setting {
            arg: format!("{WRITABLE_ROOTS}=[{}]", roots.join(",")),
            typed: Typed::Variable(ROOTS_VAR),
        });
    }
    // A key twice in one inline table is not TOML.
    let mut seen = HashSet::new();
    let trusted: Vec<String> = request
        .trusted
        .iter()
        .filter(|folder| !folder.is_empty() && seen.insert(folder.as_str()))
        .map(|folder| {
            format!(
                "{}={{{TRUST_LEVEL}={}}}",
                toml_string(folder),
                toml_string(TRUSTED)
            )
        })
        .collect();
    if !trusted.is_empty() {
        settings.push(Setting {
            arg: format!("{PROJECTS}={{{}}}", trusted.join(",")),
            typed: Typed::Variable(TRUST_VAR),
        });
    }
    for (key, value) in [(MODEL, &request.model), (EFFORT, &request.effort)] {
        if let Some(value) = value.as_deref().map(js_trim).filter(|value| plain(value)) {
            settings.push(Setting {
                arg: format!("{key}={value}"),
                typed: Typed::Client,
            });
        }
    }
    settings.push(Setting {
        arg: format!("{IMAGE_GENERATION}=true"),
        typed: Typed::Floor,
    });
    settings
}

/// Hangar's orchestration policy as Codex's settings (§4.2.1). Each is what
/// Codex's own flag stands for; flags are not used, since `app-server`
/// takes none of them.
fn permissions(mode: PermissionMode) -> &'static [(&'static str, &'static str)] {
    match mode {
        // Codex has no "edits only": commands run in the workspace-write
        // sandbox, and what would leave it asks.
        PermissionMode::AcceptEdits => &[
            (APPROVAL_POLICY, ON_REQUEST),
            (SANDBOX_MODE, WORKSPACE_WRITE),
        ],
        // `--approve-for-me` (`cli/src/main.rs`, the test
        // `approve_for_me_configures_interactive_mode`).
        PermissionMode::Auto => &[
            (APPROVALS_REVIEWER, AUTO_REVIEW),
            (APPROVAL_POLICY, ON_REQUEST),
            (SANDBOX_MODE, WORKSPACE_WRITE),
        ],
        // `--dangerously-bypass-approvals-and-sandbox`
        // (`tui/src/startup_orchestration.rs`).
        PermissionMode::BypassPermissions => {
            &[(APPROVAL_POLICY, NEVER), (SANDBOX_MODE, DANGER_FULL_ACCESS)]
        }
    }
}

/// `[mcp_servers.hangar]` as one TOML inline table, when the launch names
/// Hangar's executable.
fn hangar_server(request: &LaunchRequest) -> Option<String> {
    let executable = request
        .hangar_mcp
        .as_deref()
        .filter(|executable| !executable.is_empty())?;
    let env_vars: Vec<String> = MCP_ENV_VARS.iter().map(|name| toml_string(name)).collect();
    let mut fields = vec![
        format!("command={}", toml_string(executable)),
        format!("args=[{}]", toml_string(MCP_ARG)),
        format!("env_vars=[{}]", env_vars.join(",")),
        format!("tool_timeout_sec={MCP_TOOL_TIMEOUT_SEC}"),
    ];
    if request
        .allowed_tools
        .iter()
        .any(|rule| HANGAR_TOOLS_RULES.contains(&rule.as_str()))
    {
        fields.push(format!(
            "default_tools_approval_mode={}",
            toml_string(APPROVE)
        ));
    }
    Some(format!("{{{}}}", fields.join(",")))
}

/// The variables of the pane's shell: Hangar's own, each set when the line
/// names it and removed otherwise — so a Hangar started inside a pane never
/// hands its own down, Claude Code's config file included — then the
/// account's (`accounts::env_for`: a shadow home's `CODEX_HOME`).
fn env(request: &LaunchRequest, set: &[(&'static str, String)]) -> Vec<(String, Option<OsString>)> {
    let account =
        crate::accounts::env_for(Some(AgentKind::Codex.id()), request.account_dir.as_deref());
    std::iter::once((MCP_CONFIG_VAR.to_string(), None))
        .chain(VARIABLES.iter().map(|name| {
            let value = set
                .iter()
                .find(|(set, _)| set == name)
                .map(|(_, value)| OsString::from(value));
            (name.to_string(), value)
        }))
        .chain(
            account
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.map(OsString::from))),
        )
        .collect()
}

/// `text` as a TOML string. A literal one (`'…'`, read exactly as written)
/// when it can be: it holds no double quote for a Windows shell to mangle,
/// and a Windows path keeps its backslashes as they are. A basic one
/// (`"…"`, escaped) when the text holds a single quote or a control
/// character, which a literal string cannot.
fn toml_string(text: &str) -> String {
    if !text.contains('\'') && !text.chars().any(char::is_control) {
        return format!("'{text}'");
    }
    let mut escaped = String::with_capacity(text.len() + 2);
    escaped.push('"');
    for c in text.chars() {
        match c {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c.is_control() => escaped.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => escaped.push(c),
        }
    }
    escaped.push('"');
    escaped
}

/// A value typed as it is, on the server and on every line: a word every
/// shell passes unchanged, which Codex reads as that very string.
///
/// `-c` reads a value that is not TOML as the string it is ("`-c model=o3`
/// without the quotes": `utils/cli/src/config_override.rs`,
/// `parse_overrides`). A word starting with a letter, of letters, digits and
/// `._:/-`, is not TOML — unless it is `true`, `false`, `inf` or `nan`, which
/// are refused. A model or an effort that is not such a word is dropped from
/// the launch, server included, so the TUI and its server never disagree.
fn plain(value: &str) -> bool {
    value.starts_with(|c: char| c.is_ascii_alphabetic())
        && bare(value)
        && !["true", "false", "inf", "nan"].contains(&value)
}

/// Whether every shell reads `value` as one word, as it is.
fn bare(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-'))
}

/// `value` as one word of the line: as it is when every shell reads it so,
/// quoted for the pane's shell otherwise.
fn word(shell: Option<ShellSyntax>, value: &str) -> Option<String> {
    if bare(value) {
        Some(value.to_string())
    } else {
        quote(shell, value)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::agent::launch::{build, server as launch_server};

    const SOCKET: &str = "/run/user/1000/hangar/codex-p1.sock";
    const HANGAR: &str = "/home/Ann Lee/.local/bin/hangar-ai";
    const THREAD: &str = "0199a5c4-7b3e-7c51-a2f4-6d0e8c1b9f20";

    fn some(text: &str) -> Option<String> {
        Some(text.to_string())
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    /// A worker's launch, as an orchestrator starts one: everything set.
    fn worker(shell: Option<ShellSyntax>) -> LaunchRequest {
        LaunchRequest {
            kind: AgentKind::Codex,
            cwd: "/work/wt-1".into(),
            extra_roots: strings(&["/work/shared"]),
            resume: some(THREAD),
            model: some("gpt-5.1-codex"),
            effort: some("high"),
            permission_mode: Some(PermissionMode::BypassPermissions),
            allowed_tools: strings(&["mcp__hangar"]),
            role: some("You are Vadim, a worker.\nDo the brief and nothing else."),
            shell,
            hangar_mcp: some(HANGAR),
            trusted: strings(&["/work/wt-1"]),
            ..Default::default()
        }
    }

    fn attached(shell: Option<ShellSyntax>) -> LaunchRequest {
        LaunchRequest {
            server_socket: some(SOCKET),
            ..worker(shell)
        }
    }

    fn line(request: &LaunchRequest) -> String {
        build(request).unwrap().command_line()
    }

    fn variable<'a>(spec: &'a LaunchSpec, name: &str) -> Option<&'a OsString> {
        spec.env
            .iter()
            .find(|(set, _)| set == name)
            .and_then(|(_, value)| value.as_ref())
    }

    /// What `hangar-ai --mcp` gets, as one inline table.
    fn hangar_table(executable: &str, approve: bool) -> String {
        format!(
            "{{command={executable},args=['--mcp'],env_vars=['HANGAR_PANE_ID','HANGAR_PANE_NAME',\
             'HANGAR_AGENT','HANGAR_WORKSPACE','HANGAR_API_URL','HANGAR_API_TOKEN',\
             'HANGAR_BRIDGE_URL','HANGAR_BRIDGE_TOKEN','HANGAR_BOARD_RELAY','HANGAR_ENDPOINT_FILE'],\
             tool_timeout_sec=120.0{}}}",
            if approve {
                ",default_tools_approval_mode='approve'"
            } else {
                ""
            }
        )
    }

    /// Every setting of [`worker`], as the server gets them.
    fn worker_settings() -> Vec<String> {
        vec![
            format!(
                "mcp_servers.hangar={}",
                hangar_table("'/home/Ann Lee/.local/bin/hangar-ai'", true)
            ),
            "developer_instructions=\"You are Vadim, a worker.\\nDo the brief and nothing else.\""
                .into(),
            "approval_policy=never".into(),
            "sandbox_mode=danger-full-access".into(),
            "sandbox_workspace_write.writable_roots=['/work/shared']".into(),
            "projects={'/work/wt-1'={trust_level='trusted'}}".into(),
            "model=gpt-5.1-codex".into(),
            "model_reasoning_effort=high".into(),
            "features.image_generation=true".into(),
        ]
    }

    fn with_flags(settings: &[String]) -> Vec<String> {
        settings
            .iter()
            .flat_map(|setting| ["-c".to_string(), setting.clone()])
            .collect()
    }

    // The flags of this file were read at the pin.

    #[test]
    fn the_flags_were_read_at_the_pinned_release() {
        assert_eq!(
            CHECKED_AT,
            crate::codex_app::generated::CODEX_VERSION,
            "the protocol pin moved: read the flags and keys of agent/codex.rs again \
             in openai/codex at the new release, then move CHECKED_AT"
        );
    }

    // Attached (D5).

    #[test]
    fn the_server_listens_on_the_panes_socket_with_every_setting() {
        let server = launch_server(&attached(Some(ShellSyntax::Posix))).unwrap();
        assert_eq!(server.program, "codex");
        assert_eq!(server.cwd, "/work/wt-1");
        let mut expected = strings(&[
            "app-server",
            "--listen",
            "unix:///run/user/1000/hangar/codex-p1.sock",
        ]);
        expected.extend(with_flags(&worker_settings()));
        assert_eq!(server.args, expected);
    }

    /// The server's arguments are a process's, never a shell's: whatever the
    /// pane's shell, they are the same, nothing quoted.
    #[test]
    fn the_servers_arguments_do_not_depend_on_the_shell() {
        let posix = launch_server(&attached(Some(ShellSyntax::Posix))).unwrap();
        for shell in [
            Some(ShellSyntax::Fish),
            Some(ShellSyntax::PowerShell),
            Some(ShellSyntax::Cmd),
            None,
        ] {
            assert_eq!(launch_server(&attached(shell)).unwrap(), posix, "{shell:?}");
        }
    }

    /// What the TUI sends with each thread itself, and only that, beside the
    /// socket: the same values as the server's.
    #[test]
    fn the_attached_line_carries_what_the_tui_decides_itself() {
        for shell in [
            ShellSyntax::Posix,
            ShellSyntax::Fish,
            ShellSyntax::PowerShell,
            ShellSyntax::Cmd,
        ] {
            assert_eq!(
                line(&attached(Some(shell))),
                format!(
                    "codex --remote unix:///run/user/1000/hangar/codex-p1.sock \
                     -c approval_policy=never -c sandbox_mode=danger-full-access \
                     -c model=gpt-5.1-codex -c model_reasoning_effort=high resume {THREAD}"
                ),
                "{shell:?}"
            );
        }
    }

    /// Nothing the attached line types is a path beyond the socket's, and
    /// its shell gets none of Hangar's variables.
    #[test]
    fn the_attached_panes_shell_holds_no_setting() {
        let spec = build(&attached(Some(ShellSyntax::Posix))).unwrap();
        for name in VARIABLES {
            assert_eq!(variable(&spec, name), None, "{name}");
        }
        let typed = spec.command_line();
        for hidden in [
            "Ann Lee",
            "hangar-ai",
            "You are",
            "/work/shared",
            "projects",
        ] {
            assert!(!typed.contains(hidden), "{hidden}: {typed}");
        }
    }

    // bash, zsh, fish, PowerShell and cmd: a socket whose path needs quotes.

    fn spaced_socket(shell: ShellSyntax) -> String {
        line(&LaunchRequest {
            server_socket: some("/home/Ann Lee/.hangar/run/codex-p1.sock"),
            resume: None,
            permission_mode: None,
            model: None,
            effort: None,
            ..worker(Some(shell))
        })
    }

    #[test]
    fn bash_and_zsh_type_a_spaced_socket_in_single_quotes() {
        for id in ["bash", "zsh"] {
            let shell = ShellSyntax::of(&crate::shells::ShellInfo {
                id: id.into(),
                label: id.into(),
                program: format!("/bin/{id}"),
                args: Vec::new(),
            })
            .unwrap();
            assert_eq!(
                spaced_socket(shell),
                "codex --remote 'unix:///home/Ann Lee/.hangar/run/codex-p1.sock'",
                "{id}"
            );
        }
    }

    #[test]
    fn fish_types_a_spaced_socket_in_single_quotes() {
        assert_eq!(
            spaced_socket(ShellSyntax::Fish),
            "codex --remote 'unix:///home/Ann Lee/.hangar/run/codex-p1.sock'"
        );
    }

    #[test]
    fn powershell_and_cmd_type_a_windows_socket_in_their_own_quotes() {
        let windows = |shell| {
            line(&LaunchRequest {
                kind: AgentKind::Codex,
                server_socket: some("C:\\Users\\Ann Lee\\AppData\\Local\\Hangar\\codex-p1.sock"),
                shell: Some(shell),
                ..Default::default()
            })
        };
        assert_eq!(
            windows(ShellSyntax::PowerShell),
            "codex --remote 'unix://C:\\Users\\Ann Lee\\AppData\\Local\\Hangar\\codex-p1.sock'"
        );
        assert_eq!(
            windows(ShellSyntax::Cmd),
            "codex --remote \"unix://C:\\Users\\Ann Lee\\AppData\\Local\\Hangar\\codex-p1.sock\""
        );
    }

    /// A socket the shell cannot be typed (cmd and a `%`) leaves the pane on
    /// the floor, with no server started for it: the line and the server
    /// always agree.
    #[test]
    fn a_socket_the_shell_cannot_type_runs_the_floor() {
        let request = LaunchRequest {
            server_socket: some("C:\\100%\\codex.sock"),
            ..worker(Some(ShellSyntax::Cmd))
        };
        assert_eq!(launch_server(&request), None);
        assert!(!line(&request).contains("--remote"));
        assert!(launch_server(&LaunchRequest {
            server_socket: some(""),
            ..worker(Some(ShellSyntax::Posix))
        })
        .is_none());
    }

    // The floor (D6): the same settings, typed.

    /// The floor's line types every setting the server would get, in the
    /// same order; what holds a path or a long text through its variable.
    #[test]
    fn the_floor_types_every_setting_of_the_server() {
        let posix = build(&worker(Some(ShellSyntax::Posix))).unwrap();
        assert_eq!(
            posix.command_line(),
            format!(
                "codex -c \"$HANGAR_CODEX_MCP\" -c \"$HANGAR_CODEX_ROLE\" \
                 -c approval_policy=never -c sandbox_mode=danger-full-access \
                 -c \"$HANGAR_CODEX_ROOTS\" -c \"$HANGAR_CODEX_TRUST\" \
                 -c model=gpt-5.1-codex -c model_reasoning_effort=high \
                 -c features.image_generation=true resume {THREAD}"
            )
        );
        let settings = worker_settings();
        assert_eq!(
            variable(&posix, MCP_VAR),
            Some(&OsString::from(&settings[0]))
        );
        assert_eq!(
            variable(&posix, ROLE_VAR),
            Some(&OsString::from(&settings[1]))
        );
        assert_eq!(
            variable(&posix, ROOTS_VAR),
            Some(&OsString::from(&settings[4]))
        );
        assert_eq!(
            variable(&posix, TRUST_VAR),
            Some(&OsString::from(&settings[5]))
        );
        assert!(!posix.command_line().contains("Ann Lee"));
        assert_eq!(launch_server(&worker(Some(ShellSyntax::Posix))), None);
    }

    #[test]
    fn fish_names_the_variables_as_bash_does() {
        let fish = build(&worker(Some(ShellSyntax::Fish))).unwrap();
        let posix = build(&worker(Some(ShellSyntax::Posix))).unwrap();
        assert_eq!(fish.args, posix.args);
        assert_eq!(fish.env, posix.env);
    }

    /// PowerShell and cmd hand a variable to the program through a command
    /// line, which a double quote does not cross the same way in every
    /// version: the role, a basic TOML string for its line break, stays off
    /// their lines; the rest is typed through `$env:` and `%…%`.
    #[test]
    fn powershell_and_cmd_name_the_variables_they_can_hand_over_whole() {
        let pwsh = build(&worker(Some(ShellSyntax::PowerShell))).unwrap();
        assert_eq!(
            pwsh.command_line(),
            format!(
                "codex -c $env:HANGAR_CODEX_MCP \
                 -c approval_policy=never -c sandbox_mode=danger-full-access \
                 -c $env:HANGAR_CODEX_ROOTS -c $env:HANGAR_CODEX_TRUST \
                 -c model=gpt-5.1-codex -c model_reasoning_effort=high \
                 -c features.image_generation=true resume {THREAD}"
            )
        );
        assert_eq!(variable(&pwsh, ROLE_VAR), None);

        let cmd = build(&worker(Some(ShellSyntax::Cmd))).unwrap();
        assert_eq!(
            cmd.command_line(),
            format!(
                "codex -c \"%HANGAR_CODEX_MCP%\" \
                 -c approval_policy=never -c sandbox_mode=danger-full-access \
                 -c \"%HANGAR_CODEX_ROOTS%\" -c \"%HANGAR_CODEX_TRUST%\" \
                 -c model=gpt-5.1-codex -c model_reasoning_effort=high \
                 -c features.image_generation=true resume {THREAD}"
            )
        );
        assert_eq!(variable(&cmd, ROLE_VAR), None);

        // A role of one line without a quote is a literal string: it goes.
        let one_line = LaunchRequest {
            role: some("You are the orchestrator."),
            ..worker(Some(ShellSyntax::Cmd))
        };
        let spec = build(&one_line).unwrap();
        assert!(spec.command_line().contains("-c \"%HANGAR_CODEX_ROLE%\""));
        assert_eq!(
            variable(&spec, ROLE_VAR),
            Some(&OsString::from(
                "developer_instructions='You are the orchestrator.'"
            ))
        );
    }

    /// A Windows path is a literal TOML string, backslashes as they are, and
    /// so holds no double quote for PowerShell or cmd to mangle.
    #[test]
    fn windows_paths_reach_powershell_and_cmd_as_literal_strings() {
        let request = LaunchRequest {
            kind: AgentKind::Codex,
            extra_roots: strings(&["D:\\shared", "C:\\Users\\Ann Lee\\repo"]),
            hangar_mcp: some("C:\\Program Files\\Hangar\\hangar-ai.exe"),
            trusted: strings(&["C:\\Users\\Ann Lee\\.hangar\\worktrees\\wt-1"]),
            shell: Some(ShellSyntax::PowerShell),
            ..Default::default()
        };
        let spec = build(&request).unwrap();
        assert_eq!(
            variable(&spec, ROOTS_VAR),
            Some(&OsString::from(
                "sandbox_workspace_write.writable_roots=['D:\\shared','C:\\Users\\Ann Lee\\repo']"
            ))
        );
        assert_eq!(
            variable(&spec, TRUST_VAR),
            Some(&OsString::from(
                "projects={'C:\\Users\\Ann Lee\\.hangar\\worktrees\\wt-1'={trust_level='trusted'}}"
            ))
        );
        assert_eq!(
            variable(&spec, MCP_VAR),
            Some(&OsString::from(format!(
                "mcp_servers.hangar={}",
                hangar_table("'C:\\Program Files\\Hangar\\hangar-ai.exe'", false)
            )))
        );
        for name in [MCP_VAR, ROOTS_VAR, TRUST_VAR] {
            assert!(
                spec.command_line().contains(&format!("$env:{name}")),
                "{name}"
            );
        }
    }

    /// A shell Hangar does not know (Nushell, a WSL pane) has no variable
    /// syntax Hangar can type: the plain settings only.
    #[test]
    fn an_unknown_shell_gets_the_plain_settings_only() {
        let spec = build(&worker(None)).unwrap();
        assert_eq!(
            spec.command_line(),
            format!(
                "codex -c approval_policy=never -c sandbox_mode=danger-full-access \
                 -c model=gpt-5.1-codex -c model_reasoning_effort=high \
                 -c features.image_generation=true resume {THREAD}"
            )
        );
        for name in VARIABLES {
            assert_eq!(variable(&spec, name), None, "{name}");
        }
    }

    /// The typed floor, read by a real POSIX shell: the program receives
    /// exactly the settings the server would, each one argument.
    #[cfg(unix)]
    #[test]
    fn sh_hands_codex_the_settings_of_the_server() {
        let spec = build(&worker(Some(ShellSyntax::Posix))).unwrap();
        let script = format!(
            "codex() {{ for a in \"$@\"; do printf '%s\\0' \"$a\"; done; }}; {}",
            spec.command_line()
        );
        let mut command = std::process::Command::new("sh");
        command.arg("-c").arg(&script);
        for (name, value) in &spec.env {
            match value {
                Some(value) => command.env(name, value),
                None => command.env_remove(name),
            };
        }
        let output = command.output().expect("sh runs");
        assert!(output.status.success(), "{output:?}");
        let received: Vec<String> = String::from_utf8(output.stdout)
            .unwrap()
            .split_terminator('\0')
            .map(str::to_string)
            .collect();
        let mut expected = with_flags(&worker_settings());
        expected.extend(strings(&["resume", THREAD]));
        assert_eq!(received, expected);
    }

    // Each setting.

    #[test]
    fn each_policy_is_the_settings_its_codex_flag_stands_for() {
        let typed = |mode| {
            line(&LaunchRequest {
                kind: AgentKind::Codex,
                permission_mode: Some(mode),
                shell: Some(ShellSyntax::Posix),
                ..Default::default()
            })
        };
        let floor = "-c features.image_generation=true";
        assert_eq!(
            typed(PermissionMode::AcceptEdits),
            format!("codex -c approval_policy=on-request -c sandbox_mode=workspace-write {floor}")
        );
        assert_eq!(
            typed(PermissionMode::Auto),
            format!(
                "codex -c approvals_reviewer=auto_review -c approval_policy=on-request \
                 -c sandbox_mode=workspace-write {floor}"
            )
        );
        assert_eq!(
            typed(PermissionMode::BypassPermissions),
            format!("codex -c approval_policy=never -c sandbox_mode=danger-full-access {floor}")
        );
    }

    /// No policy: the person's own, from their configuration.
    #[test]
    fn without_a_policy_codex_keeps_the_persons_settings() {
        let request = LaunchRequest {
            kind: AgentKind::Codex,
            server_socket: some(SOCKET),
            shell: Some(ShellSyntax::Posix),
            ..Default::default()
        };
        assert_eq!(line(&request), format!("codex --remote unix://{SOCKET}"));
        assert_eq!(
            launch_server(&request).unwrap().args,
            strings(&[
                "app-server",
                "--listen",
                "unix:///run/user/1000/hangar/codex-p1.sock",
                "-c",
                "features.image_generation=true"
            ])
        );
    }

    /// Hangar's tools go through without a question only when the launch
    /// lets them (`allowedTools` of an orchestration pane); a rule for
    /// another tool means nothing to Codex.
    #[test]
    fn hangars_tools_are_approved_only_when_the_launch_lets_them_through() {
        let table = |rules: &[&str]| {
            let server = launch_server(&LaunchRequest {
                allowed_tools: strings(rules),
                ..attached(Some(ShellSyntax::Posix))
            })
            .unwrap();
            server.args[4].clone()
        };
        let approved = format!(
            "mcp_servers.hangar={}",
            hangar_table(&toml_string(HANGAR), true)
        );
        let asked = format!(
            "mcp_servers.hangar={}",
            hangar_table(&toml_string(HANGAR), false)
        );
        assert_eq!(table(&["mcp__hangar"]), approved);
        assert_eq!(table(&["mcp__hangar__*"]), approved);
        assert_eq!(table(&[]), asked);
        assert_eq!(table(&["Read", "mcp__hangar__board_list_tasks"]), asked);
    }

    #[test]
    fn without_hangars_executable_no_server_of_hangars_is_added() {
        let request = LaunchRequest {
            hangar_mcp: None,
            ..attached(Some(ShellSyntax::Posix))
        };
        assert!(!launch_server(&request)
            .unwrap()
            .args
            .iter()
            .any(|arg| arg.starts_with("mcp_servers.")));
        let floor = LaunchRequest {
            hangar_mcp: some(""),
            ..worker(Some(ShellSyntax::Posix))
        };
        assert!(!line(&floor).contains(MCP_VAR));
    }

    /// The role's text reaches Codex exactly, whatever it holds.
    #[test]
    fn the_role_is_a_toml_string_holding_it_exactly() {
        assert_eq!(toml_string("Plain role."), "'Plain role.'");
        assert_eq!(toml_string("C:\\x\\"), "'C:\\x\\'");
        assert_eq!(
            toml_string("Don't \"quote\" C:\\x\n\tnext\r\u{7}"),
            "\"Don't \\\"quote\\\" C:\\\\x\\n\\tnext\\r\\u0007\""
        );
        assert_eq!(toml_string("l'été"), "\"l'été\"");
        let server = |role: &str| {
            launch_server(&LaunchRequest {
                role: some(role),
                ..attached(Some(ShellSyntax::Posix))
            })
            .unwrap()
            .args
        };
        assert!(server("A role.").contains(&"developer_instructions='A role.'".to_string()));
        // An empty role is no role.
        assert!(!server(" \n ")
            .iter()
            .any(|arg| arg.starts_with("developer_instructions")));
    }

    /// A folder is trusted once, its dots and quotes in the value, not the
    /// key; an empty one is none.
    #[test]
    fn the_trusted_folders_are_one_table_of_folders() {
        let server = launch_server(&LaunchRequest {
            trusted: strings(&["/w/a.b", "", "/w/it's", "/w/a.b"]),
            ..attached(Some(ShellSyntax::Posix))
        })
        .unwrap();
        assert!(server.args.contains(
            &"projects={'/w/a.b'={trust_level='trusted'},\"/w/it's\"={trust_level='trusted'}}"
                .to_string()
        ));
        let none = launch_server(&LaunchRequest {
            trusted: strings(&[""]),
            ..attached(Some(ShellSyntax::Posix))
        })
        .unwrap();
        assert!(!none.args.iter().any(|arg| arg.starts_with("projects")));
    }

    /// A model or an effort is a plain word on the server and on the line,
    /// or it is not given at all: what `-c` would read as something else
    /// (`true`, a number) or a shell would split is dropped.
    #[test]
    fn a_model_that_is_not_a_plain_word_is_dropped_everywhere() {
        for model in [
            "gpt 5",
            "1.5",
            "true",
            "nan",
            "-x",
            "o3; rm -rf ~",
            "model$HOME",
            "  ",
        ] {
            let request = LaunchRequest {
                model: some(model),
                ..attached(Some(ShellSyntax::Posix))
            };
            assert!(!line(&request).contains("model="), "{model}");
            assert!(
                !launch_server(&request)
                    .unwrap()
                    .args
                    .iter()
                    .any(|arg| arg.starts_with("model=")),
                "{model}"
            );
        }
        for model in ["o3", "gpt-5.1-codex", "openai/gpt-oss-20b", "  gpt-5  "] {
            let request = LaunchRequest {
                model: some(model),
                ..attached(Some(ShellSyntax::Posix))
            };
            assert!(
                line(&request).contains(&format!("-c model={}", model.trim())),
                "{model}"
            );
        }
    }

    #[test]
    fn a_session_name_is_quoted_for_the_shell_and_dropped_when_it_cannot_be() {
        let resume = |name: &str, shell| {
            line(&LaunchRequest {
                kind: AgentKind::Codex,
                resume: some(name),
                shell,
                ..Default::default()
            })
        };
        let floor = "codex -c features.image_generation=true";
        assert_eq!(
            resume("My work", Some(ShellSyntax::Posix)),
            format!("{floor} resume 'My work'")
        );
        assert_eq!(
            resume("My work", Some(ShellSyntax::Cmd)),
            format!("{floor} resume \"My work\"")
        );
        assert_eq!(resume("100%", Some(ShellSyntax::Cmd)), floor);
        assert_eq!(resume("", Some(ShellSyntax::Posix)), floor);
    }

    // The rest of the spec.

    #[test]
    fn a_custom_command_is_the_program_of_the_line_and_of_the_server() {
        let request = LaunchRequest {
            command: some("  /opt/codex/bin/codex  "),
            ..attached(Some(ShellSyntax::Posix))
        };
        assert_eq!(build(&request).unwrap().program, "/opt/codex/bin/codex");
        assert_eq!(
            launch_server(&request).unwrap().program,
            "/opt/codex/bin/codex"
        );
    }

    /// Codex reads no config file of Hangar's, so Claude Code's variable is
    /// removed; an account's shadow home comes last, as `accounts::env_for`
    /// gives it.
    #[test]
    fn its_shell_gets_hangars_variables_then_the_accounts() {
        let main = build(&LaunchRequest {
            kind: AgentKind::Codex,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            main.env,
            [
                (MCP_CONFIG_VAR.to_string(), None),
                (MCP_VAR.to_string(), None),
                (ROLE_VAR.to_string(), None),
                (ROOTS_VAR.to_string(), None),
                (TRUST_VAR.to_string(), None),
            ]
        );

        let home = PathBuf::from("/home/me/.hangar/accounts/codex-work");
        let spec = build(&LaunchRequest {
            kind: AgentKind::Codex,
            account_dir: Some(home.clone()),
            ..Default::default()
        })
        .unwrap();
        let account: Vec<(String, Option<OsString>)> =
            crate::accounts::env_for(Some("codex"), Some(&home))
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.map(OsString::from)))
                .collect();
        assert!(account.iter().any(
            |(name, value)| name == "CODEX_HOME" && value.as_deref() == Some(home.as_os_str())
        ));
        assert_eq!(spec.env[5..], account[..]);
    }

    /// The person's configuration file is a layer below `-c`, never named:
    /// nothing of the launch points Codex at a file to write.
    #[test]
    fn nothing_names_the_persons_configuration() {
        let server = launch_server(&attached(Some(ShellSyntax::Posix))).unwrap();
        let floor = build(&worker(Some(ShellSyntax::Posix))).unwrap();
        let everything =
            server
                .args
                .iter()
                .chain(&floor.args)
                .cloned()
                .chain(floor.env.iter().filter_map(|(_, value)| {
                    value
                        .as_ref()
                        .map(|value| value.to_string_lossy().into_owned())
                }));
        for text in everything {
            assert!(!text.contains("config.toml"), "{text}");
            assert!(!text.starts_with("--profile"), "{text}");
        }
    }
}
