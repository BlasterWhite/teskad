//! A pane's shell started, and its agent's line typed into it once the
//! shell has settled: what the desktop's pane controller does when a pane
//! mounts (`panes/controller.rs`), with no terminal to measure.
//!
//! The line is core's (`hangar_core::agent::launch`, docs/native/agents.md
//! §3.4, D4): this file says what the pane asks of its agent's launch, in
//! words no agent owns ([`request`]), and the PTY starts the shell with it
//! (`pty_spawn_launch`, through [`Terminals::spawn_agent`]): the launch
//! completed from the shell as it really starts, its variables in the
//! shell's environment, and the line to type. What a pane gets follows its
//! kind's capabilities, not its name:
//!
//! - the account its pane, workspace, folder or settings chose for that kind,
//!   which the PTY brings level with main and names in the shell's
//!   environment (`CLAUDE_CONFIG_DIR`, a Codex home's `CODEX_HOME`);
//! - Hangar's MCP server ([`servers`]): for a kind that reads a config file
//!   (Claude Code), the `--mcp-config` file core writes (`mcp_scope_write`)
//!   for a workspace that chose its servers or a pane with a role, named by
//!   the variable the shell is handed; for a kind that takes overrides
//!   (Codex, D8), Hangar's executable on every pane's launch;
//! - its session resumed when the conversation is still there
//!   (`session_watch::session_to_resume`);
//! - for an orchestrator's or a worker's pane, the orchestration's permission
//!   mode and Hangar's tools let through.
//!
//! A Claude Code pane gets the hangar-bridge mod too (the settings'
//! `claude_bridge`). A Codex pane runs attached to a server of its own when
//! « Attach Hangar to Codex » is on and the server starts (D5), Codex's plain
//! TUI otherwise (the floor, D6), with its settings in the shell's
//! `HANGAR_CODEX_*` variables; the host keeps which, and why
//! (`EngineHost::agent_server`), and so does the pane's machine
//! (`live::SideChannel`). Attached, Hangar's client follows the server
//! (`agent::codex_attach`) and reports what the session does the way the mod
//! reports it, into the same machines; it lets go when the pane's shell
//! ends, the pane closes or restarts (`pty_kill`), or it is unmounted.
//!
//! [`Terminals::spawn_agent`]: super::terminals::Terminals::spawn_agent

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hangar_core::agent::codex_attach;
use hangar_core::agent::launch::{LaunchRequest, McpRequest};
use hangar_core::agent::{AgentKind, McpInjection};
use hangar_core::pty::{AgentLaunch, ServerState};

use super::terminals::{AgentStart, Spawn, TermEvent, TermSink};
use super::{EngineHost, locked};
use crate::accounts::AccountLevels;
use crate::live::{PaneStatus, SideChannel};
use crate::panes::agents::permission_mode;
use crate::panes::session_watch::session_to_resume;
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

/// What `prepare` found on the disk for the agent's launch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Found {
    /// The `--mcp-config` file written for the launch, and whether it
    /// replaces the agent's own servers.
    mcp: Option<(String, bool)>,
    /// Hangar's executable, for a kind that takes Hangar's server as
    /// overrides ([`Servers::Overrides`]).
    hangar_mcp: Option<String>,
    /// The session to resume.
    resume: Option<String>,
}

/// What the pane's start was made of.
struct Prepared {
    spawn: Spawn,
    /// Its agent's launch; `None` for a plain shell, left at its prompt.
    agent: Option<AgentLaunch>,
}

