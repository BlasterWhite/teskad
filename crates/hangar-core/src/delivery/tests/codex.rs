//! Deliveries read by Codex (docs/native/agents.md §4.4, D11), end to end: a
//! repository, a worker's branch, and a fake `codex` on PATH.
//!
//! The fake is a script written here, not a recorded session: it answers
//! `codex exec --json` the way OpenAI's codex 0.170.0 does, from the source
//! (codex-rs/exec/src/exec_events.rs and
//! event_processor_with_jsonl_output.rs: the events it prints, the last
//! message `-o` gets once the turn completes, exit 1 when it failed).

use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;

use serde_json::{json, Value};

use super::*;
use crate::agent::AgentKind;
use crate::delivery::review_agent::{self, ReviewerChoice};
use crate::delivery::review_codex;

/// Keeps what it was given beside itself — its arguments, the prompt from
/// stdin, the schema — then prints `events`, writes `answer` where `-o`
/// says (empty when there is none, as exec leaves it after a failed turn),
/// says `stderr` and exits with `status`.
const SCRIPT: &str = r#"#!/bin/sh
# A fake `codex` for Hangar's tests, written from the source of codex 0.170.0.
here=$(dirname "$0")
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.170.0"
  exit 0
fi
printf '%s\n' "$@" > "$here/args"
answer=
schema=
while [ $# -gt 0 ]; do
  case "$1" in
    -o|--output-last-message) answer=$2; shift ;;
    --output-schema) schema=$2; shift ;;
  esac
  shift
done
cat > "$here/prompt"
if [ -n "$schema" ]; then cp "$schema" "$here/schema"; fi
if [ -f "$here/stderr" ]; then cat "$here/stderr" >&2; fi
cat "$here/events"
if [ -n "$answer" ]; then
  if [ -f "$here/answer" ]; then cp "$here/answer" "$answer"; else : > "$answer"; fi
fi
exit "$(cat "$here/status")"
"#;

struct FakeCodex {
    /// The folder `codex` is in: what PATH holds.
    bin: PathBuf,
}

impl FakeCodex {
    fn new(events: &[Value], answer: Option<&str>, status: i32) -> FakeCodex {
        let bin = crate::testdir::scratch("fake-codex");
        let script = bin.join("codex");
        fs::write(&script, SCRIPT).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let lines: String = events.iter().map(|event| format!("{event}\n")).collect();
        fs::write(bin.join("events"), lines).unwrap();
        if let Some(answer) = answer {
            fs::write(bin.join("answer"), answer).unwrap();
        }
        fs::write(bin.join("status"), status.to_string()).unwrap();
        // A process another test forks while the script is being written can
        // hold it open for a moment ("text file busy"): run it once first.
        for _ in 0..50 {
            match Command::new(&script).arg("--version").output() {
                Err(err) if err.raw_os_error() == Some(libc::ETXTBSY) => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                ran => {
                    assert!(ran.unwrap().status.success());
                    break;
                }
            }
        }
        FakeCodex { bin }
    }

    fn says_on_stderr(self, said: &str) -> FakeCodex {
        fs::write(self.bin.join("stderr"), said).unwrap();
        self
    }

    /// A PATH holding the fake and nothing else: no Claude Code on it.
    fn path(&self) -> OsString {
        std::env::join_paths([&self.bin]).unwrap()
    }

    /// The reviewer `choice` gets with that PATH.
    fn reviewer(&self, choice: &ReviewerChoice) -> Box<dyn Reviewer> {
        let path = self.path();
        review_agent::pick(choice, REVIEW_TIMEOUT, move |kind| on_path(&path, kind))
    }

    fn args(&self) -> Vec<String> {
        read(&self.bin, "args")
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn prompt(&self) -> String {
        read(&self.bin, "prompt")
    }

    fn schema(&self) -> Value {
        serde_json::from_str(&read(&self.bin, "schema")).expect("the schema Codex was given")
    }
}

/// Where `kind`'s command is on `path`, by any of its names: what a pane
/// started with that PATH runs.
fn on_path(path: &OsStr, kind: AgentKind) -> Option<PathBuf> {
    std::env::split_paths(path)
        .flat_map(|dir| {
            crate::agent::info(kind)
                .binaries
                .iter()
                .map(move |name| dir.join(name))
        })
        .find(|candidate| candidate.is_file())
}

fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .map(|at| args[at + 1].as_str())
}

