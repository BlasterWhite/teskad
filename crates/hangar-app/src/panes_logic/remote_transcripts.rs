//! The conversations of an agent pane of a remote workspace, which are on
//! its node: the session hunt's listing and the context gauge's reading, of
//! the pane's agent (Claude Code's transcripts, Codex's rollouts), asked of
//! the node (`nodes::remote::{sessions, context}`) where a local pane reads
//! this machine's disk. Same cadence, same answers: a hunt round that fails
//! finds nothing, as a local listing that fails does.
//!
//! The panes' controller follows the sessions of local panes only (it lists
//! the known ones before the launch from this machine's disk): a remote
//! pane's hunt starts here instead, when its agent's command is typed
//! (`Launched`), with the sessions its node already holds as the known ones.

use std::collections::HashSet;

use hangar_core::agent::AgentKind;
use hangar_core::context::AgentContextUsage;
use hangar_core::sessions::AgentSession;

use crate::nodes::remote;

/// A round of the session hunt for a `kind` pane in `cwd` on node `node`:
/// the folders' listing of that agent's conversations, no label read. `None`
/// when the node could not say.
pub async fn hunt_listing(node: String, kind: AgentKind, cwd: String) -> Option<Vec<AgentSession>> {
    let client = remote::client_of(&node).ok()?;
    remote::sessions(&client, kind, &cwd, false).await.ok()
}

/// The sessions agent `kind` already holds for `cwd` on node `node`, so that
/// a brand new one stands out to the hunt. Empty when it cannot be listed,
/// as locally.
pub async fn known_session_ids(node: String, kind: AgentKind, cwd: String) -> HashSet<String> {
    hunt_listing(node, kind, cwd)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|session| session.id)
        .collect()
}

/// The context of agent `kind`'s session `session` in `cwd`, read on node
/// `node`, as a local pane's is read (`context::agent_context_usage`). The
/// node's answer carries no window (`nodes::remote::context`): the gauge
/// finds it by the model, and shows nothing for a model it does not know.
/// `Err` when the node did not answer: the gauge keeps its last reading
/// rather than drop to nothing while the tunnel comes back.
pub async fn context_usage(
    node: String,
    kind: AgentKind,
    session: String,
    cwd: String,
) -> Result<Option<AgentContextUsage>, String> {
    let client = remote::client_of(&node)?;
    let usage = remote::context(&client, kind, &session, &cwd).await?;
    Ok(usage.map(|usage| AgentContextUsage {
        usage,
        context_window: None,
    }))
}
