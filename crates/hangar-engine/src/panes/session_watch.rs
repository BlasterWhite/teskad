//! Following the session a pane's agent is in (`watchForSession` of
//! `lib/agents.ts`), by what that agent can do (docs/native/agents.md §3.5,
//! the `sessions` and `transcript` capabilities) rather than by its name.
//!
//! An agent whose side channel reports its session id names it outright:
//! Claude Code's mod, Codex's attached client. Then no folder is listed at
//! all. Otherwise, with the channel silent (the mod off, Codex on the floor),
//! the agent's history is diffed, in the place and format of its kind (Claude
//! Code's transcripts, Codex's rollouts): whatever conversation appears that
//! the pane did not already know about belongs to it.
//!
//! This runs for the whole life of the pane, not just at launch. Typing /new
//! or /clear inside an agent abandons the current transcript for a fresh
//! one, and a pane that stopped watching would keep pointing at the old chat,
//! and reopen it on the next resume, which is not the conversation you left.
//!
//! `SessionWatch` is the loop's state with the sleeping taken out: the host
//! asks `due`, passes the round when `reported` says the agent named its
//! session itself, runs `hunt_listing` for the watch's `kind` off the UI
//! thread when `begin_round` says a look is worth it, and hands the answer to
//! `round_done`.

use std::collections::HashSet;

use hangar_core::agent::{AgentKind, SessionId};
use hangar_core::sessions::AgentSession;

use super::agents::Claims;
use crate::live::EpochMs;
use crate::state::types::AgentId;

/// Brisk while hunting the first id...
pub const HUNT_MS: u64 = 1500;
/// ...patient once the pane has one.
pub const FOLLOW_MS: u64 = 5000;
/// A pane that looked this many times and found nothing new (one launched
/// with `--resume` never does) is as patient as one that found its session.
/// `FOLLOW_MS` is still well inside `FRESH_MS`, so a late transcript is caught.
pub const EMPTY_ROUNDS_BEFORE_FOLLOW: u32 = 3;

/// Whether a pane running `agent` can be started again on the session it was
/// in: its agent resumes a session by id (`sessions.resume`). A shell keeps
/// no session; an agent that resumes none starts afresh every time.
pub fn resumes(agent: AgentId) -> bool {
    agent
        .kind()
        .is_some_and(|kind| kind.info().caps.sessions.resume)
}

/// The session a pane's next launch resumes: `session`, when its agent
/// resumes by id and the conversation is still where that agent keeps it
/// (Claude Code's transcripts, Codex's rollouts), or that cannot be told. An
/// id with nothing behind it starts afresh instead: Claude Code names a
/// session before writing anything, and resuming that only prints "No
/// conversation found". Core's launch builder turns the id into the kind's
/// own arguments (`claude --resume <id>`, `codex resume <id>`). Blocking: it
/// looks at the disk.
pub fn session_to_resume(agent: AgentId, session: Option<&str>) -> Option<String> {
    resume_if_kept(agent, session, |kind, id| {
        hangar_core::sessions::session_exists_for(kind, id.to_string())
    })
}

/// `session_to_resume`, with `kept` saying whether a kind still holds a
/// conversation.
fn resume_if_kept(
    agent: AgentId,
    session: Option<&str>,
    kept: impl FnOnce(AgentKind, &str) -> Option<bool>,
) -> Option<String> {
    let kind = agent
        .kind()
        .filter(|kind| kind.info().caps.sessions.resume)?;
    let id = session.filter(|id| !id.is_empty())?;
    (kept(kind, id) != Some(false)).then(|| id.to_string())
}

/// The listing a hunt round runs off the UI thread: the conversations `kind`
/// keeps for `cwd` (`SessionWatch::kind`), ids and modification times from
/// the folders alone, so the labels are empty (the hunt never reads them).
pub fn hunt_listing(kind: AgentKind, cwd: String) -> Vec<AgentSession> {
    listed(hangar_core::sessions::list_session_ids_for(kind, cwd))
}

fn listed(ids: Vec<(String, u64)>) -> Vec<AgentSession> {
    ids.into_iter()
        .map(|(id, modified_ms)| AgentSession {
            id,
            label: String::new(),
            modified_ms,
        })
        .collect()
}

