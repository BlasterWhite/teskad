//! Answering an agent's questions from the phone (`remote_answer`,
//! lib/remoteAnswer.ts): Claude Code's AskUserQuestion, Codex's
//! requestUserInput. A question the pane's side channel holds open — the
//! hangar-bridge mod, Hangar's client of a Codex pane's server — takes the
//! answers as they are (`held`). Otherwise the window types into the
//! question's dialog what the person would type at the PC, so the session
//! gets the answer exactly as if it had been given there: each agent's own
//! dialog, read in its own terms (`Dialogs`). Codex's, and its approval
//! dialog, are read further down ("Codex's own dialogs").
//!
//! The keys are those of Claude Code's own dialog (2.1.x). It shows one tab
//! per question, each opening with the cursor on its first option. Enter
//! picks a single-select option and moves on to the next tab; on a
//! multi-select question it ticks the option under the cursor, and the row
//! below the options, "Next" ("Submit" on the last), moves on. The row just
//! after the options, "Type something", takes the person's own words as they
//! are pasted, and ticks itself. A review then waits for Enter on "Submit
//! answers", except after a lone single-select question, which its Enter
//! submits.
//!
//! Every step is checked on the screen before the next: nothing is typed
//! unless the dialog shows the question as Claude Code first drew it, and
//! nothing is submitted unless the screen shows what the person chose. A
//! dialog someone is answering at the PC, a key lost on the way, a Claude
//! Code that draws its dialog otherwise: each stops the answer, never
//! submitting what the person did not choose.

use std::future::Future;
use std::pin::Pin;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use hangar_core::agent::{AgentKind, Asks};
use hangar_core::chat::{Answer, Body, ChatItem, PendingQuestion, Question};

use crate::control::port::Port;
use crate::control::run_as_person;
use crate::state::{Pane, Workspace};

use super::contract::RemoteAnswered;
use super::desk::transcript_page;
use super::engine::{ChatRead, Mobile};
use super::held::{self, CODEX_OTHER, Permit};
use super::ops::{find_pane, hold, resumes};

/// The dialogs a pane's agent draws in its own terminal, which the window
/// answers with keys where no side channel takes the answer: each agent's
/// own, read in its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialogs {
    /// Claude Code 2.1's: AskUserQuestion's tabs, checked on the screen
    /// step by step; a permission prompt's Enter or Escape.
    ClaudeCode,
    /// Codex 0.170's TUI: its approval dialog and its question dialog, both
    /// checked on the screen before each key.
    Codex,
}

impl Dialogs {
    /// The dialogs of the pane's agent; none for a shell. What a terminal
    /// shows is each agent's own: the one thing here that follows the kind
    /// rather than a capability (docs/native/agents.md D3 leaves a kind's
    /// screens to its own driver code, as it does its transcript reader).
    pub fn of(pane: &Pane) -> Option<Dialogs> {
        Some(match pane.agent.kind()? {
            AgentKind::Claude => Dialogs::ClaudeCode,
            AgentKind::Codex => Dialogs::Codex,
        })
    }
}

/// One write into the pane: `text` pasted, never followed by Enter, then
/// `keys`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnswerStep {
    pub text: Option<String>,
    pub keys: Vec<String>,
}

/// A question's answer, checked against its options: the indexes picked,
/// ascending, and the person's own words.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice {
    pub picked: Vec<usize>,
    pub other: Option<String>,
}

type Io<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the pane reads and types, through pane_read and pane_send.
pub trait AnswerIo: Send + Sync {
    /// The last lines the pane's terminal shows.
    fn read(&self) -> Io<'_, Result<String, String>>;
    fn type_step(&self, step: AnswerStep) -> Io<'_, Result<(), String>>;
    fn sleep(&self, ms: u64) -> Io<'_, ()>;
}

/// How long a step gets to show on the screen, read again every `SETTLE_MS`.
const SETTLE_MS: u64 = 150;
const SETTLE_TRIES: usize = 20;
/// The question is on the screen already when the transcript says it waits.
const FIRST_TRIES: usize = 3;

pub const NOT_ON_SCREEN: &str = "The question is not on the PC's screen as Claude Code first drew it: someone may be answering it there. Nothing was typed.";
pub const NOT_REVIEWED: &str = "The PC's screen does not show these answers ready to submit, so nothing was submitted: check them in the Terminal view.";

pub fn lost_at(index: usize) -> String {
    format!(
        "Question {} did not come up on the PC as expected, so the rest was not typed: finish in the Terminal view.",
        index + 1
    )
}

/// The questions `tool_use_id` asks, when the pane waits on it; otherwise
/// the reason it does not, for the phone to show: its screen is stale
/// (`pendingQuestions`).
pub fn pending_questions(
    pending: Option<&PendingQuestion>,
    items: &[ChatItem],
    tool_use_id: &str,
) -> Result<Vec<Question>, String> {
    pending_questions_for("Claude Code", pending, items, tool_use_id)
}

/// `pending_questions` of a pane whose agent is called `agent`, which the
/// refusals name.
pub fn pending_questions_for(
    agent: &str,
    pending: Option<&PendingQuestion>,
    items: &[ChatItem],
    tool_use_id: &str,
) -> Result<Vec<Question>, String> {
    if let Some(pending) = pending
        && pending.tool_use_id == tool_use_id
    {
        return Ok(pending.questions.clone());
    }
    let asked = items.iter().find_map(|item| match &item.body {
        Body::Question {
            tool_use_id: id,
            answer,
            ..
        } if id == tool_use_id => Some(answer.is_some()),
        _ => None,
    });
    Err(match (asked, pending) {
        (Some(true), _) => "This question was already answered.".to_string(),
        (Some(false), _) => {
            format!("{agent} no longer waits on this question: it was dismissed.")
        }
        (None, Some(_)) => format!("{agent} now waits on another question."),
        (None, None) => format!("{agent} is not waiting on this question."),
    })
}

/// Line breaks and control characters out: in the dialog's field a newline
/// would answer.
fn clean_words(text: &str) -> String {
    static BREAKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\r\n\t]+").unwrap());
    let spaced = BREAKS.replace_all(text, " ");
    spaced
        .chars()
        .filter(|c| !matches!(*c as u32, 0x00..=0x1f | 0x7f..=0x9f))
        .collect::<String>()
        .trim()
        .to_string()
}

/// Refused: not one answer per question.
fn count_refused(questions: &[Question]) -> String {
    if questions.len() == 1 {
        "Give one answer: this is one question.".into()
    } else {
        format!("Give one answer per question: {} of them.", questions.len())
    }
}

/// How a refusal names question `index`.
fn name_of(questions: &[Question], index: usize) -> String {
    let question = &questions[index];
    if questions.len() == 1 {
        format!("\"{}\"", question.header)
    } else {
        format!("Question {} (\"{}\")", index + 1, question.header)
    }
}

/// One answer, its labels and the person's own words, held against its
/// question.
fn choice_for(
    name: &str,
    question: &Question,
    selected: &[String],
    other: &str,
) -> Result<Choice, String> {
    let mut unique: Vec<&String> = Vec::new();
    for label in selected {
        if !unique.contains(&label) {
            unique.push(label);
        }
    }
    let mut picked = Vec::new();
    for label in unique {
        let Some(at) = question
            .options
            .iter()
            .position(|option| option.label == *label)
        else {
            return Err(format!("\"{label}\" is not one of the options of {name}."));
        };
        picked.push(at);
    }
    picked.sort_unstable();
    let other = Some(clean_words(other)).filter(|words| !words.is_empty());
    let given = picked.len() + usize::from(other.is_some());
    if given == 0 {
        return Err(format!("{name} has no answer."));
    }
    if !question.multi_select && given > 1 {
        return Err(format!(
            "{name} takes one answer: one option, or your own words."
        ));
    }
    Ok(Choice { picked, other })
}

/// The phone's answers held against the questions; what does not fit is
/// refused (`choicesOf`).
pub fn choices_of(questions: &[Question], answers: Option<&Value>) -> Result<Vec<Choice>, String> {
    let answers = match answers.and_then(Value::as_array) {
        Some(list) if list.len() == questions.len() => list,
        _ => return Err(count_refused(questions)),
    };
    questions
        .iter()
        .zip(answers)
        .enumerate()
        .map(|(index, (question, answer))| {
            let name = name_of(questions, index);
            let selected = match answer.get("selected") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(labels)) if labels.iter().all(Value::is_string) => labels
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
                Some(_) => return Err(format!("{name}: \"selected\" is a list of option labels.")),
            };
            let other = match answer.get("other") {
                None => String::new(),
                Some(Value::String(words)) => words.clone(),
                Some(_) => {
                    return Err(format!(
                        "{name}: \"other\" is the person's own words, as text."
                    ));
                }
            };
            choice_for(&name, question, &selected, &other)
        })
        .collect()
}

/// The window's answers held against the questions, as `choices_of` holds
/// the phone's.
pub fn choices_from(questions: &[Question], answers: &[Answer]) -> Result<Vec<Choice>, String> {
    if answers.len() != questions.len() {
        return Err(count_refused(questions));
    }
    questions
        .iter()
        .zip(answers)
        .enumerate()
        .map(|(index, (question, answer))| {
            let other = answer.other.as_deref().unwrap_or("");
            choice_for(
                &name_of(questions, index),
                question,
                &answer.selected,
                other,
            )
        })
        .collect()
}

fn down(count: usize) -> impl Iterator<Item = String> {
    std::iter::repeat_n("down".to_string(), count)
}

/// What answering one question types, from its tab as first drawn: the
/// cursor on the first option. The last key is the Enter that moves on, or
/// submits (`stepsFor`).
pub fn steps_for(question: &Question, choice: &Choice) -> Vec<AnswerStep> {
    let type_row = question.options.len();
    let keys = |keys: Vec<String>| AnswerStep { text: None, keys };
    if !question.multi_select {
        if let Some(other) = &choice.other {
            return vec![
                keys(down(type_row).collect()),
                AnswerStep {
                    text: Some(other.clone()),
                    keys: vec!["enter".into()],
                },
            ];
        }
        let first = choice.picked.first().copied().unwrap_or(0);
        return vec![keys(down(first).chain(["enter".to_string()]).collect())];
    }
    let mut typed = Vec::new();
    let mut at = 0;
    for &index in &choice.picked {
        typed.extend(down(index - at));
        typed.push("enter".into());
        at = index;
    }
    match &choice.other {
        // "Next" is the row after "Type something".
        None => {
            typed.extend(down(type_row + 1 - at));
            typed.push("enter".into());
            vec![keys(typed)]
        }
        Some(other) => {
            typed.extend(down(type_row - at));
            vec![
                keys(typed),
                AnswerStep {
                    text: Some(other.clone()),
                    keys: vec!["down".into(), "enter".into()],
                },
            ]
        }
    }
}

