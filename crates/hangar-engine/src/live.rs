//! What the engine keeps of each pane and agent beside the persisted state:
//! activity, insight, the reports of each pane's side channel (the
//! teskad-bridge mod in a Claude Code, Hangar's client of a Codex pane's
//! server), rate limits, and what each pane's agent can do now ([`live_caps`]).
//! The app holds these in its read-models (`Model.live`); a Hangar with no
//! window holds them alone.

use std::collections::{HashMap, HashSet};

use hangar_core::agent::{
    Asks, Capabilities, IdleInject, Inject, RoleBriefing, RunningInject, SessionId, StateBriefing,
    TurnSignal, UsageSource,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::types::{PaneId, WsId};

/// What an agent is doing (`AgentActivity`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentActivity {
    /// Busy on its own.
    Working,
    /// Blocked until the person answers.
    Waiting,
    /// Done; the turn is the person's.
    Yours,
    #[default]
    Idle,
}

/// Wall-clock milliseconds since the Unix epoch, as the TS `Date.now()`.
///
/// Every moment P4 keeps is one of these, and every state machine is handed
/// its `now` as one: a test sets the clock, and nothing inside the logic
/// reads it. It is wall-clock rather than `Instant` because the inspector
/// shows these moments as dates and clock times
/// (`new Date(event.at).toLocaleString()`), which a monotonic instant cannot
/// give.
pub type EpochMs = u64;

/// One pane's activity (`lib/agentState.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneActivity {
    pub workspace_id: WsId,
    pub activity: AgentActivity,
    /// The pane's side channel spoke — the teskad-bridge mod in its Claude
    /// Code, or Hangar's client of its Codex server (`hello`, or any report
    /// since): the state is reported rather than guessed, and the pane takes
    /// what is queued for it without typing.
    pub bridged: bool,
}

/// How a pane's agent was started, as far as its side channel goes
/// (docs/native/agents.md §3.2, D5, D6): what its launch set up, which the
/// side channel proves by speaking (`PaneActivity::bridged`), or why there
/// is none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SideChannel {
    /// Not said: a shell, a pane mirrored from its node, a pane whose launch
    /// was not the engine's.
    #[default]
    Unknown,
    /// Set up: the teskad-bridge mod within reach of its Claude Code, or
    /// Hangar's client following the Codex pane's own server.
    Attached,
    /// None: Claude Code with the mod turned off, or Codex's plain TUI (the
    /// floor, D6). `reason`: why, as the pane's status says it, when there
    /// is something to say (`FloorReason`).
    Floor { reason: Option<String> },
}

/// What a pane's agent can do now — its live capabilities
/// (docs/native/agents.md §3.2): its kind's levels (`AgentInfo::caps`, the
/// best the tested version does) while its side channel speaks (`spoke`:
/// the pane is `bridged`), and the floor otherwise. The floor is the same
/// for every kind, read from the levels and never from the kind (D3): what
/// only a side channel carries drops to what the terminal gives.
///
/// - **turns**: guessed from the terminal (`Inferred`): its output, its
///   title, its bell and notifications (`panes::pane_agent`).
/// - **inject**: typed when the agent is idle; a running turn is not reached.
/// - **approvals, questions** a side channel holds: keys, checked on screen.
/// - **briefing**: what reaches the session through its side channel — the
///   state, and a role in the system prompt (the mod's) — goes as a labelled
///   first message; a role that rides the launch line (Codex's developer
///   instructions, on the floor's line too) stays.
/// - **sessions**: the id is hunted in the agent's history folder.
/// - **usage**: the context and the limits come from the history the agent
///   writes when it writes one; limits only a side channel of Hangar's
///   reports (the mod's) are not known. The protocol's exact limits are also
///   in the agent's history (Codex's rollout `rate_limits`, §3.5).
pub fn live_caps(caps: &Capabilities, spoke: bool) -> Capabilities {
    if spoke {
        return *caps;
    }
    let mut floor = *caps;
    if floor.turns == TurnSignal::Reported {
        floor.turns = TurnSignal::Inferred;
    }
    floor.inject = Inject {
        idle: IdleInject::Type,
        running: RunningInject::None,
    };
    let held = |asks: Asks| if asks == Asks::Held { Asks::Keys } else { asks };
    floor.approvals = held(floor.approvals);
    floor.questions = held(floor.questions);
    if floor.briefing.role == RoleBriefing::SystemPrompt {
        floor.briefing.role = RoleBriefing::FirstMessage;
    }
    floor.briefing.state = StateBriefing::FirstMessage;
    floor.sessions.id = SessionId::Hunted;
    let on_disk = if floor.transcript.is_some() {
        UsageSource::Transcript
    } else {
        UsageSource::None
    };
    floor.usage.context = match floor.usage.context {
        UsageSource::Reported | UsageSource::Protocol => on_disk,
        other => other,
    };
    floor.usage.limits = match floor.usage.limits {
        UsageSource::Reported => UsageSource::None,
        UsageSource::Protocol => on_disk,
        other => other,
    };
    floor
}