/// A transcript has to be warm to be ours. Without this a pane could adopt
/// any file it happens not to know about (one left by another window, say)
/// instead of the conversation that just started under it.
pub const FRESH_MS: u64 = 15_000;

#[derive(Debug)]
pub struct SessionWatch {
    pub cwd: String,
    /// The agent whose history a round lists.
    kind: AgentKind,
    /// Its side channel names the session whenever it speaks
    /// (`sessions.id: Reported`).
    reports: bool,
    /// The sessions the pane knew when it launched, and every one it adopted
    /// since, so the next round hunts the one after it.
    known: HashSet<String>,
    interval_ms: u64,
    /// When the next look is due; `None` while one is in flight.
    next_at: Option<EpochMs>,
    /// Set by output, consumed once per round. A pane that printed nothing
    /// since the last look cannot have opened a session, so it is skipped
    /// without touching the disk, and a transcript another pane just created
    /// is far less likely to be mistaken for this one's.
    printed: bool,
    /// Rounds in a row that adopted nothing.
    empty_rounds: u32,
}

impl SessionWatch {
    /// The watch of a pane running `agent` in `cwd`, which knew `known`
    /// sessions when it launched. `None` when there is no session to follow:
    /// a shell, an agent that resumes none (`sessions.resume`), or one that
    /// keeps no history a new session could be found in (`transcript`).
    pub fn for_agent(
        agent: AgentId,
        cwd: String,
        known: HashSet<String>,
        now: EpochMs,
    ) -> Option<SessionWatch> {
        let kind = agent.kind()?;
        let caps = kind.info().caps;
        if !caps.sessions.resume || caps.transcript.is_none() {
            return None;
        }
        Some(SessionWatch {
            cwd,
            kind,
            reports: caps.sessions.id == SessionId::Reported,
            known,
            interval_ms: HUNT_MS,
            next_at: Some(now + HUNT_MS),
            printed: false,
            empty_rounds: 0,
        })
    }

    /// The agent whose history `hunt_listing` lists for this pane.
    pub fn kind(&self) -> AgentKind {
        self.kind
    }

    /// Whether the agent names its session itself right now, so a round due
    /// is passed without listing anything (`skip_round`): its kind reports
    /// the id (`sessions.id: Reported`) and its side channel spoke (Claude
    /// Code's mod said hello, Codex's attached client follows its thread).
    /// With the channel silent the id is hunted in the kind's history.
    pub fn reported(&self, side_channel_spoke: bool) -> bool {
        self.reports && side_channel_spoke
    }

    /// The pane printed something, at `now`. A pane at rest has nobody to
    /// wake it for its rounds (they would only schedule the next one), so the
    /// round it is owed is put back on its cadence: the first slot after the
    /// output, as if every round in between had been passed.
    pub fn note_output(&mut self, now: EpochMs) {
        if !self.printed
            && let Some(at) = self.next_at
            && at < now
        {
            let slots = (now - at).div_ceil(self.interval_ms);
            self.next_at = Some(at + slots * self.interval_ms);
        }
        self.printed = true;
    }

    /// When a round with something to do comes due: only after output, since
    /// a round that follows none just schedules the next one (`begin_round`,
    /// `skip_round`). `None`: nothing to look for until the pane prints, or a
    /// look is in flight.
    pub fn wake_at(&self) -> Option<EpochMs> {
        self.next_at.filter(|_| self.printed)
    }

    pub fn due(&self, now: EpochMs) -> bool {
        self.next_at.is_some_and(|at| at <= now)
    }

    /// Starts a round. `false`: nothing was printed since the last look, so
    /// the disk is left alone and the next round is scheduled.
    pub fn begin_round(&mut self, now: EpochMs) -> bool {
        if !std::mem::take(&mut self.printed) {
            self.next_at = Some(now + self.interval_ms);
            return false;
        }
        self.next_at = None;
        true
    }

    /// Passes a round without listing: the pane's session is named by its
    /// agent (`reported`), so whatever the disk says would be thrown away.
    /// Nothing is claimed, nothing is learnt, and the next look is scheduled.
    pub fn skip_round(&mut self, now: EpochMs) {
        self.printed = false;
        self.next_at = Some(now + self.interval_ms);
    }

