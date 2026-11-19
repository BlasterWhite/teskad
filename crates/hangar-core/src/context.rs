//! Where each pane's agent stands in its context window.
//!
//! The agents already write the answer into their transcripts: Claude Code
//! stamps `usage` on every assistant message, Codex logs `token_count` events
//! with the window size included. Reading those files is the sturdy path —
//! parsing the terminal output would break on the next redraw of an agent's
//! status line.
//!
//! Transcripts grow fast, so a file is never re-read whole on every poll: a
//! cursor per file remembers how far parsing got, and each look consumes only
//! what was appended since. First contact starts near the end of the file —
//! only the last exchange matters — and pays for one full scan only when the
//! tail turns out to hold no usage line at all. A compressed Codex rollout
//! (`.jsonl.zst`) is finished and cannot be seeked into: it is decoded whole,
//! and again only when its size changes.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;

use crate::sessions::{codex, AgentKind};

/// What the last exchange of a transcript says about the context. The window
/// it fills is not in Claude Code's transcript: the frontend knows it by
/// model (`AgentContextUsage` carries it when the agent writes it down).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    /// Tokens occupying the window after the last exchange: the prompt side,
    /// fresh and cached alike, plus the reply that joins it on the next turn.
    pub used_tokens: u64,
    /// Model the transcript names, when it names one.
    pub model: Option<String>,
}

/// `ContextUsage` for either agent, with the window's size when the
/// transcript declares it: Codex does (`model_context_window`, the window it
/// works with); Claude Code leaves it to the frontend's table.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentContextUsage {
    #[serde(flatten)]
    pub usage: ContextUsage,
    pub context_window: Option<u64>,
}

/// What one transcript line contributes. The pieces arrive on different
/// lines — Codex names its model on `turn_context`, its numbers on
/// `token_count` — so the cursor accumulates facts instead of expecting one
/// line to carry them all.
enum LineFact {
    Usage {
        used: u64,
        window: Option<u64>,
        model: Option<String>,
    },
    Model(String),
}

fn parse_line(agent: AgentKind, line: &str) -> Option<LineFact> {
    match agent {
        AgentKind::Claude => parse_claude_line(line).map(|(used, model)| LineFact::Usage {
            used,
            window: None,
            model,
        }),
        AgentKind::Codex => parse_codex_line(line),
    }
}

/// One Claude Code transcript line: the tokens it leaves in the window and
/// the model it names. Only main-chain assistant messages count: subagents
/// write into the same file flagged `isSidechain`, and their context is not
/// the pane's.
fn parse_claude_line(line: &str) -> Option<(u64, Option<String>)> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("type").and_then(|t| t.as_str()) != Some("assistant") {
        return None;
    }
    if value.get("isSidechain").and_then(|s| s.as_bool()) == Some(true) {
        return None;
    }
    let message = value.get("message")?;
    let usage = message.get("usage")?;
    let count = |key: &str| usage.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
    let used = count("input_tokens")
        + count("cache_read_input_tokens")
        + count("cache_creation_input_tokens")
        + count("output_tokens");
    if used == 0 {
        return None;
    }
    let model = message
        .get("model")
        .and_then(|m| m.as_str())
        .map(str::to_string);
    Some((used, model))
}

/// One Codex rollout line. `token_count` events carry the numbers — and the
/// window — while the model name travels on `turn_context` lines. What fills
/// the window is the last response's `total_tokens`, the figure Codex's own
/// status line reads (`TokenUsage::tokens_in_context_window`).
fn parse_codex_line(line: &str) -> Option<LineFact> {
    // Most lines are neither: a substring is cheaper than a parse to tell.
    if !line.contains("\"turn_context\"") && !line.contains("\"token_count\"") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let payload = value.get("payload")?;
    match value.get("type").and_then(|t| t.as_str())? {
        "turn_context" => payload
            .get("model")
            .and_then(|m| m.as_str())
            .map(|m| LineFact::Model(m.to_string())),
        "event_msg" => {
            if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
                return None;
            }
            // Rate-limit heartbeats reuse the event with `info: null`.
            let info = payload.get("info")?;
            let used = info
                .get("last_token_usage")?
                .get("total_tokens")
                .and_then(|v| v.as_u64())
                .filter(|&n| n > 0)?;
            Some(LineFact::Usage {
                used,
                window: info
                    .get("model_context_window")
                    .and_then(|v| v.as_u64())
                    .filter(|&n| n > 0),
                model: None,
            })
        }
        _ => None,
    }
}

