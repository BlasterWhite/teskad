//! One Codex rollout line at a time into the digest (sessions/codex.rs says
//! what a rollout is).
//!
//! Codex reports what it was billed two ways. In its source of 2026-11-19,
//! every model response leaves a `token_usage_record` with its own usage and
//! its response id: that is a call, and the id recognises it in a fork's copy
//! of the history. Older rollouts have none; there, and still alongside,
//! `token_count` events carry running totals, sometimes the same one twice in
//! a row: a call is then the difference between two of them. A rollout uses
//! the records once it has one — Codex writes a response's record before its
//! `token_count` — and the differences only for what came before (a thread an
//! older Codex started).

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use super::{fnv1a, is_false, parse_time, Call, Digest, Prompt, TITLE_PROMPT};
use crate::sessions::codex::{parse_meta, user_text, Record};

/// Token counters, as Codex reports them: a response's, or running totals.
/// OpenAI counts cached and cache-written input inside `input_tokens`, and
/// reasoning inside `output_tokens`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CodexTotals {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub cache_write_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

/// What the next lines of a rollout are read against, kept in the digest so
/// a rollout that grew picks up where it stopped.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CodexState {
    /// The running totals `token_count` reported last, to take differences.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totals: Option<CodexTotals>,
    /// The model the last `turn_context` named (an index into
    /// `Digest::models`): every response until the next one is billed to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<u16>,
    /// A `token_usage_record` was read: they are the calls from then on.
    #[serde(default, skip_serializing_if = "is_false")]
    pub records: bool,
    /// The rollout's own `session_meta` was read. Another one further down
    /// is a copy, brought along with a forked history.
    #[serde(default, skip_serializing_if = "is_false")]
    pub meta: bool,
}

#[derive(Deserialize)]
struct TurnContext<'a> {
    #[serde(borrow)]
    model: Option<Cow<'a, str>>,
    /// Absent from older rollouts; the same turn's context is written again
    /// after a compaction in the middle of it.
    #[serde(borrow)]
    turn_id: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
struct UsageRecord<'a> {
    #[serde(borrow)]
    response_id: Option<Cow<'a, str>>,
    usage: Option<CodexTotals>,
}

#[derive(Deserialize)]
struct EventMsg<'a> {
    #[serde(rename = "type", borrow)]
    kind: Option<Cow<'a, str>>,
    info: Option<TokenInfo>,
}

#[derive(Deserialize)]
struct TokenInfo {
    total_token_usage: Option<CodexTotals>,
    last_token_usage: Option<CodexTotals>,
}

/// Folds one Codex rollout line into the digest.
pub fn codex_line(line: &str, digest: &mut Digest) {
    let Ok(record) = serde_json::from_str::<Record>(line) else {
        digest.bad_lines += 1;
        return;
    };
    let at = record.timestamp.as_deref().and_then(parse_time);
    if let Some(at) = at {
        digest.stamps.push(at);
    }
    let (Some(kind), Some(payload)) = (record.kind.as_deref(), record.payload) else {
        return;
    };
    let payload = payload.get();
    let mut state = digest.codex.take().unwrap_or_default();
    match kind {
        "session_meta" if !state.meta => {
            state.meta = true;
            if let Some(meta) = parse_meta(payload) {
                digest.sub = meta.sub;
                digest.session = meta.session;
                if digest.cwd.is_empty() {
                    digest.cwd = meta.cwd;
                }
            }
        }
        "turn_context" => {
            if let Ok(context) = serde_json::from_str::<TurnContext>(payload) {
                if let Some(model) = context.model.as_deref() {
                    state.model = Some(digest.model_index(model));
                }
                // One turn_context opens every turn, in every version of the
                // rollout format; a sub-agent's turns are not a person's.
                if let (Some(at), false) = (at, digest.sub) {
                    let key = context
                        .turn_id
                        .as_deref()
                        .map_or(0, |id| fnv1a(&[b"codex-turn", id.as_bytes()]));
                    let again = key != 0 && digest.prompts.last().is_some_and(|p| p.key == key);
                    if !again {
                        digest.prompts.push(Prompt { key, at });
                    }
                }
            }
        }
        "token_usage_record" => {
            if let (Ok(usage), Some(at)) = (serde_json::from_str::<UsageRecord>(payload), at) {
                if let Some(tokens) = usage.usage {
                    state.records = true;
                    let key = usage
                        .response_id
                        .as_deref()
                        .map_or(0, |id| fnv1a(&[b"codex-response", id.as_bytes()]));
                    push_call(digest, &state, at, key, tokens);
                }
            }
        }
        "event_msg" => {
            // Most events are neither a count nor a prompt; a substring is
            // cheaper than a parse to tell.
            if payload.contains("token_count") && !state.records {
                if let Some(at) = at {
                    count(digest, &mut state, at, payload);
                }
            }
            if digest.title_rank == 0 && !digest.sub {
                if let Some(text) = user_text(payload) {
                    if !text.starts_with('<') {
                        digest.offer_title(&text, TITLE_PROMPT);
                    }
                }
            }
        }
        _ => {}
    }
    digest.codex = Some(state);
}