    /// The listing of a round came back (`None`: it failed, which reads as
    /// nothing found). Answers the session to adopt, if there is one.
    /// `sessions` are sorted most recently touched first, so the first match
    /// is the newest.
    pub fn round_done(
        &mut self,
        sessions: Option<&[AgentSession]>,
        now: EpochMs,
        claims: &mut Claims,
    ) -> Option<String> {
        let fresh = sessions.and_then(|sessions| {
            sessions.iter().find(|session| {
                !self.known.contains(&session.id)
                    && !claims.contains(&session.id)
                    && now.saturating_sub(session.modified_ms) < FRESH_MS
            })
        });
        let found = fresh.map(|session| session.id.clone());
        if found.is_none() {
            self.empty_rounds += 1;
            if self.empty_rounds >= EMPTY_ROUNDS_BEFORE_FOLLOW {
                self.interval_ms = FOLLOW_MS;
            }
        } else {
            self.empty_rounds = 0;
        }
        if let Some(id) = &found {
            claims.claim(id);
            // Remembered as ours, so the next round hunts the one after it.
            self.known.insert(id.clone());
            self.interval_ms = FOLLOW_MS;
        }
        self.next_at = Some(now + self.interval_ms);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, modified_ms: u64) -> AgentSession {
        AgentSession {
            id: id.into(),
            label: id.into(),
            modified_ms,
        }
    }