/* ── Reading the dialog off the screen ────────────────────────── */

/// Text as compared with the screen: no whitespace, and none of the bar
/// Claude Code (2.1.32x) draws down the left of a question's lines — a long
/// question wraps, and the bar of each line would land in the middle of it.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && *c != GUTTER)
        .collect()
}

/// The bar down the left of a question's lines.
const GUTTER: char = '│';

/// A numbered row, or the unnumbered one that moves on ("Next", "Submit").
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    /// `None`: the row that moves on.
    n: Option<usize>,
    cursor: bool,
    text: String,
    /// Its line in the dialog.
    at: usize,
}

static NUMBERED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*([❯>])?\s*(\d+)\.\s+(.*?)\s*$").unwrap());
static MOVE_ON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*([❯>])?\s+(Next|Submit)\s*$").unwrap());
static BOX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[(.)\]\s*").unwrap());
static TYPE_SOMETHING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\[ \]\s*)?Type something").unwrap());
static LEFT_OVER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^[,"]*$"#).unwrap());

fn rows_of(lines: &[&str]) -> Vec<Row> {
    let mut rows = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        if let Some(caps) = NUMBERED.captures(line) {
            rows.push(Row {
                n: caps[2].parse().ok(),
                cursor: caps.get(1).is_some(),
                text: caps[3].to_string(),
                at,
            });
        }
        if let Some(caps) = MOVE_ON.captures(line) {
            rows.push(Row {
                n: None,
                cursor: caps.get(1).is_some(),
                text: caps[2].to_string(),
                at,
            });
        }
    }
    rows
}

/// The dialog: the lines below its tab bar, the last line of the screen
/// that shows one of the questions' headers as a tab (☐ unanswered, ☒
/// answered). Claude Code draws it at the bottom, in place of the prompt
/// (`dialogOf`).
pub fn dialog_of<'a>(screen: &'a str, questions: &[Question]) -> Option<Vec<&'a str>> {
    let lines: Vec<&str> = screen.split('\n').collect();
    let at = lines.iter().rposition(|line| {
        questions.iter().any(|question| {
            line.contains(&format!("☐ {}", question.header))
                || line.contains(&format!("☒ {}", question.header))
        })
    })?;
    Some(lines[at + 1..].to_vec())
}

/// Whether the dialog shows `question` as first drawn: its options in their
/// order, nothing ticked, nothing typed, the cursor on the first
/// (`untouched`).
pub fn untouched(lines: &[&str], question: &Question) -> bool {
    let rows = rows_of(lines);
    let numbered = |n: usize| rows.iter().find(|row| row.n == Some(n));
    let Some(first) = numbered(1) else {
        return false;
    };
    if !first.cursor || rows.iter().filter(|row| row.cursor).count() != 1 {
        return false;
    }
    if !squash(&lines[..first.at].concat()).contains(&squash(&question.question)) {
        return false;
    }
    let options_shown = question.options.iter().enumerate().all(|(index, option)| {
        let Some(row) = numbered(index + 1) else {
            return false;
        };
        let boxed = BOX.captures(&row.text);
        let ticked_right = match &boxed {
            Some(caps) => question.multi_select && &caps[1] == " ",
            None => !question.multi_select,
        };
        if !ticked_right {
            return false;
        }
        let label = match &boxed {
            Some(caps) => &row.text[caps[0].len()..],
            None => row.text.as_str(),
        };
        squash(label) == squash(&option.label)
    });
    let type_row = numbered(question.options.len() + 1);
    options_shown && type_row.is_some_and(|row| TYPE_SOMETHING.is_match(&row.text))
}

/// Whether a lone single-select question has the cursor on the person's
/// answer: its Enter submits at once, with no review to check first
/// (`chosen`).
pub fn chosen(lines: &[&str], question: &Question, choice: &Choice) -> bool {
    let rows = rows_of(lines);
    let cursor: Vec<&Row> = rows.iter().filter(|row| row.cursor).collect();
    let [row] = cursor.as_slice() else {
        return false;
    };
    let Some(other) = &choice.other else {
        let index = choice.picked.first().copied().unwrap_or(0);
        return row.n == Some(index + 1)
            && question
                .options
                .get(index)
                .is_some_and(|option| squash(&row.text) == squash(&option.label));
    };
    // Words longer than the row run on below it.
    let mut run_on = vec![row.text.as_str()];
    for line in &lines[row.at + 1..] {
        if line.trim().is_empty()
            || line.contains('─')
            || NUMBERED.is_match(line)
            || MOVE_ON.is_match(line)
        {
            break;
        }
        run_on.push(line);
    }
    row.n == Some(question.options.len() + 1) && squash(&run_on.concat()) == squash(other)
}

/// Whether the review shows, for each question, the answer asked for and
/// nothing else, with the cursor on "Submit answers". Claude Code joins a
/// question's answers with ", " and may quote the person's words
/// (`reviewed`).
pub fn reviewed(lines: &[&str], questions: &[Question], choices: &[Choice]) -> bool {
    let text = lines.join("\n");
    if !squash(&text).contains("Reviewyouranswers") {
        return false;
    }
    let rows = rows_of(lines);
    let Some(submit) = rows.iter().find(|row| row.n == Some(1)) else {
        return false;
    };
    if !submit.cursor || !submit.text.starts_with("Submit answers") {
        return false;
    }
    let body = match text.find("Ready to submit") {
        Some(end) => &text[..end],
        None => text.as_str(),
    };
    let parts: Vec<String> = body.split('●').skip(1).map(squash).collect();
    questions.iter().zip(choices).all(|(question, choice)| {
        let asked = format!("{}→", squash(&question.question));
        let Some(part) = parts.iter().find(|part| part.starts_with(&asked)) else {
            return false;
        };
        let mut left = part[asked.len()..].to_string();
        let mut pieces: Vec<String> = choice
            .picked
            .iter()
            .filter_map(|&at| question.options.get(at).map(|option| option.label.clone()))
            .chain(choice.other.clone())
            .map(|piece| squash(&piece))
            .collect();
        // The longest first: a label inside another is found in what is left.
        pieces.sort_by_key(|piece| std::cmp::Reverse(piece.chars().count()));
        for piece in pieces {
            let Some(at) = left.find(&piece) else {
                return false;
            };
            left.replace_range(at..at + piece.len(), "");
        }
        LEFT_OVER.is_match(&left)
    })
}

/* ── Typing it ────────────────────────────────────────────────── */

/// Reads the screen until the dialog passes `check`, a few times.
async fn shows(
    io: &dyn AnswerIo,
    questions: &[Question],
    check: impl Fn(&[&str]) -> bool,
    tries: usize,
) -> Result<bool, String> {
    for attempt in 0..tries {
        if attempt > 0 {
            io.sleep(SETTLE_MS).await;
        }
        let screen = io.read().await?;
        if let Some(lines) = dialog_of(&screen, questions)
            && check(&lines)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Types the answers into the dialog of `questions`, checking the screen
/// before each question and before the Enter that submits. Refused, having
/// submitted nothing, when the screen is not what that step expects
/// (`typeAnswers`).
pub async fn type_answers(
    io: &dyn AnswerIo,
    questions: &[Question],
    choices: &[Choice],
) -> Result<(), String> {
    let lone = questions.len() == 1 && !questions[0].multi_select;
    for (index, (question, choice)) in questions.iter().zip(choices).enumerate() {
        let tries = if index == 0 {
            FIRST_TRIES
        } else {
            SETTLE_TRIES
        };
        let ready = shows(io, questions, |lines| untouched(lines, question), tries).await?;
        if !ready {
            return Err(if index == 0 {
                NOT_ON_SCREEN.to_string()
            } else {
                lost_at(index)
            });
        }
        let mut steps = steps_for(question, choice);
        if lone && let Some(last) = steps.last_mut() {
            // Held back: this Enter submits, so the screen is checked first.
            last.keys.pop();
        }
        for step in steps {
            if step.text.as_deref().is_some_and(|text| !text.is_empty()) || !step.keys.is_empty() {
                io.type_step(step).await?;
            }
        }
    }
    let ready = if lone {
        shows(
            io,
            questions,
            |lines| chosen(lines, &questions[0], &choices[0]),
            SETTLE_TRIES,
        )
        .await?
    } else {
        shows(
            io,
            questions,
            |lines| reviewed(lines, questions, choices),
            SETTLE_TRIES,
        )
        .await?
    };
    if !ready {
        return Err(NOT_REVIEWED.into());
    }
    io.type_step(AnswerStep {
        text: None,
        keys: vec!["enter".into()],
    })
    .await
}

/* ── Codex's own dialogs ──────────────────────────────────────── */
//
// Codex's TUI at rust-v0.170.0, read in its source (codex-rs/tui/src/
// bottom_pane/) and in the screens its snapshot tests keep, never run
// (docs/native/agents.md §4.0). Both dialogs draw at the bottom of the
// screen, in place of the composer, the cursor on a row marked `›`:
//
// - approval_overlay.rs: a title ("Would you like to run the following
//   command?"), what is asked, numbered options each with its shortcut
//   ("› 1. Yes, proceed (y)"), then "Press enter to confirm or esc to
//   cancel". Enter takes the option under the cursor; Escape cancels the
//   request (`cancel_current_request`): for a command or a patch that is
//   "No, and tell Codex what to do differently", for permissions "No,
//   continue without permissions". Which options show depends on what Codex
//   allows for the request (`exec_options`), so the one the person chose is
//   looked for by its label, never by its place.
// - request_user_input/: "Question 1/2 (2 unanswered)", the question, its
//   numbered options with their descriptions in a column beside them, then
//   "None of the above" when the question takes the person's own words
//   instead (`isOther`), and a notes field ("› Add notes", or for a question
//   without options "› Type your answer (optional)"). Down moves the cursor;
//   Enter commits the option under it and goes to the next question, or
//   submits them all on the last; on "None of the above" it opens the notes
//   instead. A paste lands in the notes, and Enter there commits and goes
//   on (`handle_key_event`, `handle_paste`).
//
// Only the keys pane_send has are typed — arrows, Enter, Escape: a letter or
// a digit would reach Codex as a paste — and Codex's bindings for them are
// its defaults, which the dialog's own footer names.

pub const CODEX_NOT_ON_SCREEN: &str = "Codex's dialog is not on the PC's screen as Codex first drew it: someone may be answering it there. Nothing was typed.";
pub const CODEX_NOT_CHOSEN: &str = "The PC's screen does not show Codex's dialog on the answer asked, so nothing was answered: check it in the Terminal view.";
pub const CODEX_NO_SESSION: &str = "Codex's dialog offers no way to allow this for the rest of the session: nothing was typed. Allow it once, or answer in the Terminal view.";
pub const CODEX_NO_ALLOW: &str =
    "Codex's dialog offers no plain yes here: nothing was typed. Answer it in the Terminal view.";
pub const CODEX_STILL_UP: &str = "Codex's dialog was answered no, but it is still on the PC's screen, so your words were not typed: say them in the Terminal view.";
pub const CODEX_UNANSWERED: &str =
    "Codex asks to confirm questions left unanswered: finish in the Terminal view.";

/// Refused before anything is typed: Codex's dialog takes the person's own
/// words for question `index` only beside an option.
pub fn codex_no_words(index: usize) -> String {
    format!(
        "Codex's dialog takes your own words for question {} only with one of its options: pick one, or answer in the Terminal view. Nothing was typed.",
        index + 1
    )
}

/// A numbered row of either dialog, `›` before the one under the cursor.
static CODEX_ROW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(›)?\s*(\d+)\.\s+(.*?)\s*$").unwrap());
/// The approval dialog's titles (`build_options`).
static CODEX_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"^\s*(Would you like to (run the following command|send input to .+|grant these permissions|make the following edits)\?|Do you want to approve network access to ".*"\?|.+ needs your approval\.)\s*$"#,
    )
    .unwrap()
});
/// The approval dialog's footer, its keys Codex's defaults
/// (`approval_footer_hint`).
const CODEX_CONFIRM: &str = "Press enter to confirm or esc to cancel";
/// The shortcut an approval option shows after its label: "(y)", "(esc)".
static CODEX_SHORTCUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+\([^()]+\)$").unwrap());
/// The question dialog's first line (`progress_prefix_text`).
static CODEX_PROGRESS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*Question (\d+)/(\d+)(?: \(\d+ unanswered\))?\s*$").unwrap());
/// Where an option's description starts, in its column.
static CODEX_COLUMN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s{2,}").unwrap());
/// What the notes field of a question without options shows while empty.
const CODEX_ANSWER_HINT: &str = "Type your answer (optional)";
/// The dialog Codex opens when Enter submits with questions unanswered.
const CODEX_CONFIRM_UNANSWERED: &str = "Submit with unanswered questions?";

