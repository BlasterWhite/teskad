//! Codex's rollouts: the files Codex keeps a conversation (a "thread") in.
//!
//! They live under `$CODEX_HOME` (`~/.codex` by default): `sessions/` by
//! date (`YYYY/MM/DD/`), and `archived_sessions/` for the threads archived.
//! A file is named `rollout-<YYYY-MM-DDThh-mm-ss>-<thread id>.jsonl`, or
//! `…-<thread id>_<rollout id>.jsonl` when a reverted thread went on in a new
//! file. Codex compresses a rollout left alone for a week into
//! `….jsonl.zst` (one zstd frame), and writes it back plain when the thread
//! is resumed: either may be on disk, and for a moment both.
//!
//! A rollout is JSONL, one `{"timestamp", "type", "payload"}` record a line:
//! `session_meta` first (the thread's id, the root session's id, its folder,
//! where it came from), then `turn_context` at every turn (its id and model),
//! `response_item`s, `event_msg`s (`user_message`, or `item_completed` with a
//! `UserMessage` in the newer "paginated" history; `token_count`) and a
//! `token_usage_record` per model response.
//!
//! The format is Codex's own, read from openai/codex on 2026-11-19:
//! `codex-rs/protocol/src/protocol.rs` (`SessionMeta`, `TurnContextItem`,
//! `TokenUsage`, `EventMsg`), `codex-rs/history/src/rollout_payload.rs`
//! (the record types), `codex-rs/rollout/src/rollout_file_name.rs` (the
//! names) and `codex-rs/rollout/src/compression.rs` (the `.zst` files).
//! Everything here only reads: nothing is written, moved or decompressed to
//! disk.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde_json::value::RawValue;

use super::{cached, insert_cached, AgentSession, FileCache};

/// The folder of the threads `codex resume` offers, under `home()`.
pub const SESSIONS: &str = "sessions";
/// The folder archived threads are moved to, under `home()`.
pub const ARCHIVED_SESSIONS: &str = "archived_sessions";

/// Codex's home: `CODEX_HOME` when set, `~/.codex` otherwise.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
}

// ---------------------------------------------------------------------------
// The files
// ---------------------------------------------------------------------------

/// Whether a file name is a rollout's, plain or compressed.
pub(crate) fn is_rollout_name(name: &str) -> bool {
    name.starts_with("rollout-") && (name.ends_with(".jsonl") || name.ends_with(".jsonl.zst"))
}

/// Whether a path is a compressed (`.jsonl.zst`) rollout.
pub(crate) fn is_compressed(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "zst")
}

/// The thread a rollout belongs to, from its name:
/// `rollout-<19 characters of time>-<thread>[_<rollout>].jsonl[.zst]`.
pub(crate) fn thread_id_of(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let core = name
        .strip_suffix(".zst")
        .unwrap_or(name)
        .strip_suffix(".jsonl")?
        .strip_prefix("rollout-")?;
    if core.get(19..20)? != "-" {
        return None;
    }
    let ids = core.get(20..)?;
    let thread = ids.split_once('_').map_or(ids, |(thread, _)| thread);
    (!thread.is_empty()).then(|| thread.to_string())
}

/// Every rollout under `dir`: `sessions/YYYY/MM/DD/`, or the flatter
/// `archived_sessions/`.
pub(crate) fn collect_rollouts(dir: &Path, out: &mut Vec<PathBuf>) {
    walk(dir, 0, out);
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() || (kind.is_symlink() && path.is_dir()) {
            walk(&path, depth + 1, out);
        } else if is_rollout_name(&entry.file_name().to_string_lossy()) {
            out.push(path);
        }
    }
}

/// A rollout on disk, with what its metadata says.
#[derive(Debug, Clone)]
pub(crate) struct Rollout {
    pub path: PathBuf,
    pub size: u64,
    pub modified_ms: u64,
}

