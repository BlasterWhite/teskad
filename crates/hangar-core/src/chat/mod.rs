//! A Claude Code pane's conversation as chat items, read from its transcript:
//! what Hangar Mobile shows instead of a copy of the terminal screen.
//!
//! The phone asks every second while a chat is open, and a session's
//! transcript runs to tens of megabytes, so a file is never read whole. First
//! contact reads its end, back to enough items for a page; scrolling back
//! reads further, span by span; each later look reads only what was appended
//! since — and nothing at all while the file's size and time are unchanged.
//!
//! - thread.rs: the lines of a span made into items, calls paired with their
//!   results.
//! - tools.rs: what one tool call reads as — title, input, result, answers.
//! - codex.rs: a Codex thread's items made into the same chat items, from the
//!   app server's protocol or from the thread's rollout file, read here the
//!   same way (a compressed rollout, whole).
//!
//! The transcript is read in file order. A rewind that forks the
//! conversation leaves both branches in the file, and both show.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use serde::Serialize;

pub mod codex;
mod thread;
mod tools;

#[cfg(test)]
mod tests;

use thread::Thread;

/// Items per page unless the phone asks otherwise, and at most.
const PAGE_DEFAULT: usize = 60;
const PAGE_MAX: usize = 200;
/// How far back one read goes. A span whose lines are all one giant line —
/// a screenshot's base64 — doubles until a line starts in it.
const SPAN_BYTES: u64 = 512 * 1024;
/// Transcripts kept parsed: a phone or two, each on a chat or two.
const CACHED: usize = 4;

