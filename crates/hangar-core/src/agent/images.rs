//! Images a Codex session generates (docs/native/agents.md §5.3, D15): found
//! where Codex says it made one, and told to the rest of Hangar as
//! [`Event::ImageGenerated`] — what the chat view, the pane's inspector,
//! Hangar Mobile, Discord and `delegate` show or hand back.
//!
//! **Where the file is.** Codex's image tool (`imagegen`, the `image_gen`
//! extension) saves each image it makes under the home the app server gives
//! it (`app-server/src/extensions.rs`: `config.codex_home`), as
//! `generated_images/<thread>/<call>.png` — `.gif` for an animation — or,
//! with no home to save to, in the working folder's `generated_images/`
//! (`ext/image-generation/src/artifact.rs`, `tool.rs`:
//! `save_image_generation_result`; paths under `codex-rs/` at rust-v0.170.0).
//! Hangar reads the file where Codex saved it and **never moves, copies or
//! deletes it**: Codex tells its model the same ("copy it and leave the
//! original in place").
//!
//! **Where Hangar hears of it**, one place per way Codex runs (§4.1):
//!
//! - **the protocol**, for a pane Hangar is attached to and for a headless
//!   child: an `item/completed` whose item is an `imageGeneration` —
//!   [`on_item`], called by the pane's client (`codex_attach.rs`) and by the
//!   delegate runner for its children (T11a), whose ids stand where a pane's
//!   would;
//! - **the rollout**, for a pane on the floor (D6), which has no client:
//!   [`follow_rollout`] reads the thread's rollout as it grows
//!   ([`RolloutImages`]). A legacy thread records `image_generation_end`
//!   events; a paginated one (Codex 0.158 and later) records `item_completed`
//!   events instead, whose item is the extension's (`{"type": "Extension",
//!   "kind": "image_gen.generation", …}`, camelCase fields) or core's own for
//!   the Responses API's hosted tool (`{"type": "ImageGeneration", …}`,
//!   snake_case). `rollout/src/policy.rs` keeps one set or the other by the
//!   thread's history mode, never both; a call told twice is told once anyway.
//!
//! **What counts (D15).** A generation whose status is `completed`, with no
//! `failure`, and a saved path. One still running, one that failed (a usage
//! limit), and one that completed but whose file Codex could not write
//! (`save_image_generation_result` gives no path then) are no image to show:
//! the chat shows them as calls (chat/codex.rs).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use tokio::task::JoinHandle;

use crate::codex_app::generated as protocol;
use crate::host::{Event, Host};
use crate::sessions::codex::{self as rollouts, Record};

/// The status Codex gives a generation that made its image
/// (`ext/image-generation/src/tool.rs`; the Responses API's hosted tool,
/// core's `ImageGenerationItem`, says the same).
const COMPLETED: &str = "completed";

/// How often a floor pane's rollout is looked at: an image takes Codex tens
/// of seconds to make, and a look reads only what was appended.
const ROLLOUT_POLL: Duration = Duration::from_secs(2);

/// How often a rollout not found yet is looked for: finding one walks
/// Codex's dated folders (`sessions::codex::locate`).
const ROLLOUT_SEARCH: Duration = Duration::from_secs(10);

/// An image Codex generated and saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    /// The generation's call id, which is its item's: one image per call.
    pub id: String,
    /// Where Codex saved it, absolute. Codex's file: read there, never moved.
    pub path: PathBuf,
    /// What the image was made from, as Codex passed it on to the image
    /// model (`revisedPrompt`), when it says.
    pub prompt: Option<String>,
}

/// The rule (D15): where a generation's image is, when its status is
/// `completed`, it did not fail, and Codex saved it somewhere.
pub(crate) fn saved<'a>(
    status: &str,
    failed: bool,
    saved_path: Option<&'a str>,
) -> Option<&'a str> {
    if status != COMPLETED || failed {
        return None;
    }
    saved_path.filter(|path| !path.is_empty())
}

fn image(
    id: &str,
    status: &str,
    failed: bool,
    saved_path: Option<&str>,
    prompt: Option<&str>,
) -> Option<GeneratedImage> {
    let path = saved(status, failed, saved_path)?;
    Some(GeneratedImage {
        id: id.to_string(),
        path: PathBuf::from(path),
        prompt: prompt
            .map(str::trim)
            .filter(|prompt| !prompt.is_empty())
            .map(str::to_string),
    })
}

