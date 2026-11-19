//! The conversations of a remote workspace's agents, read on its node
//! (hangar-node's transcripts.rs): what hangar-core's `agent_context_usage`,
//! `list_session_ids_for`, `list_sessions_for` and the chat pages read from
//! this machine's disk for a local pane, for the agent `kind` named —
//! Claude Code's transcripts, Codex's rollouts. Reached as
//! `nodes::remote::{context, sessions, chat}` (this file is a module of
//! remote.rs).
//!
//! The kind goes as `&kind=` for any agent but Claude Code, which the node
//! reads when none is named: what is asked about Claude Code is the request
//! it always was.
//!
//! Each call takes the node by id and asks the [`directory`] for its client
//! at the moment it runs, as the panes' streams do: the tunnel's port
//! changes when the node reconnects.

use hangar_core::agent::AgentKind;
use hangar_core::context::ContextUsage;
use hangar_core::sessions::AgentSession;
use serde_json::Value;

use super::super::model::NodeClient;
use super::super::wire;
use super::directory;

/// The client of node `node`, or why there is none.
pub fn client_of(node: &str) -> Result<NodeClient, String> {
    directory(node).ok_or_else(|| format!("node {node} is not connected"))
}

/// `path` asking for agent `kind`'s conversations: as it is for Claude Code,
/// the node's default, with `&kind=` for any other.
fn of_kind(mut path: String, kind: AgentKind) -> String {
    if kind != AgentKind::Claude {
        path.push_str("&kind=");
        path.push_str(kind.id());
    }
    path
}

/// `GET /node/context`: where session `session` of a `kind` pane in `cwd`
/// stands in its context window. `Ok(None)`: no usage written yet, as
/// locally.
pub async fn context(
    client: &NodeClient,
    kind: AgentKind,
    session: &str,
    cwd: &str,
) -> Result<Option<ContextUsage>, String> {
    let path = of_kind(
        format!(
            "/node/context?session={}&cwd={}",
            wire::encode(session),
            wire::encode(cwd)
        ),
        kind,
    );
    let answer = wire::request(client, "GET", &path, None).await?.json()?;
    Ok(usage_of(answer.get("usage")))
}