/// The approval options that allow once, as each request words it
/// (`exec_options`, `patch_options`, `permissions_options`,
/// `elicitation_options`).
const CODEX_ALLOW: [&str; 4] = [
    "Yes, proceed",
    "Yes, just this once",
    "Yes, grant these permissions for this turn",
    "Yes, provide the requested info",
];
/// The approval options that allow for the rest of the session.
const CODEX_ALLOW_SESSION: [&str; 5] = [
    "Yes, and don't ask again for this command in this session",
    "Yes, and allow these permissions for this session",
    "Yes, and allow this host for this conversation",
    "Yes, and don't ask again for these files",
    "Yes, grant these permissions for this session",
];

/// The numbered rows of a dialog's lines.
fn codex_rows(lines: &[&str]) -> Vec<Row> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(at, line)| {
            let caps = CODEX_ROW.captures(line)?;
            Some(Row {
                n: caps[2].parse().ok(),
                cursor: caps.get(1).is_some(),
                text: caps[3].to_string(),
                at,
            })
        })
        .collect()
}

/// The one row under the cursor, if there is exactly one.
fn codex_cursor(rows: &[Row]) -> Option<&Row> {
    let mut under = rows.iter().filter(|row| row.cursor);
    let row = under.next()?;
    under.next().is_none().then_some(row)
}

/// Codex's approval dialog on the screen: its lines from its title down,
/// when its footer is there below them.
fn codex_approval(screen: &str) -> Option<Vec<&str>> {
    let lines: Vec<&str> = screen.split('\n').collect();
    let at = lines.iter().rposition(|line| CODEX_TITLE.is_match(line))?;
    let dialog = lines[at..].to_vec();
    dialog
        .iter()
        .any(|line| line.contains(CODEX_CONFIRM))
        .then_some(dialog)
}

/// An approval option's label, its shortcut left out.
fn codex_label(row: &Row) -> String {
    squash(&CODEX_SHORTCUT.replace(&row.text, ""))
}

/// The approval dialog's numbered options, when it shows them as Codex
/// first draws them: the cursor on the first, alone.
fn codex_approval_untouched(screen: &str) -> Option<Vec<Row>> {
    let rows = codex_rows(&codex_approval(screen)?);
    let first = codex_cursor(&rows)?;
    (first.n == Some(1)).then_some(rows)
}