// ---------------------------------------------------------------------------
// The protocol: an attached pane, a headless child
// ---------------------------------------------------------------------------

/// The image a protocol item is, when it is a completed generation.
pub fn from_item(item: &protocol::ThreadItem) -> Option<GeneratedImage> {
    let protocol::ThreadItem::ImageGeneration(item) = item else {
        return None;
    };
    image(
        &item.id,
        &item.status,
        item.failure.is_some(),
        item.saved_path.as_deref(),
        item.revised_prompt.as_deref(),
    )
}

/// Tells Hangar that `image` was made in session `session` (the Codex
/// thread) of `pane` — a pane's id, or a headless child's, which has one like
/// a pane's (§4.3): [`Event::ImageGenerated`].
pub fn raise(host: &Host, pane: &str, session: &str, image: &GeneratedImage) {
    // Nobody listening (no window yet) is nothing to report: the chat finds
    // the image again in the thread's items.
    let _ = host.emit(Event::ImageGenerated {
        pane: pane.to_string(),
        session: session.to_string(),
        path: image.path.clone(),
        prompt: image.prompt.clone(),
    });
}

/// An item Codex completed in thread `thread`, for `pane`: a pane Hangar is
/// attached to (`codex_attach.rs`, on `item/completed`) or a headless child
/// (the delegate runner, T11a, with the child's id as `pane`). When it is a
/// generated image, raises [`Event::ImageGenerated`] and returns it, for a
/// caller that keeps what its child made (`delegate`'s `images`).
pub fn on_item(
    host: &Host,
    pane: &str,
    thread: &str,
    item: &protocol::ThreadItem,
) -> Option<GeneratedImage> {
    let image = from_item(item)?;
    raise(host, pane, thread, &image);
    Some(image)
}

// ---------------------------------------------------------------------------
// The rollout: a pane on the floor
// ---------------------------------------------------------------------------

/// An `event_msg`'s type, read before the rest of it.
#[derive(Deserialize)]
struct EventType<'a> {
    #[serde(rename = "type", borrow)]
    kind: Cow<'a, str>,
}

/// A legacy thread's `image_generation_end` (`protocol/src/protocol.rs`,
/// `ImageGenerationEndEvent`): snake_case, the image's data beside it, which
/// is not read.
#[derive(Deserialize)]
struct LegacyEnd {
    call_id: String,
    status: String,
    revised_prompt: Option<String>,
    saved_path: Option<String>,
    failure: Option<Value>,
}

/// A paginated thread's `item_completed` (`ItemCompletedEvent`).
#[derive(Deserialize)]
struct ItemCompleted {
    item: TurnItem,
}

/// Core's `TurnItem` (`protocol/src/items.rs`), tagged by `type`: only the
/// two kinds an image comes in.
#[derive(Deserialize)]
#[serde(tag = "type")]
enum TurnItem {
    /// The Responses API's hosted generation, core's own
    /// (`ImageGenerationItem`): snake_case, no failure.
    ImageGeneration {
        id: String,
        status: String,
        revised_prompt: Option<String>,
        saved_path: Option<String>,
    },
    /// An item an extension owns (`ext/items/src/lib.rs`, `ExtensionItem`).
    Extension(Extension),
    #[serde(other)]
    Other,
}

/// `ExtensionItem`, tagged by `kind`: the image extension's item
/// (`ext/items/src/image_generation.rs`, `ImageGenerationItem`, camelCase).
#[derive(Deserialize)]
#[serde(tag = "kind")]
enum Extension {
    #[serde(rename = "image_gen.generation", rename_all = "camelCase")]
    ImageGeneration {
        id: String,
        status: String,
        revised_prompt: Option<String>,
        saved_path: Option<String>,
        failure: Option<Value>,
    },
    #[serde(other)]
    Other,
}