/// Who wrote a user message: the person, or Hangar (its messages start with
/// "[Hangar]").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Sender {
    Person,
    Hangar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CallState {
    Running,
    Done,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallResult {
    pub ok: bool,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub question: String,
    pub header: String,
    pub multi_select: bool,
    pub options: Vec<QuestionOption>,
}

/// What the person picked for one question: labels, and their own words when
/// they wrote some.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Answer {
    pub selected: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Body {
    User {
        text: String,
        from: Sender,
        images: u32,
    },
    /// Markdown as Claude wrote it.
    Assistant { text: String },
    /// One tool call, or reads in a row folded into one.
    Action {
        tool: String,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<CallResult>,
        state: CallState,
    },
    #[serde(rename_all = "camelCase")]
    Question {
        tool_use_id: String,
        questions: Vec<Question>,
        /// Once the call has its result; absent when it was refused.
        #[serde(skip_serializing_if = "Option::is_none")]
        answer: Option<Vec<Answer>>,
    },
    /// Conversation compacted, turn interrupted, session resumed, an error
    /// Claude Code reported.
    Notice { text: String },
    /// An image the agent generated and saved (agent/images.rs): where the
    /// file is, absolute — the agent's own file, shown where it is — and what
    /// it was made from. A generation still running, failed or left unsaved
    /// is an `Action`. Codex only: Claude Code generates none.
    Image {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatItem {
    pub id: String,
    /// When its line was written, in ms.
    pub at: i64,
    #[serde(flatten)]
    pub body: Body,
}

/// An AskUserQuestion still without its answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingQuestion {
    pub tool_use_id: String,
    pub questions: Vec<Question>,
}

/// The last tool call when it has no result yet: what a permission prompt,
/// if the pane shows one, is about.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenCall {
    pub tool: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One page of the conversation, and what waits at its end.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatPage {
    /// Oldest first.
    pub items: Vec<ChatItem>,
    /// Whether there is anything before this page.
    pub older: bool,
    /// What to pass as `before` for the page before; null without one.
    pub cursor: Option<String>,
    /// Both from the end of the conversation, whatever the page.
    pub question: Option<PendingQuestion>,
    pub open_call: Option<OpenCall>,
}

/// What the lines of a transcript are made into: Claude Code's thread
/// (thread.rs) or a Codex rollout's items (codex.rs). The newest span is read
/// first; an older one is read into a fresh one and put in front.
trait Lines {
    /// An empty one like this one, to read an older span into.
    fn fresh(&self) -> Self;
    /// Feeds one complete line, which starts at `offset` in the file.
    fn feed(&mut self, offset: u64, text: &str);
    /// Puts an older span in front of this one.
    fn prepend(&mut self, older: Self);
    /// How many items start before `before` (all of them without one).
    fn count_before(&self, before: Option<u64>) -> usize;
    /// The `limit` items just before `before`, oldest first, the offset of
    /// the first one, and whether items older than the page were read.
    fn page(&self, before: Option<u64>, limit: usize) -> (Vec<ChatItem>, Option<u64>, bool);
    fn pending_question(&self) -> Option<PendingQuestion>;
    fn open_call(&self) -> Option<OpenCall>;
}

impl Lines for Thread {
    fn fresh(&self) -> Self {
        Thread::default()
    }
    fn feed(&mut self, offset: u64, text: &str) {
        Thread::feed(self, offset, text);
    }
    fn prepend(&mut self, older: Self) {
        Thread::prepend(self, older);
    }
    fn count_before(&self, before: Option<u64>) -> usize {
        Thread::count_before(self, before)
    }
    fn page(&self, before: Option<u64>, limit: usize) -> (Vec<ChatItem>, Option<u64>, bool) {
        Thread::page(self, before, limit)
    }
    fn pending_question(&self) -> Option<PendingQuestion> {
        Thread::pending_question(self)
    }
    fn open_call(&self) -> Option<OpenCall> {
        Thread::open_call(self)
    }
}

/// One transcript as far as it was read.
struct Span<T> {
    /// The file at the last look: unchanged, the parse stands as it is.
    len: u64,
    modified: Option<SystemTime>,
    /// The span read, from a line's start to the end of the last complete
    /// line. `start` reaches 0 once the head has been read.
    start: u64,
    end: u64,
    lines: T,
}

/// A Claude Code transcript as far as it was read.
type Transcript = Span<Thread>;

impl<T: Lines> Span<T> {
    /// Nothing read yet: the span sits at the end of the file's last
    /// complete line, ready to grow both ways.
    fn at_end(path: &Path, len: u64, lines: T) -> std::io::Result<Self> {
        let end = last_line_end(path, len)?;
        Ok(Self {
            len: 0,
            modified: None,
            start: end,
            end,
            lines,
        })
    }

    /// A compressed rollout, read whole: one zstd stream cannot be read from
    /// its end. Offsets are the decoded text's. Codex does not append to a
    /// compressed rollout — it writes the thread back plain first
    /// (`rollout/src/compression.rs`, `materialize_rollout_for_append`) — so a
    /// change of the file is a new file, read again.
    fn whole(
        path: &Path,
        len: u64,
        modified: Option<SystemTime>,
        mut lines: T,
    ) -> std::io::Result<Self> {
        let mut reader = BufReader::new(crate::sessions::codex::open_rollout(path)?);
        let mut line = Vec::new();
        let mut end = 0;
        loop {
            line.clear();
            let read = reader.read_until(b'\n', &mut line)?;
            if read == 0 {
                break;
            }
            feed(&mut lines, end, &line);
            end += read as u64;
        }
        Ok(Self {
            len,
            modified,
            start: 0,
            end,
            lines,
        })
    }
}

/// One transcript's place in the cache. Its own lock covers the file reads,
/// so a slow first contact with one transcript holds nobody else.
#[derive(Default)]
struct SlotOf<T> {
    /// `None` until read, and again once the file is gone.
    transcript: Mutex<Option<Span<T>>>,
    /// When it was last asked for, to drop the coldest past `CACHED`.
    used: AtomicU64,
}

#[cfg(test)]
type Slot = SlotOf<Thread>;

#[derive(Default)]
struct CacheOf<T> {
    /// Locked only to look a slot up, insert or evict: never across I/O.
    slots: Mutex<HashMap<PathBuf, Arc<SlotOf<T>>>>,
    tick: AtomicU64,
}

type Cache = CacheOf<Thread>;

static TRANSCRIPTS: OnceLock<Cache> = OnceLock::new();

impl<T> CacheOf<T> {
    /// The slot of `path`, created if new, marked as just used.
    fn slot(&self, path: &Path) -> Result<Arc<SlotOf<T>>, String>
    where
        SlotOf<T>: Default,
    {
        let mut slots = self.slots.lock().map_err(|_| "chat cache poisoned")?;
        let slot = slots.entry(path.to_path_buf()).or_default().clone();
        slot.used.store(
            self.tick.fetch_add(1, Ordering::Relaxed) + 1,
            Ordering::Relaxed,
        );
        evict(&mut slots);
        Ok(slot)
    }
}

fn read_range(path: &Path, from: u64, to: u64) -> std::io::Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    file.seek(SeekFrom::Start(from))?;
    let mut buf = Vec::with_capacity((to - from) as usize);
    file.take(to - from).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Where the file's last complete line ends: a line mid-write waits for its
/// newline before it counts.
fn last_line_end(path: &Path, len: u64) -> std::io::Result<u64> {
    let mut to = len;
    let mut span = SPAN_BYTES;
    while to > 0 {
        let from = to.saturating_sub(span);
        let buf = read_range(path, from, to)?;
        if let Some(at) = buf.iter().rposition(|&b| b == b'\n') {
            return Ok(from + at as u64 + 1);
        }
        to = from;
        span *= 2;
    }
    Ok(0)
}

/// Feeds the complete lines of `buf`, which starts at `base` in the file.
fn feed<T: Lines>(lines: &mut T, base: u64, buf: &[u8]) {
    let mut at = 0;
    for line in buf.split_inclusive(|&b| b == b'\n') {
        let offset = base + at as u64;
        at += line.len();
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        // A line that is not UTF-8 is not JSON either.
        if let Ok(text) = std::str::from_utf8(line) {
            if !text.trim().is_empty() {
                lines.feed(offset, text);
            }
        }
    }
}

/// Reads what was appended since the last look.
fn read_new<T: Lines>(path: &Path, transcript: &mut Span<T>, len: u64) -> std::io::Result<()> {
    if len <= transcript.end {
        return Ok(());
    }
    let buf = read_range(path, transcript.end, len)?;
    // Only complete lines: one mid-write waits for its newline.
    let Some(last) = buf.iter().rposition(|&b| b == b'\n') else {
        return Ok(());
    };
    feed(&mut transcript.lines, transcript.end, &buf[..=last]);
    transcript.end += last as u64 + 1;
    Ok(())
}

/// Reads further back until `wanted` items lie before `before`, or the
/// head of the file is reached.
fn read_back<T: Lines>(
    path: &Path,
    transcript: &mut Span<T>,
    before: Option<u64>,
    wanted: usize,
) -> std::io::Result<()> {
    let mut span = SPAN_BYTES;
    while transcript.start > 0 && transcript.lines.count_before(before) < wanted {
        let from = transcript.start.saturating_sub(span);
        let buf = read_range(path, from, transcript.start)?;
        // The span begins inside a line that started earlier: that line is
        // the next span's, unless the span reaches the head. The span's last
        // byte is the newline closing the line before `start`; only a newline
        // before it starts a line inside the span.
        let skip = if from == 0 {
            0
        } else {
            match buf[..buf.len() - 1].iter().position(|&b| b == b'\n') {
                Some(at) => at + 1,
                None => {
                    // One line longer than the span: look further.
                    span *= 2;
                    continue;
                }
            }
        };
        let mut older = transcript.lines.fresh();
        feed(&mut older, from + skip as u64, &buf[skip..]);
        transcript.lines.prepend(older);
        transcript.start = from + skip as u64;
    }
    Ok(())
}

fn modified(meta: &fs::Metadata) -> Option<SystemTime> {
    meta.modified().ok()
}

/// One page of the Claude Code transcript at `path`; `None` when there is no
/// such file. `slot` holds what was parsed of it so far.
fn page_of(
    path: &Path,
    slot: &mut Option<Transcript>,
    before: Option<u64>,
    limit: usize,
) -> std::io::Result<Option<ChatPage>> {
    page_in(path, slot, before, limit, Thread::default)
}

/// One page of the transcript at `path`, made into `T`s (`empty` makes the
/// first); `None` when there is no such file. `slot` holds what was parsed of
/// it so far.
fn page_in<T: Lines>(
    path: &Path,
    slot: &mut Option<Span<T>>,
    before: Option<u64>,
    limit: usize,
    empty: impl FnOnce() -> T,
) -> std::io::Result<Option<ChatPage>> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            *slot = None;
            return Ok(None);
        }
        Err(err) => return Err(err),
    };
    let len = meta.len();
    let time = modified(&meta);

    if crate::sessions::codex::is_compressed(path) {
        let unchanged = slot
            .as_ref()
            .is_some_and(|t| t.len == len && t.modified == time);
        if !unchanged {
            // The old parse goes first: two of a long rollout would be a
            // waste, and a failed read must not leave it standing.
            *slot = None;
            *slot = Some(Span::whole(path, len, time, empty())?);
        }
    } else {
        // Shorter than what was read, or rewritten in place at the same
        // size: the file is not the one parsed, so the parse starts over.
        let stale = slot
            .as_ref()
            .is_some_and(|t| len < t.end || (len == t.len && time != t.modified));
        if stale {
            *slot = None;
        }
        if slot.is_none() {
            *slot = Some(Span::at_end(path, len, empty())?);
        }
        let transcript = slot.as_mut().expect("filled above");

        if len != transcript.len || time != transcript.modified {
            read_new(path, transcript, len)?;
            transcript.len = len;
            transcript.modified = time;
        }
        read_back(path, transcript, before, limit)?;
    }
    let transcript = slot.as_ref().expect("filled above");

    let (items, first, more) = transcript.lines.page(before, limit);
    let older = more || transcript.start > 0;
    Ok(Some(ChatPage {
        items,
        older,
        cursor: first.filter(|_| older).map(|offset| offset.to_string()),
        question: transcript.lines.pending_question(),
        open_call: transcript.lines.open_call(),
    }))
}

