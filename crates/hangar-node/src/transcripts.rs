//! The conversations the node's agents keep, read where they are written:
//! the desktop's context gauge, session picker, session hunt and chat view of
//! a remote workspace ask these instead of its own disk. Each route reads
//! the agent `kind` asks for (`claude`, `codex`): Claude Code's transcripts,
//! Codex's rollouts (in the node's `~/.codex`, which its accounts share);
//! Claude Code's when none is named, as a desktop from before Codex asks.
//!
//! - `GET /node/chat?session=&cwd=&before=&limit=[&kind=]`: one page of a
//!   session's conversation (hangar-core's `chat_page`, `chat::codex::
//!   chat_page` for a Codex thread), `{"page": <ChatPage> | null}`, null
//!   while the session has no transcript yet;
//! - `GET /node/context?session=&cwd=[&kind=]`: where the session stands in
//!   its context window, `{"usage": {"usedTokens", "model"} | null}`
//!   (`context_usage`); for Codex with `contextWindow` too, the window's
//!   size its rollout writes down (`agent_context_usage`);
//! - `GET /node/sessions?cwd=[&labels=1][&kind=]`: the conversations kept for
//!   `cwd`, newest first, `{"sessions": [{"id", "label", "modifiedMs"}],
//!   "now"}`, `now` the node's clock as it listed (epoch ms), for the desktop
//!   to read the times on its own clock: the session hunt only adopts a
//!   transcript written in the last seconds, and two machines' clocks differ.
//!   Without `labels` the listing is the folders' alone
//!   (`list_session_ids_for`: no transcript opened, the labels empty), what
//!   the session hunt asks every few seconds; with it, each label is read
//!   (`list_sessions_for`), for the session picker.
//!
//! A `kind` that names no agent is refused: answering with another agent's
//! conversations would show them as this one's.
#![cfg(target_os = "linux")]

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use hangar_core::agent::AgentKind;
use serde::Deserialize;
use serde_json::json;

use crate::api::error;

/// The agent a route reads the conversations of: the one `asked` names,
/// Claude Code when it names none; why not, for anything else.
fn kind_of(asked: Option<&str>) -> Result<AgentKind, String> {
    match asked.filter(|asked| !asked.is_empty()) {
        None => Ok(AgentKind::Claude),
        Some(asked) => AgentKind::from_id(asked)
            .ok_or_else(|| format!("{asked:?} is not an agent: claude or codex")),
    }
}

#[derive(Deserialize)]
pub struct ChatQuery {
    #[serde(default)]
    session: String,
    #[serde(default)]
    cwd: String,
    before: Option<String>,
    limit: Option<usize>,
    kind: Option<String>,
}

pub async fn chat(Query(query): Query<ChatQuery>) -> Response {
    let kind = match kind_of(query.kind.as_deref()) {
        Ok(kind) => kind,
        Err(refusal) => return error(StatusCode::BAD_REQUEST, &refusal),
    };
    // The session id is checked by core, the cursor too: a bad one is the
    // caller's to fix.
    let (session, cwd, before, limit) = (query.session, query.cwd, query.before, query.limit);
    let page = match kind {
        AgentKind::Claude => hangar_core::chat::chat_page(session, cwd, before, limit).await,
        AgentKind::Codex => hangar_core::chat::codex::chat_page(session, cwd, before, limit).await,
    };
    match page {
        Ok(page) => Json(json!({ "page": page })).into_response(),
        Err(err) => error(StatusCode::BAD_REQUEST, &err),
    }
}

#[derive(Deserialize)]
pub struct ContextQuery {
    #[serde(default)]
    session: String,
    #[serde(default)]
    cwd: String,
    kind: Option<String>,
}

pub async fn context(Query(query): Query<ContextQuery>) -> Response {
    let kind = match kind_of(query.kind.as_deref()) {
        Ok(kind) => kind,
        Err(refusal) => return error(StatusCode::BAD_REQUEST, &refusal),
    };
    let (session, cwd) = (query.session, query.cwd);
    let usage = match kind {
        // Claude Code's as it always was: its transcript names no window.
        AgentKind::Claude => json!(hangar_core::context::context_usage(session, cwd).await),
        AgentKind::Codex => {
            json!(hangar_core::context::agent_context_usage(kind, session, cwd).await)
        }
    };
    Json(json!({ "usage": usage })).into_response()
}

#[derive(Deserialize)]
pub struct SessionsQuery {
    #[serde(default)]
    cwd: String,
    /// `1` or `true`: each session's label read from its transcript.
    #[serde(default)]
    labels: Option<String>,
    kind: Option<String>,
}

fn wants_labels(asked: Option<&str>) -> bool {
    matches!(asked, Some("1" | "true"))
}

pub async fn sessions(Query(query): Query<SessionsQuery>) -> Response {
    if query.cwd.is_empty() {
        return error(StatusCode::BAD_REQUEST, "sessions are listed for a cwd");
    }
    let kind = match kind_of(query.kind.as_deref()) {
        Ok(kind) => kind,
        Err(refusal) => return error(StatusCode::BAD_REQUEST, &refusal),
    };
    let labels = wants_labels(query.labels.as_deref());
    let cwd = query.cwd;
    let listed = tokio::task::spawn_blocking(move || {
        if labels {
            hangar_core::sessions::list_sessions_for(kind, cwd)
                .into_iter()
                .map(|session| {
                    json!({
                        "id": session.id,
                        "label": session.label,
                        "modifiedMs": session.modified_ms,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            hangar_core::sessions::list_session_ids_for(kind, cwd)
                .into_iter()
                .map(
                    |(id, modified_ms)| json!({ "id": id, "label": "", "modifiedMs": modified_ms }),
                )
                .collect()
        }
    })
    .await;
    match listed {
        Ok(sessions) => Json(json!({ "sessions": sessions, "now": now_ms() })).into_response(),
        Err(err) => error(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_asked_for_in_so_many_words() {
        assert!(wants_labels(Some("1")));
        assert!(wants_labels(Some("true")));
        assert!(!wants_labels(Some("0")));
        assert!(!wants_labels(None));
    }

    /// Claude Code's when no agent is named (a desktop from before Codex),
    /// either agent by its id, and anything else refused rather than read
    /// as one of them.
    #[test]
    fn the_kind_is_claude_unless_named() {
        assert_eq!(kind_of(None), Ok(AgentKind::Claude));
        assert_eq!(kind_of(Some("")), Ok(AgentKind::Claude));
        assert_eq!(kind_of(Some("claude")), Ok(AgentKind::Claude));
        assert_eq!(kind_of(Some("codex")), Ok(AgentKind::Codex));
        assert_eq!(
            kind_of(Some("gemini")),
            Err("\"gemini\" is not an agent: claude or codex".into())
        );
        assert!(kind_of(Some("shell")).is_err());
    }
}