/// The image an `event_msg` payload says was made, legacy or paginated.
fn from_event(payload: &str) -> Option<GeneratedImage> {
    let kind: EventType = serde_json::from_str(payload).ok()?;
    match &*kind.kind {
        "image_generation_end" => {
            let end: LegacyEnd = serde_json::from_str(payload).ok()?;
            image(
                &end.call_id,
                &end.status,
                end.failure.is_some(),
                end.saved_path.as_deref(),
                end.revised_prompt.as_deref(),
            )
        }
        "item_completed" => match serde_json::from_str::<ItemCompleted>(payload).ok()?.item {
            TurnItem::ImageGeneration {
                id,
                status,
                revised_prompt,
                saved_path,
            } => image(
                &id,
                &status,
                false,
                saved_path.as_deref(),
                revised_prompt.as_deref(),
            ),
            TurnItem::Extension(Extension::ImageGeneration {
                id,
                status,
                revised_prompt,
                saved_path,
                failure,
            }) => image(
                &id,
                &status,
                failure.is_some(),
                saved_path.as_deref(),
                revised_prompt.as_deref(),
            ),
            TurnItem::Extension(Extension::Other) | TurnItem::Other => None,
        },
        _ => None,
    }
}

/// The image a rollout line says was made, and when the line was recorded
/// (ms) when it says.
fn from_line(line: &str) -> Option<(GeneratedImage, Option<i64>)> {
    // Most lines are no image, and some are megabytes (a screenshot's
    // data): every image line names its kind, the legacy `image_generation_end`
    // and the extension's `image_gen.generation` by "image_gen", core's own by
    // "ImageGeneration".
    if !(line.contains("image_gen") || line.contains("ImageGeneration")) {
        return None;
    }
    let record: Record = serde_json::from_str(line).ok()?;
    if record.kind.as_deref() != Some("event_msg") {
        return None;
    }
    let image = from_event(record.payload?.get())?;
    let at = record
        .timestamp
        .as_deref()
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.timestamp_millis());
    Some((image, at))
}

/// The image a rollout line says was made: a completed generation, from a
/// legacy or a paginated thread.
pub fn from_rollout_line(line: &str) -> Option<GeneratedImage> {
    from_line(line).map(|(image, _)| image)
}

/// The images of one thread's rollout, read as the file grows: each look
/// reads only the complete lines appended since the last, and says each call
/// once.
#[derive(Debug, Default)]
pub struct RolloutImages {
    /// Images recorded before this (ms) are history, not news: a resumed
    /// thread's older images are in its chat already.
    since: Option<i64>,
    /// The file read, and where its next unread line starts.
    path: Option<PathBuf>,
    offset: u64,
    /// A compressed rollout, read whole once: Codex never appends to one.
    whole: bool,
    /// The calls already told.
    told: HashSet<String>,
}

impl RolloutImages {
    /// Nothing read yet; the images recorded before `since` (ms), when given,
    /// are left untold. A line without a time is told.
    pub fn since(since: Option<i64>) -> Self {
        Self {
            since,
            ..Self::default()
        }
    }

