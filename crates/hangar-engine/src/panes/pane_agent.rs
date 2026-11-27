//! A pane's agent, as its terminal lives it: the state half of
//! TerminalPane.tsx (`activity`, `bridged`, `attention`, the trust question,
//! the hand-back and the reports of the pane's side channel).
//!
//! In the web app these were `useState`s of the terminal component, which
//! published what the rest of the window reads (`setPaneActivity`,
//! `updatePaneInsight`). Here the machine of every mounted pane lives in
//! `Live`, next to what it publishes, and the terminal's package
//! (`panes`) only reports what the terminal saw (`Msg`): a chunk of output, a
//! bell, a key, the shell's end. Nothing here reads a clock or the model: a
//! decision is handed the moment (`now`) and the ambient facts it depends on
//! (`Ambient`: whether the person is looking at the pane, the settings that
//! decide whether to notify), and answers what the window must do
//! (`Effect`).
//!
//! Guesses and reports. A pane's side channel reports what its agent does:
//! the teskad-bridge mod from inside a Claude Code, Hangar's own client of a
//! Codex pane's server (`agent/codex_attach.rs`), both in the same reports
//! (`BridgeEvent`), read here alike (`on_bridge`). The first report proves
//! the side channel is there (`bridged`: it spoke), and from then on every
//! guess stands down. Without one — the mod off or not loaded, Codex on its
//! floor (D6), a shell — the terminal is all there is to go by: the shape of
//! the output (`activity`: bursts and silences), the agent's own title
//! ([`title_says`]), the trust question read off the screen, the bell and OSC
//! 9 and 777. What each agent draws is read whatever the pane's kind: a
//! shell can run either, and no agent draws another's screens. What the
//! pane's agent can do, side channel or not, is its live capabilities
//! ([`Live::live_caps`]).

use hangar_core::agent::Capabilities;
use hangar_i18n::{Key, Translator, args};

use super::Live;
use super::activity::{ActivityWatcher, MIN_BUSY_MS, Signal};
use super::agent_state::activity_at_hello;
use super::agents::is_trust_prompt;
use super::bridge_events::{
    EVENT_LINE_MAX, file_base_name, first_line, newly_queued, short_session_id,
};
use crate::live::{
    AgentActivity, BridgeEvent, ContextGauge, EpochMs, ExitWhat, PaneActivity, PaneEventDetail,
    PaneStatus, QueuedPrompt, SideChannel, ToolPhase, TurnOutcome, TurnPhase, live_caps,
};
use crate::state::types::{AgentId, PaneId, WsId};

/// How long after the last output the screen is read for the trust question.
pub const TRUST_LOOK_MS: u64 = 600;

/// A pane as the machine sees it: who it is, from the persisted state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    pub id: PaneId,
    pub workspace_id: WsId,
    /// Unique across workspaces; what other panes are told it is.
    pub name: String,
    pub agent: AgentId,
    /// The session the pane resumes on its next launch.
    pub session_id: Option<String>,
}

/// What a decision depends on besides the machine: read from the window by
/// the caller, handed in so that the machine reads nothing.
#[derive(Debug, Clone, Copy)]
pub struct Ambient {
    pub now: EpochMs,
    /// The window of the pane is in front, the pane is the focused one of its
    /// workspace and the workspace is shown: it is being looked at.
    pub looking: bool,
    /// The window of the pane is in front.
    pub window_focused: bool,
    pub notify_on_idle: bool,
    pub notify_only_when_away: bool,
    pub t: Translator,
}

/// What the window has to do for the pane, beyond the state the machine kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// A desktop notification naming the pane (`notify`).
    Notify { title: String, body: String },
    /// A session of the pane's agent, new to it: store it with the pane
    /// (`onSessionCaptured`, which is `updatePane(sessionId)`).
    CaptureSession(String),
}

/// The machine of one mounted pane.
#[derive(Debug, Default)]
pub struct PaneAgent {
    /// The workspace and name the pane had when it mounted: a restart gives
    /// it a new id (and no longer finds the old one in the state), and its
    /// queue goes to the pane that has the same name.
    pub workspace_id: WsId,
    pub name: String,
    /// Where the agent stands, even when nobody reads it (a plain shell
    /// reports nothing, but a bell in it still hands back).
    pub activity: AgentActivity,
    /// The pane's side channel spoke (the teskad-bridge mod in its Claude
    /// Code, Hangar's client of its Codex server): its turns, dialogs and
    /// usage are reported rather than guessed from the terminal.
    pub bridged: bool,
    /// How its agent was started, as far as a side channel goes: said by
    /// its launch.
    pub side_channel: SideChannel,
    /// The pane handed control back while you were looking somewhere else.
    pub attention: bool,
    pub watcher: ActivityWatcher,
    /// Its agent is asking whether to trust the folder.
    trust_asked: bool,
    /// What the agent's own title last said, once it said something
    /// ([`title_says`]): from then on the title, not the shape of the
    /// output, tells its turns. `None` until then, and once the title is let
    /// go of.
    title: Option<TitleSays>,
    /// Since when the title has said a turn runs (at work or asking), until
    /// it says the turn is over.
    title_turn_since: Option<EpochMs>,
    /// When to read the screen for that question, 600 ms after the last output.
    pub trust_look_at: Option<EpochMs>,
    /// Whether the pane was attended (focused and shown) the last time we
    /// looked, so that the attention is cleared when it becomes so and not
    /// held against a window that is not in front.
    attended: bool,
    bridge_model: Option<String>,
    /// The conversation the pane's history last said it was in.
    reported_session: Option<String>,
    /// The context as the mod reports it, which beats the transcript's.
    bridge_context: Option<ContextGauge>,
    /// The context as the transcript says it.
    transcript_context: Option<ContextGauge>,
}

/// Text OSC 9 carries when it is a notification, or `None` when it is one of
/// the numeric sub-commands the terminal leaves to whoever else wants them.
///
/// Only iTerm2's `OSC 9 ; <message>` is a notification. On Windows the same
/// code is mostly used for ConEmu sub-commands that have nothing to say to
/// you: `OSC 9 ; 4 ; <state> ; <pct>` is the taskbar progress bar (PowerShell
/// 7, winget, pip) and `OSC 9 ; 9 ; <cwd>` reports the working directory
/// (ConEmu, Windows Terminal). They are told apart by their leading numeric
/// segment (a message written for a human does not start with one) and
/// left alone rather than badging the pane and popping up a notification
/// reading `4;3;0`. The parser's handler answers `false` for `None`.
#[cfg(any(test, feature = "testing"))]
pub fn osc9_notification(data: &str) -> Option<String> {
    let digits = data.chars().take_while(char::is_ascii_digit).count();
    let numeric = digits > 0
        && match data[digits..].chars().next() {
            None | Some(';') => true,
            Some(_) => false,
        };
    if numeric {
        return None;
    }
    Some(data.trim().to_string())
}

/// Text of an OSC 777 `notify;title;body`, or `None` for any other kind.
/// The body is the message, the title when there is no body; empty if both
/// are.
#[cfg(any(test, feature = "testing"))]
pub fn osc777_notification(data: &str) -> Option<String> {
    let mut parts = data.split(';');
    if parts.next() != Some("notify") {
        return None;
    }
    let title = parts.next().map_or("", str::trim);
    let body = parts.next().map_or("", str::trim);
    Some(if !body.is_empty() { body } else { title }.to_string())
}

impl Live {
    // ----- lifecycle

    /// The pane's terminal exists (`notePaneMounted`).
    pub fn mount_pane(&mut self, pane: &str, workspace_id: &str, name: &str, now: EpochMs) {
        self.hibernation.note_pane_mounted(pane, now);
        self.machines.insert(
            pane.to_string(),
            PaneAgent {
                workspace_id: workspace_id.to_string(),
                name: name.to_string(),
                ..PaneAgent::default()
            },
        );
        self.status.insert(pane.to_string(), PaneStatus::Starting);
    }

    /// Closed, restarted under a new id, or taken over by another window: a
    /// pane moving to another window keeps its state, which that window
    /// reports from now on (`handing_over`).
    pub fn unmount_pane(&mut self, pane: &str, handing_over: bool) {
        if let Some(mut machine) = self.machines.remove(pane) {
            machine.watcher.dispose();
        }
        self.hibernation.forget_pane(pane);
        self.status.remove(pane);
        self.watches.remove(pane);
        if !handing_over {
            self.agents.clear(pane);
            self.insights.clear(pane);
        }
    }

    /// A pane its node runs itself is known here from now on, whether or
    /// not a terminal follows it: the reports of its agent, which its node
    /// passes on, need a machine to land in, so that a workspace never
    /// opened still shows what its agents are doing. A machine already
    /// there is kept as it is. No status is set: the pane has no shell of
    /// this machine's to wait for.
    pub fn mirror_pane(&mut self, pane: &str, workspace_id: &str, name: &str) {
        self.machines
            .entry(pane.to_string())
            .or_insert_with(|| PaneAgent {
                workspace_id: workspace_id.to_string(),
                name: name.to_string(),
                ..PaneAgent::default()
            });
    }

    /// Where its node says the agent of a mirrored pane stands, taken as
    /// the pane's own: what was missed while its reports were not followed
    /// (before the node connected, or events lost). `bridged`: the node
    /// hears its mod, so the guesses made from its output stand down here
    /// too. An agent at rest on both sides keeps what this machine holds:
    /// the node counts a turn nobody on it looked at as handed back, which
    /// the person may have read here since.
    pub fn mirror_activity(
        &mut self,
        info: &PaneInfo,
        activity: AgentActivity,
        bridged: bool,
        now: EpochMs,
    ) {
        let Some(machine) = self.machines.get_mut(&info.id) else {
            return;
        };
        let resting = |activity| matches!(activity, AgentActivity::Idle | AgentActivity::Yours);
        let known = self.agents.get(&info.id).is_some();
        let activity = if known && resting(machine.activity) && resting(activity) {
            machine.activity
        } else {
            activity
        };
        if known && machine.activity == activity && machine.bridged == bridged {
            return;
        }
        machine.activity = activity;
        machine.bridged = bridged;
        self.publish(info, now);
    }

    // ----- the side channel, and what the agent can do now

    /// How the pane's agent was started, as far as its side channel goes,
    /// as its launch says (`host/launch.rs`): attached to one, or on the
    /// floor and why.
    pub fn set_side_channel(&mut self, pane: &str, channel: SideChannel) {
        if let Some(machine) = self.machines.get_mut(pane) {
            machine.side_channel = channel;
        }
    }

    /// How the pane's agent was started, as far as its side channel goes;
    /// `None` for a pane not mounted.
    pub fn side_channel(&self, pane: &str) -> Option<&SideChannel> {
        self.machines.get(pane).map(|machine| &machine.side_channel)
    }

    /// What the pane's agent can do now: its kind's levels while its side
    /// channel speaks, the floor until then (`live::live_caps`). What reads
    /// a pane's agent reads these, never its kind (docs/native/agents.md
    /// D3). `None` for a shell, which has no agent, and for a pane not
    /// mounted.
    pub fn live_caps(&self, info: &PaneInfo) -> Option<Capabilities> {
        let machine = self.machines.get(&info.id)?;
        let kind = info.agent.kind()?;
        Some(live_caps(&kind.info().caps, machine.bridged))
    }

    // ----- what is published