/// Parsing state of one transcript file.
#[derive(Default)]
struct Cursor {
    /// Bytes already parsed, always ending on a line boundary. A line still
    /// being written stays unconsumed until its newline lands.
    offset: u64,
    /// File length at the previous look: a compressed rollout is decoded
    /// again only when it changed.
    len: u64,
    used: Option<u64>,
    model: Option<String>,
    window: Option<u64>,
}

impl Cursor {
    fn snapshot(&self) -> Option<AgentContextUsage> {
        self.used.map(|used_tokens| AgentContextUsage {
            usage: ContextUsage {
                used_tokens,
                model: self.model.clone(),
            },
            context_window: self.window,
        })
    }
}

/// How much of a first-contact file is worth reading. The last exchange sits
/// at the end; on a transcript that has grown for days, the megabytes before
/// it are history the gauge does not need.
const TAIL_BYTES: u64 = 4 * 1024 * 1024;

/// A line longer than this is skipped unparsed: a usage line is a few hundred
/// bytes, and a giant tool result is not worth holding in memory.
const MAX_LINE: usize = 16 * 1024 * 1024;

/// One cursor per transcript, for the life of the process. Entries for panes
/// that closed linger, but each is a few dozen bytes against the megabytes of
/// re-reading they prevent. The map is locked only to look a cursor up; each
/// cursor has its own lock, held across that file's reads alone.
static CURSORS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<Cursor>>>>> = OnceLock::new();

/// Codex rollout paths by thread id: the id is inside the file name, but
/// finding it means walking every dated directory, which is only worth doing
/// once. Hits are re-checked on use — a file compressed or written back plain
/// since must not pin a path that is gone.
static CODEX_PATHS: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

/// Reads whatever the file gained since the last look and folds it into the
/// cursor. Returns the current snapshot, `None` while no usage line has been
/// seen yet.
fn advance(agent: AgentKind, path: &Path, cursor: &mut Cursor) -> Option<AgentContextUsage> {
    let len = fs::metadata(path).ok()?.len();

    if codex::is_compressed(path) {
        if cursor.len != len || cursor.used.is_none() {
            *cursor = Cursor::default();
            scan(agent, path, cursor, false);
            cursor.len = len;
        }
        return cursor.snapshot();
    }

    // Shorter than what was already parsed: the file was rewritten from
    // scratch (a restarted session), so the parse starts over with it.
    if len < cursor.offset {
        *cursor = Cursor::default();
    }

    if len > cursor.offset {
        // First contact skips to the tail; everything before is history.
        let mut skipped_head = false;
        if cursor.offset == 0 && len > TAIL_BYTES {
            cursor.offset = len - TAIL_BYTES;
            skipped_head = true;
        }
        scan(agent, path, cursor, skipped_head);
        let unnamed = agent == AgentKind::Codex && cursor.model.is_none();
        if skipped_head && (cursor.used.is_none() || unnamed) {
            // The whole tail was one giant tool line, or usage — or the
            // model, which Codex names once a turn — only appears earlier:
            // pay for the full read, once, streamed.
            *cursor = Cursor::default();
            scan(agent, path, cursor, false);
        }
    }
    cursor.len = len;
    cursor.snapshot()
}
/// Calls `each` with every complete line from the reader's position on and
/// returns the bytes those lines (and the ones skipped) took. Memory stays
/// bounded by `max_line` whatever the file holds: a longer line is drained
/// without being kept, and `each` never sees it. A line without its newline
/// yet — the agent mid-write — is left unconsumed.
fn stream_lines<R: BufRead>(mut reader: R, max_line: usize, mut each: impl FnMut(&[u8])) -> u64 {
    let mut consumed = 0u64;
    let mut line = Vec::new();
    loop {
        line.clear();
        let Ok(read) = (&mut reader)
            .take(max_line as u64 + 1)
            .read_until(b'\n', &mut line)
        else {
            break;
        };
        if read == 0 {
            break;
        }
        let mut taken = read as u64;
        if line.last() == Some(&b'\n') {
            line.pop();
            each(&line);
        } else if read > max_line {
            // Too long to keep: drain up to its newline.
            let mut ended = false;
            let mut sink = Vec::new();
            loop {
                sink.clear();
                let Ok(more) = (&mut reader).take(64 * 1024).read_until(b'\n', &mut sink) else {
                    break;
                };
                if more == 0 {
                    break;
                }
                taken += more as u64;
                if sink.last() == Some(&b'\n') {
                    ended = true;
                    break;
                }
            }
            if !ended {
                break;
            }
        } else {
            break;
        }
        consumed += taken;
    }
    consumed
}

