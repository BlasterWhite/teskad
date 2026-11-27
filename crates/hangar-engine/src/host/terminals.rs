//! The panes' shells for a host with no window: what the desktop's terminals
//! package does for its panes (spawn through core, type, read the screen,
//! kill), with an emulated screen per pane instead of a widget.
//!
//! [`Terminals`] is what the host asks; [`CorePtys`] answers it with core's
//! PTY manager on this machine, a pane's agent included: its line, and for a
//! Codex pane its own server (`pty_spawn_launch`, docs/native/agents.md D5).
//! A node keeps its own panes (their history, the desktop's streams) and
//! answers it over them.

use std::sync::Arc;

use hangar_core::agent::launch;
use hangar_core::pty::{self, AgentLaunch, PaneJob, PtyEvent, PtyManager, ServerState, Spawned};

use super::screen::{Screen, Screens};
use crate::state::PaneId;

/// The size a pane's shell starts at, with no window to measure.
pub const COLUMNS: u16 = 120;
pub const ROWS: u16 = 40;

/// A pane's shell to start (`pty_spawn_with`'s arguments). A pane whose
/// agent Hangar starts comes with its launch too ([`Terminals::spawn_agent`]),
/// which then says the folder, the MCP file and the agent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Spawn {
    pub id: PaneId,
    pub cwd: String,
    pub name: String,
    /// The pane's agent (`claude`, `codex`, `shell`).
    pub agent: String,
    /// The teskad-bridge mod is loaded into its Claude Code.
    pub bridge: bool,
    pub port_base: Option<u16>,
    pub build_env: Vec<(String, String)>,
    /// The `--mcp-config` file, handed as `TESKAD_MCP_CONFIG`.
    pub mcp_config: Option<String>,
    /// The account its agent runs on, of the agent's kind whatever the
    /// field's name (Claude Code's alone had one when it was named): the PTY
    /// reads it for `agent`, brings it level with main and names it in the
    /// shell's environment (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`). None: a
    /// shell.
    pub claude_account: Option<String>,
}

/// What a pane's shell tells the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermEvent {
    /// It printed something.
    Output(PaneId),
    /// It ended by itself, or was killed.
    Exit(PaneId),
}

/// Where a pane's events go: called on the pane's reader thread.
pub type TermSink = Arc<dyn Fn(TermEvent) + Send + Sync>;

/// What a pane's agent was started with ([`Terminals::spawn_agent`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStart {
    /// The line its shell is to type, without the Enter.
    pub line: String,
    /// Its own server, for a launch that asked to run attached to one (D5):
    /// listening, which the line attaches to, or the floor and why (D6).
    /// `None` when none was asked, or these terminals start none.
    pub server: Option<ServerState>,
}