    /// Publishes the pane's activity: for the sidebar, which counts each
    /// workspace's agents by state. A plain shell is left out unless a
    /// Claude Code in it reports: its builds and test runs would otherwise
    /// count as agents at work. And for the hibernation sweep, which needs
    /// to know since when it has rested.
    fn publish(&mut self, info: &PaneInfo, now: EpochMs) {
        let Some(machine) = self.machines.get(&info.id) else {
            return;
        };
        let (activity, bridged) = (machine.activity, machine.bridged);
        if info.agent != AgentId::Shell || bridged {
            self.agents.set(
                &info.id,
                PaneActivity {
                    workspace_id: info.workspace_id.clone(),
                    activity,
                    bridged,
                },
            );
        } else {
            self.agents.clear(&info.id);
        }
        self.hibernation.note_pane_activity(&info.id, activity, now);
    }

    fn set_activity(&mut self, info: &PaneInfo, activity: AgentActivity, now: EpochMs) {
        let Some(machine) = self.machines.get_mut(&info.id) else {
            return;
        };
        machine.activity = activity;
        self.publish(info, now);
    }

    fn set_bridged(&mut self, info: &PaneInfo, bridged: bool, now: EpochMs) {
        let Some(machine) = self.machines.get_mut(&info.id) else {
            return;
        };
        machine.bridged = bridged;
        self.publish(info, now);
    }

    /// The gauge the header shows, whichever source it came from, and the
    /// model: without the mod, the transcript is the one place that names it.
    fn refresh_gauge(&mut self, pane: &str) {
        let Some(machine) = self.machines.get(pane) else {
            return;
        };
        let from_mod = if machine.bridged {
            machine.bridge_context.clone()
        } else {
            None
        };
        let gauge = from_mod.or_else(|| machine.transcript_context.clone());
        let transcript_model = if machine.bridged {
            None
        } else {
            machine
                .transcript_context
                .as_ref()
                .and_then(|context| context.model.clone())
        };
        self.insights.update(pane, |insight| {
            insight.context = gauge;
            if let Some(model) = transcript_model {
                insight.model = Some(model);
            }
        });
    }

    /// Whether the pane shows a context read from its transcript, which a
    /// clear would take away (a clear of nothing changes nothing).
    pub fn has_transcript_context(&self, pane: &str) -> bool {
        self.machines
            .get(pane)
            .is_some_and(|machine| machine.transcript_context.is_some())
    }

    /// What the transcript says of the pane's context (`None`: nothing to
    /// show, no gauge beats a made-up zero).
    pub fn set_transcript_context(&mut self, pane: &str, gauge: Option<ContextGauge>) {
        let Some(machine) = self.machines.get_mut(pane) else {
            return;
        };
        machine.transcript_context = gauge;
        self.refresh_gauge(pane);
    }

    // ----- hand-back and announcements

    /// Says the agent is waiting for the person or done, unless they are
    /// looking at it already (`announce`).
    ///
    /// Everything needed to decide whether a hand-back is worth interrupting
    /// the person for is local to the pane: whether it is the one they are
    /// watching, and whether the window is even in front.
    fn announce(&mut self, info: &PaneInfo, body: String, amb: &Ambient) -> Option<Effect> {
        // You are already looking at it: not news. All three have to hold:
        // being the focused pane of a workspace that is off screen means the
        // hand-back happened out of sight, which is the whole point of this.
        if amb.looking {
            return None;
        }
        self.machines.get_mut(&info.id)?.attention = true;

        if !amb.notify_on_idle {
            return None;
        }
        // With the window in front, the pane badge is enough of a signal.
        if amb.notify_only_when_away && amb.window_focused {
            return None;
        }
        Some(Effect::Notify {
            title: amb
                .t
                .t_with(
                    Key::NotifyTitle,
                    &args!("name" => info.name.as_str(), "agent" => info.agent.info().label),
                )
                .into_owned(),
            body,
        })
    }

    /// The agent finished its turn: yours, unless you are already on it.
    fn hand_back(&mut self, info: &PaneInfo, body: String, amb: &Ambient) -> Vec<Effect> {
        let activity = if amb.looking {
            AgentActivity::Idle
        } else {
            AgentActivity::Yours
        };
        self.set_activity(info, activity, amb.now);
        self.announce(info, body, amb).into_iter().collect()
    }

    /// The agent asks something of the person on its own screen — an
    /// approval, a question — which no side channel reported: waiting, and
    /// said once, however many signs of it come (its title, its
    /// notification).
    fn ask(&mut self, info: &PaneInfo, body: String, amb: &Ambient) -> Vec<Effect> {
        let waiting = self
            .machines
            .get(&info.id)
            .is_some_and(|machine| machine.activity == AgentActivity::Waiting);
        if waiting {
            return Vec::new();
        }
        self.set_activity(info, AgentActivity::Waiting, amb.now);
        self.record(
            &info.id,
            PaneEventDetail::Permission { tool: None },
            amb.now,
        );
        self.announce(info, body, amb).into_iter().collect()
    }

    // ----- the guesses of a pane without a side channel

    /// A chunk of output was printed (written to the terminal) at `now`.
    #[cfg(any(test, feature = "testing"))]
    pub fn note_output(&mut self, pane: &str, now: EpochMs) {
        self.note_printed(pane, now, now);
    }

    /// The pane printed, from `first` to `last`, since the window last
    /// looked: more steps of a burst, and the trust question to look for once
    /// it settles. The moments are the reader thread's, not the look's: a
    /// pane off screen is looked at four times a second, and its burst is
    /// measured as one looked at every frame would be.
    pub fn note_printed(&mut self, pane: &str, first: EpochMs, last: EpochMs) {
        self.hibernation.note_pane_output(pane, last);
        if let Some(watch) = self.watches.get_mut(pane) {
            watch.note_output(last);
        }
        let Some(machine) = self.machines.get_mut(pane) else {
            return;
        };
        machine.watcher.push(first);
        machine.watcher.push(last);
        if !machine.bridged {
            machine.trust_look_at = Some(last + TRUST_LOOK_MS);
        }
    }

    /// The signals a watcher gave: work began, or it fell silent.
    fn on_signals(&mut self, info: &PaneInfo, signals: Vec<Signal>, amb: &Ambient) -> Vec<Effect> {
        let mut effects = Vec::new();
        for signal in signals {
            let Some(machine) = self.machines.get(&info.id) else {
                break;
            };
            // Everything that reads the output is a guess, and a pane whose
            // side channel reports its turns has no use for one: it would
            // only notify twice. Nor has one whose agent's title tells them.
            if machine.bridged || machine.title.is_some() {
                continue;
            }
            match signal {
                Signal::Busy => self.set_activity(info, AgentActivity::Working, amb.now),
                Signal::Settle if !machine.trust_asked => {
                    let body = amb.t.t(Key::NotifySettled).into_owned();
                    effects.extend(self.hand_back(info, body, amb));
                }
                Signal::Settle => {}
            }
        }
        effects
    }

    /// The watcher's checks that came due (the host calls it once
    /// `next_check` is due, every `activity::TICK_MS` during a burst).
    pub fn poll_pane(&mut self, info: &PaneInfo, idle_ms: u64, amb: &Ambient) -> Vec<Effect> {
        let Some(machine) = self.machines.get_mut(&info.id) else {
            return Vec::new();
        };
        let signals = machine.watcher.poll(amb.now, idle_ms);
        self.on_signals(info, signals, amb)
    }

    /// A bell is the agent asking for you outright, so it settles the pane
    /// without waiting out the silence. Not where the agent's title tells
    /// its turns: Codex rings for an approval as for a turn's end, which its
    /// title tells apart.
    pub fn bell(&mut self, info: &PaneInfo, amb: &Ambient) -> Vec<Effect> {
        let Some(machine) = self.machines.get_mut(&info.id) else {
            return Vec::new();
        };
        if machine.bridged || machine.title.is_some() {
            return Vec::new();
        }
        let signal = machine.watcher.ring(amb.now);
        self.on_signals(info, signal.into_iter().collect(), amb)
    }

    /// OSC 9 or OSC 777: the terminal escape codes for "raise a desktop
    /// notification". The agent already wrote the message, so it is passed
    /// through instead of being guessed at. `message` is what
    /// `osc9_notification` or `osc777_notification` read.
    ///
    /// One that asks something of the person ([`asks_the_person`]: Codex's
    /// approvals and questions) leaves the agent waiting rather than done.
    /// Where the agent's title tells its turns, a turn it already handed
    /// back is not handed back twice.
    pub fn terminal_notification(
        &mut self,
        info: &PaneInfo,
        message: &str,
        amb: &Ambient,
    ) -> Vec<Effect> {
        let Some(machine) = self.machines.get(&info.id) else {
            return Vec::new();
        };
        if machine.bridged {
            return Vec::new();
        }
        if asks_the_person(message) {
            return self.ask(info, message.to_string(), amb);
        }
        let in_turn = matches!(
            machine.activity,
            AgentActivity::Working | AgentActivity::Waiting
        );
        if machine.title.is_some() && !in_turn {
            return Vec::new();
        }
        let body = if message.is_empty() {
            amb.t.t(Key::NotifySettled).into_owned()
        } else {
            message.to_string()
        };
        self.hand_back(info, body, amb)
    }

    /// The title the pane's program set (OSC 0 or 2; empty: it let go of
    /// it). Where no side channel speaks, an agent's own title tells its
    /// turns better than the shape of its output ([`title_says`]): at work
    /// the moment it starts, waiting on the person (which the output cannot
    /// tell from done), done the moment it rests. Once it said something,
    /// the guesses from the output, the bell included, stand down until the
    /// title is let go of or the agent ends.
    ///
    /// A turn shorter than a burst the output watcher would count as work
    /// (`activity::MIN_BUSY_MS`) ends idle, without a hand-back: what Codex
    /// shows while its MCP servers start, as much as a reply you watched
    /// come.
    pub fn terminal_title(&mut self, info: &PaneInfo, title: &str, amb: &Ambient) -> Vec<Effect> {
        let Some(machine) = self.machines.get_mut(&info.id) else {
            return Vec::new();
        };
        if machine.bridged {
            return Vec::new();
        }
        let says = title_says(title);
        let before = machine.title;
        if before.is_none() && says.is_none() {
            // Someone else's title: a shell's, an agent's Hangar reads
            // otherwise.
            return Vec::new();
        }
        // Between turns Codex's title carries no mark at all.
        let now_says = says.unwrap_or(TitleSays::Resting);
        machine.title = (!title.trim().is_empty()).then_some(now_says);
        if before == Some(now_says) {
            // The spinner's next frame.
            return Vec::new();
        }
        let now = amb.now;
        match now_says {
            TitleSays::Busy => {
                machine.title_turn_since.get_or_insert(now);
                if machine.activity != AgentActivity::Working {
                    self.set_activity(info, AgentActivity::Working, now);
                }
                Vec::new()
            }
            TitleSays::Asking => {
                machine.title_turn_since.get_or_insert(now);
                let body = amb
                    .t
                    .t_with(Key::NotifyPermission, &args!("tool" => ACTION_REQUIRED))
                    .into_owned();
                self.ask(info, body, amb)
            }
            TitleSays::Resting => {
                let Some(since) = machine.title_turn_since.take() else {
                    return Vec::new();
                };
                // Already handed back (by its notification), or taken back.
                if !matches!(
                    machine.activity,
                    AgentActivity::Working | AgentActivity::Waiting
                ) {
                    return Vec::new();
                }
                if now.saturating_sub(since) < MIN_BUSY_MS {
                    self.set_activity(info, AgentActivity::Idle, now);
                    return Vec::new();
                }
                let body = amb.t.t(Key::NotifySettled).into_owned();
                self.hand_back(info, body, amb)
            }
        }
    }