    /// The images `path`, the thread's rollout now, gained since the last
    /// look. A line still being written waits for its end. Another path than
    /// the last one — a compressed rollout giving way to its plain twin when
    /// the thread is resumed, a thread archived — or a file shorter than what
    /// was read is read from its start again, the calls told staying told.
    /// Read only: neither the rollout nor an image is touched.
    pub fn read(&mut self, path: &Path) -> std::io::Result<Vec<GeneratedImage>> {
        if self.path.as_deref() != Some(path) {
            self.path = Some(path.to_path_buf());
            self.offset = 0;
            self.whole = false;
        }
        let mut found = Vec::new();
        if rollouts::is_compressed(path) {
            // Codex writes a compressed thread back plain before it appends
            // (`rollout/src/compression.rs`): a new path, read then.
            if !self.whole {
                let mut reader = BufReader::new(rollouts::open_rollout(path)?);
                self.scan(&mut reader, true, &mut found)?;
                self.whole = true;
            }
            return Ok(found);
        }
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        if len < self.offset {
            self.offset = 0;
        }
        if len == self.offset {
            return Ok(found);
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut reader = BufReader::new(file);
        self.offset += self.scan(&mut reader, false, &mut found)?;
        Ok(found)
    }

    /// Reads `reader`'s lines into `found`, and says how many bytes of
    /// complete lines it read. `to_the_end`: a last line without its newline
    /// counts (a whole file); otherwise it is one still being written.
    fn scan(
        &mut self,
        reader: &mut impl BufRead,
        to_the_end: bool,
        found: &mut Vec<GeneratedImage>,
    ) -> std::io::Result<u64> {
        let mut read = 0;
        let mut line = Vec::new();
        loop {
            line.clear();
            let n = reader.read_until(b'\n', &mut line)?;
            if n == 0 || (line.last() != Some(&b'\n') && !to_the_end) {
                break;
            }
            read += n as u64;
            // A line that is not UTF-8 is not JSON either.
            if let Ok(text) = std::str::from_utf8(&line) {
                found.extend(self.take(text.trim_end()));
            }
        }
        Ok(read)
    }

    /// The image of one line, unless it is history or told already.
    fn take(&mut self, line: &str) -> Option<GeneratedImage> {
        let (image, at) = from_line(line)?;
        if let (Some(since), Some(at)) = (self.since, at) {
            if at < since {
                return None;
            }
        }
        self.told.insert(image.id.clone()).then_some(image)
    }
}

/// Where a thread's rollout is now, or None while it has none.
type Locate = Arc<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;

/// A floor pane's rollout followed, by pane.
struct Following {
    thread: String,
    task: JoinHandle<()>,
}

fn following() -> MutexGuard<'static, HashMap<String, Following>> {
    static FOLLOWING: OnceLock<Mutex<HashMap<String, Following>>> = OnceLock::new();
    FOLLOWING
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Follows thread `thread`'s rollout for `pane`, a Codex pane on the floor
/// (D6), which no client of Hangar's hears: a task on the current tokio
/// runtime that looks at the rollout every couple of seconds and raises
/// [`Event::ImageGenerated`] for each image it gains. Images recorded before
/// `since` (ms; the pane's launch, say) are left untold; None tells every
/// image the rollout holds. The pane's session changing is another call with
/// the new thread, which replaces the follower; following the same thread
/// again changes nothing. False outside a runtime.
///
/// Its caller is whoever knows a floor pane's session (the engine, which
/// learns it from the rollouts); [`unfollow_rollout`] when the pane ends.
pub fn follow_rollout(host: &Host, pane: &str, thread: &str, since: Option<i64>) -> bool {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return false;
    };
    let locate: Locate = Arc::new(rollouts::locate);
    start(&runtime, host, pane, thread, since, locate, ROLLOUT_POLL);
    true
}

/// Stops following pane `pane`'s rollout, if it was followed.
pub fn unfollow_rollout(pane: &str) {
    let gone = following().remove(pane);
    if let Some(gone) = gone {
        gone.task.abort();
    }
}

/// [`follow_rollout`], with where rollouts are found and how often they are
/// looked at given.
fn start(
    runtime: &tokio::runtime::Handle,
    host: &Host,
    pane: &str,
    thread: &str,
    since: Option<i64>,
    locate: Locate,
    every: Duration,
) {
    let mut following = following();
    if following
        .get(pane)
        .is_some_and(|f| f.thread == thread && !f.task.is_finished())
    {
        return;
    }
    let task = runtime.spawn(follow(
        host.clone(),
        pane.to_string(),
        thread.to_string(),
        RolloutImages::since(since),
        locate,
        every,
    ));
    let replaced = following.insert(
        pane.to_string(),
        Following {
            thread: thread.to_string(),
            task,
        },
    );
    if let Some(replaced) = replaced {
        replaced.task.abort();
    }
}

async fn follow(
    host: Host,
    pane: String,
    thread: String,
    mut images: RolloutImages,
    locate: Locate,
    every: Duration,
) {
    loop {
        let (id, find) = (thread.clone(), locate.clone());
        // Off the runtime's threads: a first look reads a long rollout.
        let looked = tokio::task::spawn_blocking(move || {
            let found = find(&id).map(|path| {
                // A rollout that cannot be read now (moved between the
                // search and the read) is read at the next look.
                images.read(&path).unwrap_or_default()
            });
            (images, found)
        })
        .await;
        let Ok((back, found)) = looked else {
            return;
        };
        images = back;
        let wait = match found {
            Some(found) => {
                for image in &found {
                    raise(&host, &pane, &thread, image);
                }
                every
            }
            None => every.max(ROLLOUT_SEARCH),
        };
        tokio::time::sleep(wait).await;
    }
}