/// How many of a workspace's agents are working, waiting, or done
/// (`WorkspaceActivity`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkspaceActivity {
    pub working: usize,
    pub waiting: usize,
    pub yours: usize,
}

/// The panes, by id, that are working, waiting, or done, in no particular
/// order (`AgentRoster`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentRoster {
    pub working: Vec<PaneId>,
    pub waiting: Vec<PaneId>,
    pub yours: Vec<PaneId>,
}

/// A pane's shell, as its header lives it (`PaneStatus` of PaneStatus.ts):
/// `Starting` until the PTY runs, `Exited` once the shell ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PaneStatus {
    #[default]
    Starting,
    Running,
    Exited,
}

/// A glyph's state: the shell's own life until it runs, then the agent's
/// activity (`GlyphState` of StateGlyph.tsx, `paneGlyphState`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum GlyphState {
    Starting,
    Exited,
    Working,
    Waiting,
    Yours,
    #[default]
    Idle,
}

/// A rate-limit window of the plan, as the mod reports it (`RateLimit`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimit {
    /// `five_hour`, `seven_day`, or a gateway's own window.
    pub kind: String,
    pub percent_used: f64,
    #[serde(default)]
    pub resets_at: Option<String>,
}

/// The plan's limits, from whichever session reported last, and when.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanLimits {
    pub limits: Vec<RateLimit>,
    pub at: EpochMs,
}

/// A prompt held until the pane's agent is free (`QueuedPrompt`).
pub type QueuedPrompt = hangar_core::bridge::Queued;

/// The context gauge of a pane header (`ContextGauge`).
#[derive(Debug, Clone, PartialEq)]
pub struct ContextGauge {
    /// 0–100, toward the auto-compaction boundary.
    pub pct: f64,
    pub used_tokens: u64,
    pub window: u64,
    pub model: Option<String>,
}

/// What happened in a pane, and the facts it is about, never the sentence
/// itself (`PaneEventDetail` of paneInsight.ts). The history outlives a
/// change of language, so it is worded only when it is shown
/// (`panes_logic::pane_event_text`), in the language of that moment.
#[derive(Debug, Clone, PartialEq)]
pub enum PaneEventDetail {
    /// `id` is the short form the pane header shows.
    Session {
        id: String,
        resumed: bool,
    },
    TurnStart,
    TurnEnd {
        outcome: TurnOutcome,
        /// First line of the answer, when it had one.
        answer: Option<String>,
    },
    Permission {
        tool: Option<String>,
    },
    /// The file's base name, which is all a line of history has room for, and
    /// the pane that had edited it just before, when one had.
    Edit {
        file: String,
        overlap: Option<String>,
    },
    /// `from`: who queued it when it was not the person (`queued_by`).
    Queued {
        text: String,
        from: Option<String>,
    },
    Dropped {
        text: String,
    },
    /// Claude Code quit back to its shell, or the shell itself ended.
    Exit {
        what: ExitWhat,
    },
}

