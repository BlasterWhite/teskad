//! Answering through the ask a pane's side channel holds open, rather than
//! through keys typed into its agent's dialog.
//!
//! When the agent is about to ask the person something, its side channel
//! reports the ask with an id and waits in core for its answer (core's
//! bridge.rs, "Asks the mod holds open"), while the dialog stays up in the
//! terminal: Claude Code's hangar-bridge mod, or Hangar's own client of a
//! Codex pane's server, which holds each of Codex's requests — approvals,
//! questions — the same way (hangar-core's `agent/codex_attach.rs`). An
//! answer given here reaches the agent as the decision the person made —
//! the labels they picked, their own words, a yes or a no — with no screen
//! read back and no key that could land elsewhere. Only the person answers:
//! these are the chat's and the phone's buttons, never an agent's tool.
//!
//! Keys stay the way for what no side channel holds: a pane on a node (its
//! asks are held there), an agent that runs without its side channel, an ask
//! reported by an older mod. And the side channel must take the answer
//! within `TAKE_MS`: one that is not there to take it (its hook skipped, the
//! agent gone, the connection to Codex's server lost) has it taken back, and
//! the keys are typed after all.

use serde_json::{Map, Value, json};

use hangar_core::bridge::{self, HeldAsk};
use hangar_core::chat::Question;

use crate::control::port::Port;
use crate::nodes as away;
use crate::state::{Pane, Workspace};

use super::answer::Choice;

/// How long the mod gets to take an answer.
const TAKE_MS: u64 = 3_000;
const TAKE_POLL_MS: u64 = 50;

pub const ASK_QUESTION: &str = "AskUserQuestion";

/// The asks the pane's mod holds open here, newest first; none for a pane on
/// a node.
fn held(ws: &Workspace, pane: &Pane) -> Vec<HeldAsk> {
    if away::node_of(ws, pane).is_some() {
        return Vec::new();
    }
    let mut asks = bridge::held_asks(&pane.id);
    asks.reverse();
    asks
}

/// The ask held for the permission the pane shows on `tool`: the one with
/// the input the mod reported for the dialog (`input`), else the newest on
/// that tool.
pub fn permission(
    ws: &Workspace,
    pane: &Pane,
    tool: &str,
    input: Option<&Value>,
) -> Option<HeldAsk> {
    let asks: Vec<HeldAsk> = held(ws, pane)
        .into_iter()
        .filter(|ask| ask.tool == tool && ask.tool != ASK_QUESTION)
        .collect();
    input
        .and_then(|input| asks.iter().find(|ask| ask.input == *input).cloned())
        .or_else(|| asks.into_iter().next())
}

/// The ask held for question `tool_use_id`: the one about that call, else,
/// for an ask that could not name its call, the one asking these questions.
pub fn question(
    ws: &Workspace,
    pane: &Pane,
    tool_use_id: &str,
    questions: &[Question],
) -> Option<HeldAsk> {
    let asks: Vec<HeldAsk> = held(ws, pane)
        .into_iter()
        .filter(|ask| ask.tool == ASK_QUESTION)
        .collect();
    if let Some(ask) = asks
        .iter()
        .find(|ask| ask.tool_use_id.as_deref() == Some(tool_use_id))
    {
        return Some(ask.clone());
    }
    asks.into_iter()
        .find(|ask| ask.tool_use_id.is_none() && asks_the_same(&ask.input, questions))
}

/// The questions an ask's input puts are these, word for word.
fn asks_the_same(input: &Value, questions: &[Question]) -> bool {
    let Some(asked) = input.get("questions").and_then(Value::as_array) else {
        return false;
    };
    asked.len() == questions.len()
        && asked.iter().zip(questions).all(|(asked, question)| {
            asked.get("question").and_then(Value::as_str) == Some(question.question.as_str())
        })
}

/// What the person chose for a permission the pane waits on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Permit {
    /// Allowed once, or for the rest of the session (`session`) where the
    /// agent has such an answer — Codex's "don't ask again … in this
    /// session"; one that has none takes it as a plain allow (core's
    /// bridge.rs, `answer_ask`).
    Allow { session: bool },
    /// Denied, and the turn stopped, as both agents' dialogs do with "No, and
    /// tell … what to do differently" (Escape): the session waits for what
    /// the person says next — `words`, when they said it with the no, which
    /// reach the session as their next message, as typed there.
    Deny { words: Option<String> },
}

impl Permit {
    /// A plain yes or no.
    pub fn of(allow: bool) -> Permit {
        if allow {
            Permit::Allow { session: false }
        } else {
            Permit::Deny { words: None }
        }
    }

    pub fn allows(&self) -> bool {
        matches!(self, Permit::Allow { .. })
    }
}

/// A yes or a no to a permission. A no stops the turn, as Escape in the
/// dialog does: the agent waits for what the person says next.
pub fn permission_decision(allow: bool) -> Value {
    permit_decision(&Permit::of(allow))
}