/// Reads the screen until it passes `check`, a few times.
async fn codex_shows(
    io: &dyn AnswerIo,
    tries: usize,
    check: impl Fn(&str) -> bool,
) -> Result<bool, String> {
    for attempt in 0..tries {
        if attempt > 0 {
            io.sleep(SETTLE_MS).await;
        }
        if check(&io.read().await?) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn keys_step(keys: impl IntoIterator<Item = String>) -> AnswerStep {
    AnswerStep {
        text: None,
        keys: keys.into_iter().collect(),
    }
}

fn enter() -> AnswerStep {
    keys_step(["enter".to_string()])
}

/// Answers Codex's approval dialog as the person chose, with its own keys:
/// nothing typed unless the screen shows the dialog as Codex first drew it,
/// and the Enter that answers only once the cursor shows on the option
/// chosen. A no is its Escape.
pub async fn press_codex_approval(io: &dyn AnswerIo, permit: &Permit) -> Result<(), String> {
    let screen = io.read().await?;
    let Some(rows) = codex_approval_untouched(&screen) else {
        return Err(CODEX_NOT_ON_SCREEN.into());
    };
    let (labels, missing): (&[&str], &str) = match permit {
        Permit::Deny { .. } => return io.type_step(keys_step(["escape".to_string()])).await,
        Permit::Allow { session: false } => (&CODEX_ALLOW[..], CODEX_NO_ALLOW),
        Permit::Allow { session: true } => (&CODEX_ALLOW_SESSION[..], CODEX_NO_SESSION),
    };
    let wanted: Vec<String> = labels.iter().map(|label| squash(label)).collect();
    let Some(target) = rows
        .iter()
        .find(|row| wanted.contains(&codex_label(row)))
        .and_then(|row| row.n.map(|n| (n, codex_label(row))))
    else {
        return Err(missing.into());
    };
    let (n, label) = target;
    if n > 1 {
        io.type_step(keys_step(down(n - 1))).await?;
    }
    let chosen = |screen: &str| {
        codex_approval(screen).is_some_and(|dialog| {
            let rows = codex_rows(&dialog);
            codex_cursor(&rows).is_some_and(|row| row.n == Some(n) && codex_label(row) == label)
        })
    };
    if !codex_shows(io, SETTLE_TRIES, chosen).await? {
        return Err(CODEX_NOT_CHOSEN.into());
    }
    io.type_step(enter()).await
}

/// Waits for Codex's approval dialog to go down, as it does once answered.
pub async fn codex_approval_gone(io: &dyn AnswerIo) -> Result<(), String> {
    if codex_shows(io, SETTLE_TRIES, |screen| codex_approval(screen).is_none()).await? {
        Ok(())
    } else {
        Err(CODEX_STILL_UP.into())
    }
}

/// Codex's question dialog on the screen: the question it is at (from 1),
/// how many it asks, and its lines below that.
fn codex_question(screen: &str) -> Option<(usize, usize, Vec<&str>)> {
    let lines: Vec<&str> = screen.split('\n').collect();
    let at = lines
        .iter()
        .rposition(|line| CODEX_PROGRESS.is_match(line))?;
    let caps = CODEX_PROGRESS.captures(lines[at])?;
    Some((
        caps[1].parse().ok()?,
        caps[2].parse().ok()?,
        lines[at + 1..].to_vec(),
    ))
}

/// Whether an option row shows `label`: the start of its label column, all
/// of it unless a long label wraps.
fn codex_shows_label(row: &Row, label: &str) -> bool {
    let shown = squash(CODEX_COLUMN.split(row.text.trim()).next().unwrap_or(""));
    !shown.is_empty() && squash(label).starts_with(&shown)
}

/// The lines that start with the cursor mark: the option under it, the
/// notes field.
fn codex_marks(lines: &[&str]) -> usize {
    lines
        .iter()
        .filter(|line| line.trim_start().starts_with('›'))
        .count()
}

/// The notes field's text: from its `›` (the one line marked that is not an
/// option) down to the footer, the mark left out.
fn codex_notes(lines: &[&str]) -> Option<String> {
    let at = lines
        .iter()
        .position(|line| line.trim_start().starts_with('›') && !CODEX_ROW.is_match(line))?;
    let text: String = lines[at..]
        .iter()
        .take_while(|line| !line.contains("to submit"))
        .map(|line| line.replacen('›', "", 1))
        .collect();
    Some(squash(&text))
}

/// What the screen shows of question `index` of `total` in Codex's dialog,
/// when it shows it as first drawn: its options in their order — "None of
/// the above" after them when it offers that row (`true`) — the cursor on
/// the first, the notes closed; or for a question without options its
/// notes field, empty.
fn codex_untouched(screen: &str, index: usize, total: usize, question: &Question) -> Option<bool> {
    let (at, of, lines) = codex_question(screen)?;
    if at != index + 1 || of != total {
        return None;
    }
    let rows = codex_rows(&lines);
    if question.options.is_empty() {
        let empty = codex_notes(&lines).is_some_and(|notes| notes == squash(CODEX_ANSWER_HINT));
        let asked = squash(&lines.concat()).contains(&squash(&question.question));
        return (rows.is_empty() && codex_marks(&lines) == 1 && empty && asked).then_some(false);
    }
    let first = rows.first()?;
    if !squash(&lines[..first.at].concat()).contains(&squash(&question.question)) {
        return None;
    }
    let options = question.options.len();
    let other = rows.len() == options + 1 && codex_shows_label(&rows[options], CODEX_OTHER);
    let in_order = rows
        .iter()
        .enumerate()
        .all(|(at, row)| row.n == Some(at + 1));
    let labelled = rows
        .iter()
        .zip(&question.options)
        .all(|(row, option)| codex_shows_label(row, &option.label));
    let under = codex_cursor(&rows).is_some_and(|row| row.n == Some(1));
    ((rows.len() == options || other) && in_order && labelled && under && codex_marks(&lines) == 1)
        .then_some(other)
}

/// Whether question `index` of `total` shows the cursor on row `n`, which
/// shows `label`, and — with `words` — the notes field holding them.
fn codex_at(
    screen: &str,
    index: usize,
    total: usize,
    n: usize,
    label: &str,
    words: Option<&str>,
) -> bool {
    let Some((at, of, lines)) = codex_question(screen) else {
        return false;
    };
    let rows = codex_rows(&lines);
    let on =
        codex_cursor(&rows).is_some_and(|row| row.n == Some(n) && codex_shows_label(row, label));
    let said = words.is_none_or(|words| {
        codex_notes(&lines).is_some_and(|notes| notes.contains(&squash(words)))
    });
    at == index + 1 && of == total && on && said
}

/// Whether question `index` of `total`, one without options, shows `words`
/// in its notes field.
fn codex_said(screen: &str, index: usize, total: usize, words: &str) -> bool {
    codex_question(screen).is_some_and(|(at, of, lines)| {
        at == index + 1
            && of == total
            && codex_rows(&lines).is_empty()
            && codex_notes(&lines).is_some_and(|notes| notes.contains(&squash(words)))
    })
}

/// Whether each question takes the person's own words instead of an option
/// ("None of the above"), as Codex said when it asked them (`isOther`, in
/// the call Hangar's client of its server reported): unknown where it did
/// not say.
pub fn codex_others(input: Option<&Value>, count: usize) -> Vec<Option<bool>> {
    let asked = input
        .and_then(|input| input.get("questions"))
        .and_then(Value::as_array);
    (0..count)
        .map(|at| {
            asked
                .and_then(|asked| asked.get(at))
                .and_then(|question| question.get("isOther"))
                .and_then(Value::as_bool)
        })
        .collect()
}

/// Types the answers into Codex's question dialog, one question at a time:
/// nothing typed into a question unless the screen shows it as first drawn,
/// and no Enter — each one commits, the last submits them all — unless the
/// screen shows the answer asked under the cursor. Own words go where the
/// dialog takes them: a question without options, or "None of the above";
/// refused before anything is typed where Codex said a question has no such
/// row (`others`, `codex_others`).
pub async fn type_codex_answers(
    io: &dyn AnswerIo,
    questions: &[Question],
    choices: &[Choice],
    others: &[Option<bool>],
) -> Result<(), String> {
    for (index, (question, choice)) in questions.iter().zip(choices).enumerate() {
        let words_need_a_row = choice.other.is_some() && !question.options.is_empty();
        if words_need_a_row && others.get(index).copied().flatten() == Some(false) {
            return Err(codex_no_words(index));
        }
    }
    let total = questions.len();
    for (index, (question, choice)) in questions.iter().zip(choices).enumerate() {
        let tries = if index == 0 {
            FIRST_TRIES
        } else {
            SETTLE_TRIES
        };
        let mut offers_other = None;
        for attempt in 0..tries {
            if attempt > 0 {
                io.sleep(SETTLE_MS).await;
            }
            offers_other = codex_untouched(&io.read().await?, index, total, question);
            if offers_other.is_some() {
                break;
            }
        }
        let Some(offers_other) = offers_other else {
            return Err(if index == 0 {
                CODEX_NOT_ON_SCREEN.to_string()
            } else {
                lost_at(index)
            });
        };
        match &choice.other {
            None => {
                let at = choice.picked.first().copied().unwrap_or(0);
                let label = &question.options[at].label;
                if at > 0 {
                    io.type_step(keys_step(down(at))).await?;
                }
                let on = |screen: &str| codex_at(screen, index, total, at + 1, label, None);
                if !codex_shows(io, SETTLE_TRIES, on).await? {
                    return Err(CODEX_NOT_CHOSEN.into());
                }
            }
            Some(words) if question.options.is_empty() => {
                io.type_step(AnswerStep {
                    text: Some(words.clone()),
                    keys: Vec::new(),
                })
                .await?;
                let said = |screen: &str| codex_said(screen, index, total, words);
                if !codex_shows(io, SETTLE_TRIES, said).await? {
                    return Err(CODEX_NOT_CHOSEN.into());
                }
            }
            Some(words) => {
                if !offers_other {
                    return Err(if index == 0 {
                        codex_no_words(index)
                    } else {
                        lost_at(index)
                    });
                }
                let row = question.options.len();
                io.type_step(keys_step(down(row))).await?;
                let on = |screen: &str| codex_at(screen, index, total, row + 1, CODEX_OTHER, None);
                if !codex_shows(io, SETTLE_TRIES, on).await? {
                    return Err(CODEX_NOT_CHOSEN.into());
                }
                // Enter on "None of the above" opens its notes; the words
                // are pasted there.
                io.type_step(enter()).await?;
                io.type_step(AnswerStep {
                    text: Some(words.clone()),
                    keys: Vec::new(),
                })
                .await?;
                let said = |screen: &str| {
                    codex_at(screen, index, total, row + 1, CODEX_OTHER, Some(words))
                };
                if !codex_shows(io, SETTLE_TRIES, said).await? {
                    return Err(CODEX_NOT_CHOSEN.into());
                }
            }
        }
        io.type_step(enter()).await?;
    }
    // Every question committed, the last Enter submitted them all; a
    // question left unanswered would have Codex ask first.
    io.sleep(SETTLE_MS).await;
    if io.read().await?.contains(CODEX_CONFIRM_UNANSWERED) {
        return Err(CODEX_UNANSWERED.into());
    }
    Ok(())
}

/* ── remote_answer ────────────────────────────────────────────── */

/// Lines read to find a question's dialog: four questions' review fits well
/// within.
const ANSWER_SCREEN: usize = 120;

/// The answers as the person would write them, for a session that resumed
/// after a sleep: its dialog went with the agent the sleep stopped
/// (`answerText`).
pub fn answer_text(questions: &[Question], choices: &[Choice]) -> String {
    questions
        .iter()
        .zip(choices)
        .map(|(question, choice)| {
            let said: Vec<String> = choice
                .picked
                .iter()
                .filter_map(|&at| question.options.get(at).map(|option| option.label.clone()))
                .chain(choice.other.clone())
                .collect();
            format!("{}\n→ {}", question.question, said.join(", "))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The pane's screen and keys, through pane_read and pane_send as the
/// person.
pub struct PaneIo<'a> {
    port: &'a Port,
    workspace: String,
    pane: String,
}

impl<'a> PaneIo<'a> {
    pub fn new(port: &'a Port, ws: &Workspace, pane: &Pane) -> PaneIo<'a> {
        PaneIo {
            port,
            workspace: ws.id.clone(),
            pane: pane.id.clone(),
        }
    }
}

impl AnswerIo for PaneIo<'_> {
    fn read(&self) -> Io<'_, Result<String, String>> {
        Box::pin(async move {
            let read = run_as_person(
                "pane_read",
                json!({ "workspace": self.workspace, "pane": self.pane, "lines": ANSWER_SCREEN }),
                self.port,
            )
            .await?;
            Ok(read["text"].as_str().unwrap_or("").to_string())
        })
    }

    fn type_step(&self, step: AnswerStep) -> Io<'_, Result<(), String>> {
        Box::pin(async move {
            let mut args = json!({
                "workspace": self.workspace,
                "pane": self.pane,
                "keys": step.keys,
                "submit": false,
            });
            if let Some(text) = step.text {
                args["text"] = json!(text);
            }
            run_as_person("pane_send", args, self.port)
                .await
                .map(|_| ())
        })
    }

    fn sleep(&self, ms: u64) -> Io<'_, ()> {
        self.port.sleep(ms)
    }
}

/// Marks a pane as being answered for as long as it lives.
pub struct Answering<'a> {
    mobile: &'a Mobile,
    pane: String,
}

impl<'a> Answering<'a> {
    /// The pane marked, unless an answer is being typed into it already.
    pub fn start(mobile: &'a Mobile, pane: &str) -> Option<Answering<'a>> {
        mobile
            .book()
            .answering
            .insert(pane.to_string())
            .then(|| Answering {
                mobile,
                pane: pane.to_string(),
            })
    }
}

impl Drop for Answering<'_> {
    fn drop(&mut self) {
        self.mobile.book().answering.remove(&self.pane);
    }
}

/// `remote_answer {pane, toolUseId, answers}`: answers the question an agent
/// pane waits on, through its dialog on the PC (`answer_pane`).
pub async fn remote_answer(
    port: &Port,
    mobile: &Mobile,
    args: &Value,
) -> Result<RemoteAnswered, String> {
    let (ws, pane) = find_pane(&port.snapshot().state, args.get("pane"))?;
    let tool_use_id = args.get("toolUseId").and_then(Value::as_str).unwrap_or("");
    answer_pane(port, mobile, &ws, &pane, tool_use_id, |questions| {
        choices_of(questions, args.get("answers"))
    })
    .await
}

