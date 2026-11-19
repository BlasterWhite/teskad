//! A pane's shell started, and its agent's line typed into it once the
//! shell has settled: what the desktop's pane controller does when a pane
//! mounts (`panes/controller.rs`), with no terminal to measure.
//!
//! The line is core's (`hangar_core::agent::launch`, docs/native/agents.md
//! §3.4, D4): this file says what the pane asks of its agent's launch, in
//! words no agent owns ([`request`]), and types what the builder answers, for
//! Claude Code and Codex alike. What a pane gets follows its kind's
//! capabilities, not its name:
//!
//! - the account its pane, workspace, folder or settings chose for that kind,
//!   which the PTY brings level with main and names in the shell's
//!   environment (`CLAUDE_CONFIG_DIR`, a Codex home's `CODEX_HOME`);
//! - Hangar's MCP server ([`servers`]): for a kind that reads a config file
//!   (Claude Code), the `--mcp-config` file core writes (`mcp_scope_write`)
//!   for a workspace that chose its servers or a pane with a role, named by
//!   the variable the shell is handed; for a kind that takes overrides
//!   (Codex, D8), Hangar's executable on every pane's launch;
//! - its session resumed when its transcript is there;
//! - for an orchestrator's or a worker's pane, the orchestration's permission
//!   mode and Hangar's tools let through.
//!
//! A Claude Code pane gets the hangar-bridge mod too (the settings'
//! `claude_bridge`). A Codex pane runs attached to a server of its own when
//! « Attach Hangar to Codex » is on and the server starts (D5), its plain TUI
//! otherwise (the floor, D6): [`attached_server`] is the one place that says
//! which.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hangar_core::agent::launch::{self, LaunchRequest, McpRequest, ShellSyntax};
use hangar_core::agent::{AgentKind, McpInjection};

use super::terminals::{Spawn, TermEvent, TermSink};
use super::{EngineHost, locked};
use crate::accounts::AccountLevels;
use crate::live::PaneStatus;
use crate::panes::agents::permission_mode;
use crate::panes::wall_clock_ms;
use crate::state::roles::{ORCHESTRATION_ALLOWED_TOOLS, orchestration_of};
use crate::state::types::{AppState, McpMode, Pane, Settings, Workspace};

/// How long a shell may stay quiet after it first printed before it is
/// taken as ready for the agent's command.
const SETTLED: Duration = Duration::from_millis(500);
/// The longest a launch waits for its shell.
const LAUNCH_WAIT: Duration = Duration::from_secs(8);
const POLL: Duration = Duration::from_millis(50);

/// The variable a pane's shell holds its `--mcp-config` file in, as a POSIX
/// shell reads it: the servers' folder holds the person's name, which the
/// typed command never shows.
const MCP_ARG: &str = "\"$HANGAR_MCP_CONFIG\"";

/// The folder a pane's shell opens in: its own, else its workspace's.
pub fn folder_of(ws: &Workspace, pane: &Pane) -> String {
    pane.cwd
        .as_deref()
        .filter(|cwd| !cwd.is_empty())
        .unwrap_or(&ws.cwd)
        .to_string()
}

/// How a pane's agent gets its MCP servers, by what its kind reads
/// (`Capabilities::mcp`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Servers {
    /// Its own, as it always did: a shell, a pane opened to log an account
    /// in, a pane with no role in a workspace that kept the agent's servers.
    Own,
    /// A config file written for the launch (Claude Code's `--mcp-config`):
    /// only the servers `enabled`, strict, for a workspace in « custom »
    /// mode; Hangar's on top of the agent's own for a pane with a role.
    File { enabled: Vec<String>, strict: bool },
    /// Hangar's server as overrides of the agent's configuration, every
    /// pane (Codex, D8). A workspace in « custom » mode still has the
    /// agent's own servers beside it until core turns them off on the launch
    /// (`mcp_servers.<name>.enabled=false`, T7).
    Overrides,
}