    /// The agent's title tells nothing any more: it ended, or its shell did.
    fn forget_title(&mut self, pane: &str) {
        if let Some(machine) = self.machines.get_mut(pane) {
            machine.title = None;
            machine.title_turn_since = None;
        }
    }

    /// Whether the screen is due to be read for the agent's question about
    /// trusting a folder it has not seen (Claude Code's, Codex's: see
    /// [`trust_question`]). It comes before the session exists, so no report
    /// of a side channel can announce it, and it is drawn in one short
    /// burst the idle watcher never counts as work. The screen is read a
    /// moment after the last output instead: the pane is waiting for the
    /// person, and not done.
    pub fn trust_look_due(&self, pane: &str, now: EpochMs) -> bool {
        self.machines.get(pane).is_some_and(|machine| {
            !machine.bridged && machine.trust_look_at.is_some_and(|at| at <= now)
        })
    }

    /// When the next check of `poll_pane` or look of `trust_look_due` comes
    /// due, over every pane: the host sleeps until then (`None`: nothing to
    /// look at until a pane prints). A bridged pane's trust look is never due,
    /// so it waits for nothing.
    pub fn next_check(&self) -> Option<EpochMs> {
        self.machines
            .values()
            .flat_map(|machine| {
                let look = machine.trust_look_at.filter(|_| !machine.bridged);
                [machine.watcher.next_check(), look]
            })
            .flatten()
            .min()
    }

    /// The screen, read when `trust_look_due`.
    pub fn look_for_trust_question(
        &mut self,
        info: &PaneInfo,
        screen: &str,
        amb: &Ambient,
    ) -> Vec<Effect> {
        let Some(machine) = self.machines.get_mut(&info.id) else {
            return Vec::new();
        };
        machine.trust_look_at = None;
        if machine.bridged {
            return Vec::new();
        }
        let question = trust_question(screen);
        let asking = question.is_some();
        if let Some(question) = question.filter(|_| !machine.trust_asked) {
            machine.trust_asked = true;
            self.set_activity(info, AgentActivity::Waiting, amb.now);
            let body = match question {
                TrustQuestion::Trust => amb.t.t(Key::NotifyTrustFolder).into_owned(),
                TrustQuestion::Named(name) => amb
                    .t
                    .t_with(Key::NotifyPermission, &args!("tool" => name))
                    .into_owned(),
            };
            return self.announce(info, body, amb).into_iter().collect();
        }
        if !asking && machine.trust_asked {
            machine.trust_asked = false;
            if machine.activity == AgentActivity::Waiting {
                self.set_activity(info, AgentActivity::Working, amb.now);
            }
        }
        Vec::new()
    }

    /// A key pressed in the pane is you taking it back, or answering what it
    /// was waiting on. Keys rather than data: the terminal also answers the
    /// agent's own queries, and those are nobody's answer.
    pub fn key_pressed(&mut self, info: &PaneInfo, now: EpochMs) {
        let Some(machine) = self.machines.get(&info.id) else {
            return;
        };
        match machine.activity {
            AgentActivity::Yours => self.set_activity(info, AgentActivity::Idle, now),
            AgentActivity::Waiting => self.set_activity(info, AgentActivity::Working, now),
            _ => {}
        }
    }

    /// Looking at the pane clears its badge, and seen counts as taken back; a
    /// dialog still waiting is still waiting. `attended`: the pane is the
    /// focused one of its workspace and the workspace is shown. It acts when
    /// the pane becomes so, and (`window_back`) when its window comes back to
    /// the front with the pane already so.
    pub fn attend(&mut self, info: &PaneInfo, attended: bool, window_back: bool, now: EpochMs) {
        let Some(machine) = self.machines.get_mut(&info.id) else {
            return;
        };
        let became = attended && !machine.attended;
        machine.attended = attended;
        if !(attended && (became || window_back)) {
            return;
        }
        machine.attention = false;
        if machine.activity == AgentActivity::Yours {
            self.set_activity(info, AgentActivity::Idle, now);
        }
    }

    /// The pane's shell ended by itself (`pty:exit`). A Claude Code killed
    /// mid-turn never says goodbye: its last report would otherwise hold the
    /// pane, and the sidebar, at "working" until a restart.
    pub fn shell_exited(&mut self, info: &PaneInfo, now: EpochMs) {
        if !self.machines.contains_key(&info.id) {
            return;
        }
        self.status.insert(info.id.clone(), PaneStatus::Exited);
        self.forget_title(&info.id);
        self.set_bridged(info, false, now);
        self.set_activity(info, AgentActivity::Idle, now);
        self.insights.update(&info.id, |i| i.turn_started_at = None);
        self.record(
            &info.id,
            PaneEventDetail::Exit {
                what: ExitWhat::Shell,
            },
            now,
        );
        self.refresh_gauge(&info.id);
    }

    /// The agent's command was typed into the shell: the session starts now
    /// and says so in the history. Only for the agents that have no mod to
    /// say it (`resumed` is the session the command resumes).
    pub fn launched(&mut self, info: &PaneInfo, resumed: Option<&str>, now: EpochMs) {
        if !self.machines.contains_key(&info.id) {
            return;
        }
        self.insights
            .update(&info.id, |insight| insight.session_since = Some(now));
        self.record(
            &info.id,
            session_event(resumed.unwrap_or_default(), resumed.is_some()),
            now,
        );
    }

    // ----- the history

    /// A line in the pane's history, for the inspector. An agent that saves
    /// a file five times running gets one line for it, not five.
    fn record(&mut self, pane: &str, event: PaneEventDetail, now: EpochMs) {
        if let PaneEventDetail::Edit { file, .. } = &event
            && let Some(last) = self.insights.get(pane).events.first()
            && matches!(&last.detail, PaneEventDetail::Edit { file: last_file, .. } if last_file == file)
        {
            return;
        }
        self.insights.record_event(pane, event, now);
    }

    /// The queue as the mod holds it, and a line for each prompt that joins
    /// it.
    pub fn queue_changed(&mut self, pane: &str, pending: Vec<QueuedPrompt>, now: EpochMs) {
        if !self.machines.contains_key(pane) {
            return;
        }
        let added: Vec<QueuedPrompt> = newly_queued(&self.insights.get(pane).queue, &pending)
            .into_iter()
            .cloned()
            .collect();
        self.insights
            .update(pane, |insight| insight.queue = pending);
        for entry in added {
            self.insights.record_event(
                pane,
                PaneEventDetail::Queued {
                    text: first_line(&entry.text, EVENT_LINE_MAX),
                    from: entry.from.filter(|from| !from.is_empty()),
                },
                now,
            );
        }
    }

    // ----- what the pane's side channel reports

    /// A report of the pane's side channel: the mod from inside its Claude
    /// Code, or Hangar's client of its Codex server, which reports the same
    /// kinds (`hello` with the thread, its model and Codex's version; `turn`
    /// with the last answer as the headline; `permission` while it waits on
    /// the person; `end` when the TUI leaves). The first report of any kind
    /// is proof the side channel is there, and from then on the guesses
    /// above stand down.
    pub fn on_bridge(&mut self, info: &PaneInfo, event: BridgeEvent, amb: &Ambient) -> Vec<Effect> {
        if !self.machines.contains_key(&info.id) {
            return Vec::new();
        }
        let now = amb.now;
        if !matches!(event, BridgeEvent::End { .. }) {
            // Now rather than at the next render: a guess made in between
            // would land after the hello below and outlive it.
            self.set_bridged(info, true, now);
        }
        let mut effects = Vec::new();
        match event {
            BridgeEvent::Hello {
                session_id,
                model,
                version,
            } => {
                self.hibernation.note_agent_exited(&info.id, false);
                let current = self
                    .machines
                    .get(&info.id)
                    .map_or(AgentActivity::Idle, |m| m.activity);
                self.set_activity(info, activity_at_hello(current), now);
                // Read before the capture below makes it this session's id.
                let resumed = info.session_id.as_deref() == Some(session_id.as_str());
                if let Some(machine) = self.machines.get_mut(&info.id) {
                    machine.bridge_model = Some(model.clone());
                    machine.reported_session = Some(session_id.clone());
                }
                self.insights.update(&info.id, |insight| {
                    insight.model = Some(model).filter(|model| !model.is_empty());
                    insight.agent_version = Some(version).filter(|version| !version.is_empty());
                    insight.session_since = Some(now);
                    insight.turn_started_at = None;
                    // A session starts in its own mode, until it says.
                    insight.permission_mode = None;
                    insight.effort = None;
                });
                self.record(&info.id, session_event(&session_id, resumed), now);
                effects.extend(self.capture_session(info, &session_id));
            }
            // /clear, /resume and a branch move the pane to another
            // conversation; the first start repeats what the hello said.
            BridgeEvent::Session { session_id, source } => {
                let reported = self
                    .machines
                    .get(&info.id)
                    .and_then(|m| m.reported_session.clone());
                if reported.is_some_and(|reported| reported != session_id) {
                    if let Some(machine) = self.machines.get_mut(&info.id) {
                        machine.reported_session = Some(session_id.clone());
                    }
                    self.insights
                        .update(&info.id, |i| i.session_since = Some(now));
                    self.record(
                        &info.id,
                        session_event(&session_id, source == "resume"),
                        now,
                    );
                }
                effects.extend(self.capture_session(info, &session_id));
            }
            BridgeEvent::Turn(TurnPhase::Start) => {
                self.set_activity(info, AgentActivity::Working, now);
                self.insights
                    .update(&info.id, |i| i.turn_started_at = Some(now));
                self.record(&info.id, PaneEventDetail::TurnStart, now);
            }
            BridgeEvent::Turn(TurnPhase::End { reason, answer, .. }) => {
                let line = first_line(&answer, EVENT_LINE_MAX);
                let last_answer = Some(line.clone()).filter(|line| !line.is_empty());
                self.insights.update(&info.id, |insight| {
                    insight.turn_started_at = None;
                    insight.last_answer = last_answer.clone();
                });
                let outcome = match reason.as_str() {
                    "answer" => TurnOutcome::Answer,
                    "aborted" => TurnOutcome::Aborted,
                    _ => TurnOutcome::Failed,
                };
                self.record(
                    &info.id,
                    PaneEventDetail::TurnEnd {
                        outcome,
                        answer: last_answer,
                    },
                    now,
                );
                let body = if answer.is_empty() {
                    amb.t
                        .t(if reason == "aborted" {
                            Key::NotifyAborted
                        } else {
                            Key::NotifySettled
                        })
                        .into_owned()
                } else {
                    answer
                };
                effects.extend(self.hand_back(info, body, amb));
            }
            BridgeEvent::Permission { waiting, tool, .. } => {
                if waiting {
                    self.set_activity(info, AgentActivity::Waiting, now);
                    let tool = tool.filter(|tool| !tool.is_empty());
                    self.record(
                        &info.id,
                        PaneEventDetail::Permission { tool: tool.clone() },
                        now,
                    );
                    let body = amb
                        .t
                        .t_with(
                            Key::NotifyPermission,
                            &args!("tool" => tool.as_deref().unwrap_or("")),
                        )
                        .into_owned();
                    effects.extend(self.announce(info, body, amb));
                } else if self
                    .machines
                    .get(&info.id)
                    .is_some_and(|m| m.activity == AgentActivity::Waiting)
                {
                    self.set_activity(info, AgentActivity::Working, now);
                }
            }
            BridgeEvent::Usage {
                context,
                rate_limits,
                cost_usd,
            } => {
                let size = context.window;
                let pct = context.percent.or_else(|| {
                    context
                        .tokens
                        .filter(|_| size > 0)
                        .map(|tokens| tokens as f64 / size as f64 * 100.0)
                });
                let model = self
                    .machines
                    .get(&info.id)
                    .and_then(|m| m.bridge_model.clone());
                let gauge = pct.map(|pct| ContextGauge {
                    pct: pct.round().clamp(0.0, 100.0),
                    used_tokens: context
                        .tokens
                        .unwrap_or_else(|| (pct / 100.0 * size as f64).round().max(0.0) as u64),
                    window: size,
                    model,
                });
                if let Some(machine) = self.machines.get_mut(&info.id) {
                    machine.bridge_context = gauge;
                }
                self.refresh_gauge(&info.id);
                self.agents.set_plan_limits(rate_limits, now);
                self.insights.update(&info.id, |i| i.cost_usd = cost_usd);
            }
            BridgeEvent::Edit {
                file,
                overlap,
                overlap_pane,
            } => {
                self.insights.record_edited_file(&info.id, &file);
                self.record(
                    &info.id,
                    PaneEventDetail::Edit {
                        file: file_base_name(&file),
                        overlap: overlap.filter(|overlap| !overlap.is_empty()),
                    },
                    now,
                );
                // The pane that had edited it first says so for a moment.
                // Nothing waits on it: the mod told this agent, and both
                // edits went through.
                if let Some(first) = overlap_pane.filter(|pane| !pane.is_empty()) {
                    self.agents.flag_overlap(&first, &info.name, now);
                }
            }
            BridgeEvent::Dropped { text, .. } => {
                self.record(
                    &info.id,
                    PaneEventDetail::Dropped {
                        text: first_line(&text, EVENT_LINE_MAX),
                    },
                    now,
                );
                let body = amb
                    .t
                    .t_with(Key::NotifyDropped, &args!("text" => text.as_str()))
                    .into_owned();
                effects.extend(self.announce(info, body, amb));
            }
            // The TerminalPane has nothing to do with a call as it starts or
            // ends; the transcript has it with its result.
            BridgeEvent::Tool(ToolPhase::Start { .. } | ToolPhase::End { .. }) => {}
            // How the session runs: its model (which the gauge names), its
            // mode and its effort, each as far as the report knows it.
            BridgeEvent::Settings {
                model,
                mode,
                effort,
            } => {
                let known = |value: Option<String>| value.filter(|value| !value.is_empty());
                let model = known(model);
                if let (Some(model), Some(machine)) = (&model, self.machines.get_mut(&info.id)) {
                    machine.bridge_model = Some(model.clone());
                    if let Some(gauge) = machine.bridge_context.as_mut() {
                        gauge.model = Some(model.clone());
                    }
                }
                self.refresh_gauge(&info.id);
                let (mode, effort) = (known(mode), known(effort));
                self.insights.update(&info.id, |insight| {
                    if let Some(model) = model {
                        insight.model = Some(model);
                    }
                    if let Some(mode) = mode {
                        insight.permission_mode = Some(mode);
                    }
                    if let Some(effort) = effort {
                        insight.effort = Some(effort);
                    }
                });
            }
            // The agent exited (Claude Code, or the Codex TUI left its
            // server); whatever runs in the pane next is a shell.
            BridgeEvent::End { .. } => {
                self.hibernation.note_agent_exited(&info.id, true);
                self.forget_title(&info.id);
                self.set_bridged(info, false, now);
                if let Some(machine) = self.machines.get_mut(&info.id) {
                    machine.bridge_context = None;
                    machine.reported_session = None;
                }
                self.refresh_gauge(&info.id);
                self.set_activity(info, AgentActivity::Idle, now);
                self.insights.update(&info.id, |i| i.turn_started_at = None);
                self.record(
                    &info.id,
                    PaneEventDetail::Exit {
                        what: ExitWhat::Agent,
                    },
                    now,
                );
            }
        }
        effects
    }