/// Answers the question `tool_use_id` an agent pane waits on, through its
/// dialog on the PC: only the one its side channel reports open, or its
/// transcript shows unanswered, so a screen showing an older one is
/// refused. `choose` holds the answers against its questions. A pane not up
/// yet — asleep, or resuming — gets the answers in the queue.
pub async fn answer_pane(
    port: &Port,
    mobile: &Mobile,
    ws: &Workspace,
    pane: &Pane,
    tool_use_id: &str,
    choose: impl FnOnce(&[Question]) -> Result<Vec<Choice>, String>,
) -> Result<RemoteAnswered, String> {
    if tool_use_id.is_empty() {
        return Err("Name the question: its \"toolUseId\", from the chat.".into());
    }
    // An agent whose questions Hangar can answer, by its capabilities.
    let asks = pane
        .agent
        .kind()
        .map(|kind| kind.info())
        .filter(|info| info.caps.questions != Asks::None);
    let (agent, session_id) = match (asks, &pane.session_id) {
        (Some(info), Some(id)) if !id.is_empty() => (info.label, id.clone()),
        (Some(info), _) => {
            return Err(format!(
                "\"{}\" has no {} question to answer.",
                pane.name, info.label
            ));
        }
        (None, _) => {
            return Err(format!(
                "\"{}\" has no Claude Code question to answer.",
                pane.name
            ));
        }
    };
    let Some(_answering) = Answering::start(mobile, &pane.id) else {
        return Err(format!(
            "An answer is being typed into \"{}\" already.",
            pane.name
        ));
    };
    let cwd = pane
        .cwd
        .clone()
        .filter(|cwd| !cwd.is_empty())
        .unwrap_or_else(|| ws.cwd.clone());
    // On its node for a remote pane: the question is checked against the
    // transcript the pane writes.
    let read = ChatRead {
        session_id,
        cwd,
        before: None,
        limit: None,
    };
    let page = transcript_page(mobile, ws, pane, read).await?;
    let live = mobile.book().chat.question(&pane.id);
    let pending = live.or_else(|| page.as_ref().and_then(|page| page.question.clone()));
    let items = page
        .as_ref()
        .map(|page| page.items.as_slice())
        .unwrap_or_default();
    let questions = pending_questions_for(agent, pending.as_ref(), items, tool_use_id)?;
    let choices = choose(&questions)?;
    // The ask the pane's side channel holds open for this question takes
    // the answers as they are: nothing typed, nothing read back from the
    // screen.
    if let Some(ask) = held::question(ws, pane, tool_use_id, &questions)
        && held::give(
            port,
            pane,
            &ask,
            held::answers_for(&ask, &questions, &choices),
        )
        .await
    {
        return Ok(RemoteAnswered {
            answered: true,
            queued: None,
        });
    }
    if resumes(port, mobile, pane) {
        let text = answer_text(&questions, &choices);
        hold(port, mobile, ws, pane, Some(&text)).await?;
        return Ok(RemoteAnswered {
            answered: true,
            queued: Some(true),
        });
    }
    let dialogs = Dialogs::of(pane);
    if port.terminal(&pane.id).is_none() {
        return Err(match dialogs {
            Some(Dialogs::Codex) => CODEX_NOT_ON_SCREEN,
            _ => NOT_ON_SCREEN,
        }
        .into());
    }
    let io = PaneIo::new(port, ws, pane);
    match dialogs {
        Some(Dialogs::Codex) => {
            let input = mobile.book().chat.question_input(&pane.id, tool_use_id);
            let others = codex_others(input.as_ref(), questions.len());
            type_codex_answers(&io, &questions, &choices, &others).await?;
        }
        _ => type_answers(&io, &questions, &choices).await?,
    }
    Ok(RemoteAnswered {
        answered: true,
        queued: None,
    })
}

/// The screens of a real three-question prompt replayed, step by step, as
/// Claude Code 2.1.319 draws them in a 160-column pane: long questions and
/// descriptions wrapped, French words, a tab bar above each question and the
/// review at the end. What the window types into them is held against the
/// keys that dialog takes.
#[cfg(test)]
mod replay {
    use std::sync::Mutex;

    use hangar_core::chat::QuestionOption;

    use super::*;
    use crate::mobile::engine::locked;

    fn option(label: &str, description: &str) -> QuestionOption {
        QuestionOption {
            label: label.into(),
            description: description.into(),
        }
    }

    const Q1: &str = "Le moteur d'un serveur GMod (srcds) et son Lua tournent sur un seul thread, et le jeu ne peut pas être rendu entièrement multithread sans réécrire le moteur. Quelle approche veux-tu ?";
    const Q2: &str = "Sur quelle plateforme le serveur tourne-t-il ?";
    const Q3: &str =
        "Qu'est-ce qui ralentit ton serveur aujourd'hui ? Ça me dit quoi optimiser en premier.";

    /// The person's prompt: long descriptions on the first, a recommended
    /// option first on the second, several answers on the third.
    fn questions() -> Vec<Question> {
        vec![
            Question {
                question: Q1.into(),
                header: "Approche".into(),
                multi_select: false,
                options: vec![
                    option(
                        "Module workers Lua (Recommandé)",
                        "Module binaire C++ (gmsv_*.dll/.so) : un pool de threads, chacun avec son propre état Lua isolé, qui échange des messages avec le thread principal (comme les Web Workers). Les addons y délèguent les calculs lourds. Faisable et stable.",
                    ),
                    option(
                        "Opérations async natives",
                        "Module C++ qui exécute certaines opérations lentes sur des threads (fichiers, SQL, HTTP, JSON, compression) et rappelle un callback Lua. Plus simple, mais limité à ces opérations.",
                    ),
                    option(
                        "Les deux",
                        "Les workers Lua plus une bibliothèque d'opérations async. Plus long.",
                    ),
                    option(
                        "Patch du moteur",
                        "Hooker srcds pour paralléliser la physique ou les Think des entités. Il faut faire du reverse engineering, c'est très instable et ça casse à chaque mise à jour de GMod. Déconseillé.",
                    ),
                ],
            },
            Question {
                question: Q2.into(),
                header: "Plateforme".into(),
                multi_select: false,
                options: vec![
                    option(
                        "Windows et Linux (Recommandé)",
                        "On compile les deux binaires (win64 et linux64), avec une branche x86-64 de GMod.",
                    ),
                    option(
                        "Linux seulement",
                        "Serveur dédié Linux (cas le plus courant pour un serveur hébergé).",
                    ),
                    option("Windows seulement", "Serveur dédié Windows."),
                ],
            },
            Question {
                question: Q3.into(),
                header: "Priorité".into(),
                multi_select: true,
                options: vec![
                    option(
                        "Calculs Lua lourds",
                        "Pathfinding, IA, génération, boucles sur beaucoup d'entités.",
                    ),
                    option(
                        "I/O et base de données",
                        "SQL, fichiers, requêtes HTTP qui bloquent le tick.",
                    ),
                    option(
                        "Rien de précis",
                        "C'est une bibliothèque générique pour les développeurs d'addons.",
                    ),
                ],
            },
        ]
    }

    const WIDTH: usize = 160;

    fn rule() -> String {
        "─".repeat(WIDTH)
    }

    /// Words wrapped as Ink wraps them, each line after the first indented.
    fn wrapped(words: &str, indent: &str, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in words.split(' ') {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
        lines
            .into_iter()
            .map(|line| format!("{indent}{line}"))
            .collect()
    }

    /// The conversation above the dialog, as the pane shows it.
    fn above() -> Vec<String> {
        vec![
            "● Avant de choisir, trois questions.".into(),
            String::new(),
            "● User answered Claude's questions:".into(),
            "  ⎿  · Une ancienne question → Oui".into(),
            String::new(),
        ]
    }

    fn tabs(done: &[bool]) -> String {
        let headers = ["Approche", "Plateforme", "Priorité"];
        let tabs: Vec<String> = headers
            .iter()
            .zip(done)
            .map(|(header, done)| format!(" {} {header} ", if *done { "☒" } else { "☐" }))
            .collect();
        format!("← {} ✔ Submit  →", tabs.join(""))
    }

    /// Question `at` as the dialog draws it: the cursor on row `cursor`
    /// (0-based), the options of a multi-select question ticked by `ticked`.
    fn question_screen(
        questions: &[Question],
        at: usize,
        done: &[bool],
        cursor: usize,
        ticked: &[usize],
        typed: &str,
    ) -> String {
        let q = &questions[at];
        let mut lines = above();
        lines.push(rule());
        lines.push(tabs(done));
        lines.push(String::new());
        lines.extend(wrapped(&q.question, "", WIDTH));
        lines.push(String::new());
        let mark = |row: usize| if row == cursor { "❯" } else { " " };
        for (index, option) in q.options.iter().enumerate() {
            let boxed = match (q.multi_select, ticked.contains(&index)) {
                (false, _) => String::new(),
                (true, true) => "[✔] ".into(),
                (true, false) => "[ ] ".into(),
            };
            lines.push(format!(
                "{} {}. {boxed}{}",
                mark(index),
                index + 1,
                option.label
            ));
            lines.extend(wrapped(&option.description, "     ", WIDTH - 5));
        }
        let type_row = q.options.len();
        if q.multi_select {
            let words = if typed.is_empty() {
                "Type something"
            } else {
                typed
            };
            lines.push(format!("{} {}. [ ] {words}", mark(type_row), type_row + 1));
            let next = if at + 1 == questions.len() {
                "Submit"
            } else {
                "Next"
            };
            lines.push(format!("{}    {next}", mark(type_row + 1)));
            lines.push(rule());
            lines.push(format!("  {}. Chat about this", type_row + 2));
        } else {
            let words = if typed.is_empty() {
                "Type something."
            } else {
                typed
            };
            lines.push(format!("{} {}. {words}", mark(type_row), type_row + 1));
            lines.push(rule());
            lines.push(format!("  {}. Chat about this", type_row + 2));
        }
        lines.push(String::new());
        lines.push("Enter to select · Tab/Arrow keys to navigate · Esc to cancel".into());
        lines.join("\n")
    }

    /// The review: each question, wrapped, and its answer.
    fn review_screen(questions: &[Question], said: &[&str]) -> String {
        let mut lines = above();
        lines.push(rule());
        lines.push(rule());
        lines.push(tabs(&[true, true, true]));
        lines.push(String::new());
        lines.push("Review your answers".into());
        lines.push(String::new());
        for (q, said) in questions.iter().zip(said) {
            let mut asked = wrapped(&q.question, "   ", WIDTH - 3);
            asked[0] = format!(" ● {}", asked[0].trim_start());
            lines.extend(asked);
            lines.push(format!("   → {said}"));
        }
        lines.push(String::new());
        lines.push("Ready to submit your answers?".into());
        lines.push(String::new());
        lines.push("❯ 1. Submit answers".into());
        lines.push("  2. Cancel".into());
        lines.join("\n")
    }

    /// The screens replayed: the one shown after each count of steps typed
    /// (the last one past the end).
    struct Replay {
        screens: Vec<(usize, String)>,
        typed: Mutex<Vec<AnswerStep>>,
    }

    impl Replay {
        fn typed(&self) -> Vec<AnswerStep> {
            locked(&self.typed).clone()
        }
    }

    impl AnswerIo for Replay {
        fn read(&self) -> Io<'_, Result<String, String>> {
            let count = locked(&self.typed).len();
            let screen = self
                .screens
                .iter()
                .rev()
                .find(|(after, _)| *after <= count)
                .map(|(_, screen)| screen.clone())
                .unwrap_or_default();
            Box::pin(async move { Ok(screen) })
        }

        fn type_step(&self, step: AnswerStep) -> Io<'_, Result<(), String>> {
            locked(&self.typed).push(step);
            Box::pin(async { Ok(()) })
        }