fn servers(ws: &Workspace, pane: &Pane, kind: Option<AgentKind>) -> Servers {
    let Some(kind) = kind.filter(|_| pane.login != Some(true)) else {
        return Servers::Own;
    };
    match kind.info().caps.mcp.injection {
        McpInjection::ConfigFile => {
            match ws.mcp.as_ref().filter(|mcp| mcp.mode == McpMode::Custom) {
                Some(custom) => Servers::File {
                    enabled: custom.enabled.clone(),
                    strict: true,
                },
                None if pane.role.is_some() => Servers::File {
                    enabled: Vec::new(),
                    strict: false,
                },
                None => Servers::Own,
            }
        }
        McpInjection::Overrides => Servers::Overrides,
    }
}

/// The choices the account of `kind`'s agent in `pane` is looked up in: the
/// pane's, its workspace's, its folder's — none for a workspace its node
/// runs, whose account is chosen there — and the settings' default. What
/// `AccountLevels::of` reads for Claude Code, read for either kind.
fn account_levels(state: &AppState, ws: &Workspace, pane: &Pane, kind: AgentKind) -> AccountLevels {
    let owned = |choice: Option<&str>| choice.map(str::to_string);
    let folder = ws
        .folder_id
        .as_deref()
        .filter(|_| !ws.is_hosted())
        .and_then(|id| state.folders.iter().find(|folder| folder.id == id))
        .and_then(|folder| folder.account(kind))
        .map(str::trim)
        .filter(|id| !id.is_empty());
    AccountLevels {
        pane: owned(pane.account(kind)),
        workspace: owned(ws.account(kind)),
        folder: owned(folder),
        default: owned(state.settings.account(kind)),
    }
}

/// Hangar's own executable, whose `--mcp` is Hangar's MCP server; `None`,
/// said in the log, when it cannot be told: the pane starts without
/// Hangar's tools.
fn hangar_executable(pane: &Pane) -> Option<String> {
    match hangar_core::executable::stable() {
        Ok(path) => Some(path.to_string_lossy().into_owned()),
        Err(error) => {
            eprintln!(
                "pane {}: Hangar's executable could not be found ({error}); it starts without Hangar's tools",
                pane.id
            );
            None
        }
    }
}

/// What the pane's launch was made of.
struct Prepared {
    spawn: Spawn,
    /// The `--mcp-config` file was written ([`Spawn::mcp_config`]), and
    /// whether it replaces the agent's own servers.
    mcp: Option<bool>,
    /// Hangar's executable, for a kind that takes Hangar's server as
    /// overrides ([`Servers::Overrides`]).
    hangar_mcp: Option<String>,
    resume: Option<String>,
}

/// Off tokio's async threads: the accounts, the MCP scope and the
/// transcripts are files.
fn prepare(host: &EngineHost, ws: &Workspace, pane: &Pane) -> Prepared {
    let snapshot = host.snapshot();
    let cwd = folder_of(ws, pane);
    let kind = pane.agent.kind();
    let account = kind.map(|kind| {
        account_levels(&snapshot.state, ws, pane, kind)
            .launch_with(|| Ok(hangar_core::accounts::accounts_of(kind)))
            .id
    });
    let (scope, hangar_mcp) = match servers(ws, pane, kind) {
        Servers::Own => (None, None),
        Servers::File { enabled, strict } => (
            write_scope(host, ws, &cwd, enabled, account.clone()).map(|path| (path, strict)),
            None,
        ),
        Servers::Overrides => (None, hangar_executable(pane)),
    };
    let build_env = if pane.port_base.is_some() {
        hangar_core::build_cache::env_for(
            std::path::Path::new(&ws.cwd),
            &ws.id,
            orchestration_of(ws).shared_build_cache,
        )
    } else {
        Vec::new()
    };
    // A session whose transcript is not found is not resumed.
    let resume = kind.and_then(|kind| {
        pane.session_id
            .clone()
            .filter(|id| !id.is_empty())
            .filter(|id| hangar_core::sessions::session_exists_for(kind, id.clone()) != Some(false))
    });
    Prepared {
        spawn: Spawn {
            id: pane.id.clone(),
            cwd,
            name: pane.name.clone(),
            agent: pane.agent.as_str().to_string(),
            bridge: snapshot.state.settings.claude_bridge,
            port_base: pane.port_base.and_then(|port| u16::try_from(port).ok()),
            build_env,
            mcp_config: scope.as_ref().map(|(path, _)| path.clone()),
            claude_account: account,
        },
        mcp: scope.map(|(_, strict)| strict),
        hangar_mcp,
        resume,
    }
}