/// The rollouts under `root`, most recently written first; at equal times a
/// plain file before its compressed copy, the one Codex appends to.
pub(crate) fn rollouts_newest_first(root: &Path) -> Vec<Rollout> {
    let mut files = Vec::new();
    collect_rollouts(root, &mut files);
    let mut rollouts: Vec<Rollout> = files
        .into_iter()
        .filter_map(|path| {
            let meta = fs::metadata(&path).ok()?;
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_millis() as u64);
            Some(Rollout {
                path,
                size: meta.len(),
                modified_ms,
            })
        })
        .collect();
    rollouts.sort_by_key(|r| (std::cmp::Reverse(r.modified_ms), is_compressed(&r.path)));
    rollouts
}

/// The rollout a thread is written to now: the newest of its files under
/// any of `roots` — a reverted thread has more than one.
pub(crate) fn find_rollout(roots: &[PathBuf], thread_id: &str) -> Option<PathBuf> {
    roots
        .iter()
        .flat_map(|root| rollouts_newest_first(root))
        .filter(|r| thread_id_of(&r.path).as_deref() == Some(thread_id))
        .min_by_key(|r| (std::cmp::Reverse(r.modified_ms), is_compressed(&r.path)))
        .map(|r| r.path)
}

/// Whether a thread has a rollout under `root`; `None` when `root` cannot be
/// read, which says nothing about the thread.
pub(crate) fn rollout_exists(root: &Path, thread_id: &str) -> Option<bool> {
    fs::read_dir(root).ok()?;
    let mut files = Vec::new();
    collect_rollouts(root, &mut files);
    Some(
        files
            .iter()
            .any(|path| thread_id_of(path).as_deref() == Some(thread_id)),
    )
}

// ---------------------------------------------------------------------------
// Reading one, compressed or not
// ---------------------------------------------------------------------------

/// A zstd stream of one or more frames, decoded as one: a rollout appended
/// to after compression is a second frame after the first.
struct Frames<R: BufRead> {
    decoder: Option<ruzstd::decoding::StreamingDecoder<R, ruzstd::decoding::FrameDecoder>>,
}

impl<R: BufRead> Frames<R> {
    fn new(source: R) -> std::io::Result<Self> {
        let decoder =
            ruzstd::decoding::StreamingDecoder::new(source).map_err(std::io::Error::other)?;
        Ok(Self {
            decoder: Some(decoder),
        })
    }
}

impl<R: BufRead> Read for Frames<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let Some(decoder) = self.decoder.as_mut() else {
                return Ok(0);
            };
            let read = decoder.read(buf)?;
            if read > 0 || buf.is_empty() {
                return Ok(read);
            }
            // This frame is done; another may follow.
            let Some(done) = self.decoder.take() else {
                return Ok(0);
            };
            let mut source = done.into_inner();
            if source.fill_buf()?.is_empty() {
                return Ok(0);
            }
            // Whatever follows and is not a frame (padding, a skippable
            // frame) ends the stream rather than the read.
            self.decoder = ruzstd::decoding::StreamingDecoder::new(source).ok();
        }
    }
}

/// Opens a rollout for reading, `.jsonl` or `.jsonl.zst`, and yields its
/// decoded bytes. Read-only: nothing is written or copied.
pub(crate) fn open_rollout(path: &Path) -> std::io::Result<Box<dyn Read>> {
    let file = fs::File::open(path)?;
    if is_compressed(path) {
        Ok(Box::new(Frames::new(BufReader::with_capacity(
            256 * 1024,
            file,
        ))?))
    } else {
        Ok(Box::new(file))
    }
}

// ---------------------------------------------------------------------------
// What the lines say
// ---------------------------------------------------------------------------

/// What a `session_meta` record says about its thread.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Meta {
    /// The thread's own id: what `codex resume` takes.
    pub id: String,
    /// The conversation it is part of: its own id, or for a thread another
    /// one spawned (a sub-agent) the root thread's, which its work counts
    /// towards.
    pub session: String,
    pub cwd: String,
    /// Spawned by another thread: a sub-agent, a review, a compaction.
    pub sub: bool,
    /// Started by Codex for itself (memory consolidation, the guardian), not
    /// a conversation a person had.
    pub internal: bool,
}

