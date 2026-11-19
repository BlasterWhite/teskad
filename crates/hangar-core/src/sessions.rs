//! The conversations a pane can resume: Claude Code's transcripts and
//! Codex's rollouts (codex.rs). A plain shell pane keeps no session.
//!
//! The functions taking an `AgentKind` answer for either agent; the older
//! ones without it (`list_sessions`, `session_exists`, …) are Claude Code's,
//! kept as they were for their callers.

pub mod codex;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::agent::AgentKind;

#[derive(Debug, Clone, Serialize)]
pub struct AgentSession {
    pub id: String,
    /// First human message, used as a human-readable label in the UI.
    pub label: String,
    pub modified_ms: u64,
}

/// Claude Code flattens the project path into a single directory name by
/// replacing every non-alphanumeric character with a dash.
/// `C:\Users\me\Proj` -> `C--Users-me-Proj`
pub(crate) fn encode_project_dir(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// How much of a transcript's head is read to find its label. A single line
/// can be tens of MB (a paste), so lines are not read whole.
const LABEL_HEAD_BYTES: u64 = 256 * 1024;

/// What was read from files, by path: valid while the file's size and mtime
/// hold. The last field is when it was stored, to drop the oldest first.
pub(crate) type FileCache<T> = HashMap<PathBuf, (u64, u64, T, u64)>;

/// Labels already read, by transcript.
type LabelCache = FileCache<String>;

/// Labels kept at most; the oldest stored go first.
const LABEL_CACHE_MAX: usize = 2000;

/// What `cache` holds for `path` while its size and mtime are still these.
pub(crate) fn cached<T: Clone>(
    cache: &FileCache<T>,
    path: &Path,
    size: u64,
    modified_ms: u64,
) -> Option<T> {
    cache
        .get(path)
        .filter(|(s, m, _, _)| *s == size && *m == modified_ms)
        .map(|(_, _, value, _)| value.clone())
}

/// Stores `value` for `path`, dropping the oldest entry first when `cache`
/// already holds `max`.
pub(crate) fn insert_cached<T>(
    cache: &mut FileCache<T>,
    max: usize,
    path: &Path,
    size: u64,
    modified_ms: u64,
    value: T,
) {
    if cache.len() >= max && !cache.contains_key(path) {
        let oldest = cache
            .iter()
            .min_by_key(|(_, (_, _, _, stored))| *stored)
            .map(|(key, _)| key.clone());
        if let Some(key) = oldest {
            cache.remove(&key);
        }
    }
    let stored = cache.values().map(|v| v.3).max().map_or(0, |max| max + 1);
    cache.insert(path.to_path_buf(), (size, modified_ms, value, stored));
}

fn label_cache() -> &'static Mutex<LabelCache> {
    static CACHE: OnceLock<Mutex<LabelCache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Extracts a display label from the head of a `.jsonl` transcript: the first
/// real user message among its first 60 lines, within `LABEL_HEAD_BYTES`.
fn label_from_jsonl(path: &Path) -> String {
    let Ok(file) = fs::File::open(path) else {
        return String::new();
    };
    let mut head = Vec::new();
    if file.take(LABEL_HEAD_BYTES).read_to_end(&mut head).is_err() && head.is_empty() {
        return String::new();
    }
    let mut lines: Vec<&[u8]> = head.split(|b| *b == b'\n').collect();
    // The last piece has no newline after it: cut by the byte budget (or the
    // file's end, where it is the last line and still complete).
    let complete_end = head.len() < LABEL_HEAD_BYTES as usize;
    if !complete_end {
        lines.pop();
    }
    for line in lines.into_iter().take(60) {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(|t| t.as_str()) != Some("user") {
            continue;
        }

        let text = match value.get("message").and_then(|m| m.get("content")) {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join(" "),
            _ => continue,
        };

        let text = text.trim();
        // Skip slash commands and system-injected preambles.
        if text.is_empty() || text.starts_with('<') || text.starts_with("Caveat:") {
            continue;
        }
        let clean: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        return clean.chars().take(80).collect();
    }
    String::new()
}

fn insert_label(cache: &mut LabelCache, path: &Path, size: u64, modified_ms: u64, label: &str) {
    insert_cached(
        cache,
        LABEL_CACHE_MAX,
        path,
        size,
        modified_ms,
        label.to_string(),
    );
}

/// `label_from_jsonl`, not read again while the file's size and mtime hold.
fn cached_label(path: &Path, size: u64, modified_ms: u64) -> String {
    let hit = label_cache()
        .lock()
        .ok()
        .and_then(|c| cached(&*c, path, size, modified_ms));
    if let Some(label) = hit {
        return label;
    }
    let label = label_from_jsonl(path);
    if let Ok(mut cache) = label_cache().lock() {
        insert_label(&mut cache, path, size, modified_ms, &label);
    }
    label
}

struct Transcript {
    id: String,
    path: PathBuf,
    size: u64,
    modified_ms: u64,
}

/// The transcripts of `cwd` on every Claude account (accounts.rs), from the
/// directory listing and file metadata alone: no file is opened. Newest first.
fn transcripts_in(roots: &[PathBuf], cwd: &str) -> Vec<Transcript> {
    let mut found: Vec<Transcript> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(root.join(encode_project_dir(cwd))) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if !path.extension().is_some_and(|e| e == "jsonl") {
                continue;
            }
            let id = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !seen.insert(id.clone()) {
                continue;
            }
            let meta = entry.metadata().ok();
            let modified_ms = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            found.push(Transcript {
                id,
                path,
                size: meta.map(|m| m.len()).unwrap_or(0),
                modified_ms,
            });
        }
    }
    found.sort_by_key(|t| std::cmp::Reverse(t.modified_ms));
    found
}