    /// The id typed into the shell on the next launch, after `--resume`:
    /// anything but a plain id is refused rather than quoted, and so is the
    /// one the pane already holds.
    fn capture_session(&mut self, info: &PaneInfo, session_id: &str) -> Option<Effect> {
        let plain = !session_id.is_empty()
            && session_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !plain || info.session_id.as_deref() == Some(session_id) {
            return None;
        }
        self.claims.claim(session_id);
        Some(Effect::CaptureSession(session_id.to_string()))
    }
}

// ----- what the agents' own terminals say
//
// Read on any pane no side channel speaks for, whatever its kind: a shell
// can run either agent, and none draws another's screens. Codex's are read
// in its source at the pinned release (`agent::codex::CHECKED_AT`, paths
// under `codex-rs/`), never run (docs/native/agents.md §4.0).

/// What an agent's own title says of it ([`title_says`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleSays {
    /// At work: a turn runs, or the agent starts.
    Busy,
    /// Blocked on the person: an approval, a question.
    Asking,
    /// Its turn is over: the prompt is the person's.
    Resting,
}

/// What Codex's title says while it is blocked on the person, before the
/// rest of the title (`tui/src/chatwidget/status_surfaces.rs`,
/// `TERMINAL_TITLE_ACTION_REQUIRED_PREFIX`; the `[ . ]` form is the blink's
/// other phase).
const ACTION_REQUIRED: &str = "Action Required";
const ACTION_REQUIRED_PREFIXES: [&str; 2] = ["[ ! ] Action Required", "[ . ] Action Required"];

/// The frames of Codex's title spinner (`status_surfaces.rs`,
/// `TERMINAL_TITLE_SPINNER_FRAMES`).
const SPINNER_FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// What Codex's title says of its turn, from its source
/// (`tui/src/chatwidget/status_surfaces.rs`, `tui/src/bottom_pane/
/// {title_setup,action_required_title}.rs`, `tui/src/terminal_title.rs`):
///
/// - its items are, unless the person chose others (`tui.terminal_title`),
///   `activity`, `thread-name`, `project-name`, joined by ` | `, and by a
///   plain space next to `activity`;
/// - `activity` is a frame of its spinner while a turn runs or its MCP
///   servers start (after `● ` while the microphone listens), nothing
///   between turns, and while it is blocked on the person the whole title
///   starts with `[ ! ] Action Required` instead;
/// - the `run-state` item, when the person's items name it, says `Starting`,
///   `Working`, `Thinking`, `Waiting` (on a background terminal: the turn
///   still runs) or `Ready`.
///
/// `None` when the title carries none of these: between turns with the
/// default items, with the spinner turned off (`tui.animations`), or
/// someone else's title — a shell's, Claude Code's.
pub fn title_says(title: &str) -> Option<TitleSays> {
    let title = title.trim();
    if ACTION_REQUIRED_PREFIXES
        .iter()
        .any(|prefix| title.starts_with(prefix))
    {
        return Some(TitleSays::Asking);
    }
    let mut words = title.split_whitespace();
    let activity = match words.next() {
        Some("●") => words.next(),
        first => first,
    };
    let frame = |word: &str| {
        let mut chars = word.chars();
        chars.next().is_some_and(|c| SPINNER_FRAMES.contains(&c)) && chars.next().is_none()
    };
    if activity.is_some_and(frame) {
        return Some(TitleSays::Busy);
    }
    title.split(" | ").find_map(|item| {
        let words: Vec<&str> = item
            .split_whitespace()
            .filter(|word| *word != "●" && !frame(word))
            .collect();
        match words.as_slice() {
            ["Starting" | "Working" | "Thinking" | "Waiting"] => Some(TitleSays::Busy),
            ["Ready"] => Some(TitleSays::Resting),
            _ => None,
        }
    })
}

/// Whether an agent's desktop notification asks something of the person
/// rather than says it is done: Codex's approvals, its MCP servers'
/// elicitations, its plan-mode prompts and its questions
/// (`tui/src/chatwidget/notifications.rs`, `Notification::display`). Its
/// turn's end says its answer, or "Agent turn complete".
pub fn asks_the_person(message: &str) -> bool {
    const ASKING: [&str; 5] = [
        "Approval requested: ",
        "Approval requested by ",
        "Codex wants to edit ",
        "Plan mode prompt: ",
        "Question: ",
    ];
    ASKING.iter().any(|lead| message.starts_with(lead))
}

/// An agent's question about the folder it was started in, before its
/// session exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustQuestion {
    /// Whether to trust the folder, which Hangar says in its own words
    /// (Claude Code's « Quick safety check », `agents::is_trust_prompt`).
    Trust,
    /// A dialog the agent names itself, named as it does.
    Named(&'static str),
}

/// The heading of Codex's dialog, and the first choice of each of its forms
/// (`tui/src/onboarding/trust_directory.rs`): the folder to trust, a folder
/// opened restricted, a task that was trusted before.
const FOLDER_ACCESS: &str = "Folder access";
const FOLDER_ACCESS_CHOICES: [&str; 3] = [
    "trust and continue",
    "open restricted",
    "open existing task",
];

/// Which agent's question about its folder the screen shows, if any:
/// Claude Code's (« Quick safety check… Yes, I trust this folder ») or
/// Codex's « Folder access » dialog, with its choice. A heading and a choice
/// are both needed, wrapped or not, as for Claude Code's: a transcript that
/// merely talks about them is not the question.
pub fn trust_question(screen: &str) -> Option<TrustQuestion> {
    if is_trust_prompt(screen) {
        return Some(TrustQuestion::Trust);
    }
    let text = screen
        .split(|c: char| c.is_whitespace() || c == '\u{feff}')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let codex = text.contains(&FOLDER_ACCESS.to_lowercase())
        && FOLDER_ACCESS_CHOICES
            .iter()
            .any(|choice| text.contains(choice));
    codex.then_some(TrustQuestion::Named(FOLDER_ACCESS))
}