    fn known(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    fn watch(known_ids: &[&str]) -> SessionWatch {
        SessionWatch::for_agent(AgentId::Claude, "/work".into(), known(known_ids), 0)
            .expect("Claude Code resumes its sessions")
    }

    #[test]
    fn looks_every_second_and_a_half_while_hunting_and_every_five_once_it_found_one() {
        let mut claims = Claims::default();
        let mut watch = watch(&[]);
        assert!(!watch.due(HUNT_MS - 1));
        assert!(watch.due(HUNT_MS));

        watch.note_output(HUNT_MS);
        assert!(watch.begin_round(HUNT_MS));
        // In flight: not due again until the answer is in.
        assert!(!watch.due(HUNT_MS * 10));
        let found = watch.round_done(Some(&[session("s1", HUNT_MS - 100)]), HUNT_MS, &mut claims);
        assert_eq!(found.as_deref(), Some("s1"));
        assert!(!watch.due(HUNT_MS + FOLLOW_MS - 1));
        assert!(watch.due(HUNT_MS + FOLLOW_MS));
    }

    #[test]
    fn keeps_hunting_briskly_while_it_finds_nothing() {
        let mut claims = Claims::default();
        let mut watch = watch(&[]);
        watch.note_output(HUNT_MS);
        assert!(watch.begin_round(HUNT_MS));
        assert_eq!(watch.round_done(Some(&[]), HUNT_MS, &mut claims), None);
        assert!(watch.due(HUNT_MS * 2));
        // A listing that failed reads as nothing found.
        watch.note_output(HUNT_MS * 2);
        assert!(watch.begin_round(HUNT_MS * 2));
        assert_eq!(watch.round_done(None, HUNT_MS * 2, &mut claims), None);
        assert!(watch.due(HUNT_MS * 3));
    }

    #[test]
    fn skips_a_round_when_the_pane_printed_nothing_since_the_last_look() {
        let mut watch = watch(&[]);
        assert!(!watch.begin_round(HUNT_MS));
        assert!(!watch.due(HUNT_MS * 2 - 1));
        assert!(watch.due(HUNT_MS * 2));
        // Output counts for one round only.
        watch.note_output(HUNT_MS * 2);
        assert!(watch.begin_round(HUNT_MS * 2));
        let mut claims = Claims::default();
        watch.round_done(Some(&[]), HUNT_MS * 2, &mut claims);
        assert!(!watch.begin_round(HUNT_MS * 3));
    }

    #[test]
    fn adopts_the_newest_session_it_did_not_know_and_nobody_claimed() {
        let mut claims = Claims::default();
        claims.claim("taken");
        let mut watch = watch(&["old"]);
        watch.note_output(HUNT_MS);
        watch.begin_round(HUNT_MS);
        let now = 100_000;
        let sessions = [
            session("taken", now - 100),
            session("new", now - 200),
            session("old", now - 300),
            session("newer-but-after", now - 400),
        ];
        let found = watch.round_done(Some(&sessions), now, &mut claims);
        assert_eq!(found.as_deref(), Some("new"));
        // It is the pane's now, for it and for every other pane.
        assert!(claims.contains("new"));
    }

    #[test]
    fn does_not_adopt_a_transcript_that_is_not_warm() {
        let mut claims = Claims::default();
        let mut watch = watch(&[]);
        watch.note_output(HUNT_MS);
        watch.begin_round(HUNT_MS);
        let now = 100_000;
        let cold = [session("cold", now - FRESH_MS)];
        assert_eq!(watch.round_done(Some(&cold), now, &mut claims), None);
        watch.note_output(now + HUNT_MS);
        watch.begin_round(now + HUNT_MS);
        let warm = [session("warm", now - FRESH_MS + 1 + HUNT_MS)];
        assert_eq!(
            watch
                .round_done(Some(&warm), now + HUNT_MS, &mut claims)
                .as_deref(),
            Some("warm")
        );
    }

    #[test]
    fn follows_a_new_session_after_the_first_one() {
        // /new or /clear inside the agent opens another transcript.
        let mut claims = Claims::default();
        let mut watch = watch(&[]);
        let mut now = HUNT_MS;
        watch.note_output(now);
        watch.begin_round(now);
        assert_eq!(
            watch
                .round_done(Some(&[session("one", now)]), now, &mut claims)
                .as_deref(),
            Some("one")
        );

        now += FOLLOW_MS;
        watch.note_output(now);
        assert!(watch.begin_round(now));
        // The first is known now: only the second stands out.
        let both = [session("two", now - 10), session("one", now - 20)];
        assert_eq!(
            watch.round_done(Some(&both), now, &mut claims).as_deref(),
            Some("two")
        );
    }

    #[test]
    fn a_resumed_pane_that_finds_nothing_new_settles_to_the_patient_cadence() {
        let mut claims = Claims::default();
        let mut watch = watch(&["resumed"]);
        let mut now = HUNT_MS;
        for round in 1..=EMPTY_ROUNDS_BEFORE_FOLLOW {
            watch.note_output(now);
            assert!(watch.begin_round(now));
            let only_known = [session("resumed", now - 10)];
            assert_eq!(watch.round_done(Some(&only_known), now, &mut claims), None);
            if round < EMPTY_ROUNDS_BEFORE_FOLLOW {
                assert!(watch.due(now + HUNT_MS));
                now += HUNT_MS;
            }
        }
        assert!(!watch.due(now + FOLLOW_MS - 1));
        assert!(watch.due(now + FOLLOW_MS));
    }

    #[test]
    fn a_skipped_round_claims_nothing_and_is_rescheduled() {
        let mut watch = watch(&[]);
        watch.note_output(HUNT_MS);
        watch.skip_round(HUNT_MS);
        // The output was spent on the skipped round; the next one is on time.
        assert!(!watch.due(HUNT_MS + HUNT_MS - 1));
        assert!(watch.due(HUNT_MS * 2));
        assert!(!watch.begin_round(HUNT_MS * 2));
    }

    // -----------------------------------------------------------------------
    // By kind, on conversations these tests write
    // -----------------------------------------------------------------------

    use std::path::{Path, PathBuf};
    use std::time::{Duration, UNIX_EPOCH};

    use crate::testdir::scratch;

    /// The folder the conversations below ran in.
    const CWD: &str = "/home/dev/shop";
    /// When they were written, in ms; the rounds run just after.
    const WRITTEN: u64 = 1_795_000_000_000;
    /// Codex threads.
    const KNOWN: &str = "019a9f20-1111-7000-8000-000000000001";
    const NEW: &str = "019a9f20-2222-7000-8000-000000000002";
    const SUB: &str = "019a9f20-3333-7000-8000-000000000003";
    const ELSEWHERE: &str = "019a9f20-4444-7000-8000-000000000004";
    const NEXT: &str = "019a9f20-5555-7000-8000-000000000005";
    /// A Claude Code session.
    const CLAUDE_SESSION: &str = "4f1c2b9e-0d3a-4e5f-9a8b-7c6d5e4f3a2b";

    fn written_at(path: &Path, ms: u64) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open")
            .set_modified(UNIX_EPOCH + Duration::from_millis(ms))
            .expect("mtime");
    }