/// Keeps at most `CACHED` slots, dropping the coldest. A slot being read
/// stays alive through its `Arc` until its reader is done.
fn evict<T>(slots: &mut HashMap<PathBuf, Arc<SlotOf<T>>>) {
    while slots.len() > CACHED {
        let coldest = slots
            .iter()
            .min_by_key(|(_, slot)| slot.used.load(Ordering::Relaxed))
            .map(|(path, _)| path.clone());
        match coldest {
            Some(path) => slots.remove(&path),
            None => break,
        };
    }
}

/// A chat's arguments checked: the session id that lands in a path, the
/// cursor as an offset, the page size within bounds.
fn chat_args(
    session_id: &str,
    before: Option<&str>,
    limit: Option<usize>,
) -> Result<(Option<u64>, usize), String> {
    if !crate::context::valid_session_id(session_id) {
        return Err(format!("\"{session_id}\" is not a session id."));
    }
    let before = match before {
        Some(cursor) => Some(
            cursor
                .parse::<u64>()
                .map_err(|_| format!("\"{cursor}\" is not a cursor this chat gave."))?,
        ),
        None => None,
    };
    Ok((before, limit.unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX)))
}

fn chat_of(
    session_id: &str,
    cwd: &str,
    before: Option<&str>,
    limit: Option<usize>,
) -> Result<Option<ChatPage>, String> {
    let (before, limit) = chat_args(session_id, before, limit)?;
    let Some(path) = crate::context::claude_transcript(cwd, session_id) else {
        return Ok(None);
    };
    let cache = TRANSCRIPTS.get_or_init(Cache::default);
    let slot = cache.slot(&path)?;
    // Only this transcript's lock is held across the reads.
    let mut transcript = slot.transcript.lock().map_err(|_| "chat cache poisoned")?;
    let page = page_of(&path, &mut transcript, before, limit).map_err(|err| err.to_string())?;
    Ok(page)
}

/// One page of a Claude Code session's conversation: the newest without
/// `before`, else the one before that cursor. `None` while the session has
/// no transcript yet.
pub async fn chat_page(
    session_id: String,
    cwd: String,
    before: Option<String>,
    limit: Option<usize>,
) -> Result<Option<ChatPage>, String> {
    crate::git::off_thread(move || chat_of(&session_id, &cwd, before.as_deref(), limit)).await
}