#[cfg(test)]
mod tests {
    //! Every fixture here was written by hand (docs/native/agents.md §4.7,
    //! decision 1), never recorded: each says where its shape comes from —
    //! "from the protocol schema" (Codex 0.170.0) or "from the source"
    //! (openai/codex at rust-v0.170.0, with the file).

    use std::fs;
    use std::io::Write;

    use serde_json::json;
    use tokio::sync::broadcast;

    use super::*;
    use crate::paths::Paths;
    use crate::sessions::codex::tests::compress;

    const THREAD: &str = "019aa0b1-6c2e-7d41-8f3a-1b2c3d4e5f60";
    const SAVED: &str =
        "/home/dev/.codex/generated_images/019aa0b1-6c2e-7d41-8f3a-1b2c3d4e5f60/ig_1.png";

    /// The paginated rollout the chat's tests read, from the source (its
    /// first line names the files): one hosted generation, completed.
    const PAGINATED: &str = include_str!(
        "../sessions/fixtures/rollout-2026-11-19T16-20-05-019aa0b1-6c2e-7d41-8f3a-1b2c3d4e5f60.jsonl"
    );

    fn host() -> (Host, broadcast::Receiver<Event>) {
        let host = Host::with_paths(Paths::rooted(&crate::testdir::scratch("images-host")));
        let events = host.subscribe();
        (host, events)
    }

    /// From the protocol schema: a `ThreadItem`.
    fn item(value: Value) -> protocol::ThreadItem {
        serde_json::from_value(value).expect("a thread item")
    }

    /// A rollout `event_msg` record at second `second` of 16:40, from the
    /// source (`history/src/rollout_payload.rs`).
    fn record(second: u32, payload: Value) -> String {
        json!({
            "timestamp": format!("2026-11-19T16:40:{second:02}.000Z"),
            "type": "event_msg",
            "payload": payload,
        })
        .to_string()
    }

    /// From the source: the extension's item as a paginated rollout keeps it
    /// (`ext/items/src/lib.rs`, `image_generation.rs`; `tool.rs` fills it),
    /// inside core's `item_completed` (`protocol/src/protocol.rs`).
    fn paginated(
        second: u32,
        id: &str,
        status: &str,
        saved: Option<&str>,
        failure: Value,
    ) -> String {
        let mut item = json!({
            "type": "Extension", "kind": "image_gen.generation",
            "id": id, "status": status, "revisedPrompt": "A blue whale, paper cut-out",
            "result": "iVBORw0KGgo=", "transparentBackground": false,
            "failure": failure, "generationId": "gen-1",
        });
        if let Some(saved) = saved {
            item["savedPath"] = json!(saved);
        }
        record(
            second,
            json!({ "type": "item_completed", "thread_id": THREAD, "turn_id": "turn-1",
                    "item": item, "completed_at_ms": 1_795_106_400_000_i64 }),
        )
    }

    /// From the source: a legacy thread's `ImageGenerationEndEvent`
    /// (`protocol/src/protocol.rs`; `tool.rs`, `legacy_end_event`).
    fn legacy(second: u32, id: &str, status: &str, saved: Option<&str>) -> String {
        let mut end = json!({
            "type": "image_generation_end", "call_id": id, "status": status,
            "revised_prompt": "A blue whale, paper cut-out", "result": "iVBORw0KGgo=",
        });
        if let Some(saved) = saved {
            end["saved_path"] = json!(saved);
        }
        record(second, end)
    }

    fn whale(id: &str, path: &str) -> GeneratedImage {
        GeneratedImage {
            id: id.into(),
            path: PathBuf::from(path),
            prompt: Some("A blue whale, paper cut-out".into()),
        }
    }

    #[test]
    fn only_a_completed_generation_with_a_file_is_an_image() {
        assert_eq!(saved("completed", false, Some("/a.png")), Some("/a.png"));
        assert_eq!(saved("completed", true, Some("/a.png")), None);
        assert_eq!(saved("in_progress", false, Some("/a.png")), None);
        assert_eq!(saved("failed", false, None), None);
        assert_eq!(saved("completed", false, None), None);
        assert_eq!(saved("completed", false, Some("")), None);
    }