    /// A Codex rollout where Codex files it
    /// (`sessions/YYYY/MM/DD/rollout-<time>-<thread>.jsonl`): its
    /// `session_meta` record and the prompt. Written here from the record
    /// types of openai/codex at rust-v0.170.0 (`SessionMeta` and
    /// `SessionSource` in `codex-rs/protocol/src/protocol.rs`, the name from
    /// `codex-rs/rollout/src/rollout_file_name.rs`), not recorded from a run.
    /// `spawned_by` makes it a sub-agent of that thread.
    fn rollout(sessions: &Path, thread: &str, cwd: &str, spawned_by: Option<&str>, ms: u64) {
        let day = sessions.join("2026").join("11").join("19");
        std::fs::create_dir_all(&day).expect("day folder");
        let source = match spawned_by {
            Some(parent) => format!(
                r#"{{"subagent":{{"thread_spawn":{{"parent_thread_id":"{parent}","depth":1}}}}}}"#
            ),
            None => r#""cli""#.to_string(),
        };
        let root = spawned_by.unwrap_or(thread);
        let meta = format!(
            r#"{{"timestamp":"2026-11-19T10:00:00.000Z","type":"session_meta","payload":{{"id":"{thread}","session_id":"{root}","timestamp":"2026-11-19T10:00:00.000Z","cwd":"{cwd}","originator":"codex_cli_rs","cli_version":"0.170.0","source":{source},"model_provider":"openai","history_mode":"legacy"}}}}"#
        );
        let prompt = r#"{"timestamp":"2026-11-19T10:00:01.000Z","type":"event_msg","payload":{"type":"user_message","message":"Add a cart total to the checkout page"}}"#;
        let path = day.join(format!("rollout-2026-11-19T10-00-00-{thread}.jsonl"));
        std::fs::write(&path, format!("{meta}\n{prompt}\n")).expect("rollout");
        written_at(&path, ms);
    }

    /// A Claude Code transcript where Claude Code files it
    /// (`projects/<folder>/<session>.jsonl`, the folder spelt with every
    /// character but letters and digits a dash), holding the prompt.
    fn transcript(projects: &Path, session: &str, cwd: &str, ms: u64) {
        let folder: String = cwd
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let dir = projects.join(folder);
        std::fs::create_dir_all(&dir).expect("project folder");
        let path = dir.join(format!("{session}.jsonl"));
        std::fs::write(
            &path,
            "{\"type\":\"user\",\"message\":{\"content\":\"Add a cart total\"}}\n",
        )
        .expect("transcript");
        written_at(&path, ms);
    }

    /// What `hunt_listing` lists for the watch's kind, from `roots` instead
    /// of this machine's folders.
    fn hunt_in(watch: &SessionWatch, roots: &[PathBuf]) -> Vec<AgentSession> {
        listed(hangar_core::sessions::list_session_ids_in(
            watch.kind(),
            roots,
            &watch.cwd,
        ))
    }

    #[test]
    fn a_codex_pane_whose_client_is_silent_finds_its_thread_among_the_rollouts() {
        let sessions = scratch("watch-codex");
        rollout(&sessions, KNOWN, CWD, None, WRITTEN - 100);
        rollout(&sessions, NEW, CWD, None, WRITTEN - 300);
        // Fresher, and neither is the pane's: a sub-agent its thread
        // spawned, a thread of another folder.
        rollout(&sessions, SUB, CWD, Some(NEW), WRITTEN - 50);
        rollout(&sessions, ELSEWHERE, "/home/dev/blog", None, WRITTEN - 10);
        let roots = [sessions.clone()];
        let mut claims = Claims::default();
        let mut watch =
            SessionWatch::for_agent(AgentId::Codex, CWD.into(), known(&[KNOWN]), WRITTEN)
                .expect("Codex resumes its threads");
        // On the floor, or before its client attached: nothing names the
        // thread, so the rollouts are listed.
        assert!(!watch.reported(false));

        let mut at = WRITTEN + HUNT_MS;
        assert!(watch.due(at));
        watch.note_output(at);
        assert!(watch.begin_round(at));
        let listing = hunt_in(&watch, &roots);
        assert_eq!(
            watch.round_done(Some(&listing), at, &mut claims).as_deref(),
            Some(NEW)
        );
        assert!(claims.contains(NEW));

        // /new in Codex starts another thread: the pane follows it.
        rollout(&sessions, NEXT, CWD, None, at + 100);
        at += FOLLOW_MS;
        assert!(watch.due(at));
        watch.note_output(at);
        assert!(watch.begin_round(at));
        let listing = hunt_in(&watch, &roots);
        assert_eq!(
            watch.round_done(Some(&listing), at, &mut claims).as_deref(),
            Some(NEXT)
        );
    }

