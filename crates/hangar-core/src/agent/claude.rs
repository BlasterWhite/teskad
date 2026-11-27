//! Claude Code's launch: the flags its line carries and the variables its
//! shell starts with (docs/native/agents.md §4.2.1, the Claude Code column).
//!
//! Moved from hangar-engine's `panes/agents.rs` (`launchCommand` of
//! lib/agents.ts), which now asks [`launch::build`](super::launch::build)
//! for it (task T14a). The tests below are that file's, and the desktop's
//! that go through it, with the very same lines: Claude Code starts byte for
//! byte as it did.

use std::ffi::OsString;

use super::launch::{
    all_of, is_quotable, is_word, js_trim, mcp_env, program, resume, LaunchRequest, LaunchSpec,
};
use super::AgentKind;

/// Claude Code's line, in the order it has always been typed: the session,
/// its name and model, then the permission mode, and the variadic flags
/// last — each closed by the flag after it, `--add-dir` by nothing, since a
/// flag after it (one the custom command carries, the resume) would be
/// swallowed as one more directory.
pub(super) fn spec(request: &LaunchRequest) -> LaunchSpec {
    let mut args: Vec<String> = Vec::new();
    if let Some(id) = resume(request) {
        args.extend(AgentKind::Claude.resume_args(id));
    }
    if let Some(name) = request
        .name
        .as_deref()
        .filter(|name| !name.is_empty() && is_quotable(name))
    {
        args.push("--name".into());
        args.push(word(name, |c| is_word(c) || c == '-'));
    }
    // An alias or an id: `sonnet`, `claude-sonnet-4-5`, `opus[1m]`; the last
    // quoted, since brackets are a glob to bash and zsh.
    if let Some(model) = request
        .model
        .as_deref()
        .map(js_trim)
        .filter(|model| !model.is_empty() && is_quotable(model))
    {
        args.push("--model".into());
        args.push(word(model, |c| is_word(c) || c == '.' || c == '-'));
    }
    if let Some(mode) = request.permission_mode {
        args.push("--permission-mode".into());
        args.push(mode.id().into());
    }
    // Variadic, like the two below: the flag after it closes its list.
    let mut rules = request
        .allowed_tools
        .iter()
        .filter(|rule| all_of(rule, |c| is_word(c) || c == '*' || c == '.' || c == '-'))
        .peekable();
    if rules.peek().is_some() {
        args.push("--allowedTools".into());
        args.extend(rules.cloned());
    }
    // `--strict-mcp-config` is what makes the file the only source, and
    // closes `--mcp-config`'s list. Not strict, it may be the last flag, or
    // be closed by `--add-dir`.
    if let Some(mcp) = request.mcp.as_ref().filter(|mcp| !mcp.arg.is_empty()) {
        args.push("--mcp-config".into());
        args.push(mcp.arg.clone());
        if mcp.strict {
            args.push("--strict-mcp-config".into());
        }
    }
    let mut roots = request
        .extra_roots
        .iter()
        .filter(|root| is_quotable(root))
        .peekable();
    if roots.peek().is_some() {
        args.push("--add-dir".into());
        args.extend(roots.map(|root| format!("\"{root}\"")));
    }
    LaunchSpec {
        program: program(AgentKind::Claude, request.command.as_deref()),
        args,
        cwd: request.cwd.clone(),
        env: env(request),
    }
}

/// The value bare when every character is one a shell reads as itself,
/// between double quotes otherwise.
fn word(value: &str, plain: impl Fn(char) -> bool) -> String {
    if all_of(value, plain) {
        value.to_string()
    } else {
        format!("\"{value}\"")
    }
}