pub trait Terminals: Send + Sync + 'static {
    /// Starts the shell; blocks while it starts.
    fn spawn(&self, spawn: Spawn, sink: TermSink) -> Result<(), String>;

    /// Starts the shell of a pane whose agent Hangar launches, and says what
    /// its shell is to type: core's line (`agent::launch`) for the shell as
    /// it really started, with the launch's variables in its environment —
    /// for a Codex pane to run attached, its server started first and the
    /// line attached to it, or the floor's line when the server cannot start.
    /// Blocks while it starts, the server's wait included (ten seconds at
    /// most): never on a thread that draws.
    ///
    /// Terminals that cannot hand a shell the launch's variables start it as
    /// [`spawn`](Self::spawn) does, and type the line built for no shell in
    /// particular, with no server: Claude Code's line is the same, and
    /// Codex's runs the floor without the settings it would carry in a
    /// variable.
    fn spawn_agent(
        &self,
        spawn: Spawn,
        launch: AgentLaunch,
        sink: TermSink,
    ) -> Result<AgentStart, String> {
        let line = launch::build(&launch.request)
            .map_err(|error| error.to_string())?
            .command_line();
        self.spawn(spawn, sink)?;
        Ok(AgentStart { line, server: None })
    }
    fn write(&self, pane: &str, data: &str) -> Result<(), String>;
    fn alive(&self, pane: &str) -> bool;
    /// The pane's shell told its new size, and its screen with it: a pane
    /// someone looks at through a window is the size of that window, and
    /// the screen read here has to be the one its program draws on.
    /// Terminals nobody resizes have nothing to do.
    fn resize(&self, pane: &str, columns: u16, rows: u16) -> Result<(), String> {
        let _ = (pane, columns, rows);
        Ok(())
    }
    /// Its whole process tree.
    fn kill(&self, pane: &str) -> Result<(), String>;
    /// The last `n` lines it printed, oldest first; `None` for a pane this
    /// host does not run.
    fn lines(&self, pane: &str, n: usize) -> Option<Vec<String>>;
    /// The rows of its screen, without the history.
    fn screen(&self, pane: &str) -> Option<Vec<String>>;
    /// The title its program last set (OSC 0 or 2), empty once it let go of
    /// it: where no side channel speaks for its agent, the agent's own title
    /// tells its turns (`panes::pane_agent::title_says`). `None` when it set
    /// none, or these terminals cannot tell.
    fn title(&self, pane: &str) -> Option<String> {
        let _ = pane;
        None
    }
    /// Its program asked for pastes to be bracketed.
    fn bracketed_paste(&self, pane: &str) -> bool {
        let _ = pane;
        false
    }
    /// Its program asked for the cursor keys' application sequences.
    fn application_cursor(&self, pane: &str) -> bool {
        let _ = pane;
        false
    }
    /// What its agent has running of its own between two turns: a build, a
    /// benchmark, a watch on its CI it started in the background
    /// (`pty::pane_job`). Terminals that cannot look cannot tell.
    fn job(&self, pane: &str) -> PaneJob {
        let _ = pane;
        PaneJob::Unknown
    }
    /// Every shell, on the way out.
    fn kill_all(&self);
}

/// Core's PTYs on this machine, each with a screen.
pub struct CorePtys {
    pty: Arc<PtyManager>,
    screens: Screens,
}

impl CorePtys {
    pub fn new(pty: Arc<PtyManager>) -> CorePtys {
        CorePtys {
            pty,
            screens: Screens::default(),
        }
    }
}

impl CorePtys {
    /// The shell of `spawn`, with its agent's `launch` when it has one
    /// (`pty_spawn_launch`), its screen opened first.
    fn open(
        &self,
        spawn: Spawn,
        sink: TermSink,
        launch: Option<AgentLaunch>,
    ) -> Result<Spawned, String> {
        let id = spawn.id.clone();
        let screen = self.screens.open(&id, COLUMNS, ROWS);
        let pty = self.pty.clone();
        let pane = id.clone();
        let output: pty::PtySink = Box::new(move |event| match event {
            PtyEvent::Output(output) => {
                let answers = screen.feed(output.data.as_bytes());
                if !answers.is_empty() {
                    // Off the reader's thread: writing waits on the pane's
                    // writer, which nothing here should hold up.
                    let (pty, pane) = (pty.clone(), pane.clone());
                    std::thread::spawn(move || {
                        for answer in answers {
                            let _ = pty::pty_write(&pty, pane.clone(), answer);
                        }
                    });
                }
                sink(TermEvent::Output(pane.clone()));
            }
            PtyEvent::Exit(_) => sink(TermEvent::Exit(pane.clone())),
        });
        let spawned = match launch {
            Some(launch) => pty::pty_spawn_launch(
                &self.pty,
                output,
                spawn.id,
                None,
                COLUMNS,
                ROWS,
                Some(spawn.name),
                Some(spawn.bridge),
                spawn.port_base,
                spawn.build_env,
                None,
                spawn.claude_account,
                false,
                launch,
            ),
            None => pty::pty_spawn_with(
                &self.pty,
                output,
                spawn.id,
                spawn.cwd,
                None,
                COLUMNS,
                ROWS,
                Some(spawn.name),
                Some(spawn.bridge),
                spawn.port_base,
                spawn.build_env,
                spawn.mcp_config,
                None,
                Some(spawn.agent),
                spawn.claude_account,
                false,
            ),
        };
        if spawned.is_err() {
            self.screens.close(&id);
        }
        spawned
    }
}

