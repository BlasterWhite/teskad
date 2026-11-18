//! Which agents are on this machine: the one question features ask about
//! installs, whatever the kind.
//!
//! Hangar is usable with either agent alone (§6.3, D17): the setup sheet
//! stands in the way only when neither is found ([`any_installed`]), and the
//! pickers grey out the one that is missing.
//!
//! Claude Code's answer is the check Hangar has always made,
//! `sessions::claude_installed`: its command on PATH, after the PATH repair.
//!
//! Codex's detection is the Codex core task's (c48c7086, `install::agent`:
//! PATH, the folders its official installers use, `codex --version`), which
//! had not landed on main when this was written. Until it does, [`codex`] is
//! the one place that knows how Codex is found, and it does what Claude
//! Code's check does, by Codex's names. When it lands, that function calls
//! it, and [`Detection`] gains the version it reads.

use std::path::PathBuf;

use serde::Serialize;

use super::{info, AgentKind};

/// What was found of one agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    pub kind: AgentKind,
    /// Where its command is; `None` when it is not installed.
    pub path: Option<PathBuf>,
}

impl Detection {
    pub fn installed(&self) -> bool {
        self.path.is_some()
    }
}

/// Whether `kind` is installed, so the UI can say when a pane will not find
/// it.
pub fn installed(kind: AgentKind) -> bool {
    match kind {
        AgentKind::Claude => crate::sessions::claude_installed(),
        AgentKind::Codex => codex().is_some(),
    }
}

/// Where `kind`'s command is, as a pane started now finds it.
pub fn find(kind: AgentKind) -> Option<PathBuf> {
    match kind {
        AgentKind::Claude => on_path(AgentKind::Claude),
        AgentKind::Codex => codex(),
    }
}

/// What was found of `kind`.
pub fn detect(kind: AgentKind) -> Detection {
    Detection {
        kind,
        path: find(kind),
    }
}

/// What was found of every kind, in the pickers' order.
pub fn detect_all() -> Vec<Detection> {
    AgentKind::ALL.into_iter().map(detect).collect()
}

/// Whether Hangar has an agent to run at all: either one is enough.
pub fn any_installed() -> bool {
    AgentKind::ALL.into_iter().any(installed)
}

/// Codex's command. The seam where the Codex core task's detection
/// (`install::agent::find`) takes over; until then, PATH, as for Claude Code.
fn codex() -> Option<PathBuf> {
    on_path(AgentKind::Codex)
}

/// `kind`'s command on PATH, by any of its names.
fn on_path(kind: AgentKind) -> Option<PathBuf> {
    // Without this the answer on macOS is "nowhere": a bundled app is handed
    // the bare system PATH, and no agent installs into it.
    crate::path_env::ensure();
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    find_in(&dirs, info(kind).binaries)
}

/// The first of `dirs`, in order, holding a file (not a folder) by one of
/// `names`: the folder PATH lists first wins, as it does for the shell.
fn find_in(dirs: &[PathBuf], names: &[&str]) -> Option<PathBuf> {
    dirs.iter().find_map(|dir| {
        names
            .iter()
            .map(|name| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    #[test]
    fn codex_is_found_by_any_of_its_names() {
        let dir = tempfile::tempdir().unwrap();
        let npm = dir.path().join("npm");
        touch(&npm.join("codex.cmd"));
        let bin = dir.path().join("bin");
        touch(&bin.join("codex"));
        let names = info(AgentKind::Codex).binaries;

        assert_eq!(
            find_in(std::slice::from_ref(&npm), names),
            Some(npm.join("codex.cmd"))
        );
        assert_eq!(
            find_in(std::slice::from_ref(&bin), names),
            Some(bin.join("codex"))
        );
    }

    #[test]
    fn the_folder_path_lists_first_wins() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        touch(&first.join("claude"));
        touch(&second.join("claude"));
        let names = info(AgentKind::Claude).binaries;

        assert_eq!(
            find_in(&[first.clone(), second.clone()], names),
            Some(first.join("claude"))
        );
        assert_eq!(
            find_in(&[second.clone(), first], names),
            Some(second.join("claude"))
        );
    }

    #[test]
    fn a_folder_by_the_name_or_the_other_agent_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(bin.join("codex")).unwrap();
        touch(&bin.join("claude"));
        let missing = dir.path().join("missing");

        assert_eq!(
            find_in(&[missing, bin], info(AgentKind::Codex).binaries),
            None
        );
    }

    #[test]
    fn a_detection_is_installed_when_it_has_a_path() {
        let found = Detection {
            kind: AgentKind::Codex,
            path: Some(PathBuf::from("/usr/local/bin/codex")),
        };
        let missing = Detection {
            kind: AgentKind::Claude,
            path: None,
        };
        assert!(found.installed());
        assert!(!missing.installed());
        assert_eq!(
            serde_json::to_value(&found).unwrap(),
            serde_json::json!({ "kind": "codex", "path": "/usr/local/bin/codex" })
        );
    }
}