fn claude_sessions_in(roots: &[PathBuf], cwd: &str) -> Vec<AgentSession> {
    transcripts_in(roots, cwd)
        .into_iter()
        .map(|t| AgentSession {
            label: cached_label(&t.path, t.size, t.modified_ms),
            id: t.id,
            modified_ms: t.modified_ms,
        })
        .collect()
}

/// The ids and modification times (ms) of the conversations kept for `cwd`,
/// newest first. Cheap: a directory listing and metadata, no transcript is
/// opened. For what only needs to know which sessions exist.
pub fn list_session_ids(cwd: String) -> Vec<(String, u64)> {
    list_session_ids_for(AgentKind::Claude, cwd)
}

/// `list_session_ids` for either agent. Codex files its rollouts by date, not
/// by folder, so for it the head of each rollout is read once (its first
/// line, which names the folder) and remembered while the file is unchanged.
pub fn list_session_ids_for(agent: AgentKind, cwd: String) -> Vec<(String, u64)> {
    list_session_ids_in(agent, &history_roots(agent), &cwd)
}

/// The folders `agent` files its conversations in on this machine: the
/// `projects/` folder of every Claude Code account, Codex's `sessions/` (its
/// accounts share it). Reads the accounts' index, not the conversations.
fn history_roots(agent: AgentKind) -> Vec<PathBuf> {
    match agent {
        AgentKind::Claude => crate::accounts::projects_dirs(),
        AgentKind::Codex => codex::home()
            .map(|home| home.join(codex::SESSIONS))
            .into_iter()
            .collect(),
    }
}

/// `list_session_ids_for`, under `roots` instead of the folders `agent`
/// keeps its conversations in here: for a caller that names them itself (a
/// test, with conversations of its own). Newest first, each id once.
pub fn list_session_ids_in(agent: AgentKind, roots: &[PathBuf], cwd: &str) -> Vec<(String, u64)> {
    match agent {
        AgentKind::Claude => transcripts_in(roots, cwd)
            .into_iter()
            .map(|t| (t.id, t.modified_ms))
            .collect(),
        AgentKind::Codex => {
            let mut ids: Vec<(String, u64)> = roots
                .iter()
                .flat_map(|root| codex::session_ids_in(root, cwd))
                .collect();
            ids.sort_by_key(|(_, modified_ms)| std::cmp::Reverse(*modified_ms));
            let mut seen = HashSet::new();
            ids.retain(|(id, _)| seen.insert(id.clone()));
            ids
        }
    }
}

/// The Claude Code conversations kept for `cwd`, newest first, with their
/// labels.
pub fn list_sessions(cwd: String) -> Vec<AgentSession> {
    list_sessions_for(AgentKind::Claude, cwd)
}