impl PaneEventDetail {
    /// The `kind` string of the TS (`PaneEventKind`), for a reader that keys
    /// an icon or a filter on it.
    pub fn kind(&self) -> &'static str {
        match self {
            PaneEventDetail::Session { .. } => "session",
            PaneEventDetail::TurnStart => "turn-start",
            PaneEventDetail::TurnEnd { .. } => "turn-end",
            PaneEventDetail::Permission { .. } => "permission",
            PaneEventDetail::Edit { .. } => "edit",
            PaneEventDetail::Queued { .. } => "queued",
            PaneEventDetail::Dropped { .. } => "dropped",
            PaneEventDetail::Exit { .. } => "exit",
        }
    }
}

/// How a turn ended; `Failed` covers an error and a refusal alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    Answer,
    Aborted,
    Failed,
}

/// What ended in an `Exit` event: the agent, or the shell under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitWhat {
    Agent,
    Shell,
}

/// One line of a pane's history in the inspector (`PaneEvent`).
#[derive(Debug, Clone, PartialEq)]
pub struct PaneEvent {
    pub at: EpochMs,
    pub detail: PaneEventDetail,
}

/// The MCP servers a pane's Claude Code was started with (`PaneMcp`).
#[derive(Debug, Clone, PartialEq)]
pub struct PaneMcp {
    pub mode: PaneMcpMode,
    /// Whether Hangar's own server went into the launch (`--mcp-config`).
    /// False for a Claude Code started without a role: pinned later, it has
    /// none of the board, memory or worker tools until it restarts. `None`
    /// when the launch did not say.
    pub hangar: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaneMcpMode {
    /// Claude Code's own configuration.
    Inherit,
    /// The servers the workspace chose, Hangar's included, as they went into
    /// the launch.
    Custom { servers: Vec<String> },
}

/// What the inspector and the pane header know of a pane
/// (`lib/paneInsight.ts`); `MAX_EVENTS` 40, `MAX_EDITED_FILES` 50.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaneInsight {
    /// The model the agent runs, as the mod or the transcript names it.
    pub model: Option<String>,
    /// Its Claude Code's permission mode (`default`, `plan`, `acceptEdits`,
    /// `auto`…), as the mod last reported it.
    pub permission_mode: Option<String>,
    /// Its effort level (`low` … `max`), as the mod last reported it.
    pub effort: Option<String>,
    /// The agent's own version, from the mod's hello.
    pub agent_version: Option<String>,
    /// When this pane's current session started or was resumed.
    pub session_since: Option<EpochMs>,
    pub context: Option<ContextGauge>,
    /// What the session has cost so far, when the agent reports it.
    pub cost_usd: Option<f64>,
    /// Prompts held until the agent is free.
    pub queue: Vec<QueuedPrompt>,
    /// Most recent first, each once.
    pub edited_files: Vec<String>,
    /// Most recent first.
    pub events: Vec<PaneEvent>,
    /// First line of the agent's last answer.
    pub last_answer: Option<String>,
    /// When the running turn started; `None` between turns.
    pub turn_started_at: Option<EpochMs>,
    /// The MCP servers its Claude Code started with; `None` for other agents.
    pub mcp: Option<PaneMcp>,
}