/// Parses the complete lines between the cursor and the end of the file,
/// line by line — of the decoded stream, for a compressed rollout, read from
/// its start. `drop_first` discards the partial line a tail seek landed
/// inside of.
fn scan(agent: AgentKind, path: &Path, cursor: &mut Cursor, drop_first: bool) {
    let reader: Box<dyn Read> = if codex::is_compressed(path) {
        match codex::open_rollout(path) {
            Ok(decoded) => decoded,
            Err(_) => return,
        }
    } else {
        let Ok(mut file) = fs::File::open(path) else {
            return;
        };
        if file.seek(SeekFrom::Start(cursor.offset)).is_err() {
            return;
        }
        Box::new(file)
    };
    let mut skip = drop_first;
    let (mut used, mut model, mut window) = (cursor.used, cursor.model.clone(), cursor.window);
    let consumed = stream_lines(BufReader::new(reader), MAX_LINE, |line| {
        if std::mem::take(&mut skip) {
            return;
        }
        let Ok(line) = std::str::from_utf8(line) else {
            return;
        };
        match parse_line(agent, line) {
            Some(LineFact::Usage {
                used: tokens,
                window: declared,
                model: named,
            }) => {
                used = Some(tokens);
                // Facts arrive on different lines; a line missing one must
                // not erase what an earlier line established.
                if declared.is_some() {
                    window = declared;
                }
                if named.is_some() {
                    model = named;
                }
            }
            Some(LineFact::Model(named)) => model = Some(named),
            None => {}
        }
    });
    cursor.used = used;
    cursor.model = model;
    cursor.window = window;
    cursor.offset += consumed;
}

/// The transcript of a session: `<projects>/<encoded cwd>/<id>.jsonl` in the
/// first of `roots` that holds it.
fn find_transcript(roots: &[PathBuf], cwd: &str, session_id: &str) -> Option<PathBuf> {
    let folder = crate::sessions::encode_project_dir(cwd);
    roots
        .iter()
        .map(|root| root.join(&folder).join(format!("{session_id}.jsonl")))
        .find(|path| path.is_file())
}

/// A session's transcript, in whichever folder Claude Code files
/// conversations in (`CLAUDE_CONFIG_DIR`, a profile's own `projects/`): the
/// same roots `claude_sessions` lists from. The chat (chat/) reads the same
/// file.
pub(crate) fn claude_transcript(cwd: &str, session_id: &str) -> Option<PathBuf> {
    find_transcript(&crate::accounts::projects_dirs(), cwd, session_id)
}

/// The rollout a Codex thread is written to now, found once by walking
/// `sessions/` and remembered. A cached compressed file whose plain twin
/// appeared (the thread was resumed) gives way to it.
fn codex_transcript(thread_id: &str) -> Option<PathBuf> {
    let mut cache = CODEX_PATHS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()?;
    if let Some(path) = cache.get(thread_id) {
        let plain = codex::is_compressed(path).then(|| path.with_extension(""));
        match plain {
            Some(plain) if plain.is_file() => {
                cache.insert(thread_id.to_string(), plain.clone());
                return Some(plain);
            }
            _ if path.is_file() => return Some(path.clone()),
            _ => {
                cache.remove(thread_id);
            }
        }
    }
    let root = codex::home()?.join(codex::SESSIONS);
    let path = codex::find_rollout(&[root], thread_id)?;
    cache.insert(thread_id.to_string(), path.clone());
    Some(path)
}

/// The id lands in a path; anything beyond what our own watchers produce
/// (uuids, hex, dashes) is refused rather than resolved.
pub(crate) fn valid_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn usage_snapshot(agent: AgentKind, session_id: &str, cwd: &str) -> Option<AgentContextUsage> {
    if !valid_session_id(session_id) {
        return None;
    }
    let path = match agent {
        AgentKind::Claude => claude_transcript(cwd, session_id)?,
        AgentKind::Codex => codex_transcript(session_id)?,
    };
    let cursor = {
        let mut cursors = CURSORS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .ok()?;
        cursors.entry(path.clone()).or_default().clone()
    };
    let mut cursor = cursor.lock().ok()?;
    advance(agent, &path, &mut cursor)
}

