//! A delivery read by Codex (docs/native/agents.md §4.4, D11): `codex exec`,
//! a stable command, with a fresh thread in the temporary worktree, its
//! commands sandboxed read-only, nobody asked anything, and its answer the
//! verdict Hangar's schema describes — the prompt every reviewer gets.
//!
//! Flags, events and wording as OpenAI's codex has them at the pinned release
//! (`scripts/codex-protocol/subset.json`, 0.170.0), under `codex-rs/`:
//! `exec/src/cli.rs` and `utils/cli/src/shared_options.rs` (the flags),
//! `utils/cli/src/config_override.rs` (`-c`), `exec/src/lib.rs` (the prompt
//! read from stdin for `-`, approvals in exec, the exit status),
//! `exec/src/exec_events.rs` and `exec/src/event_processor_with_jsonl_output.rs`
//! (what `--json` prints, what `-o` holds), `core/src/client_common.rs` and
//! `codex-api/src/common.rs` (the output schema is sent strict),
//! `protocol/src/error.rs` (how a usage limit is worded); the docs page
//! `non-interactive-mode`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;

use super::{
    limit_message, on_account, run, scrub, tail, Ended, Reviewer, Spawn, Verdict, OUTPUT_BYTES,
};
use crate::agent::AgentKind;

/// `codex exec`, in the temporary worktree, able to read and nothing else.
pub(super) struct Codex {
    /// The `codex` command, where `agent::detect` found it.
    pub program: PathBuf,
    pub limit: Duration,
    /// `None`: Codex's own default (or the one its config names).
    pub model: Option<String>,
    /// The workspace's Codex account; `None`: the default one.
    pub account: Option<String>,
}

/// How Codex reads: shell commands, in a sandbox that lets them read and
/// nothing else.
pub(super) const READING: &str = "You can read any file here, and search them, with read-only commands (`cat`, \
`sed -n`, `rg`, `git show`, `git log`) to check the change in its context. The sandbox lets you read and nothing \
else: do not try to change anything, build anything or run the tests, and you need not: judge from the code.";

/// The verdict's schema — the fields of the Claude reviewer's — in the form
/// Codex sends it: strict (`output_schema_strict` is true unless a caller
/// says otherwise), so each object closes its properties and requires every
/// one of them, `line` being `null` when there is none.
pub(super) const STRICT_VERDICT_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"properties":{"approve":{"type":"boolean"},"blocking":{"type":"array","items":{"type":"object","additionalProperties":false,"properties":{"file":{"type":"string"},"line":{"type":["integer","null"]},"problem":{"type":"string"}},"required":["file","line","problem"]}},"notes":{"type":"string"}},"required":["approve","blocking","notes"]}"#;

const SCHEMA_FILE: &str = "verdict.schema.json";
const ANSWER_FILE: &str = "answer.json";

/// The schema handed to Codex and the file its answer is written to, in a
/// folder of their own, out of the worktree under review, gone with the
/// review.
struct Files {
    dir: PathBuf,
}

impl Files {
    fn new() -> std::io::Result<Files> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "hangar-review-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // What a review of an earlier Hangar with the same id left behind.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let files = Files { dir };
        std::fs::write(files.schema(), STRICT_VERDICT_SCHEMA)?;
        Ok(files)
    }

    fn schema(&self) -> PathBuf {
        self.dir.join(SCHEMA_FILE)
    }

    fn answer(&self) -> PathBuf {
        self.dir.join(ANSWER_FILE)
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// `codex exec`'s arguments for a review. The prompt is read from stdin
/// (`-`): a diff can be longer than a command line may be.
fn exec_args(model: Option<&str>, schema: &Path, answer: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "exec",
        // Its events as JSON lines on stdout: how the turn ended, and why.
        "--json",
        // A review is not a conversation anyone will resume.
        "--ephemeral",
        // The commands it runs read, and nothing else.
        "--sandbox",
        "read-only",
        // Nobody is asked anything: what would need an approval is refused,
        // an MCP tool call included (`core/src/mcp_tool_call.rs`). exec asks
        // for `never` itself, but drops it when the user's config hands
        // approvals to Codex's automatic reviewer; `-c` outranks that config.
        "-c",
        "approval_policy=never",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    args.push("--output-schema".into());
    args.push(schema.into());
    // Written once the turn completes: its last message, the verdict.
    args.push("-o".into());
    args.push(answer.into());
    if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
        args.push("-m".into());
        args.push(model.into());
    }
    args.push("-".into());
    args
}