const THREAD: &str = "0199a213-81c0-7800-8aa1-bbab2a035a53";

/// A review that read a file and answered `verdict`.
fn reviewed(verdict: &str) -> Vec<Value> {
    vec![
        json!({"type": "thread.started", "thread_id": THREAD}),
        json!({"type": "turn.started"}),
        json!({"type": "item.completed", "item": {"id": "item_0", "type": "reasoning", "text": "Reading the change in its context."}}),
        json!({"type": "item.started", "item": {"id": "item_1", "type": "command_execution", "command": "bash -lc 'cat b.rs'", "aggregated_output": "", "exit_code": null, "status": "in_progress"}}),
        json!({"type": "item.completed", "item": {"id": "item_1", "type": "command_execution", "command": "bash -lc 'cat b.rs'", "aggregated_output": "bee\n", "exit_code": 0, "status": "completed"}}),
        json!({"type": "item.completed", "item": {"id": "item_2", "type": "agent_message", "text": verdict}}),
        json!({"type": "turn.completed", "usage": {"input_tokens": 24763, "cached_input_tokens": 24448, "cache_write_input_tokens": 0, "output_tokens": 122, "reasoning_output_tokens": 0}}),
    ]
}

/// A turn that failed with `message`, reported as an error and as the turn's.
fn failed(message: &str) -> Vec<Value> {
    vec![
        json!({"type": "thread.started", "thread_id": THREAD}),
        json!({"type": "turn.started"}),
        json!({"type": "error", "message": message}),
        json!({"type": "turn.failed", "error": {"message": message}}),
    ]
}

/// A worker's branch adding b.rs.
fn delivered(tag: &str) -> Repo {
    let repo = repo(tag);
    write(&repo.worker, "b.rs", "bee\n");
    commit(&repo.worker, "Add b");
    repo
}

#[test]
fn a_delivery_codex_approves_lands_and_codex_only_read_it() {
    let repo = delivered("codex-approves");
    let verdict =
        json!({"approve": true, "blocking": [], "notes": "Adds b.rs as asked."}).to_string();
    let codex = FakeCodex::new(&reviewed(&verdict), Some(&verdict), 0);
    // Nothing chosen and no Claude Code on PATH: Codex reviews.
    let reviewer = codex.reviewer(&ReviewerChoice::default());

    let outcome = deliver_now(&request(&repo), reviewer.as_ref());

    assert_eq!(outcome.outcome, Kind::Landed, "{}", outcome.detail);
    assert_eq!(read(&repo.main, "b.rs"), "bee\n");
    let review = outcome.review.expect("the verdict");
    assert!(review.approve);
    assert_eq!(review.notes, "Adds b.rs as asked.");

    // Read-only, nobody asked, the verdict's schema, the prompt on stdin.
    let args = codex.args();
    assert_eq!(args[0], "exec");
    assert!(args.iter().any(|arg| arg == "--json"), "{args:?}");
    assert!(args.iter().any(|arg| arg == "--ephemeral"), "{args:?}");
    assert_eq!(after(&args, "--sandbox"), Some("read-only"));
    assert_eq!(after(&args, "-c"), Some("approval_policy=never"));
    assert!(!args.iter().any(|arg| arg == "-m"), "Codex's own model");
    assert_eq!(args.last().map(String::as_str), Some("-"));
    let schema = codex.schema();
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(schema["required"], json!(["approve", "blocking", "notes"]));
    assert_eq!(
        schema["properties"]["blocking"]["items"]["required"],
        json!(["file", "line", "problem"])
    );
    // The schema and the answer were the review's only, and went with it.
    let schema_file = PathBuf::from(after(&args, "--output-schema").unwrap());
    assert!(!schema_file.starts_with(&repo.root), "out of the worktree");
    assert!(!schema_file.exists());
    assert!(!Path::new(after(&args, "-o").unwrap()).exists());

    // The prompt every reviewer gets, with Codex's way of reading.
    let prompt = codex.prompt();
    assert!(prompt.contains("Add b.rs holding one line."), "{prompt}");
    assert!(prompt.contains("+bee"), "the diff is in the prompt");
    assert!(prompt.contains("Approve unless the change is wrong"));
    assert!(prompt.contains(review_codex::READING));
    assert!(!prompt.contains("with Read, and search with Grep"));
    assert_no_scratch(&repo);
}

