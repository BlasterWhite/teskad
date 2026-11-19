//! A Codex pane's own `codex app-server` (docs/native/agents.md §4.1, D5):
//! started before the pane's line is typed, with the pane's environment,
//! listening on a socket of the pane's in Hangar's runtime folder, and ended
//! with the pane — closed, restarted, put to sleep, its worker stopped, or
//! Hangar quitting (`pty.rs`, `end`). When it cannot be started, the pane runs
//! the floor (D6), and [`ServerState::Floor`] says why.
//!
//! Everything below about what Codex does was read in `openai/codex` at the
//! release `agent::codex::CHECKED_AT` names (paths under `codex-rs/`); none of
//! it was run against a Codex (§4.0). The tests start a fake `codex`, a
//! script of ours.
//!
//! # The socket
//!
//! `<runtime>/codex-<12 hex digits of the pane id's SHA-256>.sock`, the same
//! for a pane each time it starts, where `<runtime>` is `$XDG_RUNTIME_DIR/hangar`
//! on Linux when the session has one, else `<temp>/hangar-<uid>`: a folder of
//! the person's own, mode 0700, checked to be so before every start. What
//! Codex does with the path (`app-server-transport/src/transport/unix_socket.rs`,
//! `bind_private_daemon_socket`; `uds/src/daemon_directory.rs`):
//!
//! - it refuses a parent folder that is not the user's, or that others can
//!   write to;
//! - it binds the real socket as `/tmp/codex-daemon-<uid>/<SHA-256 of the
//!   path>`, mode 0600, and publishes the path asked for as a symbolic link
//!   to it once it listens — so the path appearing is the sign Hangar waits
//!   for, and a path longer than a socket address can be is no problem (its
//!   clients resolve the link, `uds/src/lib.rs`, `connect_stream`);
//! - it removes both when it ends, and a stale one at its next start.
//!
//! Hangar removes the link when the pane ends; the real socket is Codex's.
//! A server bound at the path itself (the fake, or another release) is made
//! 0600 by Hangar once it listens.
//!
//! # Its processes
//!
//! The server leads a process group of its own, and the commands it runs
//! leave it — Codex starts them in groups or sessions of their own
//! (`utils/pty/src/child_command.rs`, `ProcessMode`) — so a pane's end takes
//! the group and every process listed below the server, whatever the kill
//! asked for the shell's tree: they are the agent's. The terminal's hangup
//! does not reach a process that is not on it, so the server is asked to end
//! with `SIGTERM`, in the same grace as the pane's shell, then killed.
//!
//! On Linux the server is also told to end with Hangar itself
//! (`PR_SET_PDEATHSIG`, `SIGTERM`, which npm's `codex` launcher passes on to
//! the program it starts — an assumption from the npm package, not run), so a
//! Hangar that crashes leaves no server behind; the signal follows the thread
//! that started the process, so every server is started from one thread that
//! lives as long as Hangar. Elsewhere, and in case one survived all the same,
//! a server still listening on a pane's socket — found by its command line in
//! the process table — is ended before a new one starts for that pane.
//!
//! # Windows
//!
//! The floor, always: [`FloorReason::Unsupported`]. Codex 0.170 does listen
//! on `unix://` there (`uds/src/lib.rs`: AF_UNIX through `uds_windows`, in a
//! folder with a user-only DACL), but nothing here was ever run on Windows,
//! and ending a server's tree there takes a job object Hangar does not make
//! yet; a WSL pane, whose server would have to run inside WSL and be reached
//! through `codex app-server proxy --sock`, is the same case.

use std::fmt;
use std::time::Duration;

use serde::Serialize;

#[cfg(unix)]
pub(super) use unix::{attach_pane, CodexServer};

#[cfg(all(unix, test))]
pub(super) use unix::attach;

#[cfg(not(unix))]
pub(super) use other::{attach_pane, CodexServer};