fn write_scope(
    host: &EngineHost,
    ws: &Workspace,
    cwd: &str,
    enabled: Vec<String>,
    account: Option<String>,
) -> Option<String> {
    match hangar_core::mcp_scope::mcp_scope_write(
        host.core(),
        ws.id.clone(),
        ws.cwd.clone(),
        cwd.to_string(),
        enabled,
        None,
        account,
    ) {
        Ok(file) => Some(file.path),
        Err(error) => {
            eprintln!(
                "workspace {}: the MCP servers of a pane could not be written ({error}); it starts with its own",
                ws.id
            );
            None
        }
    }
}

/// Starts `pane` of `ws`: its machine, its shell, then its agent. `launch`
/// is what holds the pane for this start: a pane that left the state (a
/// worker's start undone, a workspace removed or put to sleep) while its
/// shell was starting is no longer held by it, and the shell is taken
/// down again, which nothing else would do.
pub(super) async fn start(host: EngineHost, ws: Workspace, pane: Pane, launch: u64) {
    if !host.held_by(&pane.id, launch) {
        return;
    }
    host.mounted(&pane, &ws);
    let prepared = {
        let (host, ws, pane) = (host.clone(), ws.clone(), pane.clone());
        tokio::task::spawn_blocking(move || prepare(&host, &ws, &pane)).await
    };
    let Ok(prepared) = prepared else {
        return failed(&host, &pane, launch, "the launch stopped");
    };
    let terminals = host.terminals().clone();
    // What the pane prints is noted where its reader thread stands, with
    // no lock; only its end goes through the host.
    let sink: TermSink = {
        let host = host.clone();
        let printed = host.printed_of(&pane.id);
        Arc::new(move |event| match event {
            TermEvent::Output(_) => printed.note(wall_clock_ms()),
            exit @ TermEvent::Exit(_) => host.on_term(exit),
        })
    };
    let spawn = prepared.spawn.clone();
    let started = tokio::task::spawn_blocking(move || terminals.spawn(spawn, sink)).await;
    match started {
        Ok(Ok(())) if !host.held_by(&pane.id, launch) => {
            // Removed meanwhile: the kill that came with the removal found
            // no shell yet.
            let (terminals, id) = (host.terminals().clone(), pane.id.clone());
            let _ = tokio::task::spawn_blocking(move || terminals.kill(&id)).await;
            // Unless another launch holds the pane since (a workspace woken
            // again): its machine is that launch's.
            if !locked(&host.0.shells).contains_key(&pane.id) {
                host.unmounted(&pane.id);
            }
            return;
        }
        Ok(Ok(())) => host.set_status(&pane.id, PaneStatus::Running),
        Ok(Err(error)) => return failed(&host, &pane, launch, &error),
        Err(error) => return failed(&host, &pane, launch, &error.to_string()),
    }
    let snapshot = host.snapshot();
    let Some(ws) = snapshot.state.workspaces.iter().find(|w| w.id == ws.id) else {
        return;
    };
    let Some(pane) = ws.panes.iter().find(|p| p.id == pane.id) else {
        return;
    };
    let settings = &snapshot.state.settings;
    let shell = attached_server(settings, pane);
    let Some(line) = command_line(ws, pane, settings, &prepared, shell) else {
        return;
    };
    settle(&host, &pane.id).await;
    let terminals = host.terminals().clone();
    let id = pane.id.clone();
    let typed =
        tokio::task::spawn_blocking(move || terminals.write(&id, &format!("{line}\r"))).await;
    match typed {
        Ok(Ok(())) => host.launched(&pane.id, prepared.resume.as_deref()),
        Ok(Err(error)) => eprintln!("pane {}: its agent could not be started: {error}", pane.id),
        Err(error) => eprintln!("pane {}: its agent could not be started: {error}", pane.id),
    }
}

/// What the shell of a pane was started with for its agent's line, beyond
/// what every pane gets: the socket of the server its agent attaches to, and
/// how the shell reads the variables the line names (`LaunchSpec::env`,
/// which the shell then holds).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LaunchShell {
    socket: Option<String>,
    syntax: Option<ShellSyntax>,
}