/// The reviewer's command: `codex exec` in `dir`, with Hangar's environment
/// minus `SCRUBBED`, and `account_env` — what accounts.rs gives a Codex pane
/// on the workspace's account (`CODEX_HOME`, `CODEX_SQLITE_HOME`) — on top.
/// Empty on the default account, which keeps the environment as it is.
fn exec_command(
    program: &Path,
    dir: &Path,
    args: Vec<OsString>,
    account_env: &[(&'static str, Option<PathBuf>)],
) -> Command {
    let mut command = Command::new(program);
    command.args(args).current_dir(dir);
    scrub(&mut command);
    on_account(&mut command, account_env);
    command
}

/// What `codex exec --json` said on its way (exec_events.rs): how its turn
/// ended, the errors it reported, its last message.
#[derive(Debug, Default)]
struct Said {
    completed: bool,
    /// `turn.failed`'s error.
    failed: Option<String>,
    /// `error` events, in order: fatal ones, and some it went past.
    errors: Vec<String>,
    /// The last `agent_message` item: with an output schema, the JSON asked
    /// for.
    last_message: Option<String>,
}

impl Said {
    fn read(out: &str) -> Said {
        let text = |value: &Value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };
        let mut said = Said::default();
        for line in out.lines() {
            let Ok(event) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            match event.get("type").and_then(Value::as_str) {
                Some("turn.completed") => said.completed = true,
                Some("turn.failed") => {
                    said.failed = Some(
                        text(&event["error"]["message"])
                            .unwrap_or_else(|| "the turn failed".into()),
                    );
                }
                Some("error") => said.errors.extend(text(&event["message"])),
                Some("item.completed") if event["item"]["type"] == "agent_message" => {
                    if let Some(message) = text(&event["item"]["text"]) {
                        said.last_message = Some(message);
                    }
                }
                _ => {}
            }
        }
        said
    }
}

/// Why no verdict came.
#[derive(Debug, PartialEq)]
enum Failure {
    /// The account is out of quota, in Codex's words.
    Limit(String),
    Other(String),
}

/// The verdict in `message`: the JSON the schema asked for, fenced or not.
/// `None` when it is no JSON at all.
fn verdict_in(message: &str) -> Option<Result<Verdict, String>> {
    let bare = message
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let value: Value = serde_json::from_str(bare).ok()?;
    Some(
        serde_json::from_value(value)
            .map_err(|err| format!("the reviewer's verdict is malformed: {err}")),
    )
}

/// The verdict of a `codex exec` that ended: what `-o` holds (`answer`),
/// else the last message of a turn that completed; otherwise why there is
/// none, from its events (`out`) and its stderr (`err`).
fn verdict_of(answer: Option<&str>, out: &str, err: &str) -> Result<Verdict, Failure> {
    let said = Said::read(out);
    let message = answer
        .map(str::trim)
        .filter(|answer| !answer.is_empty())
        .map(str::to_string)
        .or_else(|| said.last_message.clone().filter(|_| said.completed));
    if let Some(verdict) = message.as_deref().and_then(verdict_in) {
        return verdict.map_err(Failure::Other);
    }
    let reports: Vec<&str> = said
        .failed
        .iter()
        .chain(said.errors.iter().rev())
        .map(String::as_str)
        .collect();
    if let Some(limit) = reports
        .iter()
        .copied()
        .chain(std::iter::once(err))
        .find_map(usage_limit)
    {
        return Err(Failure::Limit(limit));
    }
    if let Some(report) = reports.first() {
        return Err(Failure::Other(format!("the review failed: {report}")));
    }
    if message.is_some() {
        return Err(Failure::Other(
            "the reviewer answered without a verdict".to_string(),
        ));
    }
    let stderr = tail(err.trim());
    Err(Failure::Other(if stderr.is_empty() {
        "the reviewer's answer could not be read".to_string()
    } else {
        format!("the reviewer's answer could not be read (Codex said: {stderr})")
    }))
}

/// A usage limit of the account, as Codex words it: the errors it reports as
/// `usageLimitExceeded` (`protocol/src/error.rs`): `UsageLimitReached` (« You’ve
/// hit your usage limit. … Try again at 3:00 PM. », « Your workspace is out of
/// credits. … », « You hit your spend cap … »), `QuotaExceeded` (« Quota
/// exceeded. Check your plan and billing details. ») and `UsageNotIncluded`
/// (« To use Codex with your ChatGPT plan, upgrade to Plus: … »). Codex's own
/// « rate limit exceeded » is a moment's throttling it retries, not the
/// account's quota.
fn usage_limit(said: &str) -> Option<String> {
    const WORDINGS: &[&str] = &[
        "hit your usage limit",
        "is out of credits",
        "hit your spend cap",
        "quota exceeded",
        "to use codex with your chatgpt plan",
    ];
    said.lines()
        .map(str::trim)
        .find(|line| {
            let lower = line.to_lowercase().replace('’', "'");
            WORDINGS.iter().any(|wording| lower.contains(wording))
        })
        .map(str::to_string)
}

/// The name the person gave the Codex account, for a message.
fn account_label(id: &str) -> String {
    crate::accounts::accounts_of(AgentKind::Codex)
        .into_iter()
        .find(|account| account.id == id)
        .map(|account| account.label)
        .unwrap_or_else(|| id.to_string())
}

impl Reviewer for Codex {
    /// `_claude_model` and `_claude_account` are the Claude reviewer's: a
    /// Codex review runs on `self.model` and spends `self.account`.
    fn review(
        &self,
        dir: &Path,
        _claude_model: &str,
        prompt: &str,
        _claude_account: Option<&str>,
    ) -> Result<Verdict, String> {
        let kind = AgentKind::Codex.id();
        let launch = crate::accounts::for_launch(Some(kind), self.account.as_deref());
        let env = launch
            .as_ref()
            .map(|launch| crate::accounts::env_for(Some(kind), launch.dir.as_deref()))
            .unwrap_or_default();
        let files =
            Files::new().map_err(|err| format!("could not write the review's schema: {err}"))?;
        let command = exec_command(
            &self.program,
            dir,
            exec_args(self.model.as_deref(), &files.schema(), &files.answer()),
            &env,
        );
        let finished = run(
            command,
            Spawn {
                input: Some(prompt.as_bytes().to_vec()),
                limit: self.limit,
                merged: false,
                cap: OUTPUT_BYTES,
                cancellable: true,
            },
        )
        .map_err(|err| format!("could not start {}: {err}", self.program.display()))?;
        match finished.ended {
            Ended::Cancelled => Err("Hangar closed while the review was running".to_string()),
            Ended::TimedOut => Err(format!(
                "the review did not finish within {} minutes and was stopped",
                self.limit.as_secs() / 60
            )),
            Ended::Exited => {
                let answer = std::fs::read_to_string(files.answer()).ok();
                verdict_of(answer.as_deref(), &finished.out, &finished.err).map_err(|failure| {
                    match failure {
                        Failure::Limit(said) => {
                            let account = launch.map(|launch| launch.id).unwrap_or_default();
                            limit_message("Codex", "Codex", &account_label(&account), &said)
                        }
                        Failure::Other(message) => message,
                    }
                })
            }
        }
    }

    fn reading(&self) -> &'static str {
        READING
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::SCRUBBED;
    use serde_json::json;
    use std::collections::BTreeSet;

    fn args_of(model: Option<&str>) -> Vec<String> {
        exec_args(
            model,
            Path::new("/tmp/r/schema.json"),
            Path::new("/tmp/r/answer.json"),
        )
        .into_iter()
        .map(|arg| arg.into_string().unwrap())
        .collect()
    }

    fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|arg| arg == flag)
            .map(|at| args[at + 1].as_str())
    }

    #[test]
    fn codex_reviews_read_only_nobody_asked_its_answer_shaped() {
        let args = args_of(None);
        assert_eq!(args[0], "exec");
        assert!(args.iter().any(|arg| arg == "--json"));
        assert!(args.iter().any(|arg| arg == "--ephemeral"));
        assert_eq!(after(&args, "--sandbox"), Some("read-only"));
        assert_eq!(after(&args, "-c"), Some("approval_policy=never"));
        assert_eq!(after(&args, "--output-schema"), Some("/tmp/r/schema.json"));
        assert_eq!(after(&args, "-o"), Some("/tmp/r/answer.json"));
        // Nothing that would let it write or skip the sandbox.
        for flag in [
            "--yolo",
            "--dangerously-bypass-approvals-and-sandbox",
            "--approve-for-me",
            "--add-dir",
            "--full-auto",
        ] {
            assert!(!args.iter().any(|arg| arg == flag), "{flag}");
        }
        // Codex's own model unless the workspace names one.
        assert!(!args.iter().any(|arg| arg == "-m"));
        assert_eq!(after(&args_of(Some("gpt-5.5")), "-m"), Some("gpt-5.5"));
        assert!(!args_of(Some("  ")).iter().any(|arg| arg == "-m"));
        // The prompt comes on stdin, last.
        assert_eq!(args.last().map(String::as_str), Some("-"));
        assert_eq!(
            args_of(Some("gpt-5.5")).last().map(String::as_str),
            Some("-")
        );
    }

    /// Codex sends the schema strict: each object closed, every property
    /// required.
    fn closed(schema: &Value, at: &str) {
        if schema["type"] == "object" {
            assert_eq!(schema["additionalProperties"], json!(false), "{at}");
            let properties = schema["properties"].as_object().unwrap();
            let keys: BTreeSet<&str> = properties.keys().map(String::as_str).collect();
            let required: BTreeSet<&str> = schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|key| key.as_str().unwrap())
                .collect();
            assert_eq!(keys, required, "{at}");
            for (key, property) in properties {
                closed(property, &format!("{at}.{key}"));
            }
        }
        if let Some(items) = schema.get("items") {
            closed(items, &format!("{at}[]"));
        }
    }

    #[test]
    fn the_schema_is_the_claude_reviewer_s_sent_strict() {
        let strict: Value = serde_json::from_str(STRICT_VERDICT_SCHEMA).unwrap();
        closed(&strict, "verdict");
        let claude: Value = serde_json::from_str(crate::delivery::VERDICT_SCHEMA).unwrap();
        let fields = |schema: &Value| -> BTreeSet<String> {
            schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect()
        };
        assert_eq!(fields(&strict), fields(&claude));
        assert_eq!(
            fields(&strict["properties"]["blocking"]["items"]),
            fields(&claude["properties"]["blocking"]["items"])
        );
        // A line that is not known is null, which reads as none.
        let verdict: Verdict = serde_json::from_value(json!({
            "approve": false,
            "blocking": [{"file": "a.rs", "line": null, "problem": "crashes"}],
            "notes": "",
        }))
        .unwrap();
        assert_eq!(verdict.blocking[0].line, None);
    }

    #[test]
    fn the_review_spends_the_workspace_s_codex_account() {
        let home = PathBuf::from("/home/me/.teskad/accounts/codex/work");
        let env = crate::accounts::env_for(Some("codex"), Some(&home));
        let command = exec_command(
            Path::new("codex"),
            Path::new("."),
            args_of(None).into_iter().map(OsString::from).collect(),
            &env,
        );
        let set = |name: &str| {
            command
                .get_envs()
                .find(|(key, _)| *key == std::ffi::OsStr::new(name))
                .map(|(_, value)| value.map(|value| value.to_os_string()))
        };
        assert_eq!(set("CODEX_HOME"), Some(Some(home.into_os_string())));

        // The default account keeps the environment as it is, less what a
        // Hangar pane or an agent session leaves in it.
        let launch = crate::accounts::for_launch(Some("codex"), None).unwrap();
        assert_eq!(launch.dir, None);
        let env = crate::accounts::env_for(Some("codex"), launch.dir.as_deref());
        assert!(env.is_empty());
        let command = exec_command(Path::new("codex"), Path::new("."), Vec::new(), &env);
        assert!(command
            .get_envs()
            .all(|(name, value)| value.is_none() && SCRUBBED.iter().any(|s| *s == name)));
    }

    // The events below are written from the source (codex-rs/exec/src/exec_events.rs
    // and event_processor_with_jsonl_output.rs at 0.170.0), not recorded.

    fn lines(events: &[Value]) -> String {
        events.iter().map(|event| format!("{event}\n")).collect()
    }

    fn completed(message: &str) -> String {
        lines(&[
            json!({"type": "thread.started", "thread_id": "0199a213-81c0-7800-8aa1-bbab2a035a53"}),
            json!({"type": "turn.started"}),
            json!({"type": "item.completed", "item": {"id": "item_0", "type": "command_execution", "command": "bash -lc 'cat b.rs'", "aggregated_output": "bee\n", "exit_code": 0, "status": "completed"}}),
            json!({"type": "item.completed", "item": {"id": "item_1", "type": "agent_message", "text": message}}),
            json!({"type": "turn.completed", "usage": {"input_tokens": 24763, "cached_input_tokens": 24448, "cache_write_input_tokens": 0, "output_tokens": 122, "reasoning_output_tokens": 0}}),
        ])
    }

    fn failed(message: &str) -> String {
        lines(&[
            json!({"type": "thread.started", "thread_id": "0199a213-81c0-7800-8aa1-bbab2a035a53"}),
            json!({"type": "turn.started"}),
            json!({"type": "error", "message": message}),
            json!({"type": "turn.failed", "error": {"message": message}}),
        ])
    }

    #[test]
    fn the_verdict_is_read_from_codex_s_answer() {
        let approve = r#"{"approve":true,"blocking":[],"notes":"ok"}"#;
        let block = r#"{"approve":false,"blocking":[{"file":"a.rs","line":3,"problem":"off by one"}],"notes":""}"#;

        // What `-o` holds.
        let verdict = verdict_of(Some(block), &completed(block), "").unwrap();
        assert!(!verdict.approve);
        assert_eq!(verdict.blocking[0].line, Some(3));
        // Without it, the last message of the turn that completed.
        assert!(verdict_of(None, &completed(approve), "").unwrap().passes());
        assert!(verdict_of(Some("  \n"), &completed(approve), "")
            .unwrap()
            .passes());
        // Fenced, as a model may still write it.
        let fenced = format!("```json\n{approve}\n```");
        assert!(verdict_of(Some(&fenced), "", "").unwrap().passes());

        // Words, or JSON of another shape.
        assert_eq!(
            verdict_of(Some("I approve."), &completed("I approve."), ""),
            Err(Failure::Other(
                "the reviewer answered without a verdict".into()
            ))
        );
        let Err(Failure::Other(malformed)) = verdict_of(Some(r#"{"approve":"yes"}"#), "", "")
        else {
            panic!("a verdict of another shape");
        };
        assert!(malformed.contains("malformed"), "{malformed}");

        // A turn that failed: its error, never an intermediate message.
        let mut out = lines(&[
            json!({"type": "item.completed", "item": {"id": "item_0", "type": "agent_message", "text": approve}}),
        ]);
        out.push_str(&failed("stream disconnected before completion"));
        assert_eq!(
            verdict_of(None, &out, ""),
            Err(Failure::Other(
                "the review failed: stream disconnected before completion".into()
            ))
        );
        // A Codex that stopped before any event says why on stderr.
        assert_eq!(
            verdict_of(None, "", "error: unexpected argument '--ephemeral' found\n"),
            Err(Failure::Other(
                "the reviewer's answer could not be read (Codex said: error: unexpected argument '--ephemeral' found)"
                    .into()
            ))
        );
        assert_eq!(
            verdict_of(None, "", ""),
            Err(Failure::Other(
                "the reviewer's answer could not be read".into()
            ))
        );
    }

    /// Each wording `protocol/src/error.rs` gives an error Codex reports as
    /// `usageLimitExceeded`.
    #[test]
    fn codex_s_usage_limits_are_the_account_s_limit() {
        for said in [
            "You’ve hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), visit https://chatgpt.com/settings/usage to purchase more credits or try again at Nov 21st, 2026 3:00 PM.",
            "You’ve hit your usage limit. To get more access now, send a request to your admin or try again later.",
            "You’ve hit your usage limit. Try again at 3:00 PM.",
            "You've hit your usage limit for GPT-5.5-Codex-Spark. Switch to another model now, or try again later.",
            "Your workspace is out of credits. Add credits to continue.",
            "You hit your spend cap set by the owner of your workspace. Ask an owner to increase your spend cap to continue.",
            "Quota exceeded. Check your plan and billing details.",
            "To use Codex with your ChatGPT plan, upgrade to Plus: https://chatgpt.com/explore/plus.",
        ] {
            assert_eq!(
                verdict_of(Some(""), &failed(said), ""),
                Err(Failure::Limit(said.to_string())),
                "{said}"
            );
        }
        // Said on stderr only.
        assert_eq!(
            verdict_of(
                None,
                "",
                "ERROR: You’ve hit your usage limit. Try again later.\n"
            ),
            Err(Failure::Limit(
                "ERROR: You’ve hit your usage limit. Try again later.".into()
            ))
        );
        // Throttling Codex retries, a crash: not the account's limit.
        for said in [
            "rate limit exceeded: Rate limit reached for gpt-5.5",
            "exceeded retry limit, last status: 500 Internal Server Error",
            "unexpected status 401 Unauthorized",
        ] {
            assert_eq!(
                verdict_of(None, &failed(said), ""),
                Err(Failure::Other(format!("the review failed: {said}"))),
                "{said}"
            );
        }
        // A verdict that talks of limits is a verdict.
        let verdict =
            r#"{"approve":true,"blocking":[],"notes":"Handles the usage limit the task names."}"#;
        assert!(verdict_of(Some(verdict), &completed(verdict), "")
            .unwrap()
            .passes());
    }

    #[test]
    fn a_limit_message_is_the_one_the_engine_tells() {
        let message = limit_message(
            "Codex",
            "Codex",
            "Work",
            "You’ve hit your usage limit. Try again at 3:00 PM.",
        );
        assert!(message.starts_with("the Codex account \"Work\" has reached its usage limit"));
        assert!(
            message.contains("(Codex said: You’ve hit your usage limit. Try again at 3:00 PM.)")
        );
        assert!(message.contains("Nothing is wrong with the delivery"));
    }

    #[test]
    fn the_review_s_files_go_with_it() {
        let files = Files::new().unwrap();
        let dir = files.dir.clone();
        let schema: Value =
            serde_json::from_str(&std::fs::read_to_string(files.schema()).unwrap()).unwrap();
        assert_eq!(
            schema,
            serde_json::from_str::<Value>(STRICT_VERDICT_SCHEMA).unwrap()
        );
        assert_ne!(Files::new().unwrap().dir, dir, "one folder per review");
        drop(files);
        assert!(!dir.exists());
    }
}