/// How long a server is given to listen before the pane goes to the floor
/// without it. Codex reads its configuration and binds its socket in well
/// under a second; npm's launcher adds a Node start before that.
pub(super) const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// A pane's Codex server, when its launch asked for one: what the pane's
/// status says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum ServerState {
    /// It listens on `socket`, and the pane's line attaches to it.
    Attached { socket: String },
    /// The pane runs the plain `codex` (D6), because of `reason`.
    Floor { reason: FloorReason },
}

/// Why a pane that was to run attached runs the floor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum FloorReason {
    /// This system: Windows (see the top of this file).
    Unsupported,
    /// Hangar's runtime folder could not be made, or is not the person's own.
    RuntimeFolder { detail: String },
    /// The pane's shell cannot type the socket's path (`agent::codex`,
    /// `remote`): cmd, and a path holding a `%`.
    Untypable,
    /// The program could not be started at all.
    NotStarted { program: String, detail: String },
    /// It ended before it listened; `said` is the end of what it printed.
    Exited { code: Option<i32>, said: String },
    /// It did not listen within `seconds`, and was ended.
    Timeout { seconds: u64 },
}

impl fmt::Display for FloorReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lead = "Codex runs without Hangar attached";
        match self {
            FloorReason::Unsupported => {
                write!(
                    f,
                    "{lead}: Hangar does not attach to Codex on this system yet"
                )
            }
            FloorReason::RuntimeFolder { detail } => {
                write!(f, "{lead}: its socket's folder is not usable ({detail})")
            }
            FloorReason::Untypable => {
                write!(f, "{lead}: this shell cannot type its socket's path")
            }
            FloorReason::NotStarted { program, detail } => {
                write!(
                    f,
                    "{lead}: `{program} app-server` could not start ({detail})"
                )
            }
            FloorReason::Exited { code, said } => {
                write!(f, "{lead}: its server ended before it listened")?;
                if let Some(code) = code {
                    write!(f, " (exit {code})")?;
                }
                if !said.is_empty() {
                    write!(f, ": {said}")?;
                }
                Ok(())
            }
            FloorReason::Timeout { seconds } => {
                write!(f, "{lead}: its server did not listen within {seconds} s")
            }
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::ffi::OsString;
    use std::io;
    use std::io::Read;
    use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use super::{FloorReason, ServerState};
    use crate::agent::launch::{self, LaunchRequest, ServerSpec};

    /// How often a starting server is looked at.
    const READY_POLL: Duration = Duration::from_millis(25);
    /// How long a stale server is given to end on `SIGTERM`.
    const STALE_GRACE: Duration = Duration::from_millis(300);
    /// How long what a server said is waited for once it ended.
    const SAID_WAIT: Duration = Duration::from_millis(500);
    /// What a server printed last, at most this much.
    const SAID_BYTES: usize = 4096;
    /// What a reason quotes of it, at most, in characters.
    const SAID_SHOWN: usize = 300;

    /// [`attach`], with what the pane's status says of it.
    pub(in crate::pty) fn attach_pane(
        id: &str,
        request: &mut LaunchRequest,
        env: impl FnOnce(&[(String, Option<OsString>)]) -> Vec<(OsString, OsString)>,
        wait: Duration,
    ) -> (Option<CodexServer>, ServerState) {
        match attach(id, request, env, wait) {
            Ok(server) => {
                let socket = server.socket.to_string_lossy().into_owned();
                (Some(server), ServerState::Attached { socket })
            }
            Err(reason) => (None, ServerState::Floor { reason }),
        }
    }

    /// Starts pane `id`'s server for `request`, on the pane's socket, and
    /// waits for it to listen (`wait` at most). `env` is handed the variables
    /// of the attached launch (`LaunchSpec::env`) and answers the server's
    /// whole environment: the pane's, with those applied.
    ///
    /// `request.server_socket` is the pane's socket when the server listens,
    /// and empty otherwise, so the line built from `request` afterwards
    /// attaches exactly when there is a server to attach to.
    pub(in crate::pty) fn attach(
        id: &str,
        request: &mut LaunchRequest,
        env: impl FnOnce(&[(String, Option<OsString>)]) -> Vec<(OsString, OsString)>,
        wait: Duration,
    ) -> Result<CodexServer, FloorReason> {
        request.server_socket = None;
        let socket = socket_for(id).map_err(|err| FloorReason::RuntimeFolder {
            detail: err.to_string(),
        })?;
        let Some(address) = socket.to_str() else {
            return Err(FloorReason::RuntimeFolder {
                detail: format!("{} is not UTF-8", socket.display()),
            });
        };
        request.server_socket = Some(address.to_string());
        let (Some(server), Ok(spec)) = (launch::server(request), launch::build(request)) else {
            request.server_socket = None;
            return Err(FloorReason::Untypable);
        };
        let started = CodexServer::start(id, &server, env(&spec.env), &socket, wait);
        if started.is_err() {
            request.server_socket = None;
        }
        started
    }

    /// Pane `id`'s socket, its folder made and checked first: the person's
    /// own, 0700.
    pub(in crate::pty) fn socket_for(id: &str) -> io::Result<PathBuf> {
        let folder = runtime_root();
        private_folder(&folder)?;
        Ok(folder.join(socket_name(id)))
    }

    /// The folder Hangar's sockets go in, before it is made.
    pub(super) fn runtime_root() -> PathBuf {
        #[cfg(target_os = "linux")]
        {
            let session = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute() && dir.is_dir());
            if let Some(dir) = session {
                return dir.join("hangar");
            }
        }
        std::env::temp_dir().join(format!("hangar-{}", unsafe { libc::geteuid() }))
    }

    /// The name of pane `id`'s socket in the runtime folder: the same for
    /// the pane each time, short enough for a socket's address under macOS's
    /// long temporary folder, and nothing of the id itself.
    pub(super) fn socket_name(id: &str) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(id.as_bytes());
        let hex: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();
        format!("codex-{hex}.sock")
    }

    /// Makes `folder`, 0700, or checks the one there: a folder (not a link to
    /// one) of this user's, made 0700 if it was not. Codex refuses a parent
    /// others could replace its socket in, and so does Hangar.
    pub(super) fn private_folder(folder: &Path) -> io::Result<()> {
        match std::fs::DirBuilder::new().mode(0o700).create(folder) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
        let meta = std::fs::symlink_metadata(folder)?;
        if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{} is not a folder of this user's", folder.display()),
            ));
        }
        if meta.mode() & 0o777 != 0o700 {
            std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    /// A pane's running server.
    pub(in crate::pty) struct CodexServer {
        child: Child,
        /// The group it leads: its pid, read at its start.
        group: libc::pid_t,
        socket: PathBuf,
        said: Said,
        /// Reaped: its pid is no longer ours to signal.
        ended: bool,
    }

    impl CodexServer {
        /// Starts `spec` with `env` (the whole environment it gets) and waits,
        /// `wait` at most, until it listens on `socket`. A server that ends
        /// first, or does not listen in time, is ended with what it started,
        /// and the reason says so. One already listening on `socket` — left
        /// by a Hangar that ended without ending it — is ended first.
        pub(super) fn start(
            id: &str,
            spec: &ServerSpec,
            env: Vec<(OsString, OsString)>,
            socket: &Path,
            wait: Duration,
        ) -> Result<CodexServer, FloorReason> {
            end_stale(socket);
            clear(socket).map_err(|err| FloorReason::RuntimeFolder {
                detail: err.to_string(),
            })?;
            let cwd = Some(PathBuf::from(&spec.cwd)).filter(|cwd| cwd.is_dir());
            let mut child = spawn(Launch {
                program: spec.program.clone(),
                args: spec.args.clone(),
                env,
                cwd,
            })
            .map_err(|err| FloorReason::NotStarted {
                program: spec.program.clone(),
                detail: err.to_string(),
            })?;
            let said = Said::read(child.stderr.take(), id);
            let mut server = CodexServer {
                group: child.id() as libc::pid_t,
                child,
                socket: socket.to_path_buf(),
                said,
                ended: false,
            };
            match server.wait_listening(wait) {
                Ok(()) => {
                    private_socket(socket);
                    Ok(server)
                }
                Err(reason) => {
                    server.end_now();
                    Err(reason)
                }
            }
        }

        fn wait_listening(&mut self, wait: Duration) -> Result<(), FloorReason> {
            let deadline = Instant::now() + wait;
            loop {
                if let Ok(Some(status)) = self.child.try_wait() {
                    self.ended = true;
                    return Err(FloorReason::Exited {
                        code: status.code(),
                        said: self.said.last_words(),
                    });
                }
                if listening(&self.socket) {
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    return Err(FloorReason::Timeout {
                        seconds: wait.as_millis().div_ceil(1000) as u64,
                    });
                }
                std::thread::sleep(READY_POLL);
            }
        }

        pub(in crate::pty) fn pid(&self) -> u32 {
            self.child.id()
        }

        pub(in crate::pty) fn socket(&self) -> &Path {
            &self.socket
        }

        /// Asks it to end, with its group: the first half of a pane's end.
        pub(in crate::pty) fn hang_up(&self) {
            if !self.ended {
                unsafe {
                    libc::killpg(self.group, libc::SIGTERM);
                }
            }
        }

        /// Whether it ended, and nothing is left in its group.
        pub(in crate::pty) fn gone(&mut self) -> bool {
            if !self.ended {
                self.ended = matches!(self.child.try_wait(), Ok(Some(_)));
            }
            self.ended && unsafe { libc::killpg(self.group, 0) } != 0
        }

        /// Kills what is left of it outright — its group, `below` (what was
        /// listed below it before the pane's end began) and itself — reaps
        /// it and removes its socket.
        pub(in crate::pty) fn kill(&mut self, below: &[u32]) {
            unsafe {
                libc::killpg(self.group, libc::SIGKILL);
            }
            crate::procs::terminate(below);
            if !self.ended {
                let _ = self.child.kill();
                let _ = self.child.wait();
                self.ended = true;
            }
            forget(&self.socket);
        }

        /// Ends it at once, with everything below it: a start that failed, a
        /// pane that never got its shell, a server dropped without its pane's
        /// end.
        fn end_now(&mut self) {
            let below = crate::procs::table()
                .map(|table| crate::procs::descendants(&table, self.pid(), crate::procs::started))
                .unwrap_or_default();
            self.kill(&below);
        }
    }

    impl Drop for CodexServer {
        fn drop(&mut self) {
            if self.ended {
                forget(&self.socket);
            } else {
                self.end_now();
            }
        }
    }

    /// What a server is started with, sent to the thread that starts it.
    struct Launch {
        program: String,
        args: Vec<String>,
        env: Vec<(OsString, OsString)>,
        cwd: Option<PathBuf>,
    }

    impl Launch {
        fn command(self) -> Command {
            let Launch {
                program,
                args,
                env,
                cwd,
            } = self;
            let mut command = Command::new(program);
            command
                .args(args)
                .env_clear()
                .envs(env)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                // A group of its own: ended as one, and out of the reach of a
                // Ctrl-C typed in the terminal Hangar was started from.
                .process_group(0);
            if let Some(cwd) = cwd {
                command.current_dir(cwd);
            }
            command
        }
    }

    /// Starts a server, told to end with Hangar (see the top of this file):
    /// on the one thread that starts them all, which lives as long as Hangar,
    /// since the signal is sent when the thread that started the process ends.
    #[cfg(target_os = "linux")]
    fn spawn(launch: Launch) -> io::Result<Child> {
        use std::sync::mpsc;
        use std::sync::OnceLock;

        type Job = (Launch, mpsc::Sender<io::Result<Child>>);
        static STARTER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();

        let starter = STARTER.get_or_init(|| {
            let (jobs, queue) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("codex-servers".into())
                .spawn(move || {
                    for (launch, answer) in queue {
                        let mut command = launch.command();
                        let hangar = std::process::id() as libc::pid_t;
                        // SAFETY: two system calls between fork and exec,
                        // nothing allocated.
                        unsafe {
                            command.pre_exec(move || {
                                if libc::prctl(
                                    libc::PR_SET_PDEATHSIG,
                                    libc::SIGTERM as libc::c_ulong,
                                    0,
                                    0,
                                    0,
                                ) != 0
                                {
                                    return Err(io::Error::last_os_error());
                                }
                                // Hangar ended before the call above: no
                                // signal would ever come.
                                if libc::getppid() != hangar {
                                    return Err(io::Error::from_raw_os_error(libc::ESRCH));
                                }
                                Ok(())
                            });
                        }
                        let _ = answer.send(command.spawn());
                    }
                })
                .expect("a thread to start the panes' Codex servers");
            Mutex::new(jobs)
        });
        let gone = || io::Error::other("the thread that starts Codex servers is gone");
        let (answer, answered) = mpsc::channel();
        starter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send((launch, answer))
            .map_err(|_| gone())?;
        answered.recv().map_err(|_| gone())?
    }

    #[cfg(not(target_os = "linux"))]
    fn spawn(launch: Launch) -> io::Result<Child> {
        launch.command().spawn()
    }

    /// Whether a server listens at `socket`: a socket there, or Codex's link
    /// to one, that takes a connection.
    pub(super) fn listening(socket: &Path) -> bool {
        let is_socket = std::fs::metadata(socket).is_ok_and(|meta| meta.file_type().is_socket());
        is_socket && connect(socket).is_ok()
    }

    /// A connection to `socket`, through the link resolved when the path is
    /// longer than a socket's address can be, as Codex's own clients do.
    fn connect(socket: &Path) -> io::Result<UnixStream> {
        match UnixStream::connect(socket) {
            Err(err) if err.kind() == io::ErrorKind::InvalidInput => {
                UnixStream::connect(std::fs::canonicalize(socket)?)
            }
            result => result,
        }
    }

    /// A socket bound at the path itself is made the person's alone; Codex's
    /// link points at a socket it made 0600 already.
    fn private_socket(socket: &Path) {
        let bound_here =
            std::fs::symlink_metadata(socket).is_ok_and(|meta| meta.file_type().is_socket());
        if bound_here {
            let _ = std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600));
        }
    }

    /// Makes room at `socket` for a new server: what an earlier one left
    /// there — its link or its socket — is removed; anything else stays, and
    /// is an error.
    fn clear(socket: &Path) -> io::Result<()> {
        match std::fs::symlink_metadata(socket) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
            Ok(meta) if meta.file_type().is_symlink() || meta.file_type().is_socket() => {
                std::fs::remove_file(socket)
            }
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} is not a socket", socket.display()),
            )),
        }
    }

    /// Removes a server's link or socket, and nothing else.
    fn forget(socket: &Path) {
        let _ = clear(socket);
    }

    /// Ends what still listens on `socket` from before — a server, and npm's
    /// launcher above it — with what it started: `SIGTERM`, a moment, then
    /// `SIGKILL`.
    fn end_stale(socket: &Path) {
        let Some(table) = crate::pty::jobs::table() else {
            return;
        };
        let address = format!("unix://{}", socket.display());
        let stale: Vec<u32> = table
            .iter()
            .filter(|row| row.command.contains("app-server") && names(&row.command, &address))
            .map(|row| row.pid)
            .collect();
        if stale.is_empty() {
            return;
        }
        let rows: Vec<crate::procs::Proc> = table
            .iter()
            .map(|row| crate::procs::Proc {
                pid: row.pid,
                ppid: row.ppid,
            })
            .collect();
        let mut doomed = stale.clone();
        for pid in &stale {
            doomed.extend(crate::procs::descendants(
                &rows,
                *pid,
                crate::procs::started,
            ));
        }
        let ours = |pid: &u32| libc::pid_t::try_from(*pid).ok().filter(|pid| *pid > 1);
        for pid in doomed.iter().filter_map(ours) {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
        let alive = |pid: &u32| ours(pid).is_some_and(|pid| unsafe { libc::kill(pid, 0) } == 0);
        let deadline = Instant::now() + STALE_GRACE;
        while doomed.iter().any(alive) && Instant::now() < deadline {
            std::thread::sleep(READY_POLL);
        }
        let left: Vec<u32> = doomed.into_iter().filter(alive).collect();
        crate::procs::terminate(&left);
    }

    /// Whether `command` holds `address` as a whole word: not as the start
    /// of a longer path.
    pub(super) fn names(command: &str, address: &str) -> bool {
        command.match_indices(address).any(|(at, _)| {
            command[at + address.len()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
        })
    }

    /// The end of what a server printed on its standard error, read on a
    /// thread of its own so the pipe never fills.
    struct Said {
        tail: Arc<Mutex<Vec<u8>>>,
        reader: Option<std::thread::JoinHandle<()>>,
    }

    impl Said {
        fn read(stderr: Option<std::process::ChildStderr>, id: &str) -> Said {
            let tail = Arc::new(Mutex::new(Vec::new()));
            let reader = stderr.and_then(|mut stderr| {
                let tail = tail.clone();
                std::thread::Builder::new()
                    .name(format!("codex-said-{id}"))
                    .spawn(move || {
                        let mut buf = [0u8; 4096];
                        while let Ok(n) = stderr.read(&mut buf) {
                            if n == 0 {
                                break;
                            }
                            let mut tail = tail.lock().unwrap_or_else(PoisonError::into_inner);
                            tail.extend_from_slice(&buf[..n]);
                            if tail.len() > SAID_BYTES * 2 {
                                let cut = tail.len() - SAID_BYTES;
                                tail.drain(..cut);
                            }
                        }
                    })
                    .ok()
            });
            Said { tail, reader }
        }

        /// Its last words, once it ended: the end of what it printed, on one
        /// line.
        fn last_words(&self) -> String {
            let deadline = Instant::now() + SAID_WAIT;
            while self
                .reader
                .as_ref()
                .is_some_and(|reader| !reader.is_finished())
                && Instant::now() < deadline
            {
                std::thread::sleep(READY_POLL);
            }
            let tail = self.tail.lock().unwrap_or_else(PoisonError::into_inner);
            let text = String::from_utf8_lossy(&tail);
            let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let count = flat.chars().count();
            if count <= SAID_SHOWN {
                flat
            } else {
                let end: String = flat.chars().skip(count - SAID_SHOWN).collect();
                format!("…{}", end.trim_start())
            }
        }
    }
}

#[cfg(not(unix))]
mod other {
    use std::ffi::OsString;
    use std::path::Path;
    use std::time::Duration;

    use super::{FloorReason, ServerState};
    use crate::agent::launch::LaunchRequest;

    /// No server is started here (see the top of this file): the floor.
    pub(in crate::pty) fn attach_pane(
        _id: &str,
        request: &mut LaunchRequest,
        _env: impl FnOnce(&[(String, Option<OsString>)]) -> Vec<(OsString, OsString)>,
        _wait: Duration,
    ) -> (Option<CodexServer>, ServerState) {
        request.server_socket = None;
        let reason = FloorReason::Unsupported;
        (None, ServerState::Floor { reason })
    }

    /// Never made: no server runs here.
    pub(in crate::pty) enum CodexServer {}

    impl CodexServer {
        pub(in crate::pty) fn pid(&self) -> u32 {
            match *self {}
        }

        pub(in crate::pty) fn socket(&self) -> &Path {
            match *self {}
        }

        pub(in crate::pty) fn kill(&mut self, _below: &[u32]) {
            match *self {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_reads_as_one_sentence_for_the_status() {
        let reasons = [
            FloorReason::Unsupported,
            FloorReason::RuntimeFolder {
                detail: "denied".into(),
            },
            FloorReason::Untypable,
            FloorReason::NotStarted {
                program: "codex".into(),
                detail: "not found".into(),
            },
            FloorReason::Exited {
                code: Some(2),
                said: "error: unrecognized subcommand".into(),
            },
            FloorReason::Timeout { seconds: 10 },
        ];
        for reason in &reasons {
            let text = reason.to_string();
            assert!(
                text.starts_with("Codex runs without Hangar attached: "),
                "{text}"
            );
            assert!(!text.contains('\n'), "{text}");
        }
        assert_eq!(
            reasons[4].to_string(),
            "Codex runs without Hangar attached: its server ended before it listened \
             (exit 2): error: unrecognized subcommand"
        );
    }

    #[test]
    fn the_state_reaches_the_window_tagged() {
        let floor = ServerState::Floor {
            reason: FloorReason::Timeout { seconds: 10 },
        };
        assert_eq!(
            serde_json::to_value(&floor).unwrap(),
            serde_json::json!({"mode": "floor", "reason": {"kind": "timeout", "seconds": 10}})
        );
        let attached = ServerState::Attached {
            socket: "/run/user/1000/hangar/codex-0123456789ab.sock".into(),
        };
        assert_eq!(
            serde_json::to_value(&attached).unwrap(),
            serde_json::json!({
                "mode": "attached",
                "socket": "/run/user/1000/hangar/codex-0123456789ab.sock"
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_panes_socket_is_named_after_it_the_same_each_time() {
        let id = "0f8fad5b-d9cb-469f-a165-70867728950e";
        let name = unix::socket_name(id);
        assert_eq!(name, unix::socket_name(id));
        assert_ne!(name, unix::socket_name("another pane"));
        assert!(
            name.starts_with("codex-") && name.ends_with(".sock"),
            "{name}"
        );
        assert_eq!(name.len(), "codex-".len() + 12 + ".sock".len());
        // Nothing of the id itself, whatever it holds.
        assert!(unix::socket_name("a/../../b c")
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'));
    }

    #[cfg(unix)]
    #[test]
    fn a_command_line_names_the_socket_only_as_a_whole_word() {
        let address = "unix:///run/hangar/codex-1.sock";
        assert!(unix::names(
            "codex app-server --listen unix:///run/hangar/codex-1.sock -c x=1",
            address
        ));
        assert!(unix::names(
            "codex app-server --listen unix:///run/hangar/codex-1.sock",
            address
        ));
        assert!(!unix::names(
            "codex app-server --listen unix:///run/hangar/codex-1.sock.real",
            address
        ));
        assert!(!unix::names(
            "codex app-server --listen unix:///run/hangar/codex-12.sock",
            address
        ));
    }

    /// The runtime folder is the person's own, 0700, whatever it was found
    /// as; a link standing in for it is refused.
    #[cfg(unix)]
    #[test]
    fn the_runtime_folder_is_private_and_never_a_link() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = crate::testdir::scratch("codex-run");
        let folder = scratch.join("run");
        unix::private_folder(&folder).expect("made");
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&folder), 0o700);
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        unix::private_folder(&folder).expect("checked");
        assert_eq!(mode(&folder), 0o700);

        let elsewhere = scratch.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let link = scratch.join("link");
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        let refused = unix::private_folder(&link).expect_err("a link is refused");
        assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[cfg(unix)]
    #[test]
    fn the_runtime_folder_is_the_sessions_or_a_temporary_one_of_the_users() {
        let root = unix::runtime_root();
        assert!(root.is_absolute(), "{}", root.display());
        let name = root.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name == "hangar" || name == format!("hangar-{}", unsafe { libc::geteuid() }),
            "{name}"
        );
        let socket = unix::socket_for("p-root").expect("the folder is usable");
        assert_eq!(socket.parent(), Some(root.as_path()));
    }
}