/// What a pane's side channel reports (`BridgeEvent` of `lib/bridge.ts`):
/// the teskad-bridge mod from inside a Claude Code, or Hangar's own client
/// of a Codex pane's server (`agent/codex_attach.rs`), which reports the
/// same kinds in the same shapes. Core hands it over as raw JSON
/// (`CoreEvent::Bridge`); `BridgeEvent::parse` reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum BridgeEvent {
    #[serde(rename_all = "camelCase")]
    Hello {
        session_id: String,
        /// Empty when the session names none (the mod sends `null`).
        #[serde(default, deserialize_with = "text")]
        model: String,
        #[serde(default, deserialize_with = "text")]
        version: String,
    },
    #[serde(rename_all = "camelCase")]
    Session {
        session_id: String,
        #[serde(default, deserialize_with = "text")]
        source: String,
    },
    Turn(TurnPhase),
    Permission {
        waiting: bool,
        #[serde(default)]
        tool: Option<String>,
        #[serde(default)]
        input: Option<Value>,
    },
    Tool(ToolPhase),
    #[serde(rename_all = "camelCase")]
    Usage {
        context: UsageContext,
        #[serde(default)]
        rate_limits: Vec<RateLimit>,
        #[serde(default)]
        cost_usd: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    Edit {
        file: String,
        #[serde(default)]
        overlap: Option<String>,
        #[serde(default)]
        overlap_pane: Option<String>,
    },
    Dropped {
        text: String,
        #[serde(default, deserialize_with = "text")]
        reason: String,
    },
    /// How the session runs, when that changed: what the report knows of it.
    Settings {
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        effort: Option<String>,
    },
    End {
        #[serde(default, deserialize_with = "text")]
        reason: String,
    },
}

/// A text the mod may send as `null` or leave out: empty then, as the TS
/// read it with `||`.
fn text<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

impl BridgeEvent {
    /// The report as the mod sent it, or `None` for a kind this version does
    /// not know (a newer mod), which the TS ignored as well.
    pub fn parse(raw: &Value) -> Option<BridgeEvent> {
        serde_json::from_value(raw.clone()).ok()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "lowercase")]
pub enum TurnPhase {
    Start,
    #[serde(rename_all = "camelCase")]
    End {
        /// `answer`, `aborted`, `refusal` or `error`.
        #[serde(default, deserialize_with = "text")]
        reason: String,
        #[serde(default)]
        duration_ms: u64,
        /// First line of the final answer, possibly empty.
        #[serde(default, deserialize_with = "text")]
        answer: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "lowercase")]