/// **The attached mode's seam** (docs/native/agents.md D5, D6): how the
/// shell of `pane` was started for a kind that runs attached to a server of
/// its own (Codex) — the server's socket when « Attach Hangar to Codex »
/// (`settings.codex_attach`) is on and the pane's server started, nothing
/// for the floor.
///
/// Nothing yet, whatever the setting says: hangar-core's PTY neither starts
/// a pane's server nor hands its shell the launch's variables until task
/// T4b lands (`pty_spawn_launch`, `codex_socket`). Every Codex pane runs the
/// floor meanwhile, its line carrying the settings typed as they are
/// (permission policy, model, images); those that travel in a variable of
/// the shell — Hangar's MCP server, the role, the writable roots, the
/// trusted folders (`agent::codex::{MCP_VAR, ROLE_VAR, ROOTS_VAR,
/// TRUST_VAR}`) — reach it with T4b, through this function and nowhere
/// else. A shell whose syntax is not known gets none of them rather than a
/// variable it does not hold.
fn attached_server(settings: &Settings, pane: &Pane) -> LaunchShell {
    let _ = (settings.codex_attach, &pane.id);
    LaunchShell::default()
}

/// What `pane` asks of its agent's launch, in words no agent owns: `None`
/// for a plain shell.
fn request(
    ws: &Workspace,
    pane: &Pane,
    settings: &Settings,
    prepared: &Prepared,
    shell: LaunchShell,
) -> Option<LaunchRequest> {
    let kind = pane.agent.kind()?;
    let (permission_mode, allowed_tools) = if pane.role.is_some() {
        (
            orchestration_of(ws)
                .permission_mode
                .as_deref()
                .and_then(permission_mode),
            ORCHESTRATION_ALLOWED_TOOLS
                .iter()
                .map(|tool| (*tool).to_owned())
                .collect(),
        )
    } else {
        (None, Vec::new())
    };
    Some(LaunchRequest {
        kind,
        command: settings.agent_commands.of(kind).map(str::to_string),
        cwd: folder_of(ws, pane),
        extra_roots: ws.extra_roots.clone(),
        resume: prepared.resume.clone(),
        name: Some(crate::state::pane_names::session_name(&pane.name, &ws.name)),
        model: pane.model.clone(),
        permission_mode,
        allowed_tools,
        mcp: prepared.mcp.map(|strict| McpRequest {
            file: prepared.spawn.mcp_config.clone().unwrap_or_default(),
            arg: MCP_ARG.to_string(),
            strict,
        }),
        hangar_mcp: prepared.hangar_mcp.clone(),
        shell: shell.syntax,
        server_socket: shell.socket,
        // Not chosen here: the account's folder, which the PTY resolves and
        // names in the shell's environment (`pty_spawn_with`); the brief of a
        // kind briefed at launch, the orchestration's to give (T16a); an
        // effort per pane, which nothing chooses yet; the trust of a worker's
        // worktree, until core reads the person's Codex configuration.
        ..LaunchRequest::default()
    })
}

/// The line typed into the shell, without the Enter: `None` for a plain
/// shell. Only the line is taken from the spec: the variables the shell
/// starts with are the PTY's (`pty_spawn_with`: the MCP file, the account).
fn command_line(
    ws: &Workspace,
    pane: &Pane,
    settings: &Settings,
    prepared: &Prepared,
    shell: LaunchShell,
) -> Option<String> {
    let request = request(ws, pane, settings, prepared, shell)?;
    match launch::build(&request) {
        Ok(spec) => Some(spec.command_line()),
        Err(error) => {
            eprintln!("pane {}: {error}", pane.id);
            None
        }
    }
}

/// Waits for the shell to print its prompt and fall quiet, or for
/// [`LAUNCH_WAIT`]: a command typed before the shell reads its input can be
/// lost, or echoed twice.
async fn settle(host: &EngineHost, pane: &str) {
    let began = Instant::now();
    loop {
        if began.elapsed() >= LAUNCH_WAIT {
            return;
        }
        if let Some(at) = host.last_printed(pane)
            && wall_clock_ms().saturating_sub(at) >= SETTLED.as_millis() as u64
        {
            return;
        }
        tokio::time::sleep(POLL).await;
    }
}