/// The person's choice as the ask takes it: an allow, for the session when
/// asked (`forSession`), or a no that stops the turn. The words said with a
/// no are not in it: they go to the session as the person's next message
/// (ops.rs), the way the person gives them at the PC once the dialog is
/// down — Codex's answers have no room for them, and Claude Code then reads
/// them once, as theirs.
pub fn permit_decision(permit: &Permit) -> Value {
    match permit {
        Permit::Allow { session: false } => json!({ "behavior": "allow" }),
        Permit::Allow { session: true } => json!({ "behavior": "allow", "forSession": true }),
        Permit::Deny { .. } => json!({ "behavior": "deny", "interrupt": true }),
    }
}

/// A plan approved, or sent back to keep planning: the model reads why, and
/// what to change when the person said. It is not stopped: it plans again.
pub fn plan_decision(approve: bool, feedback: Option<&str>) -> Value {
    if approve {
        return json!({ "behavior": "allow" });
    }
    let message = match feedback.map(str::trim).filter(|words| !words.is_empty()) {
        Some(words) => format!(
            "The person wants to keep planning before anything is implemented. What to change in the plan: {words}"
        ),
        None => {
            "The person wants to keep planning: revise the plan before anything is implemented."
                .to_string()
        }
    };
    json!({ "behavior": "deny", "message": message })
}

/// The answers as the dialog gives them: each question's text to the labels
/// picked and the person's own words, joined by commas.
pub fn question_decision(questions: &[Question], choices: &[Choice]) -> Value {
    let answers: Map<String, Value> = questions
        .iter()
        .zip(choices)
        .map(|(question, choice)| {
            let said: Vec<String> = choice
                .picked
                .iter()
                .filter_map(|&at| question.options.get(at).map(|option| option.label.clone()))
                .chain(choice.other.clone())
                .collect();
            (question.question.clone(), Value::String(said.join(", ")))
        })
        .collect();
    json!({ "behavior": "allow", "answers": answers })
}

/// The row Codex's question dialog adds below a question's options when it
/// takes the person's own words instead of one (`isOther`), and the prefix
/// it gives those words among the answers (codex-rs/tui/src/bottom_pane/
/// request_user_input/mod.rs at rust-v0.170.0: `OTHER_OPTION_LABEL`,
/// `submit_answers`).
pub const CODEX_OTHER: &str = "None of the above";
const CODEX_NOTE: &str = "user_note: ";

/// The answers as the ask holding the questions takes them. One that names
/// its questions by id — Codex's `requestUserInput`, as Hangar's client of
/// its server holds it (`id`, `isOther` beside each question) — gets them by
/// id, as Codex's own dialog gives them: the option picked, or "None of the
/// above" when the person's words stand in for an option and the question
/// offers that row, then the words as a note. Any other gets them by
/// question text (`question_decision`).
pub fn answers_for(ask: &HeldAsk, questions: &[Question], choices: &[Choice]) -> Value {
    let asked = ask
        .input
        .get("questions")
        .and_then(Value::as_array)
        .filter(|asked| asked.len() == questions.len());
    let ids: Option<Vec<&str>> = asked.and_then(|asked| {
        asked
            .iter()
            .map(|question| question.get("id").and_then(Value::as_str))
            .collect()
    });
    let (Some(asked), Some(ids)) = (asked, ids) else {
        return question_decision(questions, choices);
    };
    let answers: Map<String, Value> = questions
        .iter()
        .zip(choices)
        .zip(asked.iter().zip(ids))
        .map(|((question, choice), (asked, id))| {
            let other_row =
                asked.get("isOther") == Some(&Value::Bool(true)) && !question.options.is_empty();
            let mut said: Vec<String> = choice
                .picked
                .iter()
                .filter_map(|&at| question.options.get(at).map(|option| option.label.clone()))
                .collect();
            if let Some(words) = &choice.other {
                if said.is_empty() && other_row {
                    said.push(CODEX_OTHER.to_string());
                }
                said.push(format!("{CODEX_NOTE}{words}"));
            }
            (id.to_string(), json!(said))
        })
        .collect();
    json!({ "behavior": "allow", "answers": answers })
}

/// Gives ask `ask` of the pane its answer and waits for the mod to take it.
/// False when it could not be given, or nobody took it: it is taken back,
/// and the caller types the keys instead.
pub async fn give(port: &Port, pane: &Pane, ask: &HeldAsk, decision: Value) -> bool {
    if !bridge::answer_ask(&pane.id, &ask.id, decision) {
        return false;
    }
    let mut waited = 0;
    while waited < TAKE_MS {
        // Taken, or the ask ended meanwhile: the dialog is down either way.
        if !bridge::answer_untaken(&pane.id, &ask.id) {
            return true;
        }
        port.sleep(TAKE_POLL_MS).await;
        // The mod's wait runs on this runtime too: let it take the answer.
        tokio::task::yield_now().await;
        waited += TAKE_POLL_MS;
    }
    !bridge::withdraw_answer(&pane.id, &ask.id)
}

#[cfg(test)]
mod tests {
    use hangar_core::chat::QuestionOption;

    use super::*;