/// The conversations `agent` kept for `cwd`, newest first, with their labels:
/// what the session picker offers to resume. Codex's sub-agents and its own
/// background threads are left out — not conversations a person had.
pub fn list_sessions_for(agent: AgentKind, cwd: String) -> Vec<AgentSession> {
    match agent {
        AgentKind::Claude => claude_sessions_in(&crate::accounts::projects_dirs(), &cwd),
        AgentKind::Codex => codex::home()
            .map(|home| codex::sessions_in(&home.join(codex::SESSIONS), &cwd))
            .unwrap_or_default(),
    }
}

/// Whether an id can be looked for on disk: it is joined into paths and
/// compared with file names, so anything but a plain id is not ours to judge.
fn plain_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether the conversation a pane is about to resume has a transcript left:
/// `Some(false)` only when it is nowhere on disk, `None` when that cannot be
/// told.
///
/// Claude Code takes its session id at startup but writes nothing until the
/// first message. A pane reset and then left alone keeps an id with no
/// conversation behind it, and `--resume` on that id only prints "No
/// conversation found". Every project directory is searched, not just the
/// pane's: a transcript filed under another spelling of the path is still
/// worth handing to the agent.
pub fn session_exists(id: String) -> Option<bool> {
    session_exists_for(AgentKind::Claude, id)
}

/// `session_exists` for either agent. Codex's `codex resume <id>` looks in
/// `sessions/` only — an archived thread has to be unarchived first — so
/// that is where its rollout is looked for.
pub fn session_exists_for(agent: AgentKind, id: String) -> Option<bool> {
    if !plain_id(&id) {
        return None;
    }
    session_exists_in(agent, &history_roots(agent), &id)
}

/// `session_exists_for`, under `roots` instead of the folders `agent` keeps
/// its conversations in here (`list_session_ids_in`). Found under any of
/// them, or `None` when none could be read.
pub fn session_exists_in(agent: AgentKind, roots: &[PathBuf], id: &str) -> Option<bool> {
    if !plain_id(id) {
        return None;
    }
    match agent {
        AgentKind::Claude => claude_transcript_exists_in(roots, id),
        AgentKind::Codex => {
            let answers: Vec<bool> = roots
                .iter()
                .filter_map(|root| codex::rollout_exists(root, id))
                .collect();
            if answers.is_empty() {
                None
            } else {
                Some(answers.contains(&true))
            }
        }
    }
}

/// Found under any of `roots` — the folders every Claude account files its
/// conversations in — or `None` when none of them could be read.
fn claude_transcript_exists_in(roots: &[PathBuf], id: &str) -> Option<bool> {
    let answers: Vec<bool> = roots
        .iter()
        .filter_map(|root| claude_transcript_exists(root, id))
        .collect();
    if answers.is_empty() {
        None
    } else {
        Some(answers.contains(&true))
    }
}

fn claude_transcript_exists(projects: &Path, id: &str) -> Option<bool> {
    let file = format!("{id}.jsonl");
    let entries = fs::read_dir(projects).ok()?;
    let mut unsure = false;
    for entry in entries.filter_map(Result::ok) {
        let dir = entry.path();
        // A stray file (.DS_Store) is not a project folder: nothing in it.
        if !dir.is_dir() {
            continue;
        }
        match fs::metadata(dir.join(&file)) {
            Ok(meta) if meta.is_file() => return Some(true),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // Permissions, a transient I/O error: not proof of absence.
            Err(_) => unsure = true,
        }
    }
    if unsure {
        None
    } else {
        Some(false)
    }
}

