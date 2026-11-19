//! Which agents are on this machine: the one question features ask about
//! installs, whatever the kind.
//!
//! Hangar is usable with either agent alone (§6.3, D17): the setup sheet
//! stands in the way only when neither is found ([`any_installed`]), and the
//! pickers grey out the one that is missing.
//!
//! The finding itself is `install::agent`'s — the command on PATH, or where
//! its official installer puts it, and its version by asking it. This is the
//! face features see, so that they ask by kind and never call one agent's
//! own check.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::AgentKind;
use crate::install::agent::{self as install, Detected};

/// What was found of one agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    pub kind: AgentKind,
    /// Where it is and which version; `None` when it is not installed.
    pub found: Option<Detected>,
}

impl Detection {
    pub fn installed(&self) -> bool {
        self.found.is_some()
    }

    /// Where its command is.
    pub fn path(&self) -> Option<&Path> {
        self.found.as_ref().map(|found| found.path.as_path())
    }

    /// Its version number, when it said one.
    pub fn version(&self) -> Option<&str> {
        self.found.as_ref()?.version.as_deref()
    }
}

/// Whether `kind` is installed, so the UI can say when a pane will not find
/// it. Quick: nothing is run.
pub fn installed(kind: AgentKind) -> bool {
    install::installed(kind)
}

/// Where `kind`'s command is: on PATH, or where its official installer puts
/// it. Quick: nothing is run.
pub fn find(kind: AgentKind) -> Option<PathBuf> {
    install::find(kind)
}

/// Whether Hangar has an agent to run at all: either one is enough.
pub fn any_installed() -> bool {
    AgentKind::ALL.into_iter().any(installed)
}

/// Where `kind` is and which version, by asking it (`--version`, which a
/// first run of a Node shim can make slow).
pub async fn detect(kind: AgentKind) -> Detection {
    Detection {
        kind,
        found: install::detect(kind).await,
    }
}

/// [`detect`] for every kind, in the pickers' order, one after the other. A
/// view that wants each answer as soon as it comes asks [`detect`] per kind.
pub async fn detect_all() -> Vec<Detection> {
    let mut all = Vec::with_capacity(AgentKind::ALL.len());
    for kind in AgentKind::ALL {
        all.push(detect(kind).await);
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_detection_says_what_was_found() {
        let found = Detection {
            kind: AgentKind::Codex,
            found: Some(Detected {
                path: PathBuf::from("/usr/local/bin/codex"),
                on_path: true,
                version_line: Some("codex-cli 0.170.0".into()),
                version: Some("0.170.0".into()),
            }),
        };
        assert!(found.installed());
        assert_eq!(found.path(), Some(Path::new("/usr/local/bin/codex")));
        assert_eq!(found.version(), Some("0.170.0"));
        assert_eq!(
            serde_json::to_value(&found).unwrap(),
            serde_json::json!({
                "kind": "codex",
                "found": {
                    "path": "/usr/local/bin/codex",
                    "onPath": true,
                    "versionLine": "codex-cli 0.170.0",
                    "version": "0.170.0",
                },
            })
        );

        let missing = Detection {
            kind: AgentKind::Claude,
            found: None,
        };
        assert!(!missing.installed());
        assert_eq!(missing.path(), None);
        assert_eq!(missing.version(), None);
    }
}