    /// From the protocol schema: `imageGeneration` items.
    #[test]
    fn a_completed_protocol_item_is_an_image() {
        let completed = item(json!({
            "type": "imageGeneration", "id": "ig_1", "status": "completed",
            "revisedPrompt": "A blue whale, paper cut-out", "result": "iVBORw0KGgo=",
            "savedPath": SAVED, "failure": null,
        }));
        assert_eq!(from_item(&completed), Some(whale("ig_1", SAVED)));

        let quiet = item(json!({
            "type": "imageGeneration", "id": "ig_2", "status": "completed",
            "revisedPrompt": "  ", "savedPath": SAVED,
        }));
        assert_eq!(from_item(&quiet).unwrap().prompt, None);

        for not_one in [
            json!({ "type": "imageGeneration", "id": "ig_3", "status": "in_progress" }),
            json!({ "type": "imageGeneration", "id": "ig_4", "status": "failed",
                    "failure": { "type": "usageLimitExceeded", "limitId": "images", "resetsAt": null } }),
            // Completed, but the limit was met for a later one: a failure
            // says no, whatever the status.
            json!({ "type": "imageGeneration", "id": "ig_5", "status": "completed", "savedPath": SAVED,
                    "failure": { "type": "usageLimitExceeded", "limitId": "images", "resetsAt": 1_795_110_000 } }),
            // Completed, but Codex could not write the file.
            json!({ "type": "imageGeneration", "id": "ig_6", "status": "completed", "result": "iVBORw0KGgo=" }),
            json!({ "type": "imageView", "id": "v1", "path": SAVED }),
            json!({ "type": "agentMessage", "id": "m1", "text": "Here it is" }),
        ] {
            assert_eq!(from_item(&item(not_one.clone())), None, "{not_one}");
        }
    }

    /// From the protocol schema: `item/completed`'s params, for an attached
    /// pane's client or a headless child's runner.
    #[test]
    fn an_item_completed_raises_the_image_for_its_pane_and_thread() {
        let (host, mut events) = host();
        let note: protocol::ItemCompletedNotification = serde_json::from_value(json!({
            "threadId": THREAD, "turnId": "turn-1", "completedAtMs": 1_795_106_400_000_i64,
            "item": { "type": "imageGeneration", "id": "ig_1", "status": "completed",
                      "revisedPrompt": "A blue whale, paper cut-out", "savedPath": SAVED },
        }))
        .unwrap();
        let told = on_item(&host, "p-codex", &note.thread_id, &note.item);
        assert_eq!(told, Some(whale("ig_1", SAVED)));
        match events.try_recv().unwrap() {
            Event::ImageGenerated {
                pane,
                session,
                path,
                prompt,
            } => {
                assert_eq!((pane.as_str(), session.as_str()), ("p-codex", THREAD));
                assert_eq!(path, PathBuf::from(SAVED));
                assert_eq!(prompt.as_deref(), Some("A blue whale, paper cut-out"));
            }
            other => panic!("unexpected {other:?}"),
        }

        let message = item(json!({ "type": "agentMessage", "id": "m1", "text": "Done" }));
        assert_eq!(on_item(&host, "p-codex", THREAD, &message), None);
        assert!(events.try_recv().is_err(), "nothing told for a message");
    }

    #[test]
    fn rollout_lines_say_their_images_legacy_and_paginated() {
        assert_eq!(
            from_rollout_line(&legacy(1, "ig_1", "completed", Some(SAVED))),
            Some(whale("ig_1", SAVED))
        );
        assert_eq!(
            from_rollout_line(&paginated(2, "ig_2", "completed", Some(SAVED), Value::Null)),
            Some(whale("ig_2", SAVED))
        );
        // From the source: core's own item, for the Responses API's hosted
        // tool (`protocol/src/items.rs`, `ImageGenerationItem`).
        let hosted = record(
            3,
            json!({ "type": "item_completed", "thread_id": THREAD, "turn_id": "turn-1",
                    "item": { "type": "ImageGeneration", "id": "ig_3", "status": "completed",
                              "revised_prompt": "A blue whale, paper cut-out",
                              "result": "iVBORw0KGgo=", "saved_path": SAVED } }),
        );
        assert_eq!(from_rollout_line(&hosted), Some(whale("ig_3", SAVED)));

        // The rollout the chat reads: its one image, hosted, completed.
        let found: Vec<GeneratedImage> = PAGINATED.lines().filter_map(from_rollout_line).collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "ig_5e6f708192a3b4c5");
        assert!(found[0].path.ends_with(
            "generated_images/019aa0b1-6c2e-7d41-8f3a-1b2c3d4e5f60/ig_5e6f708192a3b4c5.png"
        ));
        assert_eq!(
            found[0].prompt.as_deref(),
            Some("A phone-width header with the cart total on the right")
        );