#[derive(Deserialize)]
struct MetaPayload<'a> {
    #[serde(borrow)]
    id: Option<Cow<'a, str>>,
    /// The root thread of a tree of agents; absent before late 2026.
    #[serde(borrow)]
    session_id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    parent_thread_id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    cwd: Option<Cow<'a, str>>,
    /// `"cli"`, `"vscode"`, `"exec"`, … or `{"subagent": …}`,
    /// `{"internal": …}`, `{"custom": …}`.
    source: Option<serde_json::Value>,
}

/// Reads the payload of a `session_meta` record.
pub(crate) fn parse_meta(payload: &str) -> Option<Meta> {
    let meta: MetaPayload = serde_json::from_str(payload).ok()?;
    let id = meta.id.as_deref().filter(|id| !id.is_empty())?.to_string();
    let source = meta.source.as_ref();
    let subagent = source.and_then(|s| s.get("subagent"));
    let spawner = subagent
        .and_then(|s| s.get("thread_spawn"))
        .and_then(|s| s.get("parent_thread_id"))
        .and_then(|p| p.as_str());
    let root = meta
        .session_id
        .as_deref()
        .filter(|root| !root.is_empty() && *root != id)
        .or(spawner)
        .or(meta.parent_thread_id.as_deref())
        .filter(|root| !root.is_empty() && *root != id);
    Some(Meta {
        session: root.unwrap_or(&id).to_string(),
        sub: subagent.is_some() || root.is_some(),
        internal: source.is_some_and(|s| s.get("internal").is_some()),
        cwd: meta.cwd.map(Cow::into_owned).unwrap_or_default(),
        id,
    })
}

/// A rollout record, its payload left unparsed.
#[derive(Deserialize)]
pub(crate) struct Record<'a> {
    #[serde(borrow)]
    pub timestamp: Option<Cow<'a, str>>,
    #[serde(rename = "type", borrow)]
    pub kind: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub payload: Option<&'a RawValue>,
}

/// The `Meta` of a whole line, when it is a `session_meta` record.
pub(crate) fn meta_of_line(line: &str) -> Option<Meta> {
    let record: Record = serde_json::from_str(line).ok()?;
    if record.kind.as_deref() != Some("session_meta") {
        return None;
    }
    parse_meta(record.payload?.get())
}

#[derive(Deserialize)]
struct UserEvent<'a> {
    #[serde(rename = "type", borrow)]
    kind: Option<Cow<'a, str>>,
    /// `user_message`: what was typed.
    #[serde(borrow)]
    message: Option<Cow<'a, str>>,
    /// `item_completed`: the item, a `UserMessage` for a prompt.
    #[serde(borrow)]
    item: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct UserItem<'a> {
    #[serde(rename = "type", borrow)]
    kind: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    content: Vec<UserInput<'a>>,
}

#[derive(Deserialize)]
struct UserInput<'a> {
    #[serde(rename = "type", borrow)]
    kind: Option<Cow<'a, str>>,
    #[serde(borrow)]
    text: Option<Cow<'a, str>>,
}