impl Terminals for CorePtys {
    fn spawn(&self, spawn: Spawn, sink: TermSink) -> Result<(), String> {
        self.open(spawn, sink, None).map(|_| ())
    }

    fn spawn_agent(
        &self,
        spawn: Spawn,
        launch: AgentLaunch,
        sink: TermSink,
    ) -> Result<AgentStart, String> {
        let id = spawn.id.clone();
        let spawned = self.open(spawn, sink, Some(launch))?;
        match spawned.line {
            Some(line) => Ok(AgentStart {
                line,
                server: spawned.server,
            }),
            // Core answers a line for every launch: a shell without one is
            // not left running with nothing typed into it.
            None => {
                let _ = self.kill(&id);
                Err("the agent's launch gave no line to type".into())
            }
        }
    }

    fn write(&self, pane: &str, data: &str) -> Result<(), String> {
        pty::pty_write(&self.pty, pane.to_string(), data.to_string())
    }

    fn alive(&self, pane: &str) -> bool {
        pty::pty_alive(&self.pty, pane.to_string())
    }

    fn resize(&self, pane: &str, columns: u16, rows: u16) -> Result<(), String> {
        pty::pty_resize(&self.pty, pane.to_string(), columns, rows)?;
        self.screens.resize(pane, columns, rows);
        Ok(())
    }

    fn kill(&self, pane: &str) -> Result<(), String> {
        let killed = pty::pty_kill(&self.pty, pane.to_string(), Some(true));
        self.screens.close(pane);
        killed
    }

    fn lines(&self, pane: &str, n: usize) -> Option<Vec<String>> {
        self.screens.read(pane, |screen| screen.lines(n))
    }

    fn screen(&self, pane: &str) -> Option<Vec<String>> {
        self.screens.read(pane, Screen::rows)
    }

    fn title(&self, pane: &str) -> Option<String> {
        self.screens.read(pane, Screen::title).flatten()
    }

    fn bracketed_paste(&self, pane: &str) -> bool {
        self.screens
            .read(pane, Screen::bracketed_paste)
            .unwrap_or(false)
    }

    fn application_cursor(&self, pane: &str) -> bool {
        self.screens
            .read(pane, Screen::application_cursor)
            .unwrap_or(false)
    }

    fn job(&self, pane: &str) -> PaneJob {
        pty::pane_job(&self.pty, pane)
    }

    fn kill_all(&self) {
        pty::kill_all(&self.pty);
        self.screens.clear();
    }
}

/// Agent panes on core's PTYs, through real shells: the line core builds for
/// the shell as it started, the launch's variables in that shell, and for
/// Codex its own server — a fake `codex` of the test's, never OpenAI's.
#[cfg(all(test, unix))]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use hangar_core::agent::AgentKind;
    use hangar_core::agent::codex::MCP_VAR;
    use hangar_core::agent::launch::{LaunchRequest, McpRequest, ShellSyntax};
    use hangar_core::pty::FloorReason;

    use super::*;
    use crate::testdir::scratch;

    const THREAD: &str = "0199a5c4-7b3e-7c51-a2f4-6d0e8c1b9f20";
    const HANGAR: &str = "/opt/hangar/bin/hangar-ai";

    /// The fake `codex`'s server: on `app-server --listen unix://<path>`, a
    /// socket bound at the path, listening, removed on `SIGTERM`.
    const FAKE_SERVER: &str = r#"import os, signal, socket, sys
path = sys.argv[sys.argv.index("--listen") + 1][len("unix://"):]
def bye(*_):
    try:
        os.unlink(path)
    except OSError:
        pass
    os._exit(0)
signal.signal(signal.SIGTERM, bye)
server = socket.socket(socket.AF_UNIX)
server.bind(path)
server.listen(8)
kept = []
while True:
    kept.append(server.accept()[0])