        let usage_limit =
            json!({ "type": "usageLimitExceeded", "limitId": "images", "resetsAt": null });
        for not_one in [
            legacy(4, "ig_4", "failed", None),
            legacy(5, "ig_5", "completed", None),
            paginated(6, "ig_6", "failed", None, usage_limit),
            paginated(7, "ig_7", "in_progress", None, Value::Null),
            // An `item_completed` of another kind that names an image.
            record(
                8,
                json!({ "type": "item_completed", "thread_id": THREAD, "turn_id": "turn-1",
                    "item": { "type": "AgentMessage", "id": "m1",
                              "content": [{ "type": "Text", "text": "image_gen is done" }] } }),
            ),
            // The extension's other items.
            record(
                9,
                json!({ "type": "item_completed", "thread_id": THREAD, "turn_id": "turn-1",
                    "item": { "type": "Extension", "kind": "web.search", "id": "ws_1",
                              "query": "image_gen" } }),
            ),
            // The model's own record of the call, not an event.
            json!({ "timestamp": "2026-11-19T16:40:10.000Z", "type": "response_item",
                    "payload": { "type": "image_generation_call", "id": "ig_8",
                                 "status": "completed", "result": "iVBORw0KGgo=" } })
            .to_string(),
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"image_generation_end\"".to_string(),
            "not json, image_gen".to_string(),
        ] {
            assert_eq!(from_rollout_line(&not_one), None, "{not_one}");
        }
    }

    fn append(path: &Path, text: &str) {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    fn rollout_path(tag: &str) -> PathBuf {
        crate::testdir::scratch(tag).join(format!("rollout-2026-11-19T16-40-00-{THREAD}.jsonl"))
    }

    #[test]
    fn a_growing_rollout_tells_each_image_once() {
        let path = rollout_path("images-grow");
        append(
            &path,
            &format!("{}\n", legacy(1, "ig_1", "completed", Some(SAVED))),
        );
        let mut images = RolloutImages::since(None);
        assert_eq!(images.read(&path).unwrap(), vec![whale("ig_1", SAVED)]);
        assert_eq!(images.read(&path).unwrap(), vec![], "nothing new");

        // A line still being written waits for its end.
        let second = paginated(2, "ig_2", "completed", Some("/tmp/ig_2.png"), Value::Null);
        let (head, tail) = second.split_at(40);
        append(&path, head);
        assert_eq!(images.read(&path).unwrap(), vec![]);
        append(&path, &format!("{tail}\n"));
        assert_eq!(
            images.read(&path).unwrap(),
            vec![whale("ig_2", "/tmp/ig_2.png")]
        );

        // The same call twice — a legacy and a paginated record of it — is
        // told once.
        append(
            &path,
            &format!(
                "{}\n",
                legacy(3, "ig_2", "completed", Some("/tmp/ig_2.png"))
            ),
        );
        assert_eq!(images.read(&path).unwrap(), vec![]);

        // Rewritten shorter: read again from its start, nothing told twice.
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                legacy(1, "ig_1", "completed", Some(SAVED)),
                legacy(4, "ig_4", "completed", Some("/tmp/ig_4.png"))
            ),
        )
        .unwrap();
        assert_eq!(
            images.read(&path).unwrap(),
            vec![whale("ig_4", "/tmp/ig_4.png")]
        );
    }

    #[test]
    fn images_recorded_before_the_follow_began_are_history() {
        let path = rollout_path("images-since");
        append(
            &path,
            &format!(
                "{}\n{}\n",
                legacy(1, "ig_old", "completed", Some(SAVED)),
                legacy(30, "ig_new", "completed", Some("/tmp/ig_new.png"))
            ),
        );
        // 16:40:10 on 2026-11-19.
        let since = chrono::DateTime::parse_from_rfc3339("2026-11-19T16:40:10.000Z")
            .unwrap()
            .timestamp_millis();
        let mut images = RolloutImages::since(Some(since));
        assert_eq!(
            images.read(&path).unwrap(),
            vec![whale("ig_new", "/tmp/ig_new.png")]
        );
    }

    #[test]
    fn a_compressed_rollout_is_read_whole_once_then_its_plain_twin() {
        let dir = crate::testdir::scratch("images-zst");
        let plain = dir.join(format!("rollout-2026-11-19T16-40-00-{THREAD}.jsonl"));
        let packed = dir.join(format!("rollout-2026-11-19T16-40-00-{THREAD}.jsonl.zst"));
        // No newline after the last line: a whole file's last line counts.
        let text = legacy(1, "ig_1", "completed", Some(SAVED));
        fs::write(&packed, compress(&text)).unwrap();
        let mut images = RolloutImages::since(None);
        assert_eq!(images.read(&packed).unwrap(), vec![whale("ig_1", SAVED)]);
        assert_eq!(images.read(&packed).unwrap(), vec![]);

        // Resumed: Codex writes the thread back plain and appends there.
        fs::write(
            &plain,
            format!(
                "{text}\n{}\n",
                legacy(2, "ig_2", "completed", Some("/tmp/ig_2.png"))
            ),
        )
        .unwrap();
        assert_eq!(
            images.read(&plain).unwrap(),
            vec![whale("ig_2", "/tmp/ig_2.png")]
        );
    }

    #[test]
    fn reading_leaves_codex_s_files_where_they_are() {
        let dir = crate::testdir::scratch("images-untouched");
        let picture = dir.join("generated_images").join(THREAD).join("ig_1.png");
        fs::create_dir_all(picture.parent().unwrap()).unwrap();
        fs::write(&picture, b"\x89PNG\r\n\x1a\n").unwrap();
        let path = dir.join(format!("rollout-2026-11-19T16-40-00-{THREAD}.jsonl"));
        let line = legacy(1, "ig_1", "completed", Some(picture.to_str().unwrap()));
        fs::write(&path, format!("{line}\n")).unwrap();

        let found = RolloutImages::since(None).read(&path).unwrap();
        assert_eq!(found[0].path, picture);
        assert_eq!(fs::read(&picture).unwrap(), b"\x89PNG\r\n\x1a\n");
        assert_eq!(fs::read_to_string(&path).unwrap(), format!("{line}\n"));
    }

    async fn image_event(events: &mut broadcast::Receiver<Event>) -> (String, String, PathBuf) {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("an image in time")
            .unwrap();
        match event {
            Event::ImageGenerated {
                pane,
                session,
                path,
                ..
            } => (pane, session, path),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_floor_pane_s_rollout_is_followed_until_it_is_let_go() {
        let (host, mut events) = host();
        let path = rollout_path("images-follow");
        let runtime = tokio::runtime::Handle::current();
        // The rollout appears after the follow began, as a new thread's does.
        let at = path.clone();
        let locate: Locate =
            Arc::new(move |thread: &str| (thread == THREAD && at.is_file()).then(|| at.clone()));
        let every = Duration::from_millis(20);
        start(
            &runtime,
            &host,
            "p-floor",
            THREAD,
            None,
            locate.clone(),
            every,
        );
        append(
            &path,
            &format!(
                "{}\n",
                paginated(1, "ig_1", "completed", Some(SAVED), Value::Null)
            ),
        );
        assert_eq!(
            image_event(&mut events).await,
            (
                "p-floor".to_string(),
                THREAD.to_string(),
                PathBuf::from(SAVED)
            )
        );

        // The same thread again: the follower stays, nothing is told twice.
        start(&runtime, &host, "p-floor", THREAD, None, locate, every);
        append(
            &path,
            &format!(
                "{}\n",
                legacy(2, "ig_2", "completed", Some("/tmp/ig_2.png"))
            ),
        );
        assert_eq!(
            image_event(&mut events).await.2,
            PathBuf::from("/tmp/ig_2.png")
        );

        unfollow_rollout("p-floor");
        assert!(!following().contains_key("p-floor"));
        append(
            &path,
            &format!(
                "{}\n",
                legacy(3, "ig_3", "completed", Some("/tmp/ig_3.png"))
            ),
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(events.try_recv().is_err(), "a pane let go tells nothing");
    }
}