/// What a person typed, when an `event_msg` payload is their prompt: a
/// `user_message` event in the legacy history, an `item_completed`
/// `UserMessage` in the paginated one. The instructions and environment
/// Codex sends along travel as other records and never show here.
pub(crate) fn user_text(event_payload: &str) -> Option<String> {
    // Most events are not prompts; a substring is cheaper than a parse.
    if !event_payload.contains("user_message") && !event_payload.contains("UserMessage") {
        return None;
    }
    let event: UserEvent = serde_json::from_str(event_payload).ok()?;
    let text = match event.kind.as_deref()? {
        "user_message" => event.message?.into_owned(),
        "item_completed" => {
            let item: UserItem = serde_json::from_str(event.item?.get()).ok()?;
            if item.kind.as_deref() != Some("UserMessage") {
                return None;
            }
            item.content
                .into_iter()
                .filter(|part| part.kind.as_deref() == Some("text"))
                .filter_map(|part| part.text)
                .collect::<Vec<_>>()
                .join(" ")
        }
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// `user_text` of a whole line.
fn user_text_of_line(line: &str) -> Option<String> {
    let record: Record = serde_json::from_str(line).ok()?;
    if record.kind.as_deref() != Some("event_msg") {
        return None;
    }
    user_text(record.payload?.get())
}

// ---------------------------------------------------------------------------
// Heads, for the session picker
// ---------------------------------------------------------------------------

/// How much of a rollout's head is read, decoded: the `session_meta` line
/// carries the whole base instructions, and the first prompt comes after the
/// instructions Codex sends ahead of it.
const HEAD_BYTES: u64 = 1024 * 1024;

/// Lines read after the `session_meta` one to find the first prompt.
const HEAD_LINES: usize = 400;

/// Rollouts looked at, newest first, to list a folder's sessions: the folder
/// is inside each file, so each one is opened once (and remembered).
const LISTED_MAX: usize = 2000;

/// Heads kept at most; the oldest stored go first.
const HEAD_CACHE_MAX: usize = 4000;

/// What the head of a rollout says. `label` is `None` until it was looked
/// for: listing ids only needs the first line.
#[derive(Debug, Clone, Default)]
struct Head {
    meta: Option<Meta>,
    label: Option<String>,
}

fn head_cache() -> &'static Mutex<FileCache<Head>> {
    static CACHE: OnceLock<Mutex<FileCache<Head>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Reads the head of a rollout: its `session_meta`, and with `with_label` the
/// first prompt after it (cleaned, at most 80 characters, empty when none).
fn read_head(path: &Path, with_label: bool) -> Head {
    let Ok(reader) = open_rollout(path) else {
        return Head::default();
    };
    let mut reader = BufReader::new(reader.take(HEAD_BYTES));
    let mut line = Vec::new();
    let mut head = Head::default();
    for at in 0..=HEAD_LINES {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        // A line the budget cut is not whole JSON and parses as nothing.
        let Ok(text) = std::str::from_utf8(&line) else {
            continue;
        };
        if at == 0 {
            // Every rollout opens on its `session_meta`.
            head.meta = meta_of_line(text);
            if head.meta.is_none() || !with_label {
                break;
            }
            continue;
        }
        if let Some(prompt) = user_text_of_line(text) {
            if !prompt.starts_with('<') {
                let clean = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
                head.label = Some(clean.chars().take(80).collect());
                break;
            }
        }
    }
    if with_label && head.label.is_none() {
        head.label = Some(String::new());
    }
    head
}

/// `read_head`, not read again while the file's size and mtime hold.
fn head_of(rollout: &Rollout, with_label: bool) -> Head {
    let hit = head_cache()
        .lock()
        .ok()
        .and_then(|c| cached(&*c, &rollout.path, rollout.size, rollout.modified_ms))
        .filter(|head| !with_label || head.label.is_some());
    if let Some(head) = hit {
        return head;
    }
    let head = read_head(&rollout.path, with_label);
    if let Ok(mut cache) = head_cache().lock() {
        insert_cached(
            &mut *cache,
            HEAD_CACHE_MAX,
            &rollout.path,
            rollout.size,
            rollout.modified_ms,
            head.clone(),
        );
    }
    head
}

/// A folder as a key to compare: Codex records the absolute path it ran in,
/// which may differ from the pane's by its separators, a trailing slash, a
/// `\\?\` prefix or, where the file system ignores it, its case.
fn folder_key(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let mut key = path.replace('\\', "/");
    while key.len() > 1 && key.ends_with('/') {
        key.pop();
    }
    if cfg!(any(windows, target_os = "macos")) {
        key = key.to_lowercase();
    }
    key
}

/// The threads of `cwd` under `root` that a person can resume — not
/// sub-agents nor Codex's own threads — newest first, each once with the time
/// of its newest file.
fn threads_in(root: &Path, cwd: &str) -> Vec<(Meta, Vec<Rollout>)> {
    let target = folder_key(cwd);
    let mut threads: Vec<(Meta, Vec<Rollout>)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for rollout in rollouts_newest_first(root).into_iter().take(LISTED_MAX) {
        let Some(meta) = head_of(&rollout, false).meta else {
            continue;
        };
        if meta.sub || meta.internal || folder_key(&meta.cwd) != target {
            continue;
        }
        match index.get(&meta.id) {
            Some(&at) => threads[at].1.push(rollout),
            None => {
                index.insert(meta.id.clone(), threads.len());
                threads.push((meta, vec![rollout]));
            }
        }
    }
    threads
}

/// The ids and modification times (ms) of the threads of `cwd` under `root`,
/// newest first.
pub(crate) fn session_ids_in(root: &Path, cwd: &str) -> Vec<(String, u64)> {
    threads_in(root, cwd)
        .into_iter()
        .map(|(meta, files)| (meta.id, files[0].modified_ms))
        .collect()
}

/// The threads of `cwd` under `root`, newest first, labelled by their first
/// prompt — found in an older file of the thread when the newest one (a
/// revert's) starts after it.
pub(crate) fn sessions_in(root: &Path, cwd: &str) -> Vec<AgentSession> {
    threads_in(root, cwd)
        .into_iter()
        .map(|(meta, files)| AgentSession {
            label: files
                .iter()
                .map(|file| head_of(file, true).label.unwrap_or_default())
                .find(|label| !label.is_empty())
                .unwrap_or_default(),
            modified_ms: files[0].modified_ms,
            id: meta.id,
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A rollout as Codex writes it today (legacy history, a thread run from
    /// the CLI), made from the schema in openai/codex rather than taken from
    /// anyone's machine: its session_meta, the instructions and environment
    /// sent ahead of the prompt, two turns, one model response each with its
    /// `token_usage_record` and `token_count`.
    pub(crate) const LEGACY: &str = include_str!(
        "fixtures/rollout-2026-11-18T09-12-03-019a9a3e-7c21-7d40-9b6e-2f1c4a8d5e90.jsonl"
    );
    pub(crate) const LEGACY_ID: &str = "019a9a3e-7c21-7d40-9b6e-2f1c4a8d5e90";
    pub(crate) const LEGACY_NAME: &str =
        "rollout-2026-11-18T09-12-03-019a9a3e-7c21-7d40-9b6e-2f1c4a8d5e90.jsonl";

    /// The same kind of thread in the paginated history the app server
    /// writes: prompts as `item_completed` events, no `user_message`.
    pub(crate) const PAGINATED: &str = include_str!(
        "fixtures/rollout-2026-11-19T14-40-51-019a9f12-3b44-7a10-8c2d-6e5f4a3b2c1d.jsonl"
    );
    pub(crate) const PAGINATED_ID: &str = "019a9f12-3b44-7a10-8c2d-6e5f4a3b2c1d";
    pub(crate) const PAGINATED_NAME: &str =
        "rollout-2026-11-19T14-40-51-019a9f12-3b44-7a10-8c2d-6e5f4a3b2c1d.jsonl";

    /// A sub-agent the legacy thread spawned, in a rollout of its own.
    pub(crate) const SUBAGENT: &str = include_str!(
        "fixtures/rollout-2026-11-18T09-14-40-019a9a41-0d5e-7b31-a2c4-58e7f9b0c3d2.jsonl"
    );
    pub(crate) const SUBAGENT_NAME: &str =
        "rollout-2026-11-18T09-14-40-019a9a41-0d5e-7b31-a2c4-58e7f9b0c3d2.jsonl";

    /// The folder the fixtures ran in.
    pub(crate) const CWD: &str = "/home/dev/shop";

    pub(crate) fn compress(text: &str) -> Vec<u8> {
        ruzstd::encoding::compress_to_vec(
            text.as_bytes(),
            ruzstd::encoding::CompressionLevel::Fastest,
        )
    }

    /// `root/YYYY/MM/DD/<name>` holding `bytes`, the day taken from the name.
    pub(crate) fn put(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let stamp = &name["rollout-".len()..];
        let day = root
            .join(&stamp[..4])
            .join(&stamp[5..7])
            .join(&stamp[8..10]);
        fs::create_dir_all(&day).expect("day dir");
        let path = day.join(name);
        fs::write(&path, bytes).expect("rollout");
        path
    }

    fn temp_dir(tag: &str) -> PathBuf {
        crate::testdir::scratch(&format!("codex-{tag}"))
    }

    #[test]
    fn a_rollout_name_gives_its_thread() {
        let id = "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
        let plain = format!("rollout-2026-10-07T10-00-00-{id}.jsonl");
        assert_eq!(thread_id_of(Path::new(&plain)).as_deref(), Some(id));
        let zst = format!("{plain}.zst");
        assert_eq!(thread_id_of(Path::new(&zst)).as_deref(), Some(id));
        // A reverted thread's new file: the thread, then the rollout's own id.
        let reverted =
            format!("rollout-2026-10-07T10-00-00-{id}_019a0000-0000-7000-8000-000000000001.jsonl");
        assert_eq!(thread_id_of(Path::new(&reverted)).as_deref(), Some(id));
        assert_eq!(thread_id_of(Path::new("notes.jsonl")), None);
        assert_eq!(thread_id_of(Path::new("rollout-short.jsonl")), None);
        assert!(is_rollout_name(&zst));
        assert!(!is_rollout_name("session_index.jsonl"));
    }

    #[test]
    fn a_codex_session_is_found_by_the_id_in_its_rollout_name() {
        let root = temp_dir("exists");
        put(
            &root,
            "rollout-2026-10-07T10-00-00-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b.jsonl",
            b"{}\n",
        );
        assert_eq!(
            rollout_exists(&root, "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b"),
            Some(true)
        );
        assert_eq!(
            rollout_exists(&root, "0199a1b2-0000-7e5f-8a9b-0c1d2e3f4a5b"),
            Some(false)
        );
        // A suffix of an id is not the id.
        assert_eq!(rollout_exists(&root, "0c1d2e3f4a5b"), Some(false));
        assert_eq!(rollout_exists(&root.join("missing"), "x"), None);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_fixture_heads_say_who_they_are() {
        let first = |text: &str| text.lines().next().unwrap().to_string();
        let legacy = meta_of_line(&first(LEGACY)).expect("legacy meta");
        assert_eq!(legacy.id, LEGACY_ID);
        assert_eq!(legacy.session, LEGACY_ID);
        assert_eq!(legacy.cwd, CWD);
        assert!(!legacy.sub && !legacy.internal);

        let sub = meta_of_line(&first(SUBAGENT)).expect("sub-agent meta");
        assert!(sub.sub);
        assert_eq!(sub.session, LEGACY_ID);
        assert_ne!(sub.id, LEGACY_ID);

        let paginated = meta_of_line(&first(PAGINATED)).expect("paginated meta");
        assert_eq!(paginated.id, PAGINATED_ID);
    }

    /// Older rollouts name the spawning thread only in `source`, and Codex's
    /// own background threads say so there too.
    #[test]
    fn a_meta_without_a_session_id_still_finds_its_parent() {
        let child = parse_meta(
            r#"{"id":"child","cwd":"/w","source":{"subagent":{"thread_spawn":{"parent_thread_id":"root-1","depth":1}}}}"#,
        )
        .unwrap();
        assert_eq!((child.session.as_str(), child.sub), ("root-1", true));
        let review = parse_meta(r#"{"id":"r","cwd":"/w","source":{"subagent":"review"}}"#).unwrap();
        assert!(review.sub);
        assert_eq!(review.session, "r");
        let memory =
            parse_meta(r#"{"id":"m","cwd":"/w","source":{"internal":"memory_consolidation"}}"#)
                .unwrap();
        assert!(memory.internal && !memory.sub);
        let cli = parse_meta(r#"{"id":"c","session_id":"c","cwd":"/w","source":"cli"}"#).unwrap();
        assert!(!cli.sub && !cli.internal);
        assert!(parse_meta(r#"{"cwd":"/w"}"#).is_none());
    }

    #[test]
    fn a_prompt_is_read_from_either_history() {
        assert_eq!(
            user_text(r#"{"type":"user_message","message":"  fix it ","images":[]}"#).as_deref(),
            Some("fix it")
        );
        assert_eq!(
            user_text(
                r#"{"type":"item_completed","thread_id":"t","turn_id":"u","item":{"type":"UserMessage","id":"i","content":[{"type":"text","text":"add a test","text_elements":[]},{"type":"image","image_url":"data:x"}]},"completed_at_ms":1}"#
            )
            .as_deref(),
            Some("add a test")
        );
        // The agent's own message is no prompt.
        assert_eq!(
            user_text(
                r#"{"type":"item_completed","thread_id":"t","turn_id":"u","item":{"type":"AgentMessage","id":"i","content":[{"type":"Text","text":"UserMessage"}]},"completed_at_ms":1}"#
            ),
            None
        );
        assert_eq!(user_text(r#"{"type":"token_count","info":null}"#), None);
    }

    /// The picker lists a folder's threads from plain and compressed
    /// rollouts alike, labelled by the first prompt — not by the AGENTS.md
    /// instructions or the environment Codex sends ahead of it — and leaves
    /// sub-agents and other folders out.
    #[test]
    fn the_picker_lists_a_folders_threads_plain_or_compressed() {
        let root = temp_dir("list");
        put(&root, LEGACY_NAME, LEGACY.as_bytes());
        put(&root, SUBAGENT_NAME, SUBAGENT.as_bytes());
        put(
            &root,
            &format!("{PAGINATED_NAME}.zst"),
            &compress(PAGINATED),
        );
        put(
            &root,
            "rollout-2026-11-17T08-00-00-019a9500-0000-7000-8000-00000000abcd.jsonl",
            LEGACY
                .replace(LEGACY_ID, "019a9500-0000-7000-8000-00000000abcd")
                .replace(CWD, "/home/dev/other")
                .as_bytes(),
        );

        let mut listed = sessions_in(&root, CWD);
        listed.sort_by(|a, b| a.id.cmp(&b.id));
        let summary: Vec<(&str, &str)> = listed
            .iter()
            .map(|s| (s.id.as_str(), s.label.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                (LEGACY_ID, "Add a cart total to the checkout page"),
                (PAGINATED_ID, "Rename the checkout step to review"),
            ]
        );
        let mut ids: Vec<String> = session_ids_in(&root, &format!("{CWD}/"))
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        assert_eq!(ids, [LEGACY_ID, PAGINATED_ID]);
        assert!(sessions_in(&root, "/home/dev").is_empty());
        fs::remove_dir_all(&root).ok();
    }

    /// A reverted thread goes on in a second file whose own records start
    /// after the first prompt: it is listed once, newest, with the label of
    /// the older file.
    #[test]
    fn a_reverted_thread_is_listed_once_with_its_first_prompt() {
        let root = temp_dir("revert");
        let old = put(&root, LEGACY_NAME, LEGACY.as_bytes());
        let first_line = LEGACY.lines().next().unwrap();
        let newer = put(
            &root,
            &LEGACY_NAME.replace(".jsonl", "_019a9b00-0000-7000-8000-000000000001.jsonl"),
            format!("{first_line}\n").as_bytes(),
        );
        let old_file = fs::File::options().write(true).open(&old).unwrap();
        old_file
            .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1_000_000))
            .unwrap();

        let listed = sessions_in(&root, CWD);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].label, "Add a cart total to the checkout page");
        assert_eq!(
            find_rollout(std::slice::from_ref(&root), LEGACY_ID),
            Some(newer)
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_compressed_rollout_reads_back_whole_even_in_several_frames() {
        let root = temp_dir("frames");
        // Cut on a line boundary, as an append after compression would.
        let cut = LEGACY[..LEGACY.len() / 2].rfind('\n').unwrap() + 1;
        let (head, tail) = LEGACY.split_at(cut);
        let mut bytes = compress(head);
        bytes.extend(compress(tail));
        let path = put(&root, &format!("{LEGACY_NAME}.zst"), &bytes);
        let mut text = String::new();
        open_rollout(&path)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, LEGACY);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn folders_compare_whatever_their_separators_and_trailing_slash() {
        assert_eq!(folder_key(r"C:\dev\app\"), folder_key("C:/dev/app"));
        assert_eq!(folder_key(r"\\?\C:\dev\app"), folder_key("C:/dev/app"));
        assert_eq!(folder_key("/"), "/");
    }
}