fn session_event(session_id: &str, resumed: bool) -> PaneEventDetail {
    PaneEventDetail::Session {
        id: short_session_id(session_id),
        resumed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hangar_i18n::Locale;
    use serde_json::json;

    const NOW: EpochMs = 1_000_000;

    fn info(agent: AgentId) -> PaneInfo {
        PaneInfo {
            id: "p1".into(),
            workspace_id: "w1".into(),
            name: "Ava".into(),
            agent,
            session_id: None,
        }
    }

    fn amb(now: EpochMs) -> Ambient {
        Ambient {
            now,
            looking: false,
            window_focused: false,
            notify_on_idle: true,
            notify_only_when_away: true,
            t: Translator::new(Locale::En),
        }
    }

    fn mounted(agent: AgentId) -> (Live, PaneInfo) {
        let mut live = Live::default();
        let info = info(agent);
        live.mount_pane(&info.id, &info.workspace_id, &info.name, NOW);
        (live, info)
    }

    fn activity(live: &Live, info: &PaneInfo) -> Option<AgentActivity> {
        live.agents.get(&info.id).map(|entry| entry.activity)
    }

    fn report(
        live: &mut Live,
        info: &PaneInfo,
        raw: serde_json::Value,
        amb: &Ambient,
    ) -> Vec<Effect> {
        let event = BridgeEvent::parse(&raw).expect("a known report");
        live.on_bridge(info, event, amb)
    }

    fn said(key: Key) -> String {
        Translator::new(Locale::En).t(key).into_owned()
    }

    // ----- the mod

    #[test]
    fn the_first_report_proves_the_mod_loaded_and_publishes_the_pane_as_bridged() {
        let (mut live, info) = mounted(AgentId::Claude);
        assert!(live.agents.get("p1").is_none());
        report(
            &mut live,
            &info,
            json!({"kind": "hello", "sessionId": "abc", "model": "claude-opus-5", "version": "2.1"}),
            &amb(NOW),
        );
        let published = live.agents.get("p1").unwrap();
        assert!(published.bridged);
        assert_eq!(published.workspace_id, "w1");
        assert_eq!(published.activity, AgentActivity::Idle);
    }

    #[test]
    fn a_shell_pane_is_published_only_once_a_claude_code_in_it_reports() {
        let (mut live, info) = mounted(AgentId::Shell);
        assert_eq!(activity(&live, &info), None);
        report(
            &mut live,
            &info,
            json!({"kind": "turn", "phase": "start"}),
            &amb(NOW),
        );
        assert_eq!(activity(&live, &info), Some(AgentActivity::Working));
        // The agent quits: back to a shell, and out of the count.
        report(
            &mut live,
            &info,
            json!({"kind": "end", "reason": "exit"}),
            &amb(NOW + 1),
        );
        assert_eq!(activity(&live, &info), None);
    }

    #[test]
    fn a_hello_drops_the_work_guessed_from_startup_output_and_fills_the_insight() {
        let (mut live, mut info) = mounted(AgentId::Claude);
        info.session_id = Some("abc".into());
        let a = amb(NOW);
        live.machines.get_mut("p1").unwrap().activity = AgentActivity::Working;
        let effects = report(
            &mut live,
            &info,
            json!({"kind": "hello", "sessionId": "abc", "model": "claude-opus-5", "version": "2.1.0"}),
            &a,
        );
        assert_eq!(activity(&live, &info), Some(AgentActivity::Idle));
        let insight = live.insights.get("p1");
        assert_eq!(insight.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(insight.agent_version.as_deref(), Some("2.1.0"));
        assert_eq!(insight.session_since, Some(NOW));
        assert_eq!(insight.turn_started_at, None);
        // The session was the stored one: resumed, and nothing to capture.
        assert_eq!(
            insight.events[0].detail,
            PaneEventDetail::Session {
                id: "abc".into(),
                resumed: true
            }
        );
        assert!(effects.is_empty());
    }

    #[test]
    fn a_hello_on_another_session_is_new_and_captured_once_it_is_a_plain_id() {
        let (mut live, mut info) = mounted(AgentId::Claude);
        info.session_id = Some("old-id".into());
        let effects = report(
            &mut live,
            &info,
            json!({"kind": "hello", "sessionId": "new-id_1", "model": "m", "version": "v"}),
            &amb(NOW),
        );
        assert_eq!(effects, [Effect::CaptureSession("new-id_1".into())]);
        assert!(live.claims.contains("new-id_1"));
        assert_eq!(
            live.insights.get("p1").events[0].detail,
            PaneEventDetail::Session {
                id: "new-id".into(),
                resumed: false
            }
        );
        // An id that is not a plain one is refused rather than quoted.
        let refused = report(
            &mut live,
            &info,
            json!({"kind": "session", "sessionId": "x; rm -rf ~", "source": "startup"}),
            &amb(NOW),
        );
        assert!(refused.is_empty());
    }

    #[test]
    fn a_session_event_moves_the_pane_to_another_conversation_only_when_it_differs() {
        let (mut live, info) = mounted(AgentId::Claude);
        report(
            &mut live,
            &info,
            json!({"kind": "hello", "sessionId": "one", "model": "m", "version": "v"}),
            &amb(NOW),
        );
        let events = |live: &Live| live.insights.get("p1").events.len();
        let before = events(&live);
        // The first start repeats what the hello said.
        report(
            &mut live,
            &info,
            json!({"kind": "session", "sessionId": "one", "source": "startup"}),
            &amb(NOW + 5),
        );
        assert_eq!(events(&live), before);
        // /resume lands on another one.
        report(
            &mut live,
            &info,
            json!({"kind": "session", "sessionId": "two", "source": "resume"}),
            &amb(NOW + 10),
        );
        assert_eq!(events(&live), before + 1);
        assert_eq!(live.insights.get("p1").session_since, Some(NOW + 10));
        assert_eq!(
            live.insights.get("p1").events[0].detail,
            PaneEventDetail::Session {
                id: "two".into(),
                resumed: true
            }
        );
    }

    #[test]
    fn a_turn_is_working_until_it_ends_and_then_it_is_yours() {
        let (mut live, info) = mounted(AgentId::Claude);
        report(
            &mut live,
            &info,
            json!({"kind": "turn", "phase": "start"}),
            &amb(NOW),
        );
        assert_eq!(activity(&live, &info), Some(AgentActivity::Working));
        assert_eq!(live.insights.get("p1").turn_started_at, Some(NOW));
        assert!(live.hibernation.pane_rest("p1").unwrap().busy);

        let effects = report(
            &mut live,
            &info,
            json!({"kind": "turn", "phase": "end", "reason": "answer", "durationMs": 4000, "answer": "\nDone: tests pass\nand more"}),
            &amb(NOW + 4000),
        );
        assert_eq!(activity(&live, &info), Some(AgentActivity::Yours));
        let insight = live.insights.get("p1");
        assert_eq!(insight.turn_started_at, None);
        assert_eq!(insight.last_answer.as_deref(), Some("Done: tests pass"));
        assert_eq!(
            insight.events[0].detail,
            PaneEventDetail::TurnEnd {
                outcome: TurnOutcome::Answer,
                answer: Some("Done: tests pass".into())
            }
        );
        // The person is elsewhere and the window is behind: a notification
        // with the whole answer, and the badge.
        assert_eq!(
            effects,
            [Effect::Notify {
                title: "Ava · Claude Code".into(),
                body: "\nDone: tests pass\nand more".into()
            }]
        );
        assert!(live.machines["p1"].attention);
        let rest = live.hibernation.pane_rest("p1").unwrap();
        assert!(!rest.busy && rest.resting_since == NOW + 4000);
    }

    #[test]
    fn a_turn_the_person_is_watching_ends_idle_and_says_nothing() {
        let (mut live, info) = mounted(AgentId::Claude);
        let watching = Ambient {
            looking: true,
            window_focused: true,
            ..amb(NOW)
        };
        report(
            &mut live,
            &info,
            json!({"kind": "turn", "phase": "start"}),
            &watching,
        );
        let effects = report(
            &mut live,
            &info,
            json!({"kind": "turn", "phase": "end", "reason": "answer", "durationMs": 1, "answer": "ok"}),
            &watching,
        );
        assert_eq!(activity(&live, &info), Some(AgentActivity::Idle));
        assert!(effects.is_empty());
        assert!(!live.machines["p1"].attention);
    }

    #[test]
    fn the_ways_a_turn_ends_are_told_apart_in_the_history_and_the_notification() {
        let end = |reason: &str, answer: &str| {
            let (mut live, info) = mounted(AgentId::Claude);
            let effects = report(
                &mut live,
                &info,
                json!({"kind": "turn", "phase": "end", "reason": reason, "durationMs": 1, "answer": answer}),
                &amb(NOW),
            );
            (live.insights.get("p1").events[0].detail.clone(), effects)
        };
        let (detail, effects) = end("aborted", "");
        assert_eq!(
            detail,
            PaneEventDetail::TurnEnd {
                outcome: TurnOutcome::Aborted,
                answer: None
            }
        );
        assert_eq!(
            effects,
            [Effect::Notify {
                title: "Ava · Claude Code".into(),
                body: said(Key::NotifyAborted)
            }]
        );
        let (detail, effects) = end("refusal", "");
        assert_eq!(
            detail,
            PaneEventDetail::TurnEnd {
                outcome: TurnOutcome::Failed,
                answer: None
            }
        );
        assert_eq!(
            effects,
            [Effect::Notify {
                title: "Ava · Claude Code".into(),
                body: said(Key::NotifySettled)
            }]
        );
        let (detail, _) = end("error", "Rate limited");
        assert_eq!(
            detail,
            PaneEventDetail::TurnEnd {
                outcome: TurnOutcome::Failed,
                answer: Some("Rate limited".into())
            }
        );
    }

    #[test]
    fn a_permission_dialog_waits_until_it_is_answered() {
        let (mut live, info) = mounted(AgentId::Claude);
        let effects = report(
            &mut live,
            &info,
            json!({"kind": "permission", "waiting": true, "tool": "Edit"}),
            &amb(NOW),
        );
        assert_eq!(activity(&live, &info), Some(AgentActivity::Waiting));
        assert_eq!(
            live.insights.get("p1").events[0].detail,
            PaneEventDetail::Permission {
                tool: Some("Edit".into())
            }
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(&effects[0], Effect::Notify { body, .. } if body.contains("Edit")));

        report(
            &mut live,
            &info,
            json!({"kind": "permission", "waiting": false}),
            &amb(NOW + 1),
        );
        assert_eq!(activity(&live, &info), Some(AgentActivity::Working));
        // A dialog closing on a pane that was not waiting changes nothing.
        report(
            &mut live,
            &info,
            json!({"kind": "turn", "phase": "end", "reason": "answer", "durationMs": 1, "answer": ""}),
            &amb(NOW + 2),
        );
        report(
            &mut live,
            &info,
            json!({"kind": "permission", "waiting": false}),
            &amb(NOW + 3),
        );
        assert_eq!(activity(&live, &info), Some(AgentActivity::Yours));
    }

    #[test]
    fn usage_fills_the_gauge_the_plan_limits_and_the_cost() {
        let (mut live, info) = mounted(AgentId::Claude);
        report(
            &mut live,
            &info,
            json!({"kind": "hello", "sessionId": "s", "model": "claude-opus-5", "version": "v"}),
            &amb(NOW),
        );
        report(
            &mut live,
            &info,
            json!({"kind": "usage",
                   "context": {"tokens": 84000, "window": 200000, "percent": null},
                   "rateLimits": [{"kind": "five_hour", "percentUsed": 31.5, "resetsAt": null}],
                   "costUsd": 1.25}),
            &amb(NOW + 10),
        );
        let insight = live.insights.get("p1");
        let gauge = insight.context.as_ref().unwrap();
        assert_eq!(
            (
                gauge.pct,
                gauge.used_tokens,
                gauge.window,
                gauge.model.as_deref()
            ),
            (42.0, 84_000, 200_000, Some("claude-opus-5"))
        );
        assert_eq!(insight.cost_usd, Some(1.25));
        let plan = live.agents.plan_limits().unwrap();
        assert_eq!((plan.limits.len(), plan.at), (1, NOW + 10));

        // The mod's own percentage wins, is clamped, and derives the tokens.
        report(
            &mut live,
            &info,
            json!({"kind": "usage",
                   "context": {"tokens": null, "window": 1000, "percent": 140.4},
                   "rateLimits": [], "costUsd": null}),
            &amb(NOW + 20),
        );
        let gauge = live.insights.get("p1").context.clone().unwrap();
        assert_eq!((gauge.pct, gauge.used_tokens), (100.0, 1404));
        assert_eq!(live.insights.get("p1").cost_usd, None);
        // No window known and no percentage: no gauge.
        report(
            &mut live,
            &info,
            json!({"kind": "usage",
                   "context": {"tokens": null, "window": 0, "percent": null},
                   "rateLimits": [], "costUsd": null}),
            &amb(NOW + 30),
        );
        assert_eq!(live.insights.get("p1").context, None);
        // And the plan keeps what the last report that had windows said.
        assert_eq!(live.agents.plan_limits().unwrap().at, NOW + 10);
    }

    #[test]
    fn the_gauge_falls_back_on_the_transcripts_when_the_mod_has_none() {
        let (mut live, info) = mounted(AgentId::Claude);
        let transcript = ContextGauge {
            pct: 10.0,
            used_tokens: 20_000,
            window: 200_000,
            model: Some("claude-sonnet-4-5".into()),
        };
        live.set_transcript_context("p1", Some(transcript.clone()));
        // Without the mod, the transcript names the model too.
        assert_eq!(live.insights.get("p1").context, Some(transcript.clone()));
        assert_eq!(
            live.insights.get("p1").model.as_deref(),
            Some("claude-sonnet-4-5")
        );
        report(
            &mut live,
            &info,
            json!({"kind": "usage",
                   "context": {"tokens": 100000, "window": 200000, "percent": null},
                   "rateLimits": [], "costUsd": null}),
            &amb(NOW),
        );
        assert_eq!(live.insights.get("p1").context.as_ref().unwrap().pct, 50.0);
        // The agent quits: the mod's figure goes, the transcript's is back.
        report(
            &mut live,
            &info,
            json!({"kind": "end", "reason": "exit"}),
            &amb(NOW + 1),
        );
        assert_eq!(live.insights.get("p1").context, Some(transcript));
    }

    #[test]
    fn an_edit_is_listed_once_per_file_in_a_row_and_flags_the_pane_it_overlaps() {
        let (mut live, info) = mounted(AgentId::Claude);
        let edit = |file: &str| json!({"kind": "edit", "file": file});
        report(&mut live, &info, edit("src/lib/a.ts"), &amb(NOW));
        report(&mut live, &info, edit("src/lib/a.ts"), &amb(NOW + 1));
        assert_eq!(live.insights.get("p1").events.len(), 1);
        report(&mut live, &info, edit("src/b.ts"), &amb(NOW + 2));
        report(&mut live, &info, edit("src/lib/a.ts"), &amb(NOW + 3));
        assert_eq!(live.insights.get("p1").events.len(), 3);
        assert_eq!(
            live.insights.get("p1").edited_files,
            ["src/lib/a.ts", "src/b.ts"]
        );
        assert_eq!(
            live.insights.get("p1").events[0].detail,
            PaneEventDetail::Edit {
                file: "a.ts".into(),
                overlap: None
            }
        );

        report(
            &mut live,
            &info,
            json!({"kind": "edit", "file": "NOTES.md", "overlap": "Bob", "overlapPane": "p9"}),
            &amb(NOW + 4),
        );
        assert_eq!(
            live.insights.get("p1").events[0].detail,
            PaneEventDetail::Edit {
                file: "NOTES.md".into(),
                overlap: Some("Bob".into())
            }
        );
        // The pane that edited it first is told who followed, for a while.
        assert_eq!(live.agents.conflict("p9"), Some("Ava"));
    }

    #[test]
    fn a_prompt_the_session_refused_is_listed_and_announced() {
        let (mut live, info) = mounted(AgentId::Claude);
        let effects = report(
            &mut live,
            &info,
            json!({"kind": "dropped", "text": "run the tests\nplease", "reason": "hook"}),
            &amb(NOW),
        );
        assert_eq!(
            live.insights.get("p1").events[0].detail,
            PaneEventDetail::Dropped {
                text: "run the tests".into()
            }
        );
        assert!(
            matches!(&effects[0], Effect::Notify { body, .. } if body.contains("run the tests\nplease"))
        );
    }

    #[test]
    fn calls_of_the_agent_are_not_this_machines_business() {
        let (mut live, info) = mounted(AgentId::Claude);
        let effects = report(
            &mut live,
            &info,
            json!({"kind": "tool", "phase": "start", "toolUseId": "t1", "tool": "Bash", "input": {}}),
            &amb(NOW),
        );
        assert!(effects.is_empty());
        assert!(live.insights.get("p1").events.is_empty());
        // Still proof that the mod loaded.
        assert!(live.agents.get("p1").unwrap().bridged);
    }

    #[test]
    fn the_end_of_the_agent_is_idle_unbridged_and_in_the_history() {
        let (mut live, info) = mounted(AgentId::Claude);
        report(
            &mut live,
            &info,
            json!({"kind": "turn", "phase": "start"}),
            &amb(NOW),
        );
        report(
            &mut live,
            &info,
            json!({"kind": "end", "reason": "exit"}),
            &amb(NOW + 9),
        );
        assert_eq!(activity(&live, &info), Some(AgentActivity::Idle));
        assert!(!live.agents.get("p1").unwrap().bridged);
        assert_eq!(live.insights.get("p1").turn_started_at, None);
        assert_eq!(
            live.insights.get("p1").events[0].detail,
            PaneEventDetail::Exit {
                what: ExitWhat::Agent
            }
        );
        // What runs in the pane now is a shell's business.
        assert!(live.hibernation.pane_rest("p1").unwrap().agent_exited);
        // And a hello says it is an agent again.
        report(
            &mut live,
            &info,
            json!({"kind": "hello", "sessionId": "s", "model": "", "version": ""}),
            &amb(NOW + 20),
        );
        assert!(!live.hibernation.pane_rest("p1").unwrap().agent_exited);
        // An empty model and version are nothing, not empty text.
        assert_eq!(live.insights.get("p1").model, None);
        assert_eq!(live.insights.get("p1").agent_version, None);
    }

    #[test]
    fn a_shell_that_ends_leaves_nothing_working() {
        let (mut live, info) = mounted(AgentId::Claude);
        report(
            &mut live,
            &info,
            json!({"kind": "turn", "phase": "start"}),
            &amb(NOW),
        );
        live.shell_exited(&info, NOW + 5);
        assert_eq!(live.status["p1"], PaneStatus::Exited);
        assert_eq!(activity(&live, &info), Some(AgentActivity::Idle));
        assert!(!live.agents.get("p1").unwrap().bridged);
        assert_eq!(live.insights.get("p1").turn_started_at, None);
        assert_eq!(
            live.insights.get("p1").events[0].detail,
            PaneEventDetail::Exit {
                what: ExitWhat::Shell
            }
        );
    }

    #[test]
    fn a_report_for_a_pane_nobody_mounted_is_nobodys() {
        let mut live = Live::default();
        let effects = report(
            &mut live,
            &info(AgentId::Claude),
            json!({"kind": "turn", "phase": "start"}),
            &amb(NOW),
        );
        assert!(effects.is_empty());
        assert_eq!(live.agents.get("p1"), None);
        assert!(!live.insights.contains("p1"));
    }

    // ----- the queue

    #[test]
    fn each_prompt_that_joins_the_queue_is_a_line_in_the_history() {
        let (mut live, _info) = mounted(AgentId::Claude);
        let q = |id: &str, text: &str, from: Option<&str>| QueuedPrompt {
            id: id.into(),
            text: text.into(),
            from: from.map(str::to_string),
        };
        live.queue_changed("p1", vec![q("1", "run the tests", None)], NOW);
        live.queue_changed(
            "p1",
            vec![
                q("1", "run the tests", None),
                q("2", "task done\nmore", Some("hangar")),
            ],
            NOW + 1,
        );
        assert_eq!(live.insights.get("p1").queue.len(), 2);
        let events = &live.insights.get("p1").events;
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].detail,
            PaneEventDetail::Queued {
                text: "task done".into(),
                from: Some("hangar".into())
            }
        );
        // Taking one out is not a line.
        live.queue_changed(
            "p1",
            vec![q("2", "task done\nmore", Some("hangar"))],
            NOW + 2,
        );
        assert_eq!(live.insights.get("p1").events.len(), 2);
        assert_eq!(live.insights.get("p1").queue.len(), 1);
    }

    // ----- the guesses, for a Claude Code whose mod has not spoken (or is off)

    #[test]
    fn a_long_burst_then_silence_is_work_then_a_hand_back() {
        let (mut live, info) = mounted(AgentId::Claude);
        let mut now = NOW;
        for _ in 0..40 {
            live.note_output("p1", now);
            now += 100;
        }
        let effects = live.poll_pane(&info, 3000, &amb(now));
        assert!(effects.is_empty());
        assert_eq!(activity(&live, &info), Some(AgentActivity::Working));
        assert!(live.hibernation.pane_rest("p1").unwrap().busy);

        let later = now + 5000;
        let effects = live.poll_pane(&info, 3000, &amb(later));
        assert_eq!(activity(&live, &info), Some(AgentActivity::Yours));
        assert_eq!(
            effects,
            [Effect::Notify {
                title: "Ava · Claude Code".into(),
                body: said(Key::NotifySettled)
            }]
        );
    }

    #[test]
    fn once_the_mod_speaks_the_guesses_stand_down() {
        let (mut live, info) = mounted(AgentId::Claude);
        let mut now = NOW;
        for _ in 0..40 {
            live.note_output("p1", now);
            now += 100;
        }
        report(
            &mut live,
            &info,
            json!({"kind": "hello", "sessionId": "s", "model": "m", "version": "v"}),
            &amb(now),
        );
        assert!(live.poll_pane(&info, 3000, &amb(now + 10_000)).is_empty());
        assert_eq!(activity(&live, &info), Some(AgentActivity::Idle));
        assert!(live.bell(&info, &amb(now + 20_000)).is_empty());
        assert!(
            live.terminal_notification(&info, "done", &amb(now + 20_000))
                .is_empty()
        );
    }

    #[test]
    fn a_bell_hands_back_at_once() {
        let (mut live, info) = mounted(AgentId::Claude);
        let effects = live.bell(&info, &amb(NOW));
        assert_eq!(activity(&live, &info), Some(AgentActivity::Yours));
        assert_eq!(effects.len(), 1);
        // The bell right after the silence is the same hand-back.
        let again = live.bell(&info, &amb(NOW + 500));
        assert!(again.is_empty());
    }

    #[test]
    fn a_plain_shell_that_rings_hands_back_but_is_not_counted_as_an_agent() {
        let (mut live, info) = mounted(AgentId::Shell);
        let effects = live.bell(&info, &amb(NOW));
        assert_eq!(effects.len(), 1);
        assert_eq!(live.machines["p1"].activity, AgentActivity::Yours);
        assert!(live.machines["p1"].attention);
        assert_eq!(activity(&live, &info), None);
    }

    #[test]
    fn a_terminal_notification_hands_back_with_the_agents_own_words() {
        let (mut live, info) = mounted(AgentId::Claude);
        let effects = live.terminal_notification(&info, "Build finished", &amb(NOW));
        assert_eq!(
            effects,
            [Effect::Notify {
                title: "Ava · Claude Code".into(),
                body: "Build finished".into()
            }]
        );
        let (mut live, info) = mounted(AgentId::Claude);
        let effects = live.terminal_notification(&info, "", &amb(NOW));
        assert!(
            matches!(&effects[0], Effect::Notify { body, .. } if *body == said(Key::NotifySettled))
        );
    }

    #[test]
    fn osc_9_is_a_notification_only_when_a_human_wrote_it() {
        assert_eq!(osc9_notification("Build done "), Some("Build done".into()));
        assert_eq!(osc9_notification(""), Some(String::new()));
        assert_eq!(osc9_notification("4;3;0"), None);
        assert_eq!(osc9_notification("9;C:\\work"), None);
        assert_eq!(osc9_notification("4"), None);
        assert_eq!(
            osc9_notification("4 files changed"),
            Some("4 files changed".into())
        );
        assert_eq!(osc9_notification("v2 ready"), Some("v2 ready".into()));
    }

    #[test]
    fn osc_777_is_a_notification_only_of_the_notify_kind() {
        assert_eq!(
            osc777_notification("notify;Title;Body"),
            Some("Body".into())
        );
        assert_eq!(osc777_notification("notify; Title ;"), Some("Title".into()));
        assert_eq!(osc777_notification("notify"), Some(String::new()));
        assert_eq!(osc777_notification("precmd;x;y"), None);
    }

    #[test]
    fn the_trust_question_is_waiting_and_while_it_is_up_the_silence_is_not_a_hand_back() {
        let (mut live, info) = mounted(AgentId::Claude);
        live.note_output("p1", NOW);
        assert!(!live.trust_look_due("p1", NOW + TRUST_LOOK_MS - 1));
        assert!(live.trust_look_due("p1", NOW + TRUST_LOOK_MS));

        let screen = "Quick safety check: Is this a project you created or one you trust?\n1. Yes, I trust this folder";
        let effects = live.look_for_trust_question(&info, screen, &amb(NOW + TRUST_LOOK_MS));
        assert_eq!(activity(&live, &info), Some(AgentActivity::Waiting));
        assert_eq!(
            effects,
            [Effect::Notify {
                title: "Ava · Claude Code".into(),
                body: said(Key::NotifyTrustFolder)
            }]
        );
        assert!(!live.trust_look_due("p1", NOW + 10_000));

        // The short burst that drew it falls silent: not a hand-back.
        for step in 0..10 {
            live.note_output("p1", NOW + 1000 + step * 100);
        }
        assert!(live.poll_pane(&info, 1000, &amb(NOW + 20_000)).is_empty());
        assert_eq!(activity(&live, &info), Some(AgentActivity::Waiting));

        // Answered: the question is gone and the agent is at work.
        live.note_output("p1", NOW + 30_000);
        let effects = live.look_for_trust_question(&info, "Welcome back", &amb(NOW + 30_600));
        assert!(effects.is_empty());
        assert_eq!(activity(&live, &info), Some(AgentActivity::Working));
    }

    #[test]
    fn while_the_trust_question_is_up_a_long_burst_never_ends_in_a_hand_back() {
        let (mut live, info) = mounted(AgentId::Claude);
        live.note_output("p1", NOW);
        let screen = "Quick safety check: trust this folder?";
        live.look_for_trust_question(&info, screen, &amb(NOW + TRUST_LOOK_MS));
        assert_eq!(activity(&live, &info), Some(AgentActivity::Waiting));

        for step in 0..40 {
            live.note_output("p1", NOW + 1000 + step * 100);
        }
        let effects = live.poll_pane(&info, 1000, &amb(NOW + 20_000));
        // The burst may read as work, as it did in the web app, but the
        // silence after it is not the agent handing back.
        assert!(effects.is_empty());
        assert_ne!(activity(&live, &info), Some(AgentActivity::Yours));
    }

    #[test]
    fn keys_take_the_pane_back() {
        let (mut live, info) = mounted(AgentId::Claude);
        live.set_activity(&info, AgentActivity::Yours, NOW);
        live.key_pressed(&info, NOW + 1);
        assert_eq!(activity(&live, &info), Some(AgentActivity::Idle));
        live.set_activity(&info, AgentActivity::Waiting, NOW + 2);
        live.key_pressed(&info, NOW + 3);
        assert_eq!(activity(&live, &info), Some(AgentActivity::Working));
        live.key_pressed(&info, NOW + 4);
        assert_eq!(activity(&live, &info), Some(AgentActivity::Working));
    }

    #[test]
    fn looking_at_the_pane_clears_its_badge_and_takes_the_hand_back() {
        let (mut live, info) = mounted(AgentId::Claude);
        live.bell(&info, &amb(NOW));
        assert!(live.machines["p1"].attention);
        // Not attended: nothing changes.
        live.attend(&info, false, false, NOW + 1);
        assert!(live.machines["p1"].attention);
        // The pane becomes the focused, shown one.
        live.attend(&info, true, false, NOW + 2);
        assert!(!live.machines["p1"].attention);
        assert_eq!(activity(&live, &info), Some(AgentActivity::Idle));

        // It hands back again while attended but the window is behind: held
        // until the window comes back, not cleared by every look.
        live.bell(&info, &amb(NOW + 3_000));
        assert!(live.machines["p1"].attention);
        live.attend(&info, true, false, NOW + 3_001);
        assert!(live.machines["p1"].attention);
        live.attend(&info, true, true, NOW + 3_002);
        assert!(!live.machines["p1"].attention);
    }

    #[test]
    fn a_waiting_pane_stays_waiting_when_it_is_looked_at() {
        let (mut live, info) = mounted(AgentId::Claude);
        live.set_activity(&info, AgentActivity::Waiting, NOW);
        live.attend(&info, true, false, NOW + 1);
        assert_eq!(activity(&live, &info), Some(AgentActivity::Waiting));
    }

    #[test]
    fn notifications_follow_the_settings_and_the_window() {
        let ring = |amb: Ambient| {
            let (mut live, info) = mounted(AgentId::Claude);
            let effects = live.bell(&info, &amb);
            (effects.len(), live.machines["p1"].attention)
        };
        // Window in front but on another pane: the badge is enough.
        assert_eq!(
            ring(Ambient {
                window_focused: true,
                ..amb(NOW)
            }),
            (0, true)
        );
        // Window in front, and the person said "even then".
        assert_eq!(
            ring(Ambient {
                window_focused: true,
                notify_only_when_away: false,
                ..amb(NOW)
            }),
            (1, true)
        );
        // Notifications off: the badge alone.
        assert_eq!(
            ring(Ambient {
                notify_on_idle: false,
                ..amb(NOW)
            }),
            (0, true)
        );
        // Looking at it: not news.
        assert_eq!(
            ring(Ambient {
                looking: true,
                window_focused: true,
                ..amb(NOW)
            }),
            (0, false)
        );
    }

    // ----- lifecycle

    #[test]
    fn unmounting_forgets_the_pane_unless_it_is_handed_to_another_window() {
        let (mut live, info) = mounted(AgentId::Claude);
        report(
            &mut live,
            &info,
            json!({"kind": "turn", "phase": "start"}),
            &amb(NOW),
        );
        live.unmount_pane("p1", true);
        assert!(!live.is_pane_mounted("p1"));
        assert!(live.agents.get("p1").is_some());
        assert!(live.insights.contains("p1"));
        assert!(live.hibernation.pane_rest("p1").is_none());

        live.unmount_pane("p1", false);
        assert!(live.agents.get("p1").is_none());
        assert!(!live.insights.contains("p1"));
    }

    /// Codex panes: one attached to its own server, read from Hangar's
    /// client of it as a Claude Code is from its mod, and one on the floor
    /// (D6), read from its terminal. The reports are those of
    /// `agent/codex_attach.rs`; the screens, titles and notifications are
    /// written from Codex's source at the pinned release
    /// (`agent::codex::CHECKED_AT`, paths under `codex-rs/`), never run.
    mod codex {
        use hangar_core::agent::{AgentKind, Asks, IdleInject, RunningInject, TurnSignal};

        use super::*;

        const THREAD: &str = "0199a5c4-7b3e-7c51-a2f4-6d0e8c1b9f20";

        fn permission(tool: &str) -> String {
            Translator::new(Locale::En)
                .t_with(Key::NotifyPermission, &args!("tool" => tool))
                .into_owned()
        }

        fn notified(body: &str) -> Effect {
            Effect::Notify {
                title: "Ava · Codex".into(),
                body: body.into(),
            }
        }

        fn title(live: &mut Live, info: &PaneInfo, title: &str, now: EpochMs) -> Vec<Effect> {
            live.terminal_title(info, title, &amb(now))
        }

        // ----- attached: Hangar's client reports

        #[test]
        fn an_attached_codex_pane_is_read_from_hangar_s_client_as_claude_code_is_from_its_mod() {
            let (mut live, mut info) = mounted(AgentId::Codex);
            live.set_side_channel("p1", SideChannel::Attached);
            assert_eq!(live.side_channel("p1"), Some(&SideChannel::Attached));
            // Attached, but nothing said yet: the floor's levels until it speaks.
            let before = live.live_caps(&info).expect("an agent's");
            assert_eq!(before.turns, TurnSignal::Inferred);
            assert_eq!(before.inject.idle, IdleInject::Type);

            // The client's hello: the thread, its model, Codex's version.
            let effects = report(
                &mut live,
                &info,
                json!({ "kind": "hello", "sessionId": THREAD, "model": "gpt-5.5-codex", "version": "0.170.0" }),
                &amb(NOW),
            );
            assert_eq!(effects, [Effect::CaptureSession(THREAD.into())]);
            info.session_id = Some(THREAD.into());
            let published = live.agents.get("p1").expect("published");
            assert!(published.bridged, "its side channel spoke");
            assert_eq!(published.activity, AgentActivity::Idle);
            let caps = live.live_caps(&info).unwrap();
            assert_eq!(caps, AgentKind::Codex.info().caps);
            assert_eq!(caps.turns, TurnSignal::Reported);
            assert_eq!(caps.inject.running, RunningInject::Steer);
            assert_eq!(caps.approvals, Asks::Held);
            let insight = live.insights.get("p1");
            assert_eq!(insight.model.as_deref(), Some("gpt-5.5-codex"));
            assert_eq!(insight.agent_version.as_deref(), Some("0.170.0"));
            report(
                &mut live,
                &info,
                json!({ "kind": "settings", "model": "gpt-5.5-codex", "mode": "default", "effort": "high" }),
                &amb(NOW),
            );
            assert_eq!(live.insights.get("p1").effort.as_deref(), Some("high"));

            // A turn, an approval it waits on, the answer.
            report(
                &mut live,
                &info,
                json!({ "kind": "turn", "phase": "start" }),
                &amb(NOW + 10),
            );
            assert_eq!(activity(&live, &info), Some(AgentActivity::Working));
            report(
                &mut live,
                &info,
                json!({ "kind": "permission", "waiting": true }),
                &amb(NOW + 20),
            );
            let effects = report(
                &mut live,
                &info,
                json!({
                    "kind": "permission", "waiting": true, "tool": "Bash",
                    "input": { "command": "cargo test", "cwd": "/work/hangar" },
                    "askId": "codex-1-7", "toolUseId": "item-3",
                }),
                &amb(NOW + 30),
            );
            assert_eq!(activity(&live, &info), Some(AgentActivity::Waiting));
            assert_eq!(effects, [notified(&permission("Bash"))]);
            report(
                &mut live,
                &info,
                json!({ "kind": "permission", "waiting": false }),
                &amb(NOW + 40),
            );
            assert_eq!(activity(&live, &info), Some(AgentActivity::Working));

            // Its own title and output are not listened to while it speaks.
            assert!(
                title(
                    &mut live,
                    &info,
                    "[ ! ] Action Required | project",
                    NOW + 50
                )
                .is_empty()
            );
            assert!(live.bell(&info, &amb(NOW + 60)).is_empty());
            assert_eq!(activity(&live, &info), Some(AgentActivity::Working));

            let effects = report(
                &mut live,
                &info,
                json!({ "kind": "turn", "phase": "end", "reason": "answer", "durationMs": 3150, "answer": "All 42 tests pass." }),
                &amb(NOW + 3_200),
            );
            assert_eq!(activity(&live, &info), Some(AgentActivity::Yours));
            assert_eq!(effects, [notified("All 42 tests pass.")]);
            assert_eq!(
                live.insights.get("p1").last_answer.as_deref(),
                Some("All 42 tests pass.")
            );

            // The TUI quits, or the server goes: the floor again.
            report(
                &mut live,
                &info,
                json!({ "kind": "end", "reason": "closed" }),
                &amb(NOW + 4_000),
            );
            assert!(!live.agents.get("p1").unwrap().bridged);
            assert_eq!(activity(&live, &info), Some(AgentActivity::Idle));
            assert_eq!(live.live_caps(&info).unwrap().turns, TurnSignal::Inferred);
        }

        // ----- the floor: its terminal

        #[test]
        fn a_floor_codex_pane_reads_its_turns_from_its_title() {
            let (mut live, info) = mounted(AgentId::Codex);
            let reason =
                "Codex runs without Hangar attached: its server did not listen within 10 seconds";
            live.set_side_channel(
                "p1",
                SideChannel::Floor {
                    reason: Some(reason.into()),
                },
            );
            let caps = live.live_caps(&info).unwrap();
            assert_eq!(caps.turns, TurnSignal::Inferred);
            assert_eq!(caps.approvals, Asks::Keys);

            // Between turns the default title has no mark: nothing to say yet.
            assert!(title(&mut live, &info, "Fix login timeout | project", NOW).is_empty());
            assert_eq!(activity(&live, &info), None, "nothing published");
            // A turn: the spinner, frame after frame.
            assert!(title(&mut live, &info, "⠋ Fix login timeout | project", NOW + 100).is_empty());
            assert_eq!(activity(&live, &info), Some(AgentActivity::Working));
            assert!(title(&mut live, &info, "⠙ Fix login timeout | project", NOW + 200).is_empty());
            // Its output, the bell: the title tells the turn instead.
            let mut now = NOW + 300;
            for _ in 0..40 {
                live.note_output("p1", now);
                now += 100;
            }
            assert!(live.poll_pane(&info, 1000, &amb(now + 5_000)).is_empty());
            assert!(live.bell(&info, &amb(now + 5_000)).is_empty());
            assert_eq!(activity(&live, &info), Some(AgentActivity::Working));

            // Blocked on the person: waiting, said once over the blink.
            let effects = title(
                &mut live,
                &info,
                "[ ! ] Action Required | Fix login timeout | project",
                now + 6_000,
            );
            assert_eq!(activity(&live, &info), Some(AgentActivity::Waiting));
            assert_eq!(effects, [notified(&permission("Action Required"))]);
            assert_eq!(
                live.insights.get("p1").events[0].detail,
                PaneEventDetail::Permission { tool: None }
            );
            assert!(
                title(
                    &mut live,
                    &info,
                    "[ . ] Action Required | Fix login timeout | project",
                    now + 7_000
                )
                .is_empty()
            );
            // Answered at the pane, at work again, then done: yours.
            assert!(
                title(
                    &mut live,
                    &info,
                    "⠹ Fix login timeout | project",
                    now + 8_000
                )
                .is_empty()
            );
            assert_eq!(activity(&live, &info), Some(AgentActivity::Working));
            let effects = title(&mut live, &info, "Fix login timeout | project", now + 9_000);
            assert_eq!(activity(&live, &info), Some(AgentActivity::Yours));
            assert_eq!(effects, [notified(&said(Key::NotifySettled))]);
            // Resting stays resting.
            assert!(title(&mut live, &info, "Fix login timeout | project", now + 9_500).is_empty());
        }

        #[test]
        fn a_short_spin_of_codex_s_title_ends_idle_without_a_hand_back() {
            let (mut live, info) = mounted(AgentId::Codex);
            // Its MCP servers starting, or a reply watched as it came.
            assert!(title(&mut live, &info, "⠋ project", NOW).is_empty());
            assert_eq!(activity(&live, &info), Some(AgentActivity::Working));
            assert!(title(&mut live, &info, "project", NOW + 800).is_empty());
            assert_eq!(activity(&live, &info), Some(AgentActivity::Idle));
            assert!(!live.needs_attention("p1"));
        }

        #[test]
        fn codex_s_notifications_tell_an_approval_from_the_end_of_its_turn() {
            let (mut live, info) = mounted(AgentId::Codex);
            // `tui/src/chatwidget/notifications.rs`, `Notification::display`.
            let effects =
                live.terminal_notification(&info, "Approval requested: cargo test", &amb(NOW));
            assert_eq!(activity(&live, &info), Some(AgentActivity::Waiting));
            assert_eq!(effects, [notified("Approval requested: cargo test")]);
            // The title says the same thing: not said twice.
            assert!(
                title(
                    &mut live,
                    &info,
                    "[ ! ] Action Required | project",
                    NOW + 10
                )
                .is_empty()
            );
            assert!(title(&mut live, &info, "⠋ project", NOW + 5_000).is_empty());
            let effects =
                live.terminal_notification(&info, "All 42 tests pass.", &amb(NOW + 9_000));
            assert_eq!(activity(&live, &info), Some(AgentActivity::Yours));
            assert_eq!(effects, [notified("All 42 tests pass.")]);
            // Its title rests after: already handed back.
            assert!(title(&mut live, &info, "project", NOW + 9_010).is_empty());
            assert_eq!(activity(&live, &info), Some(AgentActivity::Yours));

            for asking in [
                "Approval requested by github",
                "Codex wants to edit src/main.rs",
                "Codex wants to edit 3 files",
                "Plan mode prompt: Implement the plan?",
                "Question: Which database?",
            ] {
                assert!(asks_the_person(asking), "{asking}");
            }
            for done in [
                "Agent turn complete",
                "All 42 tests pass.",
                "Build finished",
                "",
            ] {
                assert!(!asks_the_person(done), "{done}");
            }
        }

        #[test]
        fn codex_s_folder_access_question_is_waiting() {
            let (mut live, info) = mounted(AgentId::Codex);
            // From the source: `tui/src/onboarding/trust_directory.rs`, its
            // snapshot `renders_snapshot_for_git_repo`.
            let screen = "\n  Folder access\n  /workspace/project\n\n  Trust this folder? Codex can read, edit, and run files here,\n  subject to your permission settings. Folder settings can run code\n  automatically, even without a model request. Continue only if you\n  trust these files. Your trust decision will be saved.\n\n\u{203a} 1. Trust and continue\n  2. Quit\n\n  enter continue and create sandbox \u{b7} esc quit";
            live.note_output("p1", NOW);
            assert!(live.trust_look_due("p1", NOW + TRUST_LOOK_MS));
            let effects = live.look_for_trust_question(&info, screen, &amb(NOW + TRUST_LOOK_MS));
            assert_eq!(activity(&live, &info), Some(AgentActivity::Waiting));
            assert_eq!(effects, [notified(&permission("Folder access"))]);
            // Trusted: the session starts.
            live.note_output("p1", NOW + 5_000);
            assert!(
                live.look_for_trust_question(
                    &info,
                    "\u{203a} Ask Codex to do anything",
                    &amb(NOW + 5_600)
                )
                .is_empty()
            );
            assert_eq!(activity(&live, &info), Some(AgentActivity::Working));
        }

        #[test]
        fn each_agent_s_trust_question_is_known_and_nothing_else_is() {
            // Codex's forms (`trust_directory.rs`): the folder to trust,
            // narrow enough to wrap; a folder opened restricted; a task
            // trusted before.
            let narrow = "  Folder\n  access\n  /w/p\n  Trust this\n  folder?\n\u{203a} 1. Trust and\n  continue\n  2. Quit";
            assert_eq!(
                trust_question(narrow),
                Some(TrustQuestion::Named("Folder access"))
            );
            let restricted = "  Folder access\n  /workspace/project\n  Config, hooks, and exec policies from untrusted folders stay\n\u{203a} 1. Open restricted\n  2. Back to Agent Command Center";
            assert!(trust_question(restricted).is_some());
            let existing = "  Folder access\n  /workspace/project\n\u{203a} 1. Open existing task\n  2. Back to Agent Command Center";
            assert!(trust_question(existing).is_some());
            // Claude Code's, as `agents::is_trust_prompt` reads it.
            let claude = "Quick safety check: Is this a project you created or one you trust?\n \u{276f} 1. Yes, I trust this folder";
            assert_eq!(trust_question(claude), Some(TrustQuestion::Trust));
            // Talk about them is not the question.
            assert_eq!(trust_question("> what does Folder access mean?"), None);
            assert_eq!(trust_question("press enter: trust and continue"), None);
            assert_eq!(trust_question(""), None);
        }

        #[test]
        fn codex_s_title_is_read_as_its_source_writes_it() {
            // `status_surfaces.rs`: activity, thread name, project name.
            assert_eq!(
                title_says("⠋ Fix login timeout | project"),
                Some(TitleSays::Busy)
            );
            assert_eq!(title_says("⠏ project"), Some(TitleSays::Busy));
            assert_eq!(title_says("● ⠇ thread | project"), Some(TitleSays::Busy));
            assert_eq!(
                title_says("[ ! ] Action Required | project"),
                Some(TitleSays::Asking)
            );
            assert_eq!(
                title_says("[ . ] Action Required | Fix login timeout | project"),
                Some(TitleSays::Asking)
            );
            assert_eq!(title_says("Fix login timeout | project"), None);
            assert_eq!(title_says("project"), None);
            // The run-state item, where the person's items name it.
            assert_eq!(title_says("⠋ Working | project"), Some(TitleSays::Busy));
            assert_eq!(title_says("project | Thinking"), Some(TitleSays::Busy));
            assert_eq!(title_says("Starting | project"), Some(TitleSays::Busy));
            assert_eq!(title_says("Waiting | project"), Some(TitleSays::Busy));
            assert_eq!(title_says("Ready | project"), Some(TitleSays::Resting));
            assert_eq!(title_says("project Ready"), None);
            // Someone else's: a shell's, Claude Code's.
            for other in [
                "",
                "simon@vast: ~/hangar",
                "vim src/main.rs",
                "\u{2733} Fix the login bug",
                "\u{2802} Fix the login bug",
                "\u{2810} Claude Code",
            ] {
                assert_eq!(title_says(other), None, "{other:?}");
            }
        }

        #[test]
        fn a_title_let_go_of_hands_the_guesses_back() {
            let (mut live, info) = mounted(AgentId::Codex);
            title(&mut live, &info, "⠋ project", NOW);
            // Codex quits mid-turn: it clears the title it set
            // (`App::drop`, `clear_managed_terminal_title`).
            title(&mut live, &info, "", NOW + 5_000);
            assert_eq!(activity(&live, &info), Some(AgentActivity::Yours));
            // The shell's own work is guessed again, and so is its bell.
            assert_eq!(live.bell(&info, &amb(NOW + 10_000)).len(), 1);
        }

        #[test]
        fn claude_code_s_titles_change_nothing_for_a_claude_code_without_its_mod() {
            let (mut live, info) = mounted(AgentId::Claude);
            assert!(title(&mut live, &info, "\u{2733} Fix the login bug", NOW).is_empty());
            assert!(title(&mut live, &info, "\u{2802} Fix the login bug", NOW + 10).is_empty());
            let mut now = NOW;
            for _ in 0..40 {
                live.note_output("p1", now);
                now += 100;
            }
            live.poll_pane(&info, 3000, &amb(now));
            assert_eq!(activity(&live, &info), Some(AgentActivity::Working));
            let effects = live.poll_pane(&info, 3000, &amb(now + 5000));
            assert_eq!(activity(&live, &info), Some(AgentActivity::Yours));
            assert_eq!(effects.len(), 1);
            let caps = live.live_caps(&info).unwrap();
            assert_eq!(
                (caps.turns, caps.inject.idle),
                (TurnSignal::Inferred, IdleInject::Type)
            );
            // A shell has no agent, and so no capabilities.
            let (live, shell) = mounted(AgentId::Shell);
            assert_eq!(live.live_caps(&shell), None);
        }
    }
}