/// Off tokio's async threads: the accounts, the MCP scope and the
/// transcripts are files.
fn prepare(host: &EngineHost, ws: &Workspace, pane: &Pane) -> Prepared {
    let snapshot = host.snapshot();
    let settings = &snapshot.state.settings;
    let cwd = folder_of(ws, pane);
    let kind = pane.agent.kind();
    let account = kind.map(|kind| {
        account_levels(&snapshot.state, ws, pane, kind)
            .launch_with(|| Ok(hangar_core::accounts::accounts_of(kind)))
            .id
    });
    let (mcp, hangar_mcp) = match servers(ws, pane, kind) {
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
    let found = Found {
        mcp,
        hangar_mcp,
        resume: session_to_resume(pane.agent, pane.session_id.as_deref()),
    };
    Prepared {
        spawn: Spawn {
            id: pane.id.clone(),
            cwd,
            name: pane.name.clone(),
            agent: pane.agent.as_str().to_string(),
            bridge: settings.claude_bridge,
            port_base: pane.port_base.and_then(|port| u16::try_from(port).ok()),
            build_env,
            mcp_config: found.mcp.as_ref().map(|(path, _)| path.clone()),
            claude_account: account,
        },
        agent: agent_launch(ws, pane, settings, &found),
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
    let Ok(Prepared { spawn, agent }) = prepared else {
        return failed(&host, &pane, launch, "the launch stopped");
    };
    let resumed = agent
        .as_ref()
        .and_then(|agent| agent.request.resume.clone());
    let terminals = host.terminals().clone();
    // What the pane prints is noted where its reader thread stands, with
    // no lock; only its end goes through the host.
    let sink: TermSink = {
        let host = host.clone();
        let printed = host.printed_of(&pane.id);
        Arc::new(move |event| match event {
            TermEvent::Output(_) => printed.note(wall_clock_ms()),
            TermEvent::Exit(id) => {
                // Hangar's client of its Codex server, if it had one, goes
                // with its shell (`agent::codex_attach::detach`).
                codex_attach::detach(&id);
                host.on_term(TermEvent::Exit(id));
            }
        })
    };
    // Blocking, a Codex server's wait included: on tokio's blocking pool.
    let started = tokio::task::spawn_blocking(move || match agent {
        Some(agent) => terminals.spawn_agent(spawn, agent, sink).map(Some),
        None => terminals.spawn(spawn, sink).map(|()| None),
    })
    .await;
    let agent = match started {
        Ok(Ok(_)) if !host.held_by(&pane.id, launch) => {
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
        Ok(Ok(agent)) => {
            host.set_status(&pane.id, PaneStatus::Running);
            agent
        }
        Ok(Err(error)) => return failed(&host, &pane, launch, &error),
        Err(error) => return failed(&host, &pane, launch, &error.to_string()),
    };
    // A plain shell is left at its prompt.
    let Some(AgentStart { line, server }) = agent else {
        return;
    };
    let snapshot = host.snapshot();
    let attached = match &server {
        Some(ServerState::Attached { socket }) => Some(PathBuf::from(socket)),
        _ => None,
    };
    if let Some(kind) = pane.agent.kind() {
        let channel = side_channel(kind, server.as_ref(), &snapshot.state.settings);
        locked(&host.0.model)
            .panes
            .set_side_channel(&pane.id, channel);
    }
    host.set_server(&pane.id, server);
    let here = snapshot
        .state
        .workspaces
        .iter()
        .find(|w| w.id == ws.id)
        .is_some_and(|w| w.panes.iter().any(|p| p.id == pane.id));
    if !here {
        return;
    }
    // Hangar's client of the pane's Codex server (D5): it waits for the TUI
    // the line starts to connect first, then follows its threads and
    // reports them as the mod does (`EngineHost::feed`).
    if let Some(socket) = attached {
        codex_attach::attach(host.core(), &pane.id, &pane.name, socket);
    }
    settle(&host, &pane.id).await;
    let terminals = host.terminals().clone();
    let id = pane.id.clone();
    let typed =
        tokio::task::spawn_blocking(move || terminals.write(&id, &format!("{line}\r"))).await;
    match typed {
        Ok(Ok(())) => host.launched(&pane.id, resumed.as_deref()),
        Ok(Err(error)) => eprintln!("pane {}: its agent could not be started: {error}", pane.id),
        Err(error) => eprintln!("pane {}: its agent could not be started: {error}", pane.id),
    }
}

/// What `pane` asks of its agent's launch, in words no agent owns: `None`
/// for a plain shell. The spawn completes it from the pane as it really
/// starts (`pty_spawn_launch`): the shell's syntax, the account's folder,
/// the server's socket.
fn request(
    ws: &Workspace,
    pane: &Pane,
    settings: &Settings,
    found: &Found,
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
        resume: found.resume.clone(),
        name: Some(crate::state::pane_names::session_name(&pane.name, &ws.name)),
        model: pane.model.clone(),
        permission_mode,
        allowed_tools,
        mcp: found.mcp.as_ref().map(|(file, strict)| McpRequest {
            file: file.clone(),
            arg: MCP_ARG.to_string(),
            strict: *strict,
        }),
        hangar_mcp: found.hangar_mcp.clone(),
        // Not chosen here: the brief of a kind briefed at launch, the
        // orchestration's to give (T16a); an effort per pane, which nothing
        // chooses yet; the trust of a worker's worktree, until core reads
        // the person's Codex configuration.
        ..LaunchRequest::default()
    })
}

/// The pane's agent launch: its [`request`], run attached to a server of its
/// own when « Attach Hangar to Codex » is on (`settings.codex_attach`, D5) —
/// for a kind that has one; the spawn falls back to the floor when the
/// server cannot start, and says why.
fn agent_launch(
    ws: &Workspace,
    pane: &Pane,
    settings: &Settings,
    found: &Found,
) -> Option<AgentLaunch> {
    Some(AgentLaunch {
        request: request(ws, pane, settings, found)?,
        attach: settings.codex_attach,
    })
}

/// How a pane's agent of `kind` was started, as far as its side channel goes
/// (`live::SideChannel`). A kind Hangar reaches through a server of its own
/// (`headless.app_server`: Codex, D5) is attached to the `server` its
/// launch started, or on the floor and why — and on the floor when none was
/// asked (« Attach Hangar to Codex » off) or these terminals start none.
/// Any other has the hangar-bridge mod within reach while the settings'
/// bridge is on.
fn side_channel(kind: AgentKind, server: Option<&ServerState>, settings: &Settings) -> SideChannel {
    match server {
        Some(ServerState::Attached { .. }) => SideChannel::Attached,
        Some(ServerState::Floor { reason }) => SideChannel::Floor {
            reason: Some(reason.to_string()),
        },
        None if kind.info().caps.headless.app_server => SideChannel::Floor { reason: None },
        None if settings.claude_bridge => SideChannel::Attached,
        None => SideChannel::Floor { reason: None },
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
    use hangar_core::agent::codex::MCP_VAR;
    use hangar_core::agent::launch::{self, ShellSyntax};

    use super::*;
    use crate::state::types::{
        AgentId, OrchestrationSettings, PaneRole, WorkspaceFolder, WorkspaceMcp,
    };

    const THREAD: &str = "0199a5c4-7b3e-7c51-a2f4-6d0e8c1b9f20";
    const HANGAR: &str = "/home/Ann Lee/.local/bin/hangar-ai";
    const MCP_FILE: &str = "/home/Ann Lee/.hangar/mcp/shop.json";

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

    /// What `prepare` found: the MCP file and whether it is strict, Hangar's
    /// executable, the session to resume.
    fn found(mcp: Option<bool>, hangar_mcp: Option<&str>, resume: Option<&str>) -> Found {
        Found {
            mcp: mcp.map(|strict| (MCP_FILE.to_string(), strict)),
            hangar_mcp: hangar_mcp.map(str::to_string),
            resume: resume.map(str::to_string),
        }
    }

    /// The line core builds for the pane's launch before the spawn completes
    /// it: the very line of Claude Code, which no shell, account or server
    /// changes; for Codex, what terminals that cannot hand a shell the
    /// launch's variables type (`Terminals::spawn_agent`'s default).
    fn line(ws: &Workspace, pane: &Pane, settings: &Settings, found: &Found) -> Option<String> {
        agent_launch(ws, pane, settings, found)
            .map(|agent| launch::build(&agent.request).unwrap().command_line())
    }

    // Claude Code, byte for byte as the engine typed it before core's
    // builder (the tests of `panes/agents.rs` hold the rest of its flags).

    #[test]
    fn an_orchestrator_s_claude_code_starts_as_it_always_did() {
        let ws = shop(Some("auto"));
        let leo = pane(AgentId::Claude, Some(PaneRole::Orchestrator), Some("opus"));
        let found = found(Some(false), None, Some("abc"));
        assert_eq!(
            line(&ws, &leo, &Settings::default(), &found).unwrap(),
            "claude --resume abc --name \"Leo (Shop)\" --model opus --permission-mode auto \
             --allowedTools mcp__hangar --mcp-config \"$HANGAR_MCP_CONFIG\" --add-dir \"/srv/shared\""
        );
        // The file the variable names, which the spawn hands the shell.
        let agent = agent_launch(&ws, &leo, &Settings::default(), &found).unwrap();
        assert_eq!(agent.request.cwd, "/srv/shop");
        assert_eq!(
            agent.request.mcp.map(|mcp| mcp.file).as_deref(),
            Some(MCP_FILE)
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
            line(&ws, &leo, &settings, &found(Some(true), None, None)).unwrap(),
            "ccr code --name \"Leo (Shop)\" --mcp-config \"$HANGAR_MCP_CONFIG\" \
             --strict-mcp-config --add-dir \"/srv/shared\""
        );
        // Without a file of Hangar's: its own servers.
        assert_eq!(
            line(&ws, &leo, &settings, &found(None, None, None)).unwrap(),
            "ccr code --name \"Leo (Shop)\" --add-dir \"/srv/shared\""
        );
    }

    #[test]
    fn a_shell_has_no_launch() {
        let ws = shop(None);
        let sh = pane(AgentId::Shell, None, None);
        let found = found(None, None, Some("abc"));
        assert!(agent_launch(&ws, &sh, &Settings::default(), &found).is_none());
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

    /// A Codex pane asks to run attached exactly when « Attach Hangar to
    /// Codex » is on, with everything its server and its line need: Codex's
    /// own command, Hangar's executable and its tools let through, the
    /// policy, the roots, the model, its session.
    #[test]
    fn a_codex_launch_attaches_as_the_setting_says_with_all_it_asks() {
        let ws = shop(Some("bypassPermissions"));
        let worker = codex_worker();
        let mut settings = Settings::default();
        assert!(settings.codex_attach, "on by default (D5)");
        settings.agent_commands.claude = Some("ccr code".into());
        let found = found(None, Some(HANGAR), Some(THREAD));
        let agent = agent_launch(&ws, &worker, &settings, &found).unwrap();
        assert!(agent.attach);
        let request = agent.request;
        assert_eq!(request.kind, AgentKind::Codex);
        assert_eq!(request.command, None);
        assert_eq!(request.cwd, "/srv/shop");
        assert_eq!(request.hangar_mcp.as_deref(), Some(HANGAR));
        assert_eq!(request.allowed_tools, ["mcp__hangar"]);
        assert_eq!(request.mcp, None);
        assert_eq!(request.extra_roots, ["/srv/shared"]);
        assert_eq!(request.resume.as_deref(), Some(THREAD));
        // The spawn's to fill in.
        assert_eq!(request.shell, None);
        assert_eq!(request.server_socket, None);
        assert_eq!(request.account_dir, None);

        settings.codex_attach = false;
        let agent = agent_launch(&ws, &worker, &settings, &found).unwrap();
        assert!(!agent.attach);
    }

    /// Completed as the spawn completes it (`pty_spawn_launch`): attached,
    /// the line points at the pane's server, which gets the settings; on the
    /// floor, the line names the shell's variables, which hold them. Either
    /// way, its session resumed.
    #[test]
    fn a_codex_line_attaches_to_its_server_or_names_the_floor_s_variables() {
        let ws = shop(Some("bypassPermissions"));
        let worker = codex_worker();
        let found = found(None, Some(HANGAR), Some(THREAD));
        let request = agent_launch(&ws, &worker, &Settings::default(), &found)
            .unwrap()
            .request;

        let socket = "/run/user/1000/hangar/codex-0123456789ab.sock";
        let attached = LaunchRequest {
            shell: Some(ShellSyntax::Posix),
            server_socket: Some(socket.into()),
            ..request.clone()
        };
        let typed = launch::build(&attached).unwrap().command_line();
        assert_eq!(
            typed,
            format!(
                "codex --remote unix://{socket} -c approval_policy=never \
                 -c sandbox_mode=danger-full-access -c model=gpt-5.1-codex resume {THREAD}"
            )
        );
        let server = launch::server(&attached).expect("a server").args.join(" ");
        for wanted in [
            format!("command='{HANGAR}'"),
            "default_tools_approval_mode='approve'".to_string(),
            "sandbox_workspace_write.writable_roots=['/srv/shared']".to_string(),
        ] {
            assert!(server.contains(&wanted), "{wanted}: {server}");
        }

        let floor = LaunchRequest {
            shell: Some(ShellSyntax::Posix),
            ..request
        };
        let spec = launch::build(&floor).unwrap();
        let typed = spec.command_line();
        assert!(
            typed.starts_with("codex -c \"$HANGAR_CODEX_MCP\""),
            "{typed}"
        );
        assert!(typed.ends_with(&format!("resume {THREAD}")), "{typed}");
        assert!(!typed.contains("Ann Lee"), "{typed}");
        let held = spec
            .env
            .iter()
            .find(|(name, _)| name == MCP_VAR)
            .and_then(|(_, value)| value.clone())
            .expect("Hangar's server in the shell's variable");
        assert!(
            held.to_string_lossy()
                .starts_with(&format!("mcp_servers.hangar={{command='{HANGAR}'")),
            "{held:?}"
        );
    }

    /// For no shell in particular (terminals that cannot hand one the
    /// launch's variables): the floor's plain settings, nothing of Claude
    /// Code's, no variable named.
    #[test]
    fn a_codex_line_for_no_shell_carries_the_plain_settings_only() {
        let ws = shop(Some("bypassPermissions"));
        let mut settings = Settings::default();
        settings.agent_commands.claude = Some("ccr code".into());
        let typed = line(
            &ws,
            &codex_worker(),
            &settings,
            &found(None, Some(HANGAR), Some(THREAD)),
        )
        .unwrap();
        assert_eq!(
            typed,
            format!(
                "codex -c approval_policy=never -c sandbox_mode=danger-full-access \
                 -c model=gpt-5.1-codex -c features.image_generation=true resume {THREAD}"
            )
        );
        settings.agent_commands.codex = Some(" codex --oss ".into());
        let plain = pane(AgentId::Codex, None, None);
        assert_eq!(
            line(&ws, &plain, &settings, &found(None, Some(HANGAR), None)).unwrap(),
            "codex --oss -c features.image_generation=true"
        );
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

    /// What a pane's machine is told of its side channel: Codex attached to
    /// its server, or on the floor with the reason its status gives, or
    /// without a server asked; Claude Code with the mod within reach while
    /// the bridge is on.
    #[test]
    fn the_side_channel_follows_the_server_and_the_bridge() {
        use hangar_core::pty::FloorReason;
        let on = Settings::default();
        assert!(on.claude_bridge, "the bridge is on by default");
        let attached = ServerState::Attached {
            socket: "/run/user/1000/hangar/codex-p1.sock".into(),
        };
        assert_eq!(
            side_channel(AgentKind::Codex, Some(&attached), &on),
            SideChannel::Attached
        );
        let floor = ServerState::Floor {
            reason: FloorReason::Timeout { seconds: 10 },
        };
        let SideChannel::Floor {
            reason: Some(reason),
        } = side_channel(AgentKind::Codex, Some(&floor), &on)
        else {
            panic!("a floor with its reason");
        };
        assert!(
            reason.starts_with("Codex runs without Hangar attached"),
            "{reason}"
        );
        // « Attach Hangar to Codex » off: no server asked, the floor.
        assert_eq!(
            side_channel(AgentKind::Codex, None, &on),
            SideChannel::Floor { reason: None }
        );
        assert_eq!(
            side_channel(AgentKind::Claude, None, &on),
            SideChannel::Attached
        );
        let off = Settings {
            claude_bridge: false,
            ..Settings::default()
        };
        assert_eq!(
            side_channel(AgentKind::Claude, None, &off),
            SideChannel::Floor { reason: None }
        );
    }
}