/// A `token_count` event: the difference with the totals reported before it.
fn count(digest: &mut Digest, state: &mut CodexState, at: i64, payload: &str) {
    let Ok(event) = serde_json::from_str::<EventMsg>(payload) else {
        return;
    };
    if event.kind.as_deref() != Some("token_count") {
        return;
    }
    // Rate-limit heartbeats reuse the event with `info: null`.
    let Some(total) = event.info.as_ref().and_then(|info| info.total_token_usage) else {
        return;
    };
    let last = event.info.and_then(|info| info.last_token_usage);
    let previous = state.totals.unwrap_or_default();
    state.totals = Some(total);
    let delta = if total.total_tokens < previous.total_tokens {
        // The running total went back: a new thread in the same file. What
        // this event adds is its own last call.
        last.unwrap_or(total)
    } else {
        CodexTotals {
            input_tokens: total.input_tokens.saturating_sub(previous.input_tokens),
            cached_input_tokens: total
                .cached_input_tokens
                .saturating_sub(previous.cached_input_tokens),
            cache_write_input_tokens: total
                .cache_write_input_tokens
                .saturating_sub(previous.cache_write_input_tokens),
            output_tokens: total.output_tokens.saturating_sub(previous.output_tokens),
            total_tokens: total.total_tokens - previous.total_tokens,
        }
    };
    // A forked rollout replays its parent's events verbatim: same time, same
    // running total, so the copy is recognised.
    let key = fnv1a(&[
        b"codex-count",
        &at.to_le_bytes(),
        &total.total_tokens.to_le_bytes(),
    ]);
    push_call(digest, state, at, key, delta);
}

/// One call of `tokens`, billed to the model named last. Nothing is pushed
/// for a count that bills nothing: the same total reported again, or the
/// window marked full after an overflow, which moves `total_tokens` alone.
fn push_call(digest: &mut Digest, state: &CodexState, at: i64, key: u64, tokens: CodexTotals) {
    let written = tokens.cache_write_input_tokens;
    let read = tokens.cached_input_tokens;
    // Cached and written input are kept apart from the rest, as they are
    // billed.
    let fresh = tokens.input_tokens.saturating_sub(read + written);
    if fresh + read + written + tokens.output_tokens == 0 {
        return;
    }
    let model = match state.model {
        Some(model) => model,
        None => digest.model_index("unknown"),
    };
    digest.calls.push(Call {
        key,
        at,
        model,
        input: fresh,
        output: tokens.output_tokens,
        // Codex leaves the cache lifetime to the API's default, thirty
        // minutes: the shorter write, in the slot pricing.rs bills as such.
        cache_write_5m: written,
        cache_write_1h: 0,
        cache_read: read,
        fast: false,
    });
}

