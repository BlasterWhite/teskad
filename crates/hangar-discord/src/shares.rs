//! What agents share with the phone (`phone_share`), posted on Discord too:
//! the file under the agent's name, like its other messages, in its thread
//! (a worker) or in its workspace's channel (the orchestrator, or a worker
//! whose thread is not open); past what Discord takes from a bot, a line
//! with its name and size that says it is on the phone.
//!
//! core copies each share to `<shares>/<id>/<name>` and keeps their records
//! in `<shares>/shares.json` (remote/files.rs). Its announcement
//! (`remote:shared`) exists in the desktop's build of core alone, which a
//! node's connector is built without; the records are what any Hangar
//! reads (`mobile_data::Share`). So the connector looks at the index at
//! each tick, and reads it when it changed: what it holds already when the
//! connector starts is not news.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use hangar_core::mobile_data::Share;
use hangar_engine::mobile::push::workspace_of_share;
use hangar_engine::state::types::PaneRole;

use crate::bot::Bot;
use crate::cards;
use crate::rest::{Refused, Upload};

/// The largest file Discord takes from a bot or a webhook on a server
/// with no boost: 20 MiB since September 3, 2026 (10 MiB from January
/// 2025). A boosted server takes more; a file past this one is announced
/// by a line, and so is one Discord refuses as too large.
pub const UPLOAD_LIMIT: u64 = 20 * 1024 * 1024;

/// The shares' index, beside their copies.
const INDEX: &str = "shares.json";

/// The shares' index as last read.
#[derive(Debug, Default)]
pub(crate) struct Seen {
    /// When it was last written, and its length, as last read.
    stamp: Option<(SystemTime, u64)>,
    /// The shares it held: posted, or passed over.
    ids: HashSet<String>,
}