        fn sleep(&self, _ms: u64) -> Io<'_, ()> {
            Box::pin(async {})
        }
    }

    fn keys(names: &[&str]) -> AnswerStep {
        AnswerStep {
            text: None,
            keys: names.iter().map(|name| (*name).to_string()).collect(),
        }
    }

    const OWN: &str = "on va réécrire le moteur";

    /// The window's answers: its own words on the first, the recommended
    /// option on the second, two on the third.
    fn given() -> Vec<Answer> {
        vec![
            Answer {
                selected: vec![],
                other: Some(OWN.into()),
            },
            Answer {
                selected: vec!["Windows et Linux (Recommandé)".into()],
                other: None,
            },
            Answer {
                selected: vec!["I/O et base de données".into(), "Calculs Lua lourds".into()],
                other: None,
            },
        ]
    }

    fn replay(review: &str) -> Replay {
        let questions = questions();
        let screens = vec![
            (0, question_screen(&questions, 0, &[false; 3], 0, &[], "")),
            (1, question_screen(&questions, 0, &[false; 3], 4, &[], "")),
            (
                2,
                question_screen(&questions, 1, &[true, false, false], 0, &[], ""),
            ),
            (
                3,
                question_screen(&questions, 2, &[true, true, false], 0, &[], ""),
            ),
            (4, review.to_string()),
            (5, String::new()),
        ];
        Replay {
            screens,
            typed: Mutex::new(Vec::new()),
        }
    }

    #[test]
    fn the_first_question_is_found_as_drawn_wrapped_and_in_french() {
        let questions = questions();
        let screen = question_screen(&questions, 0, &[false; 3], 0, &[], "");
        let lines = dialog_of(&screen, &questions).expect("the dialog under its tab bar");
        assert!(untouched(&lines, &questions[0]));
        // The tab of another question does not stand for this one.
        assert!(!untouched(&lines, &questions[1]));
        let moved = question_screen(&questions, 0, &[false; 3], 1, &[], "");
        let lines = dialog_of(&moved, &questions).unwrap();
        assert!(
            !untouched(&lines, &questions[0]),
            "someone moved the cursor"
        );
    }

    /// Claude Code 2.1.322 draws a bar down the left of a question's lines,
    /// read from a 160-column pane. A question long enough to wrap had a bar
    /// in the middle of its words, and was taken for one someone was
    /// answering at the PC: no answer typed, to any long question.
    #[test]
    fn a_wrapped_question_behind_its_bar_is_found_as_drawn() {
        let questions = questions();
        let screen = [
            "❯ Mon serveur GMod rame",
            "",
            "────────────────────────────────────────────────────────────────────",
            "←  ☐ Approche  ☐ Plateforme  ☐ Priorité  ✔ Submit  →",
            "",
            "│ Le moteur d'un serveur GMod (srcds) et son Lua tournent sur un seul thread, et le jeu ne peut pas être rendu entièrement multithread sans réécrire le moteur.",
            "│ Quelle approche veux-tu ?",
            "",
            "❯ 1. Module workers Lua (Recommandé)",
            "     Module binaire C++ (gmsv_*.dll/.so) : un pool de threads, chacun avec son propre état Lua isolé, qui échange des messages avec le thread principal (comme",
            "     les Web Workers). Les addons y délèguent les calculs lourds. Faisable et stable.",
            "  2. Opérations async natives",
            "     Module C++ qui exécute certaines opérations lentes sur des threads (fichiers, SQL, HTTP, JSON, compression) et rappelle un callback Lua. Plus simple, mais",
            "     limité à ces opérations.",
            "  3. Les deux",
            "     Les workers Lua plus une bibliothèque d'opérations async. Plus long.",
            "  4. Patch du moteur",
            "     Hooker srcds pour paralléliser la physique ou les Think des entités. Il faut faire du reverse engineering, c'est très instable et ça casse à chaque mise à",
            "     jour de GMod. Déconseillé.",
            "  5. Type something.",
            "────────────────────────────────────────────────────────────────────",
            "  6. Chat about this",
            "",
            "Enter to select · Tab/Arrow keys to navigate · Esc to cancel",
        ]
        .join("\n");
        let lines = dialog_of(&screen, &questions).expect("the dialog under its tab bar");
        assert!(untouched(&lines, &questions[0]));
        assert!(!untouched(&lines, &questions[1]));
    }

    #[tokio::test]
    async fn three_questions_own_words_a_default_and_several_answers_typed_then_submitted() {
        let questions = questions();
        let choices = choices_from(&questions, &given()).unwrap();
        assert_eq!(
            choices,
            [
                Choice {
                    picked: vec![],
                    other: Some(OWN.into())
                },
                Choice {
                    picked: vec![0],
                    other: None
                },
                Choice {
                    picked: vec![0, 1],
                    other: None
                },
            ]
        );
        let review = review_screen(
            &questions,
            &[
                OWN,
                "Windows et Linux (Recommandé)",
                "Calculs Lua lourds, I/O et base de données",
            ],
        );
        let io = replay(&review);
        type_answers(&io, &questions, &choices).await.unwrap();
        assert_eq!(
            io.typed(),
            [
                // 1: down to "Type something.", the words, Enter.
                keys(&["down", "down", "down", "down"]),
                AnswerStep {
                    text: Some(OWN.into()),
                    keys: vec!["enter".into()],
                },
                // 2: the first option, under the cursor.
                keys(&["enter"]),
                // 3: tick the first two, down to "Submit".
                keys(&["enter", "down", "enter", "down", "down", "down", "enter"]),
                // The review's "Submit answers".
                keys(&["enter"]),
            ]
        );
    }

    #[tokio::test]
    async fn a_review_that_shows_another_answer_is_not_submitted() {
        let questions = questions();
        let choices = choices_from(&questions, &given()).unwrap();
        let review = review_screen(
            &questions,
            &[
                OWN,
                "Linux seulement",
                "Calculs Lua lourds, I/O et base de données",
            ],
        );
        let io = replay(&review);
        assert_eq!(
            type_answers(&io, &questions, &choices).await,
            Err(NOT_REVIEWED.to_string())
        );
        assert_eq!(io.typed().len(), 4, "everything but the Enter that submits");
    }

    #[tokio::test]
    async fn a_question_someone_is_answering_at_the_pc_gets_nothing_typed() {
        let questions = questions();
        let choices = choices_from(&questions, &given()).unwrap();
        let io = Replay {
            screens: vec![(0, question_screen(&questions, 0, &[false; 3], 2, &[], ""))],
            typed: Mutex::new(Vec::new()),
        };
        assert_eq!(
            type_answers(&io, &questions, &choices).await,
            Err(NOT_ON_SCREEN.to_string())
        );
        assert!(io.typed().is_empty());
    }
}

/// Codex's own dialogs, as its TUI draws them and takes their keys at
/// rust-v0.170.0. From the source: the screens are those its snapshot tests
/// keep (codex-rs/tui/src/chatwidget/snapshots/ and
/// codex-rs/tui/src/bottom_pane/{,request_user_input/}snapshots/, named at
/// each), and the dialogs below move their cursor, take a paste and answer
/// as `approval_overlay.rs` and `request_user_input/mod.rs` do in their
/// `handle_key_event`. No Codex was run for them (docs/native/agents.md
/// §4.0, §7 decision 1).
#[cfg(test)]
mod codex_floor {
    use std::sync::Mutex;

    use hangar_core::chat::QuestionOption;

    use super::*;
    use crate::mobile::engine::locked;

    /// The lines of a snapshot's screen, its trailing blanks dropped.
    fn lines_of(screen: &str) -> Vec<String> {
        let mut lines: Vec<String> = screen
            .split('\n')
            .map(|line| line.trim_end().to_string())
            .collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        while lines.first().is_some_and(String::is_empty) {
            lines.remove(0);
        }
        lines
    }

    /// From the source: chatwidget/snapshots/
    /// codex_tui__chatwidget__tests__approval_modal_exec.snap.
    const EXEC: &str = "

  Would you like to run the following command?

  Reason: this is a test reason such as one that would be produced by the model

  $ echo hello world


› 1. Yes, proceed (y)
  2. Yes, and don't ask again for commands that start with `echo hello world` (p)
  3. No, and tell Codex what to do differently (esc)

  Press enter to confirm or esc to cancel";

    /// From the source: chatwidget/snapshots/
    /// codex_tui__chatwidget__tests__approval_modal_patch.snap.
    const PATCH: &str = "

  Would you like to make the following edits?

  Description: The model wants to apply changes
  Destination: /tmp/project/README.md


› 1. Yes, proceed (y)
  2. Yes, and don't ask again for these files (a)
  3. No, and tell Codex what to do differently (esc)

  Press enter to confirm or esc to cancel";

    /// From the source: bottom_pane/snapshots/
    /// codex_tui__bottom_pane__approval_overlay__tests__approval_overlay_permissions_prompt.snap.
    const PERMISSIONS: &str = "
  Would you like to grant these permissions?

  Reason: need workspace access

  Permission rule: network; read `/tmp/readme.txt`; write `/tmp/out.txt`


› 1. Yes, grant these permissions for this turn (y)
  2. Yes, grant for this turn with strict auto review (r)
  3. Yes, grant these permissions for this session (a)
  4. No, continue without permissions (d)

  Press enter to confirm or esc to cancel";

    /// From the source: bottom_pane/snapshots/
    /// codex_tui__bottom_pane__approval_overlay__tests__network_exec_prompt.snap.
    const NETWORK: &str = "
  Do you want to approve network access to \"example.com\"?

  Reason: network request blocked



› 1. Yes, just this once (y)
  2. Yes, and allow this host for this conversation (a)
  3. Yes, and allow this host in the future (p)
  4. No, and tell Codex what to do differently (esc)

  Press enter to confirm or esc to cancel";

    /// The conversation above a dialog, Codex's way: the person's prompt
    /// marked `›` as the dialogs mark their cursor.
    const ABOVE: &str = "› Release 2.5\n\n• Running the release checks.\n";

    /// A snapshot's screen below the conversation.
    fn on_screen(screen: &str) -> String {
        format!("{ABOVE}\n{}", lines_of(screen).join("\n"))
    }

    /// Codex's approval dialog (`approval_overlay.rs`): drawn as the
    /// snapshot `screen` shows it, the cursor moved by Down, Enter taking
    /// the option under it, Escape cancelling. With `stuck`, the keys never
    /// reach it.
    struct Approval {
        screen: Vec<String>,
        stuck: bool,
        state: Mutex<ApprovalState>,
    }

    #[derive(Default)]
    struct ApprovalState {
        cursor: usize,
        typed: Vec<AnswerStep>,
        /// The option taken, or "cancel".
        outcome: Option<String>,
    }

    impl Approval {
        fn new(screen: &str) -> Approval {
            Approval {
                screen: lines_of(screen),
                stuck: false,
                state: Mutex::new(ApprovalState::default()),
            }
        }

        fn options(&self) -> Vec<String> {
            codex_rows(&self.screen.iter().map(String::as_str).collect::<Vec<_>>())
                .into_iter()
                .map(|row| CODEX_SHORTCUT.replace(&row.text, "").into_owned())
                .collect()
        }