    #[test]
    fn each_kind_hunts_its_own_history_and_nothing_else() {
        let home = scratch("watch-kinds");
        let projects = home.join("projects");
        let sessions = home.join("sessions");
        transcript(&projects, CLAUDE_SESSION, CWD, WRITTEN - 200);
        rollout(&sessions, NEW, CWD, None, WRITTEN - 100);
        // Both folders handed to both kinds: each still finds its own only.
        let roots = [projects, sessions];
        let mut claims = Claims::default();
        let at = WRITTEN + HUNT_MS;
        for (agent, theirs) in [(AgentId::Claude, CLAUDE_SESSION), (AgentId::Codex, NEW)] {
            let mut watch = SessionWatch::for_agent(agent, CWD.into(), HashSet::new(), WRITTEN)
                .expect("a resumable agent");
            watch.note_output(at);
            assert!(watch.begin_round(at));
            let listing = hunt_in(&watch, &roots);
            let ids: Vec<&str> = listing.iter().map(|s| s.id.as_str()).collect();
            assert_eq!(ids, [theirs], "{agent:?}");
            assert_eq!(
                watch.round_done(Some(&listing), at, &mut claims).as_deref(),
                Some(theirs),
                "{agent:?}"
            );
        }
    }

    #[test]
    fn no_folder_is_listed_while_the_agent_names_its_session() {
        for kind in AgentKind::ALL {
            let agent = AgentId::from(kind);
            assert!(resumes(agent), "{kind:?}");
            let watch = SessionWatch::for_agent(agent, CWD.into(), HashSet::new(), 0)
                .expect("a resumable agent");
            assert_eq!(watch.kind(), kind);
            // Claude Code's mod and Codex's attached client say the id...
            assert!(watch.reported(true), "{kind:?}");
            // ...and with them silent, it is hunted.
            assert!(!watch.reported(false), "{kind:?}");
        }
        // A shell has no session to follow.
        assert!(!resumes(AgentId::Shell));
        assert!(SessionWatch::for_agent(AgentId::Shell, CWD.into(), HashSet::new(), 0).is_none());
    }

    #[test]
    fn a_pane_resumes_its_session_only_where_its_kind_keeps_it() {
        let home = scratch("resume-kinds");
        let projects = home.join("projects");
        let sessions = home.join("sessions");
        transcript(&projects, CLAUDE_SESSION, CWD, WRITTEN);
        rollout(&sessions, NEW, CWD, None, WRITTEN);
        let roots = [projects, sessions];
        let on_disk =
            |kind: AgentKind, id: &str| hangar_core::sessions::session_exists_in(kind, &roots, id);
        assert_eq!(
            resume_if_kept(AgentId::Codex, Some(NEW), on_disk).as_deref(),
            Some(NEW)
        );
        assert_eq!(
            resume_if_kept(AgentId::Claude, Some(CLAUDE_SESSION), on_disk).as_deref(),
            Some(CLAUDE_SESSION)
        );
        // A Codex thread is no Claude Code transcript, nor the other way
        // round: each is gone for the other kind.
        assert_eq!(resume_if_kept(AgentId::Claude, Some(NEW), on_disk), None);
        assert_eq!(
            resume_if_kept(AgentId::Codex, Some(CLAUDE_SESSION), on_disk),
            None
        );
        // Nothing to resume: no id, an empty one, a shell.
        assert_eq!(resume_if_kept(AgentId::Codex, None, on_disk), None);
        assert_eq!(resume_if_kept(AgentId::Codex, Some(""), on_disk), None);
        assert_eq!(resume_if_kept(AgentId::Shell, Some(NEW), on_disk), None);
        // Folders that cannot be read are no proof the thread is gone.
        let missing = [home.join("missing")];
        let unreadable = |kind: AgentKind, id: &str| {
            hangar_core::sessions::session_exists_in(kind, &missing, id)
        };
        assert_eq!(
            resume_if_kept(AgentId::Codex, Some(NEW), unreadable).as_deref(),
            Some(NEW)
        );
    }
}