/// Codex can report a count before the turn that names its model: those
/// calls go to the first model the rollout names, once it has named one.
pub fn name_early_calls(digest: &mut Digest) {
    let Some(unknown) = digest.models.iter().position(|model| model == "unknown") else {
        return;
    };
    let Some(named) = digest.models.iter().position(|model| model != "unknown") else {
        return;
    };
    for call in &mut digest.calls {
        if call.model as usize == unknown {
            call.model = named as u16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::read_into;
    use super::*;
    use crate::agent::AgentKind;
    use crate::sessions::codex::tests::{LEGACY, LEGACY_ID, PAGINATED, PAGINATED_ID, SUBAGENT};

    fn read(text: &str) -> Digest {
        let mut digest = Digest::new(AgentKind::Codex, false);
        let consumed = read_into(&mut digest, text.as_bytes(), true).unwrap();
        digest.offset += consumed;
        digest
    }

    /// (fresh input, cache read, cache write, output) of every call.
    fn tokens(digest: &Digest) -> Vec<(u64, u64, u64, u64)> {
        digest
            .calls
            .iter()
            .map(|c| (c.input, c.cache_read, c.cache_write_5m, c.output))
            .collect()
    }

    const META: &str = r#"{"timestamp":"2026-10-02T09:00:00.000Z","type":"session_meta","payload":{"id":"thread-1","timestamp":"2026-10-02T09:00:00Z","cwd":"/home/me/app","originator":"codex-tui","cli_version":"1.0","source":"cli"}}"#;

    fn count(at: &str, input: u64, cached: u64, output: u64) -> String {
        format!(
            r#"{{"timestamp":"{at}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"cache_write_input_tokens":0,"output_tokens":{output},"reasoning_output_tokens":0,"total_tokens":{}}},"last_token_usage":{{"input_tokens":1,"cached_input_tokens":0,"output_tokens":1,"total_tokens":2}},"model_context_window":272000}},"rate_limits":{{}}}}}}"#,
            input + output
        )
    }

    /// A turn as rollouts written before turn ids had it.
    fn turn(at: &str, model: &str) -> String {
        format!(
            r#"{{"timestamp":"{at}","type":"turn_context","payload":{{"cwd":"/home/me/app","model":"{model}","effort":"high"}}}}"#
        )
    }

    fn prompt(at: &str, text: &str) -> String {
        format!(
            r#"{{"timestamp":"{at}","type":"event_msg","payload":{{"type":"user_message","message":"{text}","images":[]}}}}"#
        )
    }

    #[test]
    fn codex_calls_are_the_differences_between_running_totals() {
        let text = [
            META.to_string(),
            turn("2026-10-02T09:00:01Z", "gpt-5.5"),
            r#"{"timestamp":"2026-10-02T09:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>x</environment_context>"}]}}"#.to_string(),
            prompt("2026-10-02T09:00:01Z", "Add a cache"),
            count("2026-10-02T09:00:10Z", 1000, 800, 50),
            // The same total again: nothing new.
            count("2026-10-02T09:00:11Z", 1000, 800, 50),
            r#"{"timestamp":"2026-10-02T09:00:12Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{}}}"#.to_string(),
            turn("2026-10-02T09:05:00Z", "gpt-6-astra"),
            prompt("2026-10-02T09:05:00Z", "And a second one"),
            count("2026-10-02T09:05:10Z", 3000, 2500, 80),
        ]
        .join("\n");
        let digest = read(&text);
        assert_eq!(digest.session, "thread-1");
        assert_eq!(digest.cwd, "/home/me/app");
        assert_eq!(digest.title, "Add a cache");
        assert_eq!(digest.prompts.len(), 2);
        assert_eq!(tokens(&digest), [(200, 800, 0, 50), (300, 1700, 0, 30)]);
        assert_eq!(digest.models[digest.calls[0].model as usize], "gpt-5.5");
        assert_eq!(digest.models[digest.calls[1].model as usize], "gpt-6-astra");
    }

    #[test]
    fn a_count_before_the_first_turn_goes_to_the_model_named_next() {
        let text = [
            META.to_string(),
            count("2026-10-02T09:00:01Z", 500, 0, 20),
            turn("2026-10-02T09:00:02Z", "gpt-5.5"),
            count("2026-10-02T09:00:10Z", 1000, 0, 50),
        ]
        .join("\n");
        let digest = read(&text);
        assert_eq!(digest.calls.len(), 2);
        assert!(digest
            .calls
            .iter()
            .all(|call| digest.models[call.model as usize] == "gpt-5.5"));
    }

    #[test]
    fn a_codex_subagent_belongs_to_its_parent_and_has_no_turns() {
        let meta = r#"{"timestamp":"2026-10-02T09:00:00Z","type":"session_meta","payload":{"id":"child","cwd":"/home/me/app","source":{"subagent":{"thread_spawn":{"parent_thread_id":"thread-1","depth":1}}}}}"#;
        let text = [
            meta.to_string(),
            turn("2026-10-02T09:00:01Z", "gpt-5.5"),
            prompt("2026-10-02T09:00:01Z", "Look around"),
            count("2026-10-02T09:00:10Z", 100, 0, 10),
        ]
        .join("\n");
        let digest = read(&text);
        assert!(digest.sub);
        assert_eq!(digest.session, "thread-1");
        assert!(digest.prompts.is_empty());
        // What the parent asked it is not the session's title.
        assert_eq!(digest.title, "");
        assert_eq!(digest.calls.len(), 1);
    }

    #[test]
    fn a_running_total_that_goes_back_counts_its_last_call() {
        let reset = count("2026-10-02T09:10:00Z", 10, 0, 5).replace(
            r#""last_token_usage":{"input_tokens":1,"cached_input_tokens":0,"output_tokens":1,"total_tokens":2}"#,
            r#""last_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":5,"total_tokens":15}"#,
        );
        let text = [
            META.to_string(),
            count("2026-10-02T09:00:10Z", 1000, 0, 50),
            reset,
        ]
        .join("\n");
        let digest = read(&text);
        assert_eq!(digest.calls.len(), 2);
        assert_eq!((digest.calls[1].input, digest.calls[1].output), (10, 5));
    }

    /// After a context overflow Codex marks the window full: the totals jump
    /// to the window's size with no tokens behind them. No call comes of it,
    /// and the next response is counted from there.
    #[test]
    fn a_window_marked_full_bills_nothing() {
        let full = r#"{"timestamp":"2026-10-02T09:00:20Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":272000},"last_token_usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":271000},"model_context_window":272000},"rate_limits":{}}}"#;
        // The next response adds to the totals marked full
        // (`TokenUsageInfo::new_or_append` in openai/codex).
        let after = count("2026-10-02T09:00:30Z", 400, 0, 20)
            .replace(r#""total_tokens":420}"#, r#""total_tokens":272420}"#);
        let text = [
            META.to_string(),
            turn("2026-10-02T09:00:01Z", "gpt-5.5"),
            count("2026-10-02T09:00:10Z", 900, 0, 100),
            full.to_string(),
            after,
        ]
        .join("\n");
        let digest = read(&text);
        assert_eq!(tokens(&digest), [(900, 0, 0, 100), (400, 0, 0, 20)]);
    }

    /// The fixture of today's format: three responses, each a
    /// `token_usage_record` followed by the `token_count` that repeats it.
    #[test]
    fn the_records_of_a_current_rollout_are_its_calls() {
        let digest = read(LEGACY);
        assert_eq!(digest.session, LEGACY_ID);
        assert_eq!(digest.cwd, "/home/dev/shop");
        assert!(!digest.sub);
        assert_eq!(digest.title, "Add a cart total to the checkout page");
        assert_eq!(digest.models, ["gpt-6.1-sol"]);
        assert_eq!(
            tokens(&digest),
            [
                (166, 0, 18176, 412),
                (332, 18176, 512, 655),
                (256, 19456, 768, 300)
            ]
        );
        assert!(digest.calls.iter().all(|call| call.key != 0));
        assert_eq!(digest.prompts.len(), 2);
        assert!(digest.prompts.iter().all(|p| p.key != 0));
        assert_eq!(digest.bad_lines, 0);
        assert!(digest.codex.as_ref().is_some_and(|state| state.records));
    }

    #[test]
    fn the_fixture_sub_agent_and_paginated_thread_read_as_theirs() {
        let sub = read(SUBAGENT);
        assert!(sub.sub);
        assert_eq!(sub.session, LEGACY_ID);
        assert!(sub.prompts.is_empty());
        assert_eq!(sub.models, ["gpt-6-luna"]);
        assert_eq!(tokens(&sub), [(56, 0, 6144, 240)]);

        let paginated = read(PAGINATED);
        assert_eq!(paginated.session, PAGINATED_ID);
        assert_eq!(paginated.title, "Rename the checkout step to review");
        assert_eq!(paginated.models, ["gpt-5.6-sol"]);
        assert_eq!(tokens(&paginated), [(106, 0, 15104, 520)]);
        assert_eq!(paginated.prompts.len(), 1);
    }

    /// A rollout read in two goes — it grew in between — gives what one read
    /// of the whole gives: the state carried over knows the records began.
    #[test]
    fn a_rollout_read_in_two_goes_matches_one_read() {
        let cut = LEGACY[..LEGACY.len() / 2].rfind('\n').unwrap() + 1;
        let mut digest = Digest::new(AgentKind::Codex, false);
        let first = read_into(&mut digest, &LEGACY.as_bytes()[..cut], false).unwrap();
        digest.offset += first;
        // Through the cache on disk, as the next scan would find it.
        let mut digest: Digest =
            serde_json::from_str(&serde_json::to_string(&digest).unwrap()).unwrap();
        let rest = read_into(&mut digest, &LEGACY.as_bytes()[cut..], false).unwrap();
        digest.offset += rest;
        assert_eq!(digest, read(LEGACY));
    }

    /// The same turn's context again after a compaction mid-turn is not a
    /// second turn; a fork's copy of the parent's `session_meta` changes
    /// nothing.
    #[test]
    fn a_turn_written_again_and_a_copied_meta_change_nothing() {
        let mut lines: Vec<&str> = LEGACY.lines().collect();
        let first_turn = lines
            .iter()
            .position(|line| line.contains(r#""type":"turn_context""#))
            .unwrap();
        let repeated = lines[first_turn];
        let copied_meta = SUBAGENT.lines().next().unwrap();
        lines.insert(first_turn + 3, repeated);
        lines.insert(first_turn + 4, copied_meta);
        let digest = read(&lines.join("\n"));
        assert_eq!(digest.prompts.len(), 2);
        assert!(!digest.sub);
        assert_eq!(digest.session, LEGACY_ID);
    }
}