/// When the index at `dir` was last written, and its length.
fn stamp_of(dir: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(dir.join(INDEX)).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// The records of the index at `dir`, oldest first; `None` while there is
/// none, or while it does not read (being written: read again next time).
fn records(dir: &Path) -> Option<Vec<Share>> {
    let raw = std::fs::read_to_string(dir.join(INDEX)).ok()?;
    let mut shares: Vec<Share> = serde_json::from_str(&raw).ok()?;
    shares.sort_by_key(|share| share.at);
    Some(shares)
}

/// A name of one part: a share's id and name are joined to the shares'
/// folder, and nothing else than its copy is ever read.
fn one_part(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}

/// Where the copy of `share` is, under the shares' folder `dir`.
pub(crate) fn copy_of(dir: &Path, share: &Share) -> Option<PathBuf> {
    (one_part(&share.id) && one_part(&share.name)).then(|| dir.join(&share.id).join(&share.name))
}

/// What Discord says of a file too large for it.
pub(crate) fn too_large(error: &Refused) -> bool {
    error.status == 413 || error.message.to_ascii_lowercase().contains("too large")
}

impl Bot {
    /// The folder of the shares' copies and their index.
    fn shares_dir(&self) -> Option<PathBuf> {
        self.host().shares().or_else(|| {
            self.0
                .dir
                .parent()
                .map(|dir| dir.join("remote").join("shares"))
        })
    }

    /// What was shared before the connector started is not news.
    pub(crate) fn mark_shares_seen(&self) {
        let Some(dir) = self.shares_dir() else {
            return;
        };
        let stamp = stamp_of(&dir);
        let Some(shares) = records(&dir) else {
            return;
        };
        let mut state = self.state();
        state.shares.stamp = stamp;
        state.shares.ids = shares.into_iter().map(|share| share.id).collect();
    }

    /// The shares made since the last look, each given to its agent's lane
    /// to be posted. In the shares' lane: one look at a time.
    pub(crate) async fn look_at_shares(&self) {
        let Some(dir) = self.shares_dir() else {
            return;
        };
        let Some(stamp) = stamp_of(&dir) else {
            return;
        };
        if self.state().shares.stamp == Some(stamp) {
            return;
        }
        let Some(shares) = records(&dir) else {
            return;
        };
        let fresh: Vec<Share> = {
            let mut state = self.state();
            state.shares.stamp = Some(stamp);
            let fresh: Vec<Share> = shares
                .iter()
                .filter(|share| !state.shares.ids.contains(&share.id))
                .cloned()
                .collect();
            // A share out of the index (core keeps the last 200) never
            // comes back to it.
            state.shares.ids = shares.into_iter().map(|share| share.id).collect();
            fresh
        };
        // Nothing is posted before the person is paired: what was shared
        // meanwhile is not posted afterwards either.
        if self.owner().is_none() {
            return;
        }
        for share in fresh {
            self.post_share_soon(&dir, share);
        }
    }

    /// `share` given to the lane of the agent that shared it, after what it
    /// said before: only from a workspace this Hangar has a channel for.
    fn post_share_soon(&self, dir: &Path, share: Share) {
        let snapshot = self.host().snapshot();
        let Some(ws) = workspace_of_share(&snapshot.state, &share) else {
            return;
        };
        if self.place(&ws.id).is_none() {
            return;
        }
        let Some(copy) = copy_of(dir, &share) else {
            return;
        };
        // The pane named in a share is its name, or its id when it has none.
        let pane = ws
            .panes
            .iter()
            .find(|pane| pane.name == share.pane)
            .or_else(|| ws.panes.iter().find(|pane| pane.id == share.pane))
            .map(|pane| pane.id.clone());
        let lane = pane
            .clone()
            .unwrap_or_else(|| format!("workspace:{}", ws.id));
        let (bot, ws) = (self.clone(), ws.id.clone());
        self.0.lanes.run(
            &lane,
            async move { bot.post_share(ws, pane, share, copy).await },
        );
    }

    /// `share` posted under its agent's name: in its thread when it is a
    /// worker whose thread is open, in the workspace's channel otherwise;
    /// the file attached when Discord takes it, a line that says it is on
    /// the phone when it is too large.
    pub(crate) async fn post_share(
        &self,
        ws: String,
        pane: Option<String>,
        share: Share,
        copy: PathBuf,
    ) {
        let words = self.words();
        let snapshot = self.host().snapshot();
        let found = pane.as_deref().and_then(|id| {
            snapshot
                .state
                .workspaces
                .iter()
                .find(|w| w.id == ws)
                .and_then(|w| w.panes.iter().find(|p| p.id == id))
        });
        let name = found.map_or_else(|| share.pane.clone(), |pane| pane.name.clone());
        let thread = found
            .filter(|pane| pane.role == Some(PaneRole::Worker))
            .and_then(|pane| {
                self.state()
                    .layout
                    .workers
                    .get(&pane.id)
                    .and_then(|worker| worker.thread.clone())
            });
        let said = cards::Shared {
            name: &name,
            file: &share.name,
            size: share.size,
            caption: share.caption.as_deref(),
        };
        if share.size <= UPLOAD_LIMIT {
            let read = tokio::task::spawn_blocking(move || std::fs::read(copy)).await;
            let bytes = match read {
                Ok(Ok(bytes)) => bytes,
                _ => {
                    eprintln!("discord: the copy of the share {} cannot be read", share.id);
                    return;
                }
            };
            let attachment = cards::attachment_name(&share.name);
            let card = cards::shared(
                words,
                &said,
                &attachment,
                cards::shown_in_place(&share.mime),
            );
            let upload = Upload {
                name: attachment,
                bytes,
            };
            match self
                .post_as(&ws, &name, thread.as_deref(), card, vec![upload])
                .await
            {
                Ok(_) => return,
                // Larger than this server takes: said by a line instead.
                Err(error) if too_large(&error) => {}
                Err(error) => {
                    eprintln!("discord: a file {name} shared: {error}");
                    return;
                }
            }
        }
        let line = cards::shared_away(words, &said);
        if let Err(error) = self
            .post_as(&ws, &name, thread.as_deref(), line, Vec::new())
            .await
        {
            eprintln!("discord: a file {name} shared: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share(id: &str, name: &str) -> Share {
        Share {
            id: id.into(),
            at: 1,
            workspace: "/srv/shop".into(),
            pane: "Ada".into(),
            name: name.into(),
            mime: "image/png".into(),
            kind: Default::default(),
            size: 3,
            caption: None,
        }
    }

    #[test]
    fn only_a_share_s_own_copy_is_ever_read() {
        let dir = Path::new("/data/remote/shares");
        assert_eq!(
            copy_of(dir, &share("abc", "shot.png")),
            Some(dir.join("abc").join("shot.png"))
        );
        for (id, name) in [
            ("..", "shot.png"),
            ("abc", "../../etc/passwd"),
            ("abc", ".."),
            ("a/b", "shot.png"),
            ("abc", "c:\\x"),
            ("", "shot.png"),
        ] {
            assert_eq!(copy_of(dir, &share(id, name)), None, "{id} {name}");
        }
    }

    #[test]
    fn discord_s_refusal_of_a_large_file_is_told_apart() {
        let refused = |status: u16, message: &str| Refused {
            status,
            message: message.into(),
        };
        assert!(too_large(&refused(413, "")));
        assert!(too_large(&refused(400, "Request entity too large")));
        assert!(!too_large(&refused(403, "Missing Permissions")));
    }

    #[test]
    fn the_index_is_read_oldest_first_and_new_shares_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        assert!(records(dir.path()).is_none());
        assert!(stamp_of(dir.path()).is_none());
        let mut late = share("b", "b.png");
        late.at = 9;
        let early = share("a", "a.png");
        let index = serde_json::to_string(&vec![late, early]).unwrap();
        std::fs::write(dir.path().join(INDEX), index).unwrap();
        let read = records(dir.path()).unwrap();
        let ids: Vec<&str> = read.iter().map(|share| share.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        assert!(stamp_of(dir.path()).is_some());
        // Being written: not read, tried again.
        std::fs::write(dir.path().join(INDEX), "[{").unwrap();
        assert!(records(dir.path()).is_none());
    }
}