"#;

    /// A fake `codex` in a scratch folder: a shell script that serves as a
    /// Codex app-server does, for as long as it is let.
    fn fake_codex() -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let codex = scratch("engine-fake-codex").join("codex");
        let script = format!(
            "#!/bin/sh\n\
             # A fake codex, for Teskad's tests: never OpenAI's program.\n\
             [ \"$1\" = probe ] && exit 0\n\
             exec python3 -I -c '{FAKE_SERVER}' \"$@\"\n"
        );
        std::fs::write(&codex, script).unwrap();
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A file just written cannot be run while a process another test
        // forked meanwhile still holds it open (ETXTBSY, 26): tried until it
        // can.
        for _ in 0..100 {
            match std::process::Command::new(&codex).arg("probe").status() {
                Err(error) if error.raw_os_error() == Some(26) => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => break,
            }
        }
        codex
    }

    fn ptys() -> (CorePtys, Arc<PtyManager>) {
        let manager = Arc::new(PtyManager::default());
        (CorePtys::new(manager.clone()), manager)
    }

    fn spawn(id: &str, cwd: &Path, agent: AgentKind) -> Spawn {
        Spawn {
            id: id.into(),
            cwd: cwd.to_string_lossy().into_owned(),
            name: "Ada".into(),
            agent: agent.id().into(),
            ..Spawn::default()
        }
    }

    fn codex(cwd: &Path, command: Option<&Path>) -> LaunchRequest {
        LaunchRequest {
            kind: AgentKind::Codex,
            command: command.map(|path| path.display().to_string()),
            cwd: cwd.to_string_lossy().into_owned(),
            model: Some("gpt-5.1-codex".into()),
            resume: Some(THREAD.into()),
            hangar_mcp: Some(HANGAR.into()),
            ..LaunchRequest::default()
        }
    }

    /// What the pane's shell prints for `$name`, typed into it.
    fn shell_holds(ptys: &CorePtys, pane: &str, name: &str, value: &str) -> bool {
        ptys.write(pane, &format!("printf 'got:%s\\n' \"${name}\"\r"))
            .unwrap();
        let wanted = format!("got:{value}");
        for _ in 0..300 {
            let lines = ptys.lines(pane, 80).unwrap_or_default();
            if lines
                .iter()
                .any(|line| line.trim_start().starts_with(&wanted))
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("{name} never read {value}: {:?}", ptys.lines(pane, 80));
    }

    /// How the shell the PTY starts names a variable.
    fn variable(name: &str) -> String {
        let shell = hangar_core::shells::default_shell().expect("a shell");
        ShellSyntax::of(&shell)
            .expect("a shell Teskad types for")
            .variable(name)
    }

    /// Claude Code: core's line, and its shell holding the MCP file the line
    /// names; no server.
    #[test]
    fn a_claude_code_pane_types_core_s_line_with_its_mcp_file_in_its_shell() {
        let (ptys, _) = ptys();
        let cwd = scratch("engine-agent-claude");
        let request = LaunchRequest {
            kind: AgentKind::Claude,
            cwd: cwd.to_string_lossy().into_owned(),
            name: Some("Ada".into()),
            mcp: Some(McpRequest {
                file: "/srv/mcp/ws-1.json".into(),
                arg: "\"$TESKAD_MCP_CONFIG\"".into(),
                strict: false,
            }),
            ..LaunchRequest::default()
        };
        let started = ptys
            .spawn_agent(
                spawn("claude-1", &cwd, AgentKind::Claude),
                AgentLaunch {
                    request,
                    attach: true,
                },
                Arc::new(|_| {}),
            )
            .expect("a shell");
        assert_eq!(
            started,
            AgentStart {
                line: "claude --name Ada --mcp-config \"$TESKAD_MCP_CONFIG\"".into(),
                server: None,
            }
        );
        assert!(shell_holds(
            &ptys,
            "claude-1",
            "TESKAD_MCP_CONFIG",
            "/srv/mcp/ws-1.json"
        ));
        ptys.kill("claude-1").unwrap();
    }

    /// « Attach Teskad to Codex » off: the floor, its settings in the
    /// shell's variables, which its line names; its session resumed.
    #[test]
    fn a_floor_codex_pane_s_shell_holds_the_settings_its_line_names() {
        let (ptys, manager) = ptys();
        let cwd = scratch("engine-agent-floor");
        let started = ptys
            .spawn_agent(
                spawn("codex-floor", &cwd, AgentKind::Codex),
                AgentLaunch {
                    request: codex(&cwd, None),
                    attach: false,
                },
                Arc::new(|_| {}),
            )
            .expect("a shell");
        assert_eq!(started.server, None);
        assert_eq!(
            started.line,
            format!(
                "codex -c {} -c model=gpt-5.1-codex -c features.image_generation=true resume {THREAD}",
                variable(MCP_VAR)
            )
        );
        assert!(shell_holds(
            &ptys,
            "codex-floor",
            MCP_VAR,
            &format!("mcp_servers.teskad={{command='{HANGAR}'")
        ));
        assert_eq!(pty::codex_socket(&manager, "codex-floor"), None);
        ptys.kill("codex-floor").unwrap();
    }

    /// Attached: the pane's server listens before the line is typed, the
    /// line points at it and resumes the session, and the server ends with
    /// the pane.
    #[test]
    fn an_attached_codex_pane_s_line_points_at_its_own_server() {
        let (ptys, manager) = ptys();
        let cwd = scratch("engine-agent-attached");
        let fake = fake_codex();
        let started = ptys
            .spawn_agent(
                spawn("codex-attached", &cwd, AgentKind::Codex),
                AgentLaunch {
                    request: codex(&cwd, Some(&fake)),
                    attach: true,
                },
                Arc::new(|_| {}),
            )
            .expect("a shell");
        let socket = match &started.server {
            Some(ServerState::Attached { socket }) => socket.clone(),
            other => panic!("not attached: {other:?}"),
        };
        assert_eq!(
            started.line,
            format!(
                "{} --remote unix://{socket} -c model=gpt-5.1-codex resume {THREAD}",
                fake.display()
            )
        );
        assert!(std::os::unix::net::UnixStream::connect(&socket).is_ok());
        assert_eq!(
            pty::codex_socket(&manager, "codex-attached"),
            Some(PathBuf::from(&socket))
        );
        ptys.kill("codex-attached").unwrap();
        assert_eq!(pty::codex_socket(&manager, "codex-attached"), None);
    }

    /// A server that cannot start: the pane runs the floor, says why, and
    /// its shell holds the floor's settings all the same.
    #[test]
    fn a_codex_server_that_cannot_start_leaves_the_pane_on_the_floor() {
        let (ptys, _) = ptys();
        let cwd = scratch("engine-agent-no-server");
        let missing = cwd.join("no-such-codex");
        let started = ptys
            .spawn_agent(
                spawn("codex-missing", &cwd, AgentKind::Codex),
                AgentLaunch {
                    request: codex(&cwd, Some(&missing)),
                    attach: true,
                },
                Arc::new(|_| {}),
            )
            .expect("a shell");
        let reason = match &started.server {
            Some(ServerState::Floor { reason }) => reason.clone(),
            other => panic!("not on the floor: {other:?}"),
        };
        assert!(
            matches!(
                reason,
                FloorReason::NotStarted { .. } | FloorReason::Exited { .. }
            ),
            "{reason:?}"
        );
        assert!(
            reason
                .to_string()
                .starts_with("Codex runs without Teskad attached"),
            "{reason}"
        );
        assert_eq!(
            started.line,
            format!(
                "{} -c {} -c model=gpt-5.1-codex -c features.image_generation=true resume {THREAD}",
                missing.display(),
                variable(MCP_VAR)
            )
        );
        assert!(shell_holds(
            &ptys,
            "codex-missing",
            MCP_VAR,
            &format!("mcp_servers.teskad={{command='{HANGAR}'")
        ));
        ptys.kill("codex-missing").unwrap();
    }
}