fn usage_of(usage: Option<&Value>) -> Option<ContextUsage> {
    let usage = usage.filter(|usage| !usage.is_null())?;
    Some(ContextUsage {
        used_tokens: usage.get("usedTokens")?.as_u64()?,
        model: usage
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// `GET /node/sessions`: the conversations agent `kind` kept for `cwd` on
/// the node, newest first, their times on this machine's clock. With
/// `labels`, each label read from its transcript (the session picker);
/// without, the folders' listing alone, labels empty (the session hunt).
pub async fn sessions(
    client: &NodeClient,
    kind: AgentKind,
    cwd: &str,
    labels: bool,
) -> Result<Vec<AgentSession>, String> {
    let mut path = format!("/node/sessions?cwd={}", wire::encode(cwd));
    if labels {
        path.push_str("&labels=1");
    }
    let path = of_kind(path, kind);
    let answer = wire::request(client, "GET", &path, None).await?.json()?;
    Ok(sessions_of(&answer, now_ms()))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The sessions of an answer, each time moved from the node's clock (`now`,
/// as it listed) to ours (`here`, as it came back): the hunt adopts only a
/// transcript touched in the last seconds, which two clocks apart by more
/// than that would never see. An answer without `now` is taken as it is.
fn sessions_of(answer: &Value, here: u64) -> Vec<AgentSession> {
    let shift = answer
        .get("now")
        .and_then(Value::as_u64)
        .map_or(0, |there| i128::from(here) - i128::from(there));
    answer
        .get("sessions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|session| {
            let id = session.get("id")?.as_str()?.to_owned();
            let modified = i128::from(session.get("modifiedMs")?.as_u64()?);
            Some(AgentSession {
                id,
                label: session
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                modified_ms: u64::try_from((modified + shift).max(0)).unwrap_or(0),
            })
        })
        .collect()
}

/// `GET /node/chat`: one page of the conversation of agent `kind`'s session
/// `session`, as hangar-core's chat pages write it for the mobile and the
/// chat view (`{items, older, cursor, question, openCall}`): the newest
/// without `before`, else the one before that cursor. `Ok(None)` while the
/// session has no transcript yet.
///
/// The chat view reads a remote workspace's conversation through this
/// (`mobile::desk::chat`). The page is the JSON core serialises: `ChatPage` is
/// serialise-only in core, so it is handed over as it came.
#[allow(
    dead_code,
    reason = "for the chat view's conversation, which calls it next"
)]
pub async fn chat(
    client: &NodeClient,
    kind: AgentKind,
    session: &str,
    cwd: &str,
    before: Option<&str>,
    limit: Option<usize>,
) -> Result<Option<Value>, String> {
    let mut path = format!(
        "/node/chat?session={}&cwd={}",
        wire::encode(session),
        wire::encode(cwd)
    );
    if let Some(before) = before {
        path.push_str(&format!("&before={}", wire::encode(before)));
    }
    if let Some(limit) = limit {
        path.push_str(&format!("&limit={limit}"));
    }
    let path = of_kind(path, kind);
    let mut answer = wire::request(client, "GET", &path, None).await?.json()?;
    Ok(answer
        .get_mut("page")
        .map(Value::take)
        .filter(|page| !page.is_null()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::nodes::testing::{answer, listen, read_request};

    #[test]
    fn a_usage_reads_back_and_null_is_none() {
        assert_eq!(
            usage_of(Some(
                &json!({ "usedTokens": 1200, "model": "claude-opus-5-5" })
            )),
            Some(ContextUsage {
                used_tokens: 1200,
                model: Some("claude-opus-5-5".into()),
            })
        );
        assert_eq!(usage_of(Some(&Value::Null)), None);
        assert_eq!(usage_of(None), None);
    }

    #[test]
    fn the_times_move_to_this_clock() {
        // The node is 60 s behind: a transcript it touched 2 s ago is 2 s
        // old here too.
        let answer = json!({
            "sessions": [{ "id": "s1", "label": "fix", "modifiedMs": 98_000 }],
            "now": 100_000,
        });
        let sessions = sessions_of(&answer, 160_000);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "s1");
        assert_eq!(sessions[0].label, "fix");
        assert_eq!(sessions[0].modified_ms, 158_000);
        // Without the node's clock, as they came.
        let bare = json!({ "sessions": [{ "id": "s1", "modifiedMs": 5 }] });
        assert_eq!(sessions_of(&bare, 160_000)[0].modified_ms, 5);
    }

    #[tokio::test]
    async fn the_routes_are_asked_with_the_session_and_the_cwd() {
        let (listener, client) = listen().await;
        let node = tokio::spawn(async move {
            let mut lines = Vec::new();
            let bodies = [
                r#"{"usage":{"usedTokens":42,"model":null}}"#.to_owned(),
                r#"{"sessions":[{"id":"s2","label":"","modifiedMs":7}]}"#.to_owned(),
                r#"{"page":{"items":[],"older":false,"cursor":null}}"#.to_owned(),
                r#"{"page":null}"#.to_owned(),
                r#"{"error":"\"x\" is not a session id."}"#.to_owned(),
            ];
            for (at, body) in bodies.iter().enumerate() {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_request(&mut stream).await;
                assert!(request.head.contains("Authorization: Bearer test-token"));
                lines.push(request.line().to_owned());
                answer(&mut stream, if at == 4 { 400 } else { 200 }, body).await;
            }
            lines
        });
        let usage = context(&client, AgentKind::Claude, "s1", "/home/hangar/my demo").await;
        assert_eq!(
            usage,
            Ok(Some(ContextUsage {
                used_tokens: 42,
                model: None
            }))
        );
        let listed = sessions(&client, AgentKind::Claude, "/home/hangar/demo", true)
            .await
            .unwrap();
        assert_eq!(listed[0].id, "s2");
        let page = chat(&client, AgentKind::Claude, "s1", "/w", Some("12"), Some(30)).await;
        assert_eq!(page.unwrap().unwrap()["older"], false);
        assert_eq!(
            chat(&client, AgentKind::Claude, "s1", "/w", None, None).await,
            Ok(None)
        );
        assert_eq!(
            chat(&client, AgentKind::Claude, "x", "/w", None, None).await,
            Err("\"x\" is not a session id.".into())
        );
        let lines = node.await.unwrap();
        assert_eq!(
            lines,
            [
                "GET /node/context?session=s1&cwd=%2Fhome%2Fhangar%2Fmy%20demo HTTP/1.1",
                "GET /node/sessions?cwd=%2Fhome%2Fhangar%2Fdemo&labels=1 HTTP/1.1",
                "GET /node/chat?session=s1&cwd=%2Fw&before=12&limit=30 HTTP/1.1",
                "GET /node/chat?session=s1&cwd=%2Fw HTTP/1.1",
                "GET /node/chat?session=x&cwd=%2Fw HTTP/1.1",
            ]
        );
    }

    /// A Codex pane's conversations are asked by its kind, after every
    /// other parameter; a Claude Code pane's carry none (the node's default),
    /// as they always did.
    #[tokio::test]
    async fn a_codex_pane_s_routes_name_its_kind() {
        let (listener, client) = listen().await;
        let node = tokio::spawn(async move {
            let mut lines = Vec::new();
            let bodies = [
                r#"{"usage":{"usedTokens":9000,"model":"gpt-5.1-codex","contextWindow":272000}}"#,
                r#"{"sessions":[{"id":"0199a5c4","label":"","modifiedMs":7}],"now":7}"#,
                r#"{"sessions":[],"now":7}"#,
                r#"{"page":{"items":[],"older":false,"cursor":null}}"#,
            ];
            for body in bodies {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_request(&mut stream).await;
                lines.push(request.line().to_owned());
                answer(&mut stream, 200, body).await;
            }
            lines
        });
        let usage = context(&client, AgentKind::Codex, "0199a5c4", "/w").await;
        assert_eq!(usage.unwrap().unwrap().used_tokens, 9000);
        let listed = sessions(&client, AgentKind::Codex, "/w", false).await;
        assert_eq!(listed.unwrap()[0].id, "0199a5c4");
        let labelled = sessions(&client, AgentKind::Codex, "/w", true).await;
        assert!(labelled.unwrap().is_empty());
        let page = chat(
            &client,
            AgentKind::Codex,
            "0199a5c4",
            "/w",
            Some("3"),
            Some(30),
        )
        .await;
        assert_eq!(page.unwrap().unwrap()["older"], false);
        assert_eq!(
            node.await.unwrap(),
            [
                "GET /node/context?session=0199a5c4&cwd=%2Fw&kind=codex HTTP/1.1",
                "GET /node/sessions?cwd=%2Fw&kind=codex HTTP/1.1",
                "GET /node/sessions?cwd=%2Fw&labels=1&kind=codex HTTP/1.1",
                "GET /node/chat?session=0199a5c4&cwd=%2Fw&before=3&limit=30&kind=codex HTTP/1.1",
            ]
        );
    }
}
