//! The models offered for each agent (docs/native/agents.md §6.6): the names
//! the panel's model fields suggest, and the model lines of a pane's menu.
//!
//! An agent Hangar reaches through a server of its own (`headless.app_server`:
//! Codex) is asked which models it offers: `model/list`, on a server started
//! for the question and stopped once it has answered, at most once a run. That
//! list moves with each of its releases and with the person's account, so it
//! is never written here. Any other agent is offered the aliases its command
//! line takes (Claude Code's `opus`, `sonnet`, `haiku`), which every version
//! of it knows.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use iced::Task;

use hangar_core::codex_app::generated::{Model as Offer, ModelListParams};
use hangar_core::codex_app::{Client, Endpoint, Options, app_server_command, requests};

use super::Msg;
use super::form::MODEL_SUGGESTIONS;
use crate::message::Message;
use crate::state::Model;
use crate::state::types::AgentKind;

/// The most pages of `model/list` read: a server that keeps handing out a
/// cursor is not followed for ever.
const MAX_PAGES: usize = 10;

/// The longest the question waits for each answer. A first start through
/// npm's shim can be slow; a menu waiting on it shows that it is asking.
const PATIENCE: Duration = Duration::from_secs(20);

/// What the agents that list their own models were asked, and answered.
#[derive(Debug, Default)]
pub struct Lists {
    /// Asked, no answer yet.
    asking: HashSet<AgentKind>,
    /// Their models, the default first. One that could not answer is not
    /// here, and is asked again the next time they are wanted.
    known: HashMap<AgentKind, Vec<String>>,
}

/// Whether `kind`'s models are asked of its own server.
fn listed_by_its_server(kind: AgentKind) -> bool {
    kind.info().caps.headless.app_server
}

/// The models offered for `kind`'s agent, its default first: `None` while its
/// server has not said (it is being asked, or it could not be).
pub fn offered(m: &Model, kind: AgentKind) -> Option<Vec<String>> {
    if !listed_by_its_server(kind) {
        return Some(Vec::from(MODEL_SUGGESTIONS.map(str::to_owned)));
    }
    m.live.orchestration_ui.models.known.get(&kind).cloned()
}

/// Whether `kind`'s server is being asked for its models.
pub fn asking(m: &Model, kind: AgentKind) -> bool {
    m.live.orchestration_ui.models.asking.contains(&kind)
}

/// Asks `kind`'s server for its models, unless its agent does not list its
/// own, they are known or being asked, or it is not on this computer. The
/// answer comes back as `Msg::Models`.
pub fn ask(m: &mut Model, kind: AgentKind) -> Task<Message> {
    let lists = &m.live.orchestration_ui.models;
    let wanted = listed_by_its_server(kind)
        && !lists.known.contains_key(&kind)
        && !lists.asking.contains(&kind)
        && crate::shell::setup::installed(m, kind);
    if !wanted {
        return Task::none();
    }
    m.live.orchestration_ui.models.asking.insert(kind);
    Task::perform(list_models(kind), move |answer| {
        Message::OrchestrationUi(Msg::Models(kind, answer))
    })
}

/// [`ask`] of every agent that lists its own models.
pub fn ask_every(m: &mut Model) -> Task<Message> {
    let tasks: Vec<Task<Message>> = AgentKind::ALL
        .into_iter()
        .map(|kind| ask(m, kind))
        .collect();
    Task::batch(tasks)
}

/// `kind`'s server answered: its models are kept, or, if it could not say,
/// it is asked again the next time they are wanted.
pub fn answered(m: &mut Model, kind: AgentKind, answer: Result<Vec<String>, String>) {
    let lists = &mut m.live.orchestration_ui.models;
    lists.asking.remove(&kind);
    match answer {
        Ok(names) if !names.is_empty() => {
            lists.known.insert(kind, names);
        }
        Ok(_) => {}
        Err(error) => eprintln!(
            "orchestration: {} did not list its models: {error}",
            kind.info().label
        ),
    }
}