pub enum ToolPhase {
    #[serde(rename_all = "camelCase")]
    Start {
        tool_use_id: String,
        tool: String,
        input: Value,
    },
    #[serde(rename_all = "camelCase")]
    End { tool_use_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageContext {
    #[serde(default)]
    pub tokens: Option<u64>,
    /// 0 when the agent does not say.
    #[serde(default)]
    pub window: u64,
    #[serde(default)]
    pub percent: Option<f64>,
}

/// Deliveries running, per workspace, by task id (`lib/deliveryRuns.ts`):
/// what the snapshot carries for the engines.
pub type Delivering = HashMap<WsId, HashSet<String>>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bridge_reports_read_with_their_nested_phase() {
        let end = json!({"kind": "turn", "phase": "end", "reason": "answer",
                         "durationMs": 1200, "answer": "Done."});
        assert_eq!(
            BridgeEvent::parse(&end),
            Some(BridgeEvent::Turn(TurnPhase::End {
                reason: "answer".into(),
                duration_ms: 1200,
                answer: "Done.".into(),
            }))
        );
        let start = json!({"kind": "turn", "phase": "start"});
        assert_eq!(
            BridgeEvent::parse(&start),
            Some(BridgeEvent::Turn(TurnPhase::Start))
        );
        assert_eq!(BridgeEvent::parse(&json!({"kind": "future"})), None);
    }

    /// A side channel that speaks gives the kind's levels; without one,
    /// both kinds fall to the same floor, each keeping what rides its
    /// launch or its history rather than the side channel.
    #[test]
    fn the_live_capabilities_fall_to_the_floor_until_the_side_channel_speaks() {
        use hangar_core::agent::AgentKind;
        for kind in AgentKind::ALL {
            let caps = kind.info().caps;
            assert_eq!(live_caps(&caps, true), caps, "{kind:?} speaking");
            let floor = live_caps(&caps, false);
            assert_eq!(floor.turns, TurnSignal::Inferred, "{kind:?}");
            assert_eq!(
                floor.inject,
                Inject {
                    idle: IdleInject::Type,
                    running: RunningInject::None,
                },
                "{kind:?}"
            );
            assert_eq!((floor.approvals, floor.questions), (Asks::Keys, Asks::Keys));
            assert_eq!(floor.briefing.state, StateBriefing::FirstMessage);
            assert_eq!(floor.sessions.id, SessionId::Hunted);
            // What does not depend on a side channel stays the kind's.
            assert_eq!(floor.launch, caps.launch);
            assert_eq!(floor.transcript, caps.transcript);
            assert_eq!(floor.mcp, caps.mcp);
            assert_eq!(floor.sessions.resume, caps.sessions.resume);
        }
        // Claude Code without its mod: no role in the system prompt, plan
        // limits unknown, the context from its transcript as ever.
        let claude = live_caps(&AgentKind::Claude.info().caps, false);
        assert_eq!(claude.briefing.role, RoleBriefing::FirstMessage);
        assert_eq!(claude.usage.context, UsageSource::Transcript);
        assert_eq!(claude.usage.limits, UsageSource::None);
        // Codex on the floor: its role still on its line, usage and limits
        // from its rollouts.
        let codex = live_caps(&AgentKind::Codex.info().caps, false);
        assert_eq!(codex.briefing.role, RoleBriefing::DeveloperInstructions);
        assert_eq!(codex.usage.context, UsageSource::Transcript);
        assert_eq!(codex.usage.limits, UsageSource::Transcript);
    }

    /// The shapes of crates/hangar-core/claude-mod/teskad-bridge/hooks/
    /// register.js, nulls and extra keys included: a report that fails to
    /// read is a report lost, a hello among them.
    #[test]
    fn bridge_reports_read_what_the_mod_really_sends() {
        let settings =
            json!({"kind": "settings", "model": "claude-opus-5-5", "mode": null, "effort": "high"});
        assert_eq!(
            BridgeEvent::parse(&settings),
            Some(BridgeEvent::Settings {
                model: Some("claude-opus-5-5".into()),
                mode: None,
                effort: Some("high".into()),
            })
        );
        let hello = json!({"kind": "hello", "sessionId": "s1", "model": null, "version": "2.1"});
        assert_eq!(
            BridgeEvent::parse(&hello),
            Some(BridgeEvent::Hello {
                session_id: "s1".into(),
                model: String::new(),
                version: "2.1".into(),
            })
        );
        let usage = json!({"kind": "usage",
            "context": {"tokens": null, "window": 200000, "percent": 41.5},
            "rateLimits": [{"kind": "five_hour", "percentUsed": 12, "resetsAt": null}],
            "costUsd": null});
        let Some(BridgeEvent::Usage {
            context,
            rate_limits,
            cost_usd,
        }) = BridgeEvent::parse(&usage)
        else {
            panic!("a usage report");
        };
        assert_eq!(
            (context.tokens, context.window, context.percent),
            (None, 200_000, Some(41.5))
        );
        assert_eq!((rate_limits[0].percent_used, cost_usd), (12.0, None));
        // The mod names the pane that edited it, which the window does not use.
        assert_eq!(
            BridgeEvent::parse(
                &json!({"kind": "edit", "file": "a.ts", "name": "Ava", "overlap": "Bob", "overlapPane": "p9"})
            ),
            Some(BridgeEvent::Edit {
                file: "a.ts".into(),
                overlap: Some("Bob".into()),
                overlap_pane: Some("p9".into()),
            })
        );
        let ended = json!({"kind": "turn", "phase": "end", "reason": "aborted", "durationMs": 0, "answer": null});
        assert!(matches!(
            BridgeEvent::parse(&ended),
            Some(BridgeEvent::Turn(TurnPhase::End { ref answer, .. })) if answer.is_empty()
        ));
        assert_eq!(
            BridgeEvent::parse(&json!({"kind": "end"})),
            Some(BridgeEvent::End {
                reason: String::new()
            })
        );
        let tool = json!({"kind": "tool", "phase": "start", "toolUseId": "t1", "tool": "Bash", "input": {"command": "ls"}});
        assert!(matches!(
            BridgeEvent::parse(&tool),
            Some(BridgeEvent::Tool(ToolPhase::Start { .. }))
        ));
    }
}