        fn draw(&self) -> String {
            let state = locked(&self.state);
            if state.outcome.is_some() {
                return format!("{ABOVE}\n› ");
            }
            let mut row = 0;
            let dialog: Vec<String> = self
                .screen
                .iter()
                .map(|line| match CODEX_ROW.captures(line) {
                    Some(caps) => {
                        let mark = if row == state.cursor { "›" } else { " " };
                        row += 1;
                        format!("{mark} {}. {}", &caps[2], &caps[3])
                    }
                    None => line.clone(),
                })
                .collect();
            format!("{ABOVE}\n{}", dialog.join("\n"))
        }

        fn typed(&self) -> Vec<AnswerStep> {
            locked(&self.state).typed.clone()
        }

        fn outcome(&self) -> Option<String> {
            locked(&self.state).outcome.clone()
        }
    }

    impl AnswerIo for Approval {
        fn read(&self) -> Io<'_, Result<String, String>> {
            let screen = self.draw();
            Box::pin(async move { Ok(screen) })
        }

        fn type_step(&self, step: AnswerStep) -> Io<'_, Result<(), String>> {
            let options = self.options();
            let mut state = locked(&self.state);
            state.typed.push(step.clone());
            if !self.stuck && state.outcome.is_none() {
                for key in &step.keys {
                    match key.as_str() {
                        "down" => state.cursor = (state.cursor + 1) % options.len(),
                        "up" => state.cursor = (state.cursor + options.len() - 1) % options.len(),
                        "enter" => state.outcome = Some(options[state.cursor].clone()),
                        "escape" => state.outcome = Some("cancel".into()),
                        _ => {}
                    }
                }
            }
            Box::pin(async { Ok(()) })
        }

        fn sleep(&self, _ms: u64) -> Io<'_, ()> {
            Box::pin(async {})
        }
    }

    fn keys(names: &[&str]) -> AnswerStep {
        AnswerStep {
            text: None,
            keys: names.iter().map(|name| (*name).to_string()).collect(),
        }
    }

    const ONCE: Permit = Permit::Allow { session: false };
    const SESSION: Permit = Permit::Allow { session: true };

    #[test]
    fn the_snapshots_read_as_codex_s_dialog_as_first_drawn() {
        for screen in [EXEC, PATCH, PERMISSIONS, NETWORK] {
            let drawn = on_screen(screen);
            let rows = codex_approval_untouched(&drawn).expect("the dialog, untouched");
            assert_eq!(rows[0].n, Some(1));
            // The simulated dialog draws the snapshot itself.
            assert_eq!(
                lines_of(&Approval::new(screen).draw()),
                lines_of(&drawn),
                "{screen}"
            );
        }
        // A conversation without the dialog, a cursor someone moved.
        assert!(codex_approval_untouched(ABOVE).is_none());
        let moved = EXEC
            .replace("› 1. Yes, proceed", "  1. Yes, proceed")
            .replace("  3. No, and tell", "› 3. No, and tell");
        assert!(codex_approval_untouched(&moved).is_none());
        // Its footer gone: not the dialog any more.
        let cut = EXEC.replace(CODEX_CONFIRM, "");
        assert!(codex_approval(&cut).is_none());
    }

    #[tokio::test]
    async fn an_allow_is_enter_on_yes_proceed_checked_under_the_cursor() {
        for screen in [EXEC, PATCH] {
            let dialog = Approval::new(screen);
            press_codex_approval(&dialog, &ONCE).await.unwrap();
            assert_eq!(dialog.typed(), [keys(&["enter"])]);
            assert_eq!(dialog.outcome().as_deref(), Some("Yes, proceed"));
        }
        let network = Approval::new(NETWORK);
        press_codex_approval(&network, &ONCE).await.unwrap();
        assert_eq!(network.outcome().as_deref(), Some("Yes, just this once"));
    }

    #[tokio::test]
    async fn an_allow_for_the_session_finds_its_option_by_label() {
        let permissions = Approval::new(PERMISSIONS);
        press_codex_approval(&permissions, &SESSION).await.unwrap();
        assert_eq!(
            permissions.typed(),
            [keys(&["down", "down"]), keys(&["enter"])]
        );
        assert_eq!(
            permissions.outcome().as_deref(),
            Some("Yes, grant these permissions for this session")
        );

        let patch = Approval::new(PATCH);
        press_codex_approval(&patch, &SESSION).await.unwrap();
        assert_eq!(
            patch.outcome().as_deref(),
            Some("Yes, and don't ask again for these files")
        );

        let network = Approval::new(NETWORK);
        press_codex_approval(&network, &SESSION).await.unwrap();
        assert_eq!(
            network.outcome().as_deref(),
            Some("Yes, and allow this host for this conversation")
        );
    }

    #[tokio::test]
    async fn a_command_codex_lets_through_only_by_prefix_is_not_allowed_for_the_session() {
        // Its second option is a rule kept beyond the session: not this.
        let exec = Approval::new(EXEC);
        assert_eq!(
            press_codex_approval(&exec, &SESSION).await,
            Err(CODEX_NO_SESSION.to_string())
        );
        assert!(exec.typed().is_empty());
        assert_eq!(exec.outcome(), None);
    }

    #[tokio::test]
    async fn a_no_is_escape_and_the_words_wait_for_the_dialog_to_go() {
        let exec = Approval::new(EXEC);
        let no = Permit::Deny {
            words: Some("use cargo test".into()),
        };
        press_codex_approval(&exec, &no).await.unwrap();
        assert_eq!(exec.typed(), [keys(&["escape"])]);
        assert_eq!(exec.outcome().as_deref(), Some("cancel"));
        assert_eq!(codex_approval_gone(&exec).await, Ok(()));

        let stuck = Approval {
            stuck: true,
            ..Approval::new(PATCH)
        };
        press_codex_approval(&stuck, &no).await.unwrap();
        assert_eq!(
            codex_approval_gone(&stuck).await,
            Err(CODEX_STILL_UP.to_string())
        );
    }

    #[tokio::test]
    async fn nothing_is_typed_unless_the_screen_shows_the_dialog_as_first_drawn() {
        let moved = Approval::new(EXEC);
        locked(&moved.state).cursor = 2;
        assert_eq!(
            press_codex_approval(&moved, &Permit::Deny { words: None }).await,
            Err(CODEX_NOT_ON_SCREEN.to_string())
        );
        assert!(moved.typed().is_empty());

        // The Down lost on the way: the cursor is not on the option asked,
        // and no Enter answers whatever it is on.
        let stuck = Approval {
            stuck: true,
            ..Approval::new(PERMISSIONS)
        };
        assert_eq!(
            press_codex_approval(&stuck, &SESSION).await,
            Err(CODEX_NOT_CHOSEN.to_string())
        );
        assert_eq!(stuck.typed(), [keys(&["down", "down"])]);
    }

    /* ── the question dialog ──────────────────────────────────── */

    /// From the source: bottom_pane/request_user_input/snapshots/
    /// codex_tui__bottom_pane__request_user_input__tests__request_user_input_options.snap.
    const OPTIONS: &str = "
  Question 1/1 (1 unanswered)
  Choose an option.

  › 1. Option 1           First choice.
    2. Option 2           Second choice.
    3. Option 3           Third choice.
    4. None of the above  Optionally, add details in notes (tab)

  tab to add notes | enter to submit answer | esc to interrupt";

    /// From the source: …request_user_input_multi_question_first.snap.
    const MULTI_FIRST: &str = "
  Question 1/2 (2 unanswered)
  Choose an option.

  › 1. Option 1  First choice.
    2. Option 2  Second choice.
    3. Option 3  Third choice.

  tab to add notes | enter to submit answer | ←/→ to navigate questions | esc to interrupt";

    /// From the source: …request_user_input_freeform.snap.
    const FREEFORM: &str = "
  Question 1/1 (1 unanswered)
  Share details.

  › Type your answer (optional)



  enter to submit answer | esc to interrupt";

    /// A question as Codex asks it: its options, and whether it offers
    /// "None of the above" (`isOther`).
    struct Asked {
        question: Question,
        other: bool,
    }

    fn asked(text: &str, labels: &[(&str, &str)], other: bool) -> Asked {
        Asked {
            question: Question {
                question: text.into(),
                header: String::new(),
                multi_select: false,
                options: labels
                    .iter()
                    .map(|(label, description)| QuestionOption {
                        label: (*label).into(),
                        description: (*description).into(),
                    })
                    .collect(),
            },
            other,
        }
    }

    fn options(other: bool) -> Asked {
        asked(
            "Choose an option.",
            &[
                ("Option 1", "First choice."),
                ("Option 2", "Second choice."),
                ("Option 3", "Third choice."),
            ],
            other,
        )
    }

    fn freeform() -> Asked {
        asked("Share details.", &[], false)
    }

    /// One question's state in the dialog.
    #[derive(Clone, Default)]
    struct QuestionState {
        cursor: usize,
        committed: bool,
        notes: String,
        notes_open: bool,
    }

    /// Codex's question dialog (`request_user_input/mod.rs`): drawn as its
    /// snapshots show it, Down moving the cursor, Enter committing the
    /// option under it and going on (or opening the notes on "None of the
    /// above", or committing the notes), a paste landing in the notes; the
    /// last Enter submits the answers as Codex sends them
    /// (`submit_answers`), or asks to confirm the questions unanswered.
    struct QuestionDialog {
        asked: Vec<Asked>,
        state: Mutex<DialogState>,
    }

    #[derive(Default)]
    struct DialogState {
        at: usize,
        notes_focus: bool,
        answers: Vec<QuestionState>,
        typed: Vec<AnswerStep>,
        submitted: Option<Vec<Vec<String>>>,
        confirming: bool,
    }

    impl QuestionDialog {
        fn new(asked: Vec<Asked>) -> QuestionDialog {
            let answers = asked
                .iter()
                .map(|asked| QuestionState {
                    notes_open: asked.question.options.is_empty(),
                    ..QuestionState::default()
                })
                .collect();
            let notes_focus = asked[0].question.options.is_empty();
            QuestionDialog {
                asked,
                state: Mutex::new(DialogState {
                    answers,
                    notes_focus,
                    ..DialogState::default()
                }),
            }
        }

        fn rows(&self, at: usize) -> usize {
            let asked = &self.asked[at];
            asked.question.options.len() + usize::from(asked.other)
        }

        fn draw(&self) -> String {
            let state = locked(&self.state);
            if state.submitted.is_some() {
                return format!("{ABOVE}\n› ");
            }
            if state.confirming {
                return format!(
                    "{ABOVE}\n  {CODEX_CONFIRM_UNANSWERED}\n  1 unanswered question\n\n  › 1. Proceed  Submit with 1 unanswered question\n    2. Go back  Return to the first unanswered question\n\n  Press enter to confirm or esc to go back"
                );
            }
            let asked = &self.asked[state.at];
            let answer = &state.answers[state.at];
            let total = self.asked.len();
            let unanswered = state.answers.iter().filter(|a| !a.committed).count();
            let mut lines = vec![if unanswered > 0 {
                format!(
                    "  Question {}/{total} ({unanswered} unanswered)",
                    state.at + 1
                )
            } else {
                format!("  Question {}/{total}", state.at + 1)
            }];
            lines.push(format!("  {}", asked.question.question));
            lines.push(String::new());
            let mut rows: Vec<(String, String)> = asked
                .question
                .options
                .iter()
                .map(|option| (option.label.clone(), option.description.clone()))
                .collect();
            if asked.other {
                rows.push((
                    CODEX_OTHER.to_string(),
                    "Optionally, add details in notes (tab)".to_string(),
                ));
            }
            let width = rows.iter().map(|(label, _)| label.chars().count()).max();
            for (at, (label, description)) in rows.iter().enumerate() {
                let mark = if at == answer.cursor { "›" } else { " " };
                let width = width.unwrap_or(0);
                lines.push(format!(
                    "  {mark} {}. {label:<width$}  {description}",
                    at + 1
                ));
            }
            let has_options = !rows.is_empty();
            if answer.notes_open {
                if has_options {
                    lines.push(String::new());
                }
                let shown = if answer.notes.is_empty() {
                    if has_options {
                        "Add notes"
                    } else {
                        CODEX_ANSWER_HINT
                    }
                } else {
                    &answer.notes
                };
                lines.push(format!("  › {shown}"));
                lines.extend([String::new(), String::new(), String::new()]);
            } else {
                lines.push(String::new());
            }
            let last = state.at + 1 == total;
            let submit = if total > 1 && last {
                "enter to submit all"
            } else {
                "enter to submit answer"
            };
            lines.push(match (has_options, answer.notes_open, total > 1) {
                (true, false, false) => format!("  tab to add notes | {submit} | esc to interrupt"),
                (true, false, true) => format!(
                    "  tab to add notes | {submit} | ←/→ to navigate questions | esc to interrupt"
                ),
                (true, true, _) => format!("  tab or esc to clear notes | {submit}"),
                (false, _, false) => format!("  {submit} | esc to interrupt"),
                (false, _, true) => {
                    format!("  {submit} | ⌃p / ⌃n change question | esc to interrupt")
                }
            });
            format!("{ABOVE}\n{}", lines.join("\n"))
        }

        fn go_on(&self, state: &mut DialogState) {
            if state.at + 1 < self.asked.len() {
                state.at += 1;
                state.notes_focus = self.asked[state.at].question.options.is_empty();
                return;
            }
            if state.answers.iter().any(|answer| !answer.committed) {
                state.confirming = true;
                return;
            }
            let submitted = self
                .asked
                .iter()
                .zip(&state.answers)
                .map(|(asked, answer)| {
                    let mut said = Vec::new();
                    if !asked.question.options.is_empty() {
                        said.push(if answer.cursor < asked.question.options.len() {
                            asked.question.options[answer.cursor].label.clone()
                        } else {
                            CODEX_OTHER.to_string()
                        });
                    }
                    if !answer.notes.trim().is_empty() {
                        said.push(format!("user_note: {}", answer.notes.trim()));
                    }
                    said
                })
                .collect();
            state.submitted = Some(submitted);
        }

        fn key(&self, state: &mut DialogState, key: &str) {
            let at = state.at;
            let rows = self.rows(at);
            let other_row = self.asked[at].other && state.answers[at].cursor + 1 == rows;
            match key {
                "down" if rows > 0 => {
                    let answer = &mut state.answers[at];
                    answer.cursor = (answer.cursor + 1) % rows;
                    answer.committed = false;
                }
                "enter" if !state.notes_focus && other_row => {
                    state.notes_focus = true;
                    state.answers[at].notes_open = true;
                }
                "enter" if state.notes_focus && rows == 0 && state.answers[at].notes.is_empty() => {
                    self.go_on(state);
                }
                "enter" => {
                    state.answers[at].committed = true;
                    self.go_on(state);
                }
                _ => {}
            }
        }

        fn typed(&self) -> Vec<AnswerStep> {
            locked(&self.state).typed.clone()
        }

        fn submitted(&self) -> Option<Vec<Vec<String>>> {
            locked(&self.state).submitted.clone()
        }
    }

    impl AnswerIo for QuestionDialog {
        fn read(&self) -> Io<'_, Result<String, String>> {
            let screen = self.draw();
            Box::pin(async move { Ok(screen) })
        }

        fn type_step(&self, step: AnswerStep) -> Io<'_, Result<(), String>> {
            let mut state = locked(&self.state);
            state.typed.push(step.clone());
            if state.submitted.is_none() && !state.confirming {
                if let Some(text) = &step.text {
                    let at = state.at;
                    state.notes_focus = true;
                    let answer = &mut state.answers[at];
                    answer.notes_open = true;
                    answer.notes.push_str(text);
                    answer.committed = false;
                }
                for key in &step.keys {
                    self.key(&mut state, key);
                }
            }
            Box::pin(async { Ok(()) })
        }

        fn sleep(&self, _ms: u64) -> Io<'_, ()> {
            Box::pin(async {})
        }
    }

    fn questions(dialog: &QuestionDialog) -> Vec<Question> {
        dialog
            .asked
            .iter()
            .map(|asked| asked.question.clone())
            .collect()
    }

    fn others(dialog: &QuestionDialog) -> Vec<Option<bool>> {
        dialog.asked.iter().map(|asked| Some(asked.other)).collect()
    }

    fn picked(at: usize) -> Choice {
        Choice {
            picked: vec![at],
            other: None,
        }
    }

    fn words(text: &str) -> Choice {
        Choice {
            picked: Vec::new(),
            other: Some(text.into()),
        }
    }

    #[test]
    fn the_question_snapshots_read_as_codex_s_dialog_as_first_drawn() {
        let cases = [
            (OPTIONS, vec![options(true)], Some(true)),
            (MULTI_FIRST, vec![options(false), freeform()], Some(false)),
            (FREEFORM, vec![freeform()], Some(false)),
        ];
        for (screen, asking, offers_other) in cases {
            let dialog = QuestionDialog::new(asking);
            // The simulated dialog draws the snapshot itself.
            assert_eq!(lines_of(&dialog.draw()), lines_of(&on_screen(screen)));
            let total = dialog.asked.len();
            let first = &dialog.asked[0].question;
            assert_eq!(
                codex_untouched(&dialog.draw(), 0, total, first),
                offers_other,
                "{screen}"
            );
            // Not the question it is at, nor another one's.
            assert_eq!(codex_untouched(&dialog.draw(), 1, total, first), None);
            let other = asked("Pick a size.", &[("S", ""), ("M", "")], false).question;
            assert_eq!(codex_untouched(&dialog.draw(), 0, total, &other), None);
        }
    }

    #[tokio::test]
    async fn an_option_then_the_person_s_words_are_typed_and_submitted_as_codex_sends_them() {
        let dialog = QuestionDialog::new(vec![options(false), freeform()]);
        let choices = [picked(1), words("ship it friday")];
        type_codex_answers(&dialog, &questions(&dialog), &choices, &others(&dialog))
            .await
            .unwrap();
        assert_eq!(
            dialog.typed(),
            [
                keys(&["down"]),
                keys(&["enter"]),
                AnswerStep {
                    text: Some("ship it friday".into()),
                    keys: Vec::new(),
                },
                keys(&["enter"]),
            ]
        );
        assert_eq!(
            dialog.submitted(),
            Some(vec![
                vec!["Option 2".to_string()],
                vec!["user_note: ship it friday".to_string()],
            ])
        );
    }

    #[tokio::test]
    async fn the_person_s_words_go_through_none_of_the_above_where_it_is_offered() {
        let dialog = QuestionDialog::new(vec![options(true)]);
        type_codex_answers(
            &dialog,
            &questions(&dialog),
            &[words("option 4, really")],
            &others(&dialog),
        )
        .await
        .unwrap();
        assert_eq!(
            dialog.typed(),
            [
                keys(&["down", "down", "down"]),
                keys(&["enter"]),
                AnswerStep {
                    text: Some("option 4, really".into()),
                    keys: Vec::new(),
                },
                keys(&["enter"]),
            ]
        );
        assert_eq!(
            dialog.submitted(),
            Some(vec![vec![
                CODEX_OTHER.to_string(),
                "user_note: option 4, really".to_string()
            ]])
        );
    }

    #[tokio::test]
    async fn words_where_the_dialog_takes_none_are_refused_before_any_key() {
        // Codex said the question offers no such row.
        let dialog = QuestionDialog::new(vec![options(false), options(false)]);
        let choices = [picked(0), words("neither")];
        assert_eq!(
            type_codex_answers(&dialog, &questions(&dialog), &choices, &others(&dialog)).await,
            Err(codex_no_words(1))
        );
        assert!(dialog.typed().is_empty());
        // Codex did not say: the screen does, before the question's keys.
        let unknown = [None, None];
        assert_eq!(
            type_codex_answers(
                &dialog,
                &questions(&dialog),
                &[words("neither"), picked(0)],
                &unknown
            )
            .await,
            Err(codex_no_words(0))
        );
        assert!(dialog.typed().is_empty());
    }

    #[tokio::test]
    async fn a_question_someone_is_answering_at_the_pc_gets_nothing_typed() {
        let dialog = QuestionDialog::new(vec![options(true)]);
        locked(&dialog.state).answers[0].cursor = 2;
        assert_eq!(
            type_codex_answers(&dialog, &questions(&dialog), &[picked(0)], &others(&dialog)).await,
            Err(CODEX_NOT_ON_SCREEN.to_string())
        );
        assert!(dialog.typed().is_empty());
        assert_eq!(dialog.submitted(), None);
    }

    #[test]
    fn what_codex_said_of_each_question_is_read_from_the_call() {
        let input = json!({ "questions": [
            { "id": "a", "question": "?", "isOther": true },
            { "id": "b", "question": "?" },
        ] });
        assert_eq!(codex_others(Some(&input), 3), [Some(true), None, None]);
        assert_eq!(codex_others(None, 1), [None]);
    }
}
