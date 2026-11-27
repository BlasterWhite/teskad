//! The agents' official installers, each run as a child process whose every
//! line is handed to the setup sheet as it comes.
//!
//! The commands are hangar-core's (`install::agent::installer`), each from
//! its maker's page word for word. Claude Code (code.claude.com/docs/en/setup):
//! `curl -fsSL https://claude.ai/install.sh | bash` on macOS and Linux, `irm
//! https://claude.ai/install.ps1 | iex` in Windows PowerShell. Codex
//! (learn.chatgpt.com/docs/codex/cli): `curl -fsSL
//! https://chatgpt.com/codex/install.sh | sh` and `irm
//! https://chatgpt.com/codex/install.ps1 | iex`. All four install for the user
//! alone (`~/.local/bin`, or `%LOCALAPPDATA%\Programs\OpenAI\Codex\bin`), so no
//! `sudo` and no administrator: the child gets no terminal and no stdin, and
//! could not ask for a password if it wanted to. Hangar downloads nothing
//! itself: the maker's script does.
//!
//! Codex's scripts ask questions of their own (« Start Codex now? », « Uninstall
//! the existing npm-managed Codex now? »), and `install.sh` asks them on
//! `/dev/tty` even without a stdin: a Hangar started from a terminal would
//! wait on that terminal. They are told not to ask with the switch the scripts
//! document, `CODEX_NON_INTERACTIVE=1` (scripts/install/install.sh and
//! install.ps1 of openai/codex, rust-v0.170.0), which answers no to both.
//!
//! A debug build can be handed another command in `TESKAD_DEV_CLAUDE_INSTALLER`
//! or `TESKAD_DEV_CODEX_INSTALLER` (run by `sh -c`, or PowerShell on Windows),
//! so the setup is tried end to end without installing anything on the
//! machine of whoever tries it.

use hangar_core::agent::AgentKind;
use hangar_core::install::agent as ways;
use iced::futures::{SinkExt, Stream};
use tokio::io::{AsyncBufReadExt, BufReader};

use super::Msg;
use crate::message::Message;

/// The lines of output kept: an installer's whole log is a few dozen, so this
/// only bounds one gone wrong.
pub const KEPT_LINES: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// Linux and macOS: the shell script.
    Unix,
    Windows,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) { Os::Windows } else { Os::Unix }
    }
}

/// What is run, and the command line the sheet shows for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installer {
    pub program: String,
    pub args: Vec<String>,
    /// Set in the child's environment, besides the plain-text ones.
    pub env: Vec<(String, String)>,
    /// As the person would type it.
    pub shown: String,
}

/// The page `kind`'s install commands come from, which the sheet points to
/// when an install fails.
pub fn docs_url(kind: AgentKind) -> &'static str {
    ways::installer(kind).docs_url
}

/// The installer of `kind` for `os`: the official command, run by the shell
/// the docs run it in. `pipefail`, so a `curl` that fails (offline, a 403)
/// fails the install instead of piping nothing into a shell that exits 0.
pub fn official(os: Os, kind: AgentKind) -> Installer {
    let official = ways::installer(kind);
    let line = match os {
        Os::Unix => official.unix,
        Os::Windows => official.windows,
    };
    in_shell(os, kind, line, true)
}

/// `line` run the way the installer of `os` is run, in the environment
/// `kind`'s installer is given.
fn in_shell(os: Os, kind: AgentKind, line: &str, pipefail: bool) -> Installer {
    let (program, mut args) = match os {
        Os::Unix if pipefail => ("bash", vec!["-o", "pipefail", "-c"]),
        Os::Unix => ("sh", vec!["-c"]),
        Os::Windows => (
            "powershell",
            vec![
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
            ],
        ),
    };
    args.push(line);
    Installer {
        program: program.into(),
        args: args.into_iter().map(str::to_string).collect(),
        env: env_of(kind)
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
        shown: line.into(),
    }
}