#[test]
fn codex_s_blocking_items_send_the_delivery_back_with_where_and_what() {
    let repo = delivered("codex-blocks");
    let start = head(&repo.main, "main");
    let verdict = json!({
        "approve": false,
        "blocking": [
            {"file": "b.rs", "line": 1, "problem": "b.rs holds `bee`, the task asks for one line of Rust."},
            {"file": "src/app.ts", "line": null, "problem": "app is never told about b."},
        ],
        "notes": "",
    })
    .to_string();
    let codex = FakeCodex::new(&reviewed(&verdict), Some(&verdict), 0);
    let choice = ReviewerChoice {
        agent: Some(AgentKind::Codex),
        codex_model: Some("gpt-5.5".into()),
        codex_account: None,
    };

    let outcome = deliver_now(&request(&repo), codex.reviewer(&choice).as_ref());

    assert_eq!(
        outcome.outcome,
        Kind::ChangesRequested,
        "{}",
        outcome.detail
    );
    assert!(outcome.detail.contains("b.rs:1"), "{}", outcome.detail);
    assert!(outcome.detail.contains("app is never told about b."));
    let review = outcome.review.expect("the verdict");
    assert_eq!(review.blocking.len(), 2);
    assert_eq!(review.blocking[0].line, Some(1));
    assert_eq!(review.blocking[1].line, None);
    assert_eq!(head(&repo.main, "main"), start, "nothing landed");
    // The workspace's Codex model.
    assert_eq!(after(&codex.args(), "-m"), Some("gpt-5.5"));
}

#[test]
fn codex_s_usage_limit_is_the_limit_reached_outcome_not_a_failed_review() {
    let repo = delivered("codex-limit");
    let start = head(&repo.main, "main");
    let said = "You’ve hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), visit \
https://chatgpt.com/settings/usage to purchase more credits or try again at Nov 21st, 2026 3:00 PM.";
    let codex = FakeCodex::new(&failed(said), None, 1);

    let outcome = deliver_now(
        &request(&repo),
        codex.reviewer(&ReviewerChoice::default()).as_ref(),
    );

    assert_eq!(outcome.outcome, Kind::Error, "{}", outcome.detail);
    // What the engine reads as « review not run (usage limit) »
    // (delivery_text::USAGE_LIMIT), with the account and Codex's words.
    assert!(
        outcome.detail.contains("has reached its usage limit"),
        "{}",
        outcome.detail
    );
    assert!(
        outcome.detail.contains("the Codex account"),
        "{}",
        outcome.detail
    );
    assert!(
        outcome.detail.contains(&format!("(Codex said: {said})")),
        "{}",
        outcome.detail
    );
    assert!(outcome
        .detail
        .contains("Nothing is wrong with the delivery"));
    assert_eq!(outcome.review, None);
    assert_eq!(head(&repo.main, "main"), start, "nothing merged");
}

#[test]
fn a_codex_that_fails_or_cannot_start_says_why() {
    let repo = delivered("codex-fails");
    let start = head(&repo.main, "main");
    let unauthorized =
        "unexpected status 401 Unauthorized: Missing bearer or basic authentication in header";
    let codex = FakeCodex::new(&failed(unauthorized), None, 1);

    let outcome = deliver_now(
        &request(&repo),
        codex.reviewer(&ReviewerChoice::default()).as_ref(),
    );

    assert_eq!(outcome.outcome, Kind::Error, "{}", outcome.detail);
    assert!(
        outcome
            .detail
            .contains(&format!("could not run: the review failed: {unauthorized}")),
        "{}",
        outcome.detail
    );
    assert!(!outcome.detail.contains("usage limit"));
    assert_eq!(head(&repo.main, "main"), start);

    // A Codex too old for a flag stops before any event: its stderr says why.
    let old = FakeCodex::new(&[], None, 2).says_on_stderr(
        "error: unexpected argument '--ephemeral' found\n\nUsage: codex exec [OPTIONS] [PROMPT]\n",
    );
    let outcome = deliver_now(
        &request(&repo),
        old.reviewer(&ReviewerChoice::default()).as_ref(),
    );
    assert_eq!(outcome.outcome, Kind::Error, "{}", outcome.detail);
    assert!(
        outcome
            .detail
            .contains("(Codex said: error: unexpected argument '--ephemeral' found"),
        "{}",
        outcome.detail
    );
    assert_eq!(head(&repo.main, "main"), start);
}