/// Where a pane's Claude Code stands in its context window, according to its
/// own transcript. `None` covers every "show nothing" case: a session not
/// written yet, a file with no usage line so far.
pub async fn context_usage(session_id: String, cwd: String) -> Option<ContextUsage> {
    agent_context_usage(AgentKind::Claude, session_id, cwd)
        .await
        .map(|reading| reading.usage)
}

/// Where a pane's agent stands in its context window, according to its own
/// transcript: Claude Code's for its session in `cwd`, Codex's rollout for
/// its thread (found by id wherever its date put it). `None` as for
/// `context_usage`.
pub async fn agent_context_usage(
    agent: AgentKind,
    session_id: String,
    cwd: String,
) -> Option<AgentContextUsage> {
    crate::git::off_thread(move || Ok(usage_snapshot(agent, &session_id, &cwd)))
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // Shapes lifted from real transcripts on disk, trimmed to the fields
    // that matter plus the neighbours they sit among.
    const CLAUDE_LINE: &str = r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-fable-5","usage":{"input_tokens":2,"cache_creation_input_tokens":10143,"cache_read_input_tokens":152141,"output_tokens":3674,"service_tier":"standard"}},"uuid":"x"}"#;
    const CLAUDE_SIDECHAIN: &str = r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-fable-5","usage":{"input_tokens":9,"cache_creation_input_tokens":0,"cache_read_input_tokens":50000,"output_tokens":1,"service_tier":"standard"}}}"#;

    fn temp_file(tag: &str) -> PathBuf {
        let dir = crate::testdir::scratch(&format!("context-{tag}"));
        dir.join("transcript.jsonl")
    }

    fn advance_once(path: &Path) -> Option<ContextUsage> {
        advance_agent_once(AgentKind::Claude, path).map(|reading| reading.usage)
    }

    fn advance_agent_once(agent: AgentKind, path: &Path) -> Option<AgentContextUsage> {
        // Each test keeps its own cursor: the static map would leak state
        // between tests sharing a path.
        advance(agent, path, &mut Cursor::default())
    }

    /// `advance` on Claude Code's transcript, as the tests below read it.
    fn advance_claude(path: &Path, cursor: &mut Cursor) -> Option<ContextUsage> {
        advance(AgentKind::Claude, path, cursor).map(|reading| reading.usage)
    }

    #[test]
    fn claude_line_sums_all_four_token_kinds() {
        let (used, model) = parse_claude_line(CLAUDE_LINE).expect("usage");
        assert_eq!(used, 2 + 10143 + 152141 + 3674);
        assert_eq!(model.as_deref(), Some("claude-fable-5"));
    }

    #[test]
    fn claude_sidechain_lines_are_ignored() {
        assert!(parse_claude_line(CLAUDE_SIDECHAIN).is_none());
    }

    #[test]
    fn incremental_reads_only_consume_complete_lines() {
        let path = temp_file("incremental");
        fs::write(&path, format!("{CLAUDE_LINE}\n")).unwrap();

        let mut cursor = Cursor::default();
        let first = advance_claude(&path, &mut cursor).unwrap();
        assert_eq!(first.used_tokens, 165960);

        // A partial line — the agent mid-write — changes nothing yet.
        let updated = CLAUDE_LINE.replace("\"input_tokens\":2", "\"input_tokens\":900002");
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(file, "{}", &updated[..40]).unwrap();
        let mid = advance_claude(&path, &mut cursor).unwrap();
        assert_eq!(mid.used_tokens, 165960);

        // The newline lands: the new figure replaces the old.
        writeln!(file, "{}", &updated[40..]).unwrap();
        let after = advance_claude(&path, &mut cursor).unwrap();
        assert_eq!(after.used_tokens, 165960 - 2 + 900002);
    }

    #[test]
    fn rewritten_file_resets_the_cursor() {
        let path = temp_file("rewrite");
        fs::write(&path, format!("{CLAUDE_LINE}\n{CLAUDE_LINE}\n")).unwrap();
        let mut cursor = Cursor::default();
        advance_claude(&path, &mut cursor).unwrap();

        // A fresh, shorter transcript at the same path — a restarted session.
        let smaller = CLAUDE_LINE.replace(
            "\"cache_read_input_tokens\":152141",
            "\"cache_read_input_tokens\":10",
        );
        fs::write(&path, format!("{smaller}\n")).unwrap();
        let after = advance_claude(&path, &mut cursor).unwrap();
        assert_eq!(after.used_tokens, 2 + 10143 + 10 + 3674);
    }

    #[test]
    fn first_contact_on_a_large_file_still_finds_an_early_usage_line() {
        let path = temp_file("tail");
        // The only usage line sits at the head, followed by more filler than
        // the tail window covers: the fallback full read has to kick in.
        let filler = format!("{{\"type\":\"noise\",\"pad\":\"{}\"}}\n", "x".repeat(4096));
        let mut body = format!("{CLAUDE_LINE}\n");
        while (body.len() as u64) <= TAIL_BYTES {
            body.push_str(&filler);
        }
        fs::write(&path, &body).unwrap();
        let snapshot = advance_once(&path).unwrap();
        assert_eq!(snapshot.used_tokens, 165960);
    }

    #[test]
    fn the_model_survives_lines_that_do_not_name_it() {
        let path = temp_file("model");
        let unnamed = CLAUDE_LINE.replace("\"model\":\"claude-fable-5\",", "");
        fs::write(&path, format!("{CLAUDE_LINE}\n{unnamed}\n")).unwrap();
        let snapshot = advance_once(&path).unwrap();
        assert_eq!(snapshot.used_tokens, 165960);
        assert_eq!(snapshot.model.as_deref(), Some("claude-fable-5"));
    }

    #[test]
    fn the_gauge_reads_a_transcript_from_a_custom_config_root() {
        let base = temp_file("roots").parent().unwrap().to_path_buf();
        let (main, custom) = (base.join("main"), base.join("custom").join("projects"));
        let folder = crate::sessions::encode_project_dir("/work/app");
        fs::create_dir_all(&main).unwrap();
        fs::create_dir_all(custom.join(&folder)).unwrap();
        fs::write(
            custom.join(&folder).join("sess-1.jsonl"),
            format!("{CLAUDE_LINE}\n"),
        )
        .unwrap();
        let roots = [main, custom];
        let path = find_transcript(&roots, "/work/app", "sess-1").expect("found in 2nd root");
        assert_eq!(advance_once(&path).unwrap().used_tokens, 165960);
        assert!(find_transcript(&roots, "/work/app", "other").is_none());
        assert!(find_transcript(&roots, "/elsewhere", "sess-1").is_none());
    }

    #[test]
    fn streaming_never_hands_over_a_line_past_the_bound() {
        let data = format!("short\n{}\nafter\npartial", "x".repeat(300));
        let mut seen = Vec::new();
        let consumed = stream_lines(std::io::Cursor::new(data.as_bytes()), 16, |l| {
            seen.push(String::from_utf8_lossy(l).into_owned())
        });
        // The 300-byte line is drained unseen; the line mid-write stays.
        assert_eq!(seen, ["short", "after"]);
        assert_eq!(consumed as usize, data.len() - "partial".len());
    }

    #[test]
    fn the_full_fallback_skips_a_giant_line_and_still_finds_usage() {
        let path = temp_file("giant");
        let mut file = fs::File::create(&path).unwrap();
        writeln!(file, "{CLAUDE_LINE}").unwrap();
        // Bigger than the tail window and than any line worth keeping.
        let giant = vec![b'x'; MAX_LINE + 1024];
        file.write_all(&giant).unwrap();
        writeln!(file).unwrap();
        let mut cursor = Cursor::default();
        let snapshot = advance_claude(&path, &mut cursor).unwrap();
        assert_eq!(snapshot.used_tokens, 165960);
        assert_eq!(cursor.offset, fs::metadata(&path).unwrap().len());
    }

    #[test]
    fn session_ids_that_look_like_paths_are_refused() {
        for agent in AgentKind::ALL {
            assert!(usage_snapshot(agent, "../../etc/passwd", "C:\\x").is_none());
            assert!(usage_snapshot(agent, "", "C:\\x").is_none());
        }
    }

    // Codex: shapes from the rollout fixtures (sessions/fixtures/), which
    // follow the schema in openai/codex.
    const CODEX_COUNT: &str = r#"{"timestamp":"t","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":29917611},"last_token_usage":{"input_tokens":98340,"cached_input_tokens":97024,"cache_write_input_tokens":0,"output_tokens":483,"reasoning_output_tokens":0,"total_tokens":98823},"model_context_window":353400},"rate_limits":{}}}"#;
    const CODEX_NULL_INFO: &str = r#"{"timestamp":"t","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{}}}"#;
    const CODEX_TURN: &str = r#"{"timestamp":"t","type":"turn_context","payload":{"turn_id":"u-1","cwd":"C:\\x","model":"gpt-5.6-sol","effort":"high"}}"#;

    #[test]
    fn codex_count_carries_window_and_null_info_does_not_parse() {
        match parse_codex_line(CODEX_COUNT) {
            Some(LineFact::Usage {
                used,
                window,
                model,
            }) => {
                assert_eq!(used, 98823);
                assert_eq!(window, Some(353400));
                assert_eq!(model, None);
            }
            _ => panic!("expected usage"),
        }
        assert!(parse_codex_line(CODEX_NULL_INFO).is_none());
        match parse_codex_line(CODEX_TURN) {
            Some(LineFact::Model(model)) => assert_eq!(model, "gpt-5.6-sol"),
            _ => panic!("expected model"),
        }
    }

    #[test]
    fn codex_model_survives_lines_that_do_not_name_it() {
        let path = temp_file("codex");
        fs::write(
            &path,
            format!("{CODEX_TURN}\n{CODEX_COUNT}\n{CODEX_NULL_INFO}\n"),
        )
        .unwrap();
        let snapshot = advance_agent_once(AgentKind::Codex, &path).unwrap();
        assert_eq!(snapshot.usage.used_tokens, 98823);
        assert_eq!(snapshot.context_window, Some(353400));
        assert_eq!(snapshot.usage.model.as_deref(), Some("gpt-5.6-sol"));
    }

    /// The fixture's last response left 20,780 tokens in a 258,400 window.
    #[test]
    fn a_codex_rollout_feeds_the_gauge_plain_or_compressed() {
        use crate::sessions::codex::tests::{compress, LEGACY, LEGACY_NAME};
        let dir = temp_file("codex-fixture").parent().unwrap().to_path_buf();
        let plain = dir.join(LEGACY_NAME);
        fs::write(&plain, LEGACY).unwrap();
        let zst = dir.join(format!("{LEGACY_NAME}.zst"));
        fs::write(&zst, compress(LEGACY)).unwrap();
        for path in [&plain, &zst] {
            let reading = advance_agent_once(AgentKind::Codex, path).expect("usage");
            assert_eq!(reading.usage.used_tokens, 20780);
            assert_eq!(reading.usage.model.as_deref(), Some("gpt-6.1-sol"));
            assert_eq!(reading.context_window, Some(258400));
        }
        // Claude Code's parser finds nothing to say about a rollout.
        assert!(advance_once(&plain).is_none());
    }

    #[test]
    fn a_compressed_rollout_is_decoded_again_only_when_it_changes() {
        use crate::sessions::codex::tests::compress;
        let path = temp_file("zst-change").with_file_name("rollout-x-1.jsonl.zst");
        fs::write(&path, compress(&format!("{CODEX_TURN}\n{CODEX_COUNT}\n"))).unwrap();
        let mut cursor = Cursor::default();
        let first = advance(AgentKind::Codex, &path, &mut cursor).unwrap();
        assert_eq!(first.usage.used_tokens, 98823);
        // A longer rollout in its place: read again.
        let more = CODEX_COUNT.replace("\"total_tokens\":98823", "\"total_tokens\":120000");
        fs::write(
            &path,
            compress(&format!("{CODEX_TURN}\n{CODEX_COUNT}\n{more}\n")),
        )
        .unwrap();
        let after = advance(AgentKind::Codex, &path, &mut cursor).unwrap();
        assert_eq!(after.usage.used_tokens, 120000);
    }

    /// A long turn's tail holds no `turn_context`: the model is looked for
    /// before it, once.
    #[test]
    fn a_codex_tail_without_its_turn_still_names_the_model() {
        let path = temp_file("codex-tail");
        let filler = format!(
            "{{\"type\":\"response_item\",\"pad\":\"{}\"}}\n",
            "x".repeat(4096)
        );
        let mut body = format!("{CODEX_TURN}\n");
        while (body.len() as u64) <= TAIL_BYTES {
            body.push_str(&filler);
        }
        body.push_str(CODEX_COUNT);
        body.push('\n');
        fs::write(&path, &body).unwrap();
        let reading = advance_agent_once(AgentKind::Codex, &path).unwrap();
        assert_eq!(reading.usage.model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(reading.usage.used_tokens, 98823);
    }
}