/// What `kind`'s installer is told besides the plain text: Codex's, not to
/// ask anything.
fn env_of(kind: AgentKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        AgentKind::Claude => &[],
        AgentKind::Codex => &[("CODEX_NON_INTERACTIVE", "1")],
    }
}

/// The variable a debug build reads a stand-in for `kind`'s installer from.
fn fake_env(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "TESKAD_DEV_CLAUDE_INSTALLER",
        AgentKind::Codex => "TESKAD_DEV_CODEX_INSTALLER",
    }
}

/// The installer of `kind` to run: the official one, or in a debug build the
/// command its `TESKAD_DEV_*_INSTALLER` names.
pub fn chosen(kind: AgentKind) -> Installer {
    let os = Os::current();
    match fake(std::env::var(fake_env(kind)).ok()) {
        Some(line) => in_shell(os, kind, &line, false),
        None => official(os, kind),
    }
}

/// The stand-in command, in a debug build only: a release build always runs
/// the official installer.
pub(super) fn fake(asked: Option<String>) -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    asked.filter(|line| !line.trim().is_empty())
}

/// How the installer ended: its exit code, `None` when a signal ended it, or
/// why it could not be started.
pub type Exit = Result<Option<i32>, String>;

/// Runs `installer` of `kind`, sending each line it prints (stdout and
/// stderr, in the order they come) and then how it ended, all tagged with
/// `kind` and `run`.
pub fn run(installer: Installer, kind: AgentKind, run: u64) -> impl Stream<Item = Message> {
    iced::stream::channel(64, async move |mut output| {
        let wrap = |msg: Msg| Message::Shell(crate::shell::Action::Setup(msg));
        let mut command = tokio::process::Command::new(&installer.program);
        command
            .args(&installer.args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // Plain text for the sheet: no colours, no cursor tricks.
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .envs(installer.env.iter().map(|(name, value)| (name, value)))
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            /// `CREATE_NO_WINDOW`: no console flashes up over Hangar.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = output
                    .send(wrap(Msg::Exited(kind, run, Err(error.to_string()))))
                    .await;
                return;
            }
        };
        let mut out = child.stdout.take().map(BufReader::new);
        let mut err = child.stderr.take().map(BufReader::new);
        let mut out_buf = Vec::new();
        let mut err_buf = Vec::new();
        while out.is_some() || err.is_some() {
            let line = tokio::select! {
                read = next_line(&mut out, &mut out_buf), if out.is_some() => {
                    if read.is_none() { out = None; }
                    read
                }
                read = next_line(&mut err, &mut err_buf), if err.is_some() => {
                    if read.is_none() { err = None; }
                    read
                }
            };
            if let Some(line) = line {
                let _ = output.send(wrap(Msg::Output(kind, run, line))).await;
            }
        }
        let exit = child
            .wait()
            .await
            .map(|status| status.code())
            .map_err(|error| error.to_string());
        let _ = output.send(wrap(Msg::Exited(kind, run, exit))).await;
    })
}

/// The next line of `reader`, without its ending; `None` at the end.
async fn next_line<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut Option<BufReader<R>>,
    buf: &mut Vec<u8>,
) -> Option<String> {
    let reader = reader.as_mut()?;
    buf.clear();
    match reader.read_until(b'\n', buf).await {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(clean(&String::from_utf8_lossy(buf))),
    }
}

/// A line as the sheet shows it: its ending gone, what a carriage return
/// wrote over gone, and any escape sequence an installer printed regardless
/// of `NO_COLOR` taken out.
pub fn clean(line: &str) -> String {
    let line = line.trim_end_matches(['\n', '\r']);
    let line = line.rsplit('\r').next().unwrap_or(line);
    let mut cleaned = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // CSI: ESC [ parameters, then one final byte in @..~.
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            } else {
                chars.next();
            }
            continue;
        }
        if !c.is_control() || c == '\t' {
            cleaned.push(c);
        }
    }
    cleaned
}
