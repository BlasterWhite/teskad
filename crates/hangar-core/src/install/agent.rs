//! The agent CLIs on this machine: where each one is, which version it is,
//! and how its makers say to install it.
//!
//! Nothing here installs anything: the setup sheet runs the official command
//! (hangar-app's `shell/setup`), and these are the commands it is given.
//!
//! Sources, read on 2026-11-19:
//! - Claude Code: code.claude.com/docs/en/setup (the native installer, which
//!   puts `claude` in `~/.local/bin`).
//! - Codex: learn.chatgpt.com/docs/codex/cli (developers.openai.com/codex/cli
//!   redirects there) and the README of github.com/openai/codex: the
//!   standalone installers, `curl -fsSL https://chatgpt.com/codex/install.sh
//!   | sh` on macOS and Linux and `irm https://chatgpt.com/codex/install.ps1
//!   | iex` in PowerShell, or `npm install -g @openai/codex`, or `brew install
//!   --cask codex`. The scripts themselves say where `codex` goes:
//!   `$CODEX_INSTALL_DIR`, else `~/.local/bin` (install.sh) or
//!   `%LOCALAPPDATA%\Programs\OpenAI\Codex\bin`, added to the user's PATH in
//!   the registry (install.ps1).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;

use crate::sessions::AgentKind;

/// How an agent is installed, as its makers document it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installer {
    /// The page the commands come from, to point to when an install fails.
    pub docs_url: &'static str,
    /// The official command on macOS and Linux, a script piped to a shell.
    /// It installs for the user alone: no `sudo`.
    pub unix: &'static str,
    /// The official command in Windows PowerShell, for the user alone too.
    pub windows: &'static str,
    /// The other documented ways, each through a package manager.
    pub alternatives: &'static [Alternative],
}

/// An install through a package manager, offered when that manager is on
/// the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alternative {
    /// The manager's command, to look for on PATH first (`npm`, `brew`).
    pub tool: &'static str,
    /// The line that installs, as it is typed.
    pub command: &'static str,
}

/// The official way to install `agent`.
pub fn installer(agent: AgentKind) -> Installer {
    match agent {
        AgentKind::Claude => Installer {
            docs_url: "https://code.claude.com/docs/en/setup",
            unix: "curl -fsSL https://claude.ai/install.sh | bash",
            windows: "irm https://claude.ai/install.ps1 | iex",
            alternatives: &[],
        },
        AgentKind::Codex => Installer {
            docs_url: "https://learn.chatgpt.com/docs/codex/cli",
            unix: "curl -fsSL https://chatgpt.com/codex/install.sh | sh",
            windows: "irm https://chatgpt.com/codex/install.ps1 | iex",
            alternatives: &[
                Alternative {
                    tool: "npm",
                    command: "npm install -g @openai/codex",
                },
                Alternative {
                    tool: "brew",
                    command: "brew install --cask codex",
                },
            ],
        },
    }
}

/// The names an agent's command goes by: the native binary, and on Windows
/// the shim npm writes (`.cmd`).
fn binaries(agent: AgentKind) -> &'static [&'static str] {
    match agent {
        AgentKind::Claude => &["claude.exe", "claude.cmd", "claude"],
        AgentKind::Codex => &["codex.exe", "codex.cmd", "codex"],
    }
}

/// The folders the official installer puts `agent` in. Looked in besides
/// PATH: on Windows the installer adds its folder to the user's PATH in the
/// registry, which a Hangar already running never sees.
fn install_dirs(agent: AgentKind) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if agent == AgentKind::Codex {
        if let Some(chosen) = std::env::var_os("CODEX_INSTALL_DIR").filter(|v| !v.is_empty()) {
            folders.push(PathBuf::from(chosen));
        }
        if cfg!(windows) {
            if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
                folders.push(
                    PathBuf::from(local)
                        .join("Programs")
                        .join("OpenAI")
                        .join("Codex")
                        .join("bin"),
                );
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        folders.push(home.join(".local").join("bin"));
    }
    folders
}

/// The first of `dirs` holding `agent`'s command, by any of its names.
fn find_in(dirs: &[PathBuf], agent: AgentKind) -> Option<PathBuf> {
    dirs.iter().find_map(|dir| {
        binaries(agent)
            .iter()
            .map(|bin| dir.join(bin))
            .find(|candidate| candidate.is_file())
    })
}

/// Where `agent`'s command is on PATH, as a pane started now finds it.
pub fn on_path(agent: AgentKind) -> Option<PathBuf> {
    // Without this the answer on macOS is "nowhere": a bundled app is handed
    // the bare system PATH, and no agent installs into it.
    crate::path_env::ensure();
    let paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    find_in(&paths, agent)
}

/// Where `agent`'s command is: on PATH, or else where its official installer
/// puts it.
pub fn find(agent: AgentKind) -> Option<PathBuf> {
    on_path(agent).or_else(|| find_in(&install_dirs(agent), agent))
}

/// Whether `agent` is installed, so the UI can say when a pane will not find
/// it.
pub fn installed(agent: AgentKind) -> bool {
    find(agent).is_some()
}

