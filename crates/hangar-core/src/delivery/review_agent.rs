//! Which agent reads a delivery (docs/native/agents.md §4.4, D11).
//!
//! A workspace's « Reviewer » setting names Claude Code or Codex — « same
//! agent as the orchestrator » is resolved by the engine, which passes the
//! setting as a [`ReviewerChoice`] (T13/T16). The agent it names reviews when
//! it is installed; else whichever is, Claude Code first. Hangar on a machine
//! with Codex alone still gets its deliveries reviewed (§6.9).
//!
//! This is the one place of the review that tells the agent kinds apart, as
//! `agent::launch` is for panes (D3): each kind's reviewer is its own driver,
//! [`Claude`] in delivery.rs and [`Codex`] in review_codex.rs.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use super::review_codex::Codex;
use super::{claude_program, Claude, Reviewer, Verdict};
use crate::agent::{self, AgentKind};

/// The workspace's reviewer setting, as the engine passes it to
/// [`delivery_run_with`](super::delivery_run_with).
///
/// One per delivery run: the deliveries checked together with it (the merge
/// queue, batch.rs) are read by the reviewer of the run that checks them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewerChoice {
    /// The agent the setting names, « same as the orchestrator » resolved;
    /// `None`: Claude Code when it is installed, else Codex.
    #[serde(default)]
    pub agent: Option<AgentKind>,
    /// The model a Codex review runs on; `None`: Codex's own default. A
    /// Claude review runs on `Delivery::reviewer_model`.
    #[serde(default)]
    pub codex_model: Option<String>,
    /// The workspace's Codex account, which a Codex review spends; `None`:
    /// the default one. A Claude review spends `Delivery::claude_account`.
    #[serde(default)]
    pub codex_account: Option<String>,
}

const NOBODY: &str =
    "neither Claude Code (`claude`) nor Codex (`codex`) was found, so nobody could review the delivery";

/// The kind that reviews: `chosen` when it is installed and runs one prompt
/// headless (`caps.headless.one_shot`), else the first such kind installed,
/// in the pickers' order; `None` when there is none.
pub(super) fn reviewing_kind(
    chosen: Option<AgentKind>,
    installed: impl Fn(AgentKind) -> bool,
) -> Option<AgentKind> {
    chosen
        .into_iter()
        .chain(AgentKind::ALL)
        .find(|kind| agent::info(*kind).caps.headless.one_shot && installed(*kind))
}

/// Where `kind`'s command is, as its reviewer runs it: Claude Code where its
/// reviewer has always looked (`claude_program`), Codex where a pane finds
/// it (`agent::detect`).
fn program(kind: AgentKind) -> Option<PathBuf> {
    match kind {
        AgentKind::Claude => claude_program(),
        AgentKind::Codex => agent::detect::find(kind),
    }
}

/// The reviewer of `choice` on this machine.
pub(super) fn reviewer(choice: &ReviewerChoice, limit: Duration) -> Box<dyn Reviewer> {
    pick(choice, limit, program)
}

/// [`reviewer`], each kind's command looked for by `find`.
pub(super) fn pick(
    choice: &ReviewerChoice,
    limit: Duration,
    find: impl Fn(AgentKind) -> Option<PathBuf>,
) -> Box<dyn Reviewer> {
    let Some(kind) = reviewing_kind(choice.agent, |kind| find(kind).is_some()) else {
        return Box::new(Nobody);
    };
    match (kind, find(kind)) {
        // It finds its command itself, where it was just seen.
        (AgentKind::Claude, _) => Box::new(Claude { limit }),
        (AgentKind::Codex, Some(program)) => Box::new(Codex {
            program,
            limit,
            model: choice.codex_model.clone(),
            account: choice.codex_account.clone(),
        }),
        // Gone between two looks.
        (AgentKind::Codex, None) => Box::new(Nobody),
    }
}

/// No agent to review with: the delivery waits, and says why (§3.5).
struct Nobody;

impl Reviewer for Nobody {
    fn review(&self, _: &Path, _: &str, _: &str, _: Option<&str>) -> Result<Verdict, String> {
        Err(NOBODY.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentKind::{Claude, Codex};

    #[test]
    fn the_setting_names_the_reviewer_and_an_installed_one_stands_in() {
        let both = |_: AgentKind| true;
        let only = |there: AgentKind| move |kind: AgentKind| kind == there;
        let none = |_: AgentKind| false;

        // No setting: Claude Code when it is there, else Codex.
        assert_eq!(reviewing_kind(None, both), Some(Claude));
        assert_eq!(reviewing_kind(None, only(Claude)), Some(Claude));
        assert_eq!(reviewing_kind(None, only(Codex)), Some(Codex));
        // The agent the setting names.
        assert_eq!(reviewing_kind(Some(Codex), both), Some(Codex));
        assert_eq!(reviewing_kind(Some(Claude), both), Some(Claude));
        // Named but not installed: the one that is.
        assert_eq!(reviewing_kind(Some(Codex), only(Claude)), Some(Claude));
        assert_eq!(reviewing_kind(Some(Claude), only(Codex)), Some(Codex));
        // Neither.
        assert_eq!(reviewing_kind(None, none), None);
        assert_eq!(reviewing_kind(Some(Codex), none), None);
    }

    #[test]
    fn each_agent_reviews_with_its_own_tools() {
        let at = |there: AgentKind| {
            move |kind: AgentKind| (kind == there).then(|| PathBuf::from("/opt/bin/agent"))
        };
        let limit = Duration::from_secs(60);
        let choice = ReviewerChoice::default();
        assert_eq!(
            pick(&choice, limit, at(Claude)).reading(),
            super::super::CLAUDE_READING
        );
        assert_eq!(
            pick(&choice, limit, at(Codex)).reading(),
            super::super::review_codex::READING
        );
        let codex = ReviewerChoice {
            agent: Some(Codex),
            ..ReviewerChoice::default()
        };
        assert_eq!(
            pick(&codex, limit, |_| Some(PathBuf::from("/opt/bin/agent"))).reading(),
            super::super::review_codex::READING
        );
    }

    #[test]
    fn without_an_agent_the_review_says_so() {
        let reviewer = pick(&ReviewerChoice::default(), Duration::from_secs(60), |_| {
            None
        });
        let err = reviewer
            .review(Path::new("."), "sonnet", "Review this.", None)
            .unwrap_err();
        assert!(err.contains("neither Claude Code"), "{err}");
        assert!(err.contains("Codex (`codex`)"), "{err}");
    }

    /// What the engine sends once the workspace has the setting (T13/T16).
    #[test]
    fn the_setting_reads_as_the_engine_writes_it() {
        let choice: ReviewerChoice = serde_json::from_value(serde_json::json!({
            "agent": "codex",
            "codexModel": "gpt-5.5",
            "codexAccount": "work",
        }))
        .unwrap();
        assert_eq!(
            choice,
            ReviewerChoice {
                agent: Some(Codex),
                codex_model: Some("gpt-5.5".into()),
                codex_account: Some("work".into()),
            }
        );
        let unset: ReviewerChoice = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(unset, ReviewerChoice::default());
        assert_eq!(unset.agent, None);
    }
}