/// The shell could not start: the pane says so and stays down. Not when
/// the pane is no longer this launch's (it left the state meanwhile).
fn failed(host: &EngineHost, pane: &Pane, launch: u64, error: &str) {
    if !host.held_by(&pane.id, launch) {
        return;
    }
    eprintln!("pane {} ({}) could not start: {error}", pane.id, pane.name);
    host.set_status(&pane.id, PaneStatus::Exited);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::types::{
        AgentId, OrchestrationSettings, PaneRole, WorkspaceFolder, WorkspaceMcp,
    };

    const THREAD: &str = "0199a5c4-7b3e-7c51-a2f4-6d0e8c1b9f20";
    const HANGAR: &str = "/home/Ann Lee/.local/bin/hangar-ai";

    fn shop(permission_mode: Option<&str>) -> Workspace {
        Workspace {
            id: "shop".into(),
            name: "Shop".into(),
            cwd: "/srv/shop".into(),
            extra_roots: vec!["/srv/shared".into()],
            orchestration: Some(OrchestrationSettings {
                permission_mode: permission_mode.map(str::to_string),
                ..OrchestrationSettings::default()
            }),
            ..Workspace::default()
        }
    }

    fn pane(agent: AgentId, role: Option<PaneRole>, model: Option<&str>) -> Pane {
        Pane {
            id: "p1".into(),
            name: "Leo".into(),
            agent,
            role,
            model: model.map(str::to_string),
            ..Pane::default()
        }
    }

    /// What `prepare` made of a pane: its MCP file and whether it is strict,
    /// Hangar's executable, the session to resume.
    fn prepared(mcp: Option<bool>, hangar_mcp: Option<&str>, resume: Option<&str>) -> Prepared {
        Prepared {
            spawn: Spawn {
                mcp_config: mcp.map(|_| "/home/Ann Lee/.hangar/mcp/shop.json".into()),
                ..Spawn::default()
            },
            mcp,
            hangar_mcp: hangar_mcp.map(str::to_string),
            resume: resume.map(str::to_string),
        }
    }

    fn line(
        ws: &Workspace,
        pane: &Pane,
        settings: &Settings,
        prepared: &Prepared,
    ) -> Option<String> {
        command_line(
            ws,
            pane,
            settings,
            prepared,
            attached_server(settings, pane),
        )
    }

    // Claude Code, byte for byte as the engine typed it before core's
    // builder (the tests of `panes/agents.rs` hold the rest of its flags).

    #[test]
    fn an_orchestrator_s_claude_code_starts_as_it_always_did() {
        let ws = shop(Some("auto"));
        let leo = pane(AgentId::Claude, Some(PaneRole::Orchestrator), Some("opus"));
        assert_eq!(
            line(
                &ws,
                &leo,
                &Settings::default(),
                &prepared(Some(false), None, Some("abc"))
            )
            .unwrap(),
            "claude --resume abc --name \"Leo (Shop)\" --model opus --permission-mode auto \
             --allowedTools mcp__hangar --mcp-config \"$HANGAR_MCP_CONFIG\" --add-dir \"/srv/shared\""
        );
    }

    #[test]
    fn a_plain_claude_code_keeps_its_custom_command_and_a_custom_workspace_s_servers() {
        let ws = shop(Some("bypassPermissions"));
        let leo = pane(AgentId::Claude, None, None);
        let mut settings = Settings::default();
        settings.agent_commands.claude = Some("  ccr code ".into());
        settings.agent_commands.codex = Some("codex --oss".into());
        assert_eq!(
            line(&ws, &leo, &settings, &prepared(Some(true), None, None)).unwrap(),
            "ccr code --name \"Leo (Shop)\" --mcp-config \"$HANGAR_MCP_CONFIG\" \
             --strict-mcp-config --add-dir \"/srv/shared\""
        );
        // Without a file of Hangar's: its own servers.
        assert_eq!(
            line(&ws, &leo, &settings, &prepared(None, None, None)).unwrap(),
            "ccr code --name \"Leo (Shop)\" --add-dir \"/srv/shared\""
        );
    }

    #[test]
    fn a_shell_types_nothing() {
        let ws = shop(None);
        let sh = pane(AgentId::Shell, None, None);
        let settings = Settings::default();
        assert_eq!(
            line(&ws, &sh, &settings, &prepared(None, None, Some("abc"))),
            None
        );
        assert_eq!(
            request(
                &ws,
                &sh,
                &settings,
                &prepared(None, None, None),
                LaunchShell::default()
            ),
            None
        );
    }

    // Codex.

    /// A Codex worker, as an orchestrator starts one.
    fn codex_worker() -> Pane {
        pane(
            AgentId::Codex,
            Some(PaneRole::Worker),
            Some("gpt-5.1-codex"),
        )
    }

    /// Until the PTY starts a pane's server (T4b), a Codex pane runs the
    /// floor whatever « Attach Hangar to Codex » says: Codex's own command,
    /// the orchestration's policy, its model and images on the line, its
    /// session resumed — nothing of Claude Code's, nothing that would name a
    /// variable its shell does not hold.
    #[test]
    fn a_codex_pane_runs_the_floor_until_its_server_can_start() {
        let ws = shop(Some("bypassPermissions"));
        let worker = codex_worker();
        let mut settings = Settings::default();
        assert!(settings.codex_attach, "on by default (D5)");
        settings.agent_commands.claude = Some("ccr code".into());
        let prepared = prepared(None, Some(HANGAR), Some(THREAD));
        assert_eq!(attached_server(&settings, &worker), LaunchShell::default());
        let typed = line(&ws, &worker, &settings, &prepared).unwrap();
        assert_eq!(
            typed,
            format!(
                "codex -c approval_policy=never -c sandbox_mode=danger-full-access \
                 -c model=gpt-5.1-codex -c features.image_generation=true resume {THREAD}"
            )
        );
        assert!(!typed.contains("$HANGAR"), "{typed}");
        let request = request(&ws, &worker, &settings, &prepared, LaunchShell::default()).unwrap();
        assert_eq!(launch::server(&request), None);
        // Off, the same.
        settings.codex_attach = false;
        assert_eq!(line(&ws, &worker, &settings, &prepared).unwrap(), typed);
    }

    #[test]
    fn a_codex_pane_types_codex_s_own_command() {
        let ws = shop(None);
        let leo = pane(AgentId::Codex, None, None);
        let mut settings = Settings::default();
        settings.agent_commands.codex = Some(" codex --oss ".into());
        assert_eq!(
            line(&ws, &leo, &settings, &prepared(None, Some(HANGAR), None)).unwrap(),
            "codex --oss -c features.image_generation=true"
        );
    }

    /// Once the seam answers (T4b): the pane's line attaches to its server,
    /// which gets everything the launch asks — Hangar's server and its tools
    /// let through, the policy, the roots, the model.
    #[test]
    fn a_codex_pane_whose_server_started_attaches_to_it() {
        let ws = shop(Some("bypassPermissions"));
        let worker = codex_worker();
        let started = LaunchShell {
            socket: Some("/run/user/1000/hangar/codex-p1.sock".into()),
            syntax: Some(ShellSyntax::Posix),
        };
        let request = request(
            &ws,
            &worker,
            &Settings::default(),
            &prepared(None, Some(HANGAR), Some(THREAD)),
            started,
        )
        .unwrap();
        assert_eq!(request.hangar_mcp.as_deref(), Some(HANGAR));
        assert_eq!(request.allowed_tools, ["mcp__hangar"]);
        assert_eq!(request.mcp, None);
        let typed = launch::build(&request).unwrap().command_line();
        assert!(
            typed.starts_with("codex --remote unix:///run/user/1000/hangar/codex-p1.sock "),
            "{typed}"
        );
        assert!(typed.ends_with(&format!("resume {THREAD}")), "{typed}");
        let server = launch::server(&request).expect("a server");
        let args = server.args.join(" ");
        for wanted in [
            "app-server --listen unix:///run/user/1000/hangar/codex-p1.sock",
            &format!("command='{HANGAR}'"),
            "default_tools_approval_mode='approve'",
            "approval_policy=never",
            "sandbox_workspace_write.writable_roots=['/srv/shared']",
            "model=gpt-5.1-codex",
        ] {
            assert!(args.contains(wanted), "{wanted}: {args}");
        }
    }

    // What `prepare` reads.

    #[test]
    fn each_kind_gets_hangar_s_server_its_own_way() {
        let mut ws = shop(None);
        let plain = |agent| pane(agent, None, None);
        let worker = |agent| pane(agent, Some(PaneRole::Worker), None);
        assert_eq!(
            servers(&ws, &plain(AgentId::Claude), Some(AgentKind::Claude)),
            Servers::Own
        );
        assert_eq!(
            servers(&ws, &worker(AgentId::Claude), Some(AgentKind::Claude)),
            Servers::File {
                enabled: Vec::new(),
                strict: false
            }
        );
        assert_eq!(
            servers(&ws, &plain(AgentId::Codex), Some(AgentKind::Codex)),
            Servers::Overrides
        );
        assert_eq!(
            servers(&ws, &worker(AgentId::Codex), Some(AgentKind::Codex)),
            Servers::Overrides
        );
        assert_eq!(servers(&ws, &plain(AgentId::Shell), None), Servers::Own);

        ws.mcp = Some(WorkspaceMcp {
            mode: McpMode::Custom,
            enabled: vec!["github".into()],
        });
        for claude in [plain(AgentId::Claude), worker(AgentId::Claude)] {
            assert_eq!(
                servers(&ws, &claude, Some(AgentKind::Claude)),
                Servers::File {
                    enabled: vec!["github".into()],
                    strict: true
                }
            );
        }
        assert_eq!(
            servers(&ws, &plain(AgentId::Codex), Some(AgentKind::Codex)),
            Servers::Overrides
        );

        // A pane opened to log an account in starts with nothing of Hangar's.
        for (agent, kind) in [
            (AgentId::Claude, AgentKind::Claude),
            (AgentId::Codex, AgentKind::Codex),
        ] {
            let login = Pane {
                login: Some(true),
                ..worker(agent)
            };
            assert_eq!(servers(&ws, &login, Some(kind)), Servers::Own);
        }
    }

    fn filed(state: &mut AppState, ws: &mut Workspace) {
        state.folders.push(WorkspaceFolder {
            id: "f1".into(),
            claude_account: Some(" pro ".into()),
            codex_account: Some("team".into()),
            ..WorkspaceFolder::default()
        });
        ws.folder_id = Some("f1".into());
    }

    /// Claude Code's account is looked up where it always was; Codex's in
    /// the same places, its own field of each.
    #[test]
    fn the_account_is_looked_up_level_by_level_for_the_pane_s_kind() {
        let mut state = AppState::default();
        let mut ws = shop(None);
        filed(&mut state, &mut ws);
        ws.claude_account = Some("work".into());
        state.settings.claude_account = Some("main".into());
        state.settings.codex_account = Some("perso".into());
        let mut leo = pane(AgentId::Codex, None, None);
        leo.codex_account = Some("@default".into());

        let claude = account_levels(&state, &ws, &leo, AgentKind::Claude);
        assert_eq!(claude, AccountLevels::of(&state, &ws, Some(&leo)));
        assert_eq!(
            account_levels(&state, &ws, &leo, AgentKind::Codex),
            AccountLevels {
                pane: Some("@default".into()),
                workspace: None,
                folder: Some("team".into()),
                default: Some("perso".into()),
            }
        );
        // « Default » chosen on the pane: the settings' Codex account, which
        // must be one of this machine's.
        let codex = account_levels(&state, &ws, &leo, AgentKind::Codex);
        let listed = |ids: &[&str]| {
            ids.iter()
                .map(|id| crate::accounts::ClaudeAccount {
                    id: (*id).into(),
                    label: (*id).into(),
                    is_main: false,
                    logged_in: true,
                    email: None,
                    plan: None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(codex.launch_with(|| Ok(listed(&["perso"]))).id, "perso");
        assert_eq!(codex.launch_with(|| Ok(listed(&[]))).id, "main");

        // A workspace its node runs has no folder level.
        ws.hosted = Some(true);
        assert_eq!(
            account_levels(&state, &ws, &leo, AgentKind::Codex).folder,
            None
        );
        assert_eq!(
            account_levels(&state, &ws, &leo, AgentKind::Claude),
            AccountLevels::of(&state, &ws, Some(&leo))
        );
    }
}