/// Whether Claude Code is on PATH, so the UI can say when a pane will not
/// find it. `crate::install::agent` answers the same for either agent, with
/// the path and the version.
pub fn claude_installed() -> bool {
    crate::install::agent::on_path(AgentKind::Claude).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        crate::testdir::scratch(&format!("sessions-{tag}"))
    }

    /// The case a reset leaves behind: an id Claude Code handed out, and no
    /// transcript anywhere because nobody wrote to it.
    #[test]
    fn a_claude_session_with_no_transcript_is_reported_gone() {
        let projects = temp_dir("claude");
        let here = projects.join("C--repo");
        let elsewhere = projects.join("C--other-spelling");
        fs::create_dir_all(&here).expect("project dir");
        fs::create_dir_all(&elsewhere).expect("project dir");
        // A stray file among the project folders is not a reason to doubt.
        fs::write(projects.join(".DS_Store"), "x").expect("stray file");
        fs::write(elsewhere.join("kept-1.jsonl"), "{}\n").expect("transcript");

        assert_eq!(claude_transcript_exists(&projects, "kept-1"), Some(true));
        assert_eq!(
            claude_transcript_exists(&projects, "never-written"),
            Some(false)
        );
        // No directory to look in is not proof of anything.
        assert_eq!(
            claude_transcript_exists(&projects.join("missing"), "kept-1"),
            None
        );

        fs::remove_dir_all(&projects).ok();
    }

    /// The main account's folder, which every account's profile shares, and
    /// a profile that kept one of its own: each conversation listed once.
    #[test]
    fn claude_sessions_are_read_from_every_accounts_folder_once() {
        let main = temp_dir("claude-main");
        let own = temp_dir("claude-own");
        let project = encode_project_dir(r"C:\repo");
        for (root, id) in [(&main, "shared-1"), (&own, "own-2"), (&own, "shared-1")] {
            let dir = root.join(&project);
            fs::create_dir_all(&dir).expect("project dir");
            fs::write(dir.join(format!("{id}.jsonl")), "{}\n").expect("transcript");
        }
        let roots = [main.clone(), own.clone()];

        let mut ids: Vec<String> = claude_sessions_in(&roots, r"C:\repo")
            .into_iter()
            .map(|session| session.id)
            .collect();
        ids.sort();
        assert_eq!(ids, ["own-2", "shared-1"]);
        assert_eq!(claude_transcript_exists_in(&roots, "own-2"), Some(true));
        assert_eq!(claude_transcript_exists_in(&roots, "nowhere"), Some(false));
        assert_eq!(
            claude_transcript_exists_in(&[main.join("missing")], "own-2"),
            None
        );

        fs::remove_dir_all(&main).ok();
        fs::remove_dir_all(&own).ok();
    }

    /// The label is the first prompt the person typed, not a slash command
    /// or the preamble Claude Code injects before it.
    #[test]
    fn a_session_is_labelled_by_its_first_real_prompt() {
        let root = temp_dir("label");
        let dir = root.join(encode_project_dir("/work/app"));
        fs::create_dir_all(&dir).expect("project dir");
        let transcript = [
            r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"fix   the bug"}]}}"#,
        ]
        .join("\n");
        fs::write(dir.join("s-1.jsonl"), transcript).expect("transcript");

        let listed = claude_sessions_in(std::slice::from_ref(&root), "/work/app");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "s-1");
        assert_eq!(listed[0].label, "fix the bug");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ids_and_mtimes_are_listed_newest_first_without_opening_files() {
        let root = temp_dir("ids");
        let dir = root.join(encode_project_dir("/work/ids"));
        fs::create_dir_all(&dir).expect("project dir");
        fs::write(dir.join("old.jsonl"), "{}\n").expect("transcript");
        fs::write(dir.join("new.jsonl"), "{}\n").expect("transcript");
        fs::write(dir.join("notes.txt"), "x").expect("other file");
        let old = fs::File::options()
            .write(true)
            .open(dir.join("old.jsonl"))
            .expect("open");
        old.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1000))
            .expect("mtime");

        let found = transcripts_in(std::slice::from_ref(&root), "/work/ids");
        let ids: Vec<(&str, u64)> = found
            .iter()
            .map(|t| (t.id.as_str(), t.modified_ms))
            .collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].0, "new");
        assert_eq!(ids[1], ("old", 1_000_000));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_huge_first_line_is_not_read_whole() {
        let root = temp_dir("huge");
        let path = root.join("big.jsonl");
        // 50 MB on one line, then the real prompt: the head budget ends first.
        let mut text = String::from(r#"{"type":"user","message":{"content":""#);
        text.push_str(&"a".repeat(50 * 1024 * 1024));
        text.push_str("\"}}\n");
        text.push_str(r#"{"type":"user","message":{"content":"later"}}"#);
        fs::write(&path, text).expect("transcript");

        // Nothing readable within the budget: no label, and no 50 MB read.
        assert_eq!(label_from_jsonl(&path), "");

        // A short line inside the budget is still found.
        let ok = root.join("ok.jsonl");
        fs::write(&ok, "{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}").expect("write");
        assert_eq!(label_from_jsonl(&ok), "hi");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_cached_label_is_reread_when_the_file_changes() {
        let root = temp_dir("cache");
        let path = root.join("c.jsonl");
        fs::write(&path, r#"{"type":"user","message":{"content":"first"}}"#).expect("write");
        assert_eq!(cached_label(&path, 10, 1), "first");

        // Same (size, mtime): served from the cache, the file is not read.
        fs::write(&path, r#"{"type":"user","message":{"content":"second"}}"#).expect("write");
        assert_eq!(cached_label(&path, 10, 1), "first");
        // A new mtime (or size) invalidates it.
        assert_eq!(cached_label(&path, 10, 2), "second");
        assert_eq!(cached_label(&path, 11, 2), "second");

        fs::remove_dir_all(&root).ok();
    }

    /// A project folder that cannot be searched is not proof the transcript
    /// is gone.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_project_folder_does_not_report_a_session_gone() {
        use std::os::unix::fs::PermissionsExt;
        let projects = temp_dir("denied");
        let locked = projects.join("C--locked");
        fs::create_dir_all(&locked).expect("project dir");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
        // Running as root ignores the mode: nothing to prove then.
        if fs::metadata(locked.join("x.jsonl"))
            .is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
        {
            assert_eq!(claude_transcript_exists(&projects, "x"), None);
        }
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
        fs::remove_dir_all(&projects).ok();
    }

    #[test]
    fn the_label_cache_drops_its_oldest_entry_when_full() {
        let mut cache = LabelCache::new();
        for i in 0..LABEL_CACHE_MAX {
            insert_label(&mut cache, Path::new(&format!("/t/{i}")), 1, 1, "l");
        }
        insert_label(&mut cache, Path::new("/t/new"), 1, 1, "l");
        assert_eq!(cache.len(), LABEL_CACHE_MAX);
        assert!(!cache.contains_key(Path::new("/t/0")));
        assert!(cache.contains_key(Path::new("/t/new")));
    }

    #[test]
    fn an_id_that_is_not_plain_is_left_to_the_agent() {
        assert_eq!(session_exists("../escape".into()), None);
        assert_eq!(session_exists(String::new()), None);
        for agent in AgentKind::ALL {
            assert_eq!(session_exists_for(agent, "../escape".into()), None);
            assert_eq!(session_exists_for(agent, "a b".into()), None);
            assert_eq!(session_exists_in(agent, &[], "../escape"), None);
        }
    }

    /// Codex's folders named by the caller: a thread found in either is
    /// listed once, by its newest file, and is there to resume; a folder that
    /// cannot be read says nothing.
    #[test]
    fn codex_threads_are_listed_and_found_under_the_folders_named() {
        use codex::tests::{
            put, CWD, LEGACY, LEGACY_ID, LEGACY_NAME, PAGINATED, PAGINATED_ID, PAGINATED_NAME,
        };
        let first = temp_dir("codex-roots-1");
        let second = temp_dir("codex-roots-2");
        let older = put(&first, LEGACY_NAME, LEGACY.as_bytes());
        let paginated = put(&first, PAGINATED_NAME, PAGINATED.as_bytes());
        let newer = put(&second, LEGACY_NAME, LEGACY.as_bytes());
        let at = |path: &Path, secs: u64| {
            fs::File::options()
                .write(true)
                .open(path)
                .expect("open")
                .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(secs))
                .expect("mtime");
        };
        at(&older, 1_000);
        at(&paginated, 2_000);
        at(&newer, 3_000);
        let roots = [first.clone(), second.clone()];

        assert_eq!(
            list_session_ids_in(AgentKind::Codex, &roots, CWD),
            [
                (LEGACY_ID.to_string(), 3_000_000),
                (PAGINATED_ID.to_string(), 2_000_000)
            ]
        );
        assert_eq!(
            session_exists_in(AgentKind::Codex, &roots, LEGACY_ID),
            Some(true)
        );
        assert_eq!(
            session_exists_in(
                AgentKind::Codex,
                &roots,
                "019a0000-0000-7000-8000-000000000000"
            ),
            Some(false)
        );
        let gone = first.join("not-there");
        assert_eq!(
            session_exists_in(AgentKind::Codex, &[gone], LEGACY_ID),
            None
        );
    }
}