/// The MCP config file's variable, then the account: a profile's folder as
/// `CLAUDE_CONFIG_DIR` (`accounts::env_for`), the main account keeping the
/// environment Hangar has — what `pty.rs` applies to a Claude Code pane.
fn env(request: &LaunchRequest) -> Vec<(String, Option<OsString>)> {
    let account =
        crate::accounts::env_for(Some(AgentKind::Claude.id()), request.account_dir.as_deref());
    std::iter::once(mcp_env(request))
        .chain(
            account
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.map(OsString::from))),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::agent::launch::{build, McpRequest, MCP_CONFIG_VAR};
    use crate::agent::PermissionMode;

    fn some(text: &str) -> Option<String> {
        Some(text.to_string())
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    /// The MCP servers of a launch as the shell reads their file: `arg`.
    fn mcp(arg: &str, strict: bool) -> Option<McpRequest> {
        Some(McpRequest {
            file: "/home/Ann Lee/.teskad/mcp/ws-1.json".into(),
            arg: arg.into(),
            strict,
        })
    }

    /// The line typed for a Claude Code request.
    fn line(request: LaunchRequest) -> String {
        build(&request).unwrap().command_line()
    }

    // launchCommand with a model

    #[test]
    fn starts_claude_code_on_the_panes_model() {
        assert_eq!(
            line(LaunchRequest {
                model: some("sonnet"),
                ..Default::default()
            }),
            "claude --model sonnet"
        );
        assert_eq!(
            line(LaunchRequest {
                resume: some("abc"),
                model: some("claude-sonnet-4-5"),
                name: some("Ava"),
                ..Default::default()
            }),
            "claude --resume abc --name Ava --model claude-sonnet-4-5"
        );
    }

    #[test]
    fn quotes_a_model_a_shell_would_read_as_a_glob_and_drops_one_it_could_not_quote() {
        assert_eq!(
            line(LaunchRequest {
                model: some("opus[1m]"),
                ..Default::default()
            }),
            "claude --model \"opus[1m]\""
        );
        assert_eq!(
            line(LaunchRequest {
                model: some("opus\"; rm -rf ~"),
                ..Default::default()
            }),
            "claude"
        );
    }

    #[test]
    fn keeps_the_model_ahead_of_the_variadic_flags() {
        assert_eq!(
            line(LaunchRequest {
                model: some("haiku"),
                mcp: mcp("/m.json", true),
                extra_roots: strings(&["/x"]),
                ..Default::default()
            }),
            "claude --model haiku --mcp-config /m.json --strict-mcp-config --add-dir \"/x\""
        );
    }

    /// A shell is no kind (`AgentKind::from_id("shell")` is none): it never
    /// reaches the builder, so has no line.
    #[test]
    fn changes_nothing_without_a_model() {
        assert_eq!(line(LaunchRequest::default()), "claude");
        assert_eq!(
            line(LaunchRequest {
                model: some("  "),
                ..Default::default()
            }),
            "claude"
        );
    }

    // launchCommand for an orchestration pane

    #[test]
    fn adds_hangars_server_on_top_of_claude_codes_own_and_lets_its_tools_through() {
        assert_eq!(
            line(LaunchRequest {
                model: some("sonnet"),
                permission_mode: Some(PermissionMode::Auto),
                allowed_tools: strings(&["mcp__teskad"]),
                mcp: mcp("/h.json", false),
                extra_roots: strings(&["/x"]),
                ..Default::default()
            }),
            "claude --model sonnet --permission-mode auto --allowedTools mcp__teskad --mcp-config /h.json --add-dir \"/x\""
        );
        // The last flag, nothing to close its list.
        assert_eq!(
            line(LaunchRequest {
                mcp: mcp("/h.json", false),
                ..Default::default()
            }),
            "claude --mcp-config /h.json"
        );
    }

    /// `mcpConfigArg` of lib/mcpScope.ts for the shell families the test
    /// names; the real one is the desktop's (`mcp::scope::mcp_config_arg`).
    fn mcp_config_arg(family: &str) -> &'static str {
        match family {
            "powershell" => "$env:TESKAD_MCP_CONFIG",
            "cmd" => "\"%TESKAD_MCP_CONFIG%\"",
            "posix" | "fish" => "\"$TESKAD_MCP_CONFIG\"",
            "nu" => "$env.TESKAD_MCP_CONFIG",
            other => panic!("no such family {other}"),
        }
    }

    #[test]
    fn never_shows_the_config_files_path_in_any_shell_that_holds_it_in_a_variable() {
        let typed = |family: &str, strict: bool| {
            line(LaunchRequest {
                resume: some("abc"),
                name: some("Bo"),
                model: some("sonnet"),
                permission_mode: Some(PermissionMode::Auto),
                allowed_tools: strings(&["mcp__teskad"]),
                mcp: mcp(mcp_config_arg(family), strict),
                extra_roots: strings(&["D:\\shared"]),
                ..Default::default()
            })
        };
        let flags = "claude --resume abc --name Bo --model sonnet --permission-mode auto --allowedTools mcp__teskad";
        assert_eq!(
            typed("powershell", false),
            format!("{flags} --mcp-config $env:TESKAD_MCP_CONFIG --add-dir \"D:\\shared\"")
        );
        assert_eq!(
            typed("cmd", false),
            format!("{flags} --mcp-config \"%TESKAD_MCP_CONFIG%\" --add-dir \"D:\\shared\"")
        );
        assert_eq!(
            typed("posix", false),
            format!("{flags} --mcp-config \"$TESKAD_MCP_CONFIG\" --add-dir \"D:\\shared\"")
        );
        assert_eq!(
            typed("fish", false),
            format!("{flags} --mcp-config \"$TESKAD_MCP_CONFIG\" --add-dir \"D:\\shared\"")
        );
        assert_eq!(
            typed("nu", false),
            format!("{flags} --mcp-config $env.TESKAD_MCP_CONFIG --add-dir \"D:\\shared\"")
        );
        // A workspace's own server choice: the same variable, strict.
        assert_eq!(
            typed("powershell", true),
            format!(
                "{flags} --mcp-config $env:TESKAD_MCP_CONFIG --strict-mcp-config --add-dir \"D:\\shared\""
            )
        );
        for family in ["powershell", "cmd", "posix", "fish", "nu"] {
            let typed = typed(family, true);
            assert!(
                !["Ann Lee", "AppData", "ws-1"]
                    .iter()
                    .any(|secret| typed.contains(secret)),
                "{family}: {typed}"
            );
        }
    }

    /// A mode the command line would misread never becomes a
    /// [`PermissionMode`], so reaches no line.
    #[test]
    fn keeps_the_persons_mode_without_one_and_drops_what_a_shell_would_misread() {
        assert_eq!(line(LaunchRequest::default()), "claude");
        assert_eq!(
            line(LaunchRequest {
                permission_mode: PermissionMode::from_id("auto; rm -rf ~"),
                ..Default::default()
            }),
            "claude"
        );
        assert_eq!(
            line(LaunchRequest {
                allowed_tools: strings(&["mcp__teskad", "Bash(rm *)"]),
                ..Default::default()
            }),
            "claude --allowedTools mcp__teskad"
        );
    }

    #[test]
    fn each_orchestration_mode_is_typed_by_its_id() {
        for mode in PermissionMode::ALL {
            assert_eq!(
                line(LaunchRequest {
                    permission_mode: Some(mode),
                    ..Default::default()
                }),
                format!("claude --permission-mode {}", mode.id())
            );
        }
    }

    // What the TS suite did not spell out, but its code does.

    #[test]
    fn a_custom_command_is_trimmed_and_an_empty_one_is_the_default() {
        let program = |command: &str| {
            build(&LaunchRequest {
                command: some(command),
                ..Default::default()
            })
            .unwrap()
            .program
        };
        assert_eq!(program("  ccr code  "), "ccr code");
        assert_eq!(program("   "), "claude");
        // The resume arguments go after a custom launcher.
        assert_eq!(
            line(LaunchRequest {
                resume: some("abc"),
                command: some("  ccr code  "),
                ..Default::default()
            }),
            "ccr code --resume abc"
        );
    }

    #[test]
    fn names_a_session_with_a_space_in_quotes() {
        assert_eq!(
            line(LaunchRequest {
                name: some("Ava (My Work)"),
                ..Default::default()
            }),
            "claude --name \"Ava (My Work)\""
        );
        // A name that could end the quoting is not typed.
        assert_eq!(
            line(LaunchRequest {
                name: some("Ava \"x"),
                ..Default::default()
            }),
            "claude"
        );
    }

    #[test]
    fn drops_the_extra_roots_a_shell_could_misread() {
        assert_eq!(
            line(LaunchRequest {
                extra_roots: strings(&["/ok one", "/bad$HOME", "/bad`x`", "/bad\nrm"]),
                ..Default::default()
            }),
            "claude --add-dir \"/ok one\""
        );
    }

    // The desktop's tests that go through `launch_command`
    // (hangar-app/src/panes/controller.rs).

    #[test]
    fn an_orchestration_pane_gets_its_workspace_mode_and_hangars_tools() {
        assert_eq!(
            line(LaunchRequest {
                permission_mode: Some(PermissionMode::AcceptEdits),
                allowed_tools: strings(&["mcp__teskad"]),
                ..Default::default()
            }),
            "claude --permission-mode acceptEdits --allowedTools mcp__teskad"
        );
    }

    /// The four cases of mcpScope.test.ts, "launchCommand with a server
    /// choice": the file quoted for PowerShell as `quote_arg` quotes it
    /// (the fourth, a shell left alone, has no kind to ask for).
    const PS_FILE: &str = "'C:\\data\\mcp\\ws-1.json'";

    #[test]
    fn a_server_choice_goes_between_the_resume_and_the_directories() {
        assert_eq!(line(LaunchRequest::default()), "claude");
        assert_eq!(
            line(LaunchRequest {
                resume: some("abc"),
                name: some("Ava"),
                mcp: mcp(PS_FILE, true),
                extra_roots: strings(&["D:\\shared"]),
                ..Default::default()
            }),
            format!(
                "claude --resume abc --name Ava --mcp-config {PS_FILE} --strict-mcp-config --add-dir \"D:\\shared\""
            )
        );
    }

    #[test]
    fn a_server_choice_keeps_a_custom_launcher_in_front() {
        assert_eq!(
            line(LaunchRequest {
                command: some("claude --model haiku"),
                mcp: mcp(PS_FILE, true),
                ..Default::default()
            }),
            format!("claude --model haiku --mcp-config {PS_FILE} --strict-mcp-config")
        );
    }

    // Beyond the line.

    /// Each flag and each value is a word of its own, as the shell splits
    /// the line.
    #[test]
    fn every_flag_and_value_is_an_argument_of_its_own() {
        let spec = build(&LaunchRequest {
            resume: some("abc"),
            name: some("Ava (My Work)"),
            model: some("opus[1m]"),
            permission_mode: Some(PermissionMode::BypassPermissions),
            allowed_tools: strings(&["mcp__teskad", "Read"]),
            mcp: mcp("\"$TESKAD_MCP_CONFIG\"", true),
            extra_roots: strings(&["/a", "/b c"]),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(spec.program, "claude");
        assert_eq!(
            spec.args,
            [
                "--resume",
                "abc",
                "--name",
                "\"Ava (My Work)\"",
                "--model",
                "\"opus[1m]\"",
                "--permission-mode",
                "bypassPermissions",
                "--allowedTools",
                "mcp__teskad",
                "Read",
                "--mcp-config",
                "\"$TESKAD_MCP_CONFIG\"",
                "--strict-mcp-config",
                "--add-dir",
                "\"/a\"",
                "\"/b c\"",
            ]
        );
    }

    /// Claude Code is briefed by the mod and starts on the person's effort
    /// (`Launch::effort` is off): neither reaches its line. It starts in the
    /// pane's folder, which its line does not name either.
    #[test]
    fn the_role_and_the_effort_stay_off_its_line() {
        let spec = build(&LaunchRequest {
            cwd: "/work/repo".into(),
            effort: some("high"),
            role: some("You are a worker."),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(spec.command_line(), "claude");
        assert_eq!(spec.cwd, "/work/repo");
    }

    /// What `pty.rs` applies to a Claude Code pane: the config file's
    /// variable, set or removed, then the account's (`apply_mcp_config`,
    /// `apply_account`).
    #[test]
    fn its_shell_gets_the_config_files_variable_and_the_accounts_folder() {
        let main = build(&LaunchRequest::default()).unwrap();
        assert_eq!(main.env, [(MCP_CONFIG_VAR.to_string(), None)]);

        let profile = PathBuf::from("/home/me/.teskad/accounts/work");
        let spec = build(&LaunchRequest {
            mcp: mcp("\"$TESKAD_MCP_CONFIG\"", false),
            account_dir: Some(profile.clone()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            spec.env,
            [
                (
                    "TESKAD_MCP_CONFIG".to_string(),
                    Some(OsString::from("/home/Ann Lee/.teskad/mcp/ws-1.json"))
                ),
                (
                    "CLAUDE_CONFIG_DIR".to_string(),
                    Some(OsString::from(&profile))
                ),
                ("CLAUDE_SECURESTORAGE_CONFIG_DIR".to_string(), None),
            ]
        );
        // The same pairs the accounts module hands the PTY.
        let account: Vec<(String, Option<OsString>)> =
            crate::accounts::env_for(Some(crate::accounts::CLAUDE_AGENT), Some(&profile))
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.map(OsString::from)))
                .collect();
        assert_eq!(spec.env[1..], account[..]);
    }
}