    fn question(text: &str, labels: &[&str]) -> Question {
        Question {
            question: text.into(),
            header: String::new(),
            multi_select: true,
            options: labels
                .iter()
                .map(|label| QuestionOption {
                    label: (*label).into(),
                    description: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn answers_are_the_labels_then_the_persons_words() {
        let questions = [
            question("Which color?", &["Red", "Blue"]),
            question("Which sizes?", &["S", "M", "L"]),
        ];
        let choices = [
            Choice {
                picked: vec![1],
                other: None,
            },
            Choice {
                picked: vec![0, 2],
                other: Some("XXL too".into()),
            },
        ];
        assert_eq!(
            question_decision(&questions, &choices),
            json!({
                "behavior": "allow",
                "answers": { "Which color?": "Blue", "Which sizes?": "S, L, XXL too" },
            })
        );
    }

    #[test]
    fn a_no_stops_the_turn_as_escape_does() {
        assert_eq!(permission_decision(true), json!({ "behavior": "allow" }));
        assert_eq!(
            permission_decision(false),
            json!({ "behavior": "deny", "interrupt": true })
        );
    }

    #[test]
    fn a_plan_sent_back_carries_what_to_change_and_does_not_stop_the_turn() {
        assert_eq!(
            plan_decision(true, Some("ignored")),
            json!({ "behavior": "allow" })
        );
        let kept = plan_decision(false, Some("  Split step 2 in two  "));
        assert_eq!(kept["behavior"], "deny");
        assert!(
            kept["message"]
                .as_str()
                .unwrap()
                .ends_with(": Split step 2 in two")
        );
        assert!(kept.get("interrupt").is_none());
        let plain = plan_decision(false, Some("   "));
        assert!(plain["message"].as_str().unwrap().contains("keep planning"));
    }

    #[test]
    fn a_permission_is_allowed_once_or_for_the_session_or_denied_stopping_the_turn() {
        assert_eq!(
            permit_decision(&Permit::Allow { session: false }),
            json!({ "behavior": "allow" })
        );
        assert_eq!(
            permit_decision(&Permit::Allow { session: true }),
            json!({ "behavior": "allow", "forSession": true })
        );
        // The words go to the session after, as the person's: not in the
        // answer, which Codex has no room for.
        assert_eq!(
            permit_decision(&Permit::Deny {
                words: Some("use cargo".into())
            }),
            json!({ "behavior": "deny", "interrupt": true })
        );
        assert_eq!(
            permit_decision(&Permit::of(false)),
            permission_decision(false)
        );
        assert_eq!(
            permit_decision(&Permit::of(true)),
            permission_decision(true)
        );
    }

    fn ask(input: Value) -> HeldAsk {
        bridge::hold_ask("p-held-answers-for", "a1", ASK_QUESTION, None, input);
        let held = bridge::held_asks("p-held-answers-for")
            .pop()
            .expect("the ask held");
        bridge::settle_ask("p-held-answers-for", "a1");
        held
    }

    /// Codex's requestUserInput, as Hangar's client of its server holds it
    /// (hangar-core agent/codex_attach.rs): AskUserQuestion's shape, each
    /// question's id and `isOther` beside.
    #[test]
    fn codex_s_questions_are_answered_by_id_as_its_own_dialog_answers_them() {
        let questions = [
            question("Which color?", &["Red", "Blue"]),
            question("Why?", &[]),
            question("Which size?", &["S", "M"]),
        ];
        let questions = questions.map(|mut q| {
            q.multi_select = false;
            q
        });
        let held = ask(json!({ "questions": [
            { "id": "color", "question": "Which color?", "isOther": true,
              "options": [{ "label": "Red" }, { "label": "Blue" }] },
            { "id": "why", "question": "Why?", "isOther": false, "options": [] },
            { "id": "size", "question": "Which size?", "isOther": false,
              "options": [{ "label": "S" }, { "label": "M" }] },
        ] }));
        let choices = [
            Choice {
                picked: Vec::new(),
                other: Some("teal".into()),
            },
            Choice {
                picked: Vec::new(),
                other: Some("it matches".into()),
            },
            Choice {
                picked: vec![1],
                other: None,
            },
        ];
        assert_eq!(
            answers_for(&held, &questions, &choices),
            json!({ "behavior": "allow", "answers": {
                "color": ["None of the above", "user_note: teal"],
                "why": ["user_note: it matches"],
                "size": ["M"],
            } })
        );

        // Claude Code's AskUserQuestion names no ids: by question text.
        let claude = ask(json!({ "questions": [
            { "question": "Which color?", "options": [] },
            { "question": "Why?", "options": [] },
            { "question": "Which size?", "options": [] },
        ] }));
        assert_eq!(
            answers_for(&claude, &questions, &choices),
            question_decision(&questions, &choices)
        );
    }

    #[test]
    fn an_ask_without_its_call_is_matched_by_its_questions() {
        let questions = [question("Which color?", &["Red"])];
        let input = json!({ "questions": [{ "question": "Which color?", "options": [] }] });
        assert!(asks_the_same(&input, &questions));
        let other = json!({ "questions": [{ "question": "Which size?" }] });
        assert!(!asks_the_same(&other, &questions));
        assert!(!asks_the_same(&json!({}), &questions));
    }
}