/// `model/list`, every page, on a server of `kind`'s own started for the
/// question (`<agent> app-server --listen stdio://`) and stopped after it.
async fn list_models(kind: AgentKind) -> Result<Vec<String>, String> {
    let program = hangar_core::agent::detect::find(kind)
        .ok_or_else(|| format!("{} was not found", kind.info().label))?;
    let command = app_server_command(&program);
    // No console window flashes up for it on Windows (CREATE_NO_WINDOW).
    #[cfg(windows)]
    let command = {
        let mut command = command;
        command.creation_flags(0x0800_0000);
        command
    };
    let options = Options {
        request_timeout: PATIENCE,
        ..Options::default()
    };
    let (client, _events) = Client::connect(Endpoint::Stdio(command), options)
        .await
        .map_err(|error| error.to_string())?;
    let mut offers = Vec::new();
    let mut cursor = None;
    let mut answer = Ok(());
    for _ in 0..MAX_PAGES {
        let params = ModelListParams {
            cursor: cursor.take(),
            ..ModelListParams::default()
        };
        match client.request::<requests::ModelList>(params).await {
            Ok(page) => {
                offers.extend(page.data);
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
            Err(error) => {
                answer = Err(error.to_string());
                break;
            }
        }
    }
    client.shutdown().await;
    answer.map(|()| names_of(&offers))
}

/// The names a launch takes (`model`, else `id`) of the models offered: the
/// ones Codex's own picker hides left out, its default first, each once.
fn names_of(offers: &[Offer]) -> Vec<String> {
    let mut shown: Vec<&Offer> = offers.iter().filter(|offer| !offer.hidden).collect();
    // Stable: the others keep the order the server gave.
    shown.sort_by_key(|offer| !offer.is_default);
    let mut names: Vec<String> = Vec::new();
    for offer in shown {
        let name = if offer.model.trim().is_empty() {
            offer.id.trim()
        } else {
            offer.model.trim()
        };
        if !name.is_empty() && !names.iter().any(|known| known == name) {
            names.push(name.to_owned());
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use hangar_core::codex_app::generated::ModelListResponse;

    /// A `model/list` answer, written from the protocol schema (its required
    /// properties, as the pinned release's schema sample has them).
    fn page(models: serde_json::Value) -> Vec<Offer> {
        let answer: ModelListResponse =
            serde_json::from_value(json!({ "data": models })).expect("a model/list answer");
        answer.data
    }

    fn offer(id: &str, model: &str, hidden: bool, default: bool) -> serde_json::Value {
        json!({
            "id": id,
            "model": model,
            "displayName": id,
            "description": "",
            "hidden": hidden,
            "isDefault": default,
            "defaultReasoningEffort": "medium",
            "supportedReasoningEfforts": []
        })
    }

    #[test]
    fn codex_s_models_are_named_as_a_launch_takes_them_its_default_first() {
        let offers = page(json!([
            offer("gpt-a", "gpt-a", false, false),
            offer("gpt-b-preset", "gpt-b", false, true),
            offer("gpt-hidden", "gpt-hidden", true, false),
            offer("gpt-c", "", false, false),
            offer("gpt-a-again", "gpt-a", false, false),
        ]));
        assert_eq!(names_of(&offers), ["gpt-b", "gpt-a", "gpt-c"]);
        assert!(names_of(&[]).is_empty());
    }

    #[test]
    fn claude_code_is_offered_its_aliases_and_codex_what_its_server_said() {
        let mut m = Model::default();
        assert_eq!(
            offered(&m, AgentKind::Claude),
            Some(vec!["opus".into(), "sonnet".into(), "haiku".into()])
        );
        // Codex is asked; not installed, it is not.
        assert_eq!(offered(&m, AgentKind::Codex), None);
        drop(ask(&mut m, AgentKind::Codex));
        assert!(!asking(&m, AgentKind::Codex));
        // Claude Code is never asked: it lists nothing of its own.
        m.ui.claude_installed = true;
        drop(ask(&mut m, AgentKind::Claude));
        assert!(!asking(&m, AgentKind::Claude));

        // Installed, it is asked once, whatever opens meanwhile.
        m.ui.codex_installed = true;
        drop(ask(&mut m, AgentKind::Codex));
        assert!(asking(&m, AgentKind::Codex));
        drop(ask_every(&mut m));
        assert!(asking(&m, AgentKind::Codex));

        // A failure is forgotten, to be asked again; an answer is kept.
        answered(&mut m, AgentKind::Codex, Err("no login".into()));
        assert!(!asking(&m, AgentKind::Codex));
        assert_eq!(offered(&m, AgentKind::Codex), None);
        answered(&mut m, AgentKind::Codex, Ok(vec!["gpt-b".into()]));
        assert_eq!(offered(&m, AgentKind::Codex), Some(vec!["gpt-b".into()]));
        drop(ask(&mut m, AgentKind::Codex));
        assert!(!asking(&m, AgentKind::Codex));
    }
}