/// An agent found on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detected {
    pub path: PathBuf,
    /// Found on PATH, so a pane finds it by name. When not, it was installed
    /// since Hangar started (Windows' PATH is read once): it runs by its full
    /// path until the next start.
    pub on_path: bool,
    /// The first line `--version` printed (`codex-cli 0.131.0`,
    /// `2.1.3 (Claude Code)`), `None` when it did not answer in time or
    /// failed.
    pub version_line: Option<String>,
    /// The version number in it.
    pub version: Option<String>,
}

/// The longest `--version` may take: a first run can be slow (a Node shim
/// starting, an antivirus scan), but one that hangs says nothing more later.
const VERSION_TIMEOUT: Duration = Duration::from_secs(15);

/// Hides the console window Windows would open for every child of a GUI app.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Where `agent` is and which version, by asking it (`<agent> --version`).
/// `None` when it is not installed.
pub async fn detect(agent: AgentKind) -> Option<Detected> {
    let (path, on_path) = match crate::git::off_thread(move || Ok(on_path(agent)))
        .await
        .ok()
        .flatten()
    {
        Some(path) => (path, true),
        None => (find_in(&install_dirs(agent), agent)?, false),
    };
    let version_line = version_of(&path).await;
    Some(Detected {
        version: version_line.as_deref().and_then(parse_version),
        version_line,
        on_path,
        path,
    })
}

/// The first line `<path> --version` prints, if it answers in time.
async fn version_of(path: &Path) -> Option<String> {
    let mut command = std::process::Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("NO_COLOR", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut command = tokio::process::Command::from(command);
    command.kill_on_drop(true);
    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// The version number in a `--version` line: its last word that starts with
/// a digit (`codex-cli 0.131.0` → `0.131.0`, `2.1.3 (Claude Code)` →
/// `2.1.3`).
fn parse_version(line: &str) -> Option<String> {
    line.split_whitespace()
        .rev()
        .map(|word| word.trim_start_matches('v'))
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_installs_the_way_openai_documents_it() {
        let codex = installer(AgentKind::Codex);
        assert_eq!(
            codex.unix,
            "curl -fsSL https://chatgpt.com/codex/install.sh | sh"
        );
        assert_eq!(
            codex.windows,
            "irm https://chatgpt.com/codex/install.ps1 | iex"
        );
        let npm = codex.alternatives.iter().find(|way| way.tool == "npm");
        assert_eq!(
            npm.map(|way| way.command),
            Some("npm install -g @openai/codex")
        );
        // Claude Code's are the ones the setup sheet runs.
        assert_eq!(
            installer(AgentKind::Claude).unix,
            "curl -fsSL https://claude.ai/install.sh | bash"
        );
    }

    #[test]
    fn an_agent_is_found_by_any_of_its_names_in_the_first_folder_holding_it() {
        let root = crate::testdir::scratch("install-agent");
        let (empty, npm, native) = (root.join("empty"), root.join("npm"), root.join("native"));
        for dir in [&empty, &npm, &native] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(npm.join("codex.cmd"), "@echo off").unwrap();
        std::fs::write(native.join("codex"), "").unwrap();
        // A folder named like the command is not the command.
        std::fs::create_dir_all(empty.join("claude")).unwrap();

        let dirs = [empty.clone(), npm.clone(), native.clone()];
        assert_eq!(
            find_in(&dirs, AgentKind::Codex),
            Some(npm.join("codex.cmd"))
        );
        assert_eq!(
            find_in(&dirs[2..], AgentKind::Codex),
            Some(native.join("codex"))
        );
        assert_eq!(find_in(&dirs, AgentKind::Claude), None);
    }

    #[test]
    fn the_version_is_the_last_number_of_the_line() {
        assert_eq!(
            parse_version("codex-cli 0.131.0").as_deref(),
            Some("0.131.0")
        );
        assert_eq!(
            parse_version("2.1.3 (Claude Code)").as_deref(),
            Some("2.1.3")
        );
        assert_eq!(parse_version("codex v1.2.0").as_deref(), Some("1.2.0"));
        assert_eq!(parse_version("codex-cli"), None);
    }

    /// A stand-in `codex` that prints its version: what `detect` reads of the
    /// real one.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_version_is_asked_of_the_command_itself() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::testdir::scratch("install-version");
        let fake = dir.join("codex");
        std::fs::write(&fake, "#!/bin/sh\necho 'codex-cli 0.131.0'\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A file just written can be busy for a moment when another test's
        // child inherited it between fork and exec (ETXTBSY): ask again.
        let mut line = None;
        for _ in 0..5 {
            line = version_of(&fake).await;
            if line.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(line.as_deref(), Some("codex-cli 0.131.0"));

        let failing = dir.join("broken");
        std::fs::write(&failing, "#!/bin/sh\nexit 3\n").unwrap();
        std::fs::set_permissions(&failing, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(version_of(&failing).await, None);
        assert_eq!(version_of(&dir.join("missing")).await, None);
    }
}
