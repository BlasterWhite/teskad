//! The persisted model: `AppState` and what it holds, as `src/types.ts`
//! declared it and as `state.json` stores it (inventory §1.2 and §1.4).
//!
//! The file is shared with the old app while people may go back and forth,
//! so these types follow its shape exactly rather than what would be nicest
//! in Rust:
//!
//! - keys are camelCase, as the TS wrote them;
//! - `T | null` fields are `Option<T>` written as `null`, and `?` fields are
//!   `Option<T>` left out when absent (`kits` and `claudeAccount` are absent
//!   on purpose rather than empty);
//! - a field this version added is written only when it holds something
//!   other than its default, so a file that never used it is written back as
//!   it was, and an older Hangar reads it as before;
//! - every struct the TS spread (`{...ws, ...patch}`) keeps the keys it does
//!   not know in `extra`, so a file written by a newer version, or edited by
//!   hand, loses nothing when this one saves it;
//! - a missing field takes its default (`DEFAULT_SETTINGS` for the
//!   settings), and a malformed one is read leniently (`lenient.rs`), since a
//!   strict parse error would quarantine the person's workspaces.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub use hangar_core::agent::AgentKind;
use hangar_i18n::Locale;

/// The theme new settings name: hangar-theme's default (`hangar_theme::DEFAULT_ID`,
/// which the app's tests hold equal to this; the engine draws nothing and
/// does not depend on the themes).
pub const DEFAULT_THEME_ID: &str = "teskad-dark";

/// A workspace's id, as the TS made it (`crypto.randomUUID()` or older
/// formats): any string.
pub type WsId = String;
/// A pane's id. A respawn gives the pane a new one (inventory §1.3).
pub type PaneId = String;

/// Unknown keys of a struct, kept as they were read.
pub type Extra = Map<String, Value>;

/// The most panes a workspace holds (`MAX_PANES`).
pub const MAX_PANES: usize = 16;

/// Bounds of the interface size, in percent (`UI_SCALE_MIN`/`UI_SCALE_MAX`).
pub const UI_SCALE_MIN: f64 = 90.0;
pub const UI_SCALE_MAX: f64 = 130.0;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppState {
    pub workspaces: Vec<Workspace>,
    /// In sidebar order.
    pub folders: Vec<WorkspaceFolder>,
    pub active_workspace_id: Option<WsId>,
    pub settings: Settings,
    #[serde(flatten)]
    pub extra: Extra,
}

/// What a pane runs (`AgentId`): one of the agents Hangar runs, Claude Code
/// or Codex (its [`kind`](AgentId::kind), whose capabilities the features
/// read), or a plain shell.
///
/// The old app also offered `gemini` and `opencode`; a pane saved with one
/// of them opens as a shell (`RETIRED_AGENTS`, lenient.rs). Codex, which 2.0
/// dropped too, is run again (docs/native/agents.md): a pane 1.x saved as
/// `codex` reads as Codex.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentId {
    #[default]
    Shell,
    Claude,
    Codex,
}

/// The agents the old app could run and this one no longer does, by the id
/// `state.json` holds, with the name a pane's note gives them.
pub const RETIRED_AGENTS: [(&str, &str); 2] = [("gemini", "Gemini CLI"), ("opencode", "OpenCode")];

/// One row of `AGENTS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentInfo {
    pub id: AgentId,
    pub label: &'static str,
    /// Whether it can be relaunched on a previous session (`--resume`).
    pub resumable: bool,
}

/// `AGENTS`, in the order the pickers list them, and the agents a request
/// may name (`control::run::agent_arg`).
///
/// Codex is not among them yet: the pickers and the pane tools offer it once
/// they can start it and say why when they cannot (docs/native/agents.md
/// T16b, T22, T24b). Until then a pane is Codex only as the file says so.
pub const AGENTS: [AgentInfo; 2] = [
    AgentInfo {
        id: AgentId::Shell,
        label: "Shell",
        resumable: false,
    },
    AgentInfo {
        id: AgentId::Claude,
        label: "Claude Code",
        resumable: true,
    },
];

/// Codex's row, out of `AGENTS` until the pickers offer it. Not resumable
/// here yet: Hangar starts it again on its session once its launch does
/// (docs/native/agents.md T14a, T15).
const CODEX: AgentInfo = AgentInfo {
    id: AgentId::Codex,
    label: "Codex",
    resumable: false,
};

impl AgentId {
    pub fn info(self) -> &'static AgentInfo {
        if self == AgentId::Codex {
            return &CODEX;
        }
        AGENTS
            .iter()
            .find(|agent| agent.id == self)
            .unwrap_or(&AGENTS[0])
    }

    /// The id as `state.json` writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentId::Shell => "shell",
            AgentId::Claude => "claude",
            AgentId::Codex => "codex",
        }
    }

    /// The agent it runs, whose capabilities the features read
    /// (`hangar_core::agent`); none for a shell.
    pub fn kind(self) -> Option<AgentKind> {
        match self {
            AgentId::Shell => None,
            AgentId::Claude => Some(AgentKind::Claude),
            AgentId::Codex => Some(AgentKind::Codex),
        }
    }
}

impl From<AgentKind> for AgentId {
    fn from(kind: AgentKind) -> AgentId {
        match kind {
            AgentKind::Claude => AgentId::Claude,
            AgentKind::Codex => AgentId::Codex,
        }
    }
}

/// `claude` or `codex`, whichever `kind` is: what reads a field kept once
/// per kind (an account, a launch command), so that nothing else matches on
/// the kind.
fn per_kind<T>(kind: AgentKind, claude: T, codex: T) -> T {
    match kind {
        AgentKind::Claude => claude,
        AgentKind::Codex => codex,
    }
}

/// A pane's part in its workspace's orchestration (`PaneRole`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneRole {
    Orchestrator,
    Worker,
    Manager,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pane {
    pub id: PaneId,
    /// Unique across all workspaces, stable across splits, moves, restarts.
    pub name: String,
    pub agent: AgentId,
    /// The agent session to resume on the next launch.
    pub session_id: Option<String>,
    /// Overrides the workspace root.
    pub cwd: Option<String>,
    /// Overrides the workspace shell.
    pub shell_id: Option<String>,
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<PaneRole>,
    /// A worker's board task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// `--model` of its agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// First port a worker may bind (`TESKAD_PORT_BASE`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port_base: Option<u32>,
    /// A worker in the workspace's own folder (no worktree).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_folder: Option<bool>,
    /// The node this pane runs on when it is not its workspace's: a worker
    /// a local workspace sent to a node (nodes/away.rs). Its shell, its
    /// transcripts and its queue are there. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// The Claude account over its workspace's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_account: Option<String>,
    /// The Codex account over its workspace's: one field per kind, as for
    /// the workspace (docs/native/agents.md §6.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_account: Option<String>,
    /// A Claude Code pane opened to log an account in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<bool>,
    /// The agent the file said this pane ran, when it is one this version
    /// no longer runs (`RETIRED_AGENTS`): the reader made the pane a shell
    /// and set this, so its bar says why once, as it first starts
    /// (panes/controller.rs). Never written.
    #[serde(skip)]
    pub retired_agent: Option<&'static str>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Pane {
    /// Its own choice of account for `kind`'s agent, as written.
    pub fn account(&self, kind: AgentKind) -> Option<&str> {
        per_kind(kind, &self.claude_account, &self.codex_account).as_deref()
    }

    pub fn account_mut(&mut self, kind: AgentKind) -> &mut Option<String> {
        per_kind(kind, &mut self.claude_account, &mut self.codex_account)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SavedCommand {
    pub id: String,
    /// Empty falls back to the command itself.
    pub label: String,
    pub command: String,
    pub auto_run: bool,
    pub broadcast: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Workspace {
    pub id: WsId,
    pub name: String,
    pub cwd: String,
    /// The other folders of a `.code-workspace`.
    pub extra_roots: Vec<String>,
    pub panes: Vec<Pane>,
    /// Overrides the global theme.
    pub theme_id: Option<String>,
    /// Overrides the global default shell.
    pub shell_id: Option<String>,
    pub tree: Option<SplitNode>,
    /// The panes of the grid that are not terminals: what the previews show
    /// (a file, a 3D model, a kit's tab), each a leaf of `tree` beside the
    /// terminals. Written only while there is one.
    ///
    /// Hangar 1.x knows only `panes`, and everything that treats them as
    /// terminals (their shells, the MCP pane tools, broadcast, hibernation)
    /// keeps reading `panes` alone. 1.x carries this field over as it was
    /// (it spreads the workspace), and its `normalizeTree` drops the leaves
    /// it does not know, laying its terminals out again as an even grid.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub view_panes: Vec<ViewPane>,
    /// The panes of the grid pinned to one of its edges, terminals or views,
    /// the first pinned the outermost: each keeps its edge and its size
    /// there while the other panes open, close, move or are laid out again,
    /// which lay themselves out in the room the pins leave
    /// (`layout::pinned_tree`). `tree` holds them where they are, so a
    /// Hangar that does not know pins draws them in their place. Written
    /// only while there is one; 1.x and the versions before keep the field
    /// as it was (they spread the workspace) and lay the grid out as if
    /// there were none. The mirror's own for a hosted workspace, as its
    /// tree is: its node is told nothing of them (`hosted.rs`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<PanePin>,
    pub saved_commands: Vec<SavedCommand>,
    /// The navigator folder it is filed under.
    pub folder_id: Option<String>,
    /// Shown in a window of its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detached: Option<bool>,
    /// Where that window last stood, in logical pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_bounds: Option<WindowBounds>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<WorkspaceMcp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kits: Option<Vec<WorkspaceKit>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orchestration: Option<OrchestrationSettings>,
    /// The project manager's workspace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hq: Option<bool>,
    /// Ports Hangar Mobile may open for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mobile_ports: Option<Vec<u16>>,
    /// The Claude account it chose. Absent: none, it runs on its folder's
    /// (`WorkspaceFolder::claude_account`, read, never copied), else the
    /// default; `"@default"` (`accounts::DEFAULT_CHOICE`): the default, over
    /// its folder's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_account: Option<String>,
    /// The Codex account it chose, read as `claude_account` is: one field per
    /// kind, so that its orchestrator and its workers may run on accounts of
    /// either (docs/native/agents.md §6.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_hooks: Option<WorktreeHooks>,
    /// Shown as the chat view rather than its grid, while the `chat-view`
    /// experiment is on (`chat_view::shown`). Written only while it is: off
    /// is the absence, so a file that never used it stays as it was, and an
    /// experiment turned off leaves the grid without touching the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_view: Option<bool>,
    /// The node it lives on (experiment `nodes`, docs/native/nodes.md §5):
    /// the id of one of `settings.nodes`. Its `cwd` is then a folder of that
    /// machine, its terminals run there and its board is the node's. Absent
    /// for a workspace of this machine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// Its node runs it itself: this Hangar mirrors it. The node holds its
    /// state and starts its panes, with this Hangar there or not; what is
    /// kept here is a copy, brought up to date from the node
    /// (`state::hosted`), with this Hangar's own presentation of it. Absent
    /// for a workspace this Hangar runs, on this machine or on a node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hosted: Option<bool>,
    /// On its way to its node (`hosted` is set too), which has not said
    /// whether it took it: how it came to be handed over, kept until the
    /// node's own list settles it. Nothing of it is followed on the node
    /// meanwhile, and nothing of it is run from here. Written with the
    /// rest, so a Hangar closed before the answer settles it at its next
    /// start. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handover: Option<Handover>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// How a workspace came to be handed to its node (`Workspace::handover`),
/// which says what becomes of it if the node did not take it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Handover {
    /// Just made here, for the node to run from the start: taken back out.
    Created,
    /// Driven from this computer until then: driven from here again.
    Moved,
    /// A mirror its node no longer had, sent back to it: a mirror the node
    /// does not have again.
    Restored,
}

/// A pane of the grid that shows a preview rather than a terminal. Its id is
/// the id of the preview's entry it draws (`preview::store::Entry`): what it
/// shows, its title and its state are the preview store's, which keeps the
/// kit tabs across a restart in `kit-tabs.json`. A view whose entry does not
/// come back (a file, which the previews never kept) is dropped once its
/// workspace is opened (`preview::panes`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ViewPane {
    pub id: PaneId,
    #[serde(flatten)]
    pub extra: Extra,
}

impl ViewPane {
    pub fn new(id: impl Into<PaneId>) -> ViewPane {
        ViewPane {
            id: id.into(),
            extra: Extra::new(),
        }
    }
}

/// An edge of the grid a pinned pane keeps to (`PanePin::edge`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PinEdge {
    Left,
    Right,
    Top,
    Bottom,
}

impl PinEdge {
    /// Every edge, in the order a tie between two is settled: the sides
    /// first, which keep a terminal's columns, the right one first, where
    /// the agents' layout leaves room beside the lead.
    pub const ALL: [PinEdge; 4] = [PinEdge::Right, PinEdge::Left, PinEdge::Bottom, PinEdge::Top];

    /// The split that carves a pane out on this edge: side by side for the
    /// left and the right, stacked for the top and the bottom.
    pub fn dir(self) -> SplitDir {
        match self {
            PinEdge::Left | PinEdge::Right => SplitDir::Row,
            PinEdge::Top | PinEdge::Bottom => SplitDir::Col,
        }
    }

    /// Whether the pinned pane is its split's first side (`a`): on the left
    /// or at the top.
    pub fn leads(self) -> bool {
        matches!(self, PinEdge::Left | PinEdge::Top)
    }

    /// The name `state.json` and the pane tools write it under.
    pub fn as_str(self) -> &'static str {
        match self {
            PinEdge::Left => "left",
            PinEdge::Right => "right",
            PinEdge::Top => "top",
            PinEdge::Bottom => "bottom",
        }
    }
}

/// A pane pinned to an edge of its workspace's grid (`Workspace::pins`).
///
/// Its size is along the edge's axis (a width on the left or the right, a
/// height at the top or the bottom), kept two ways: in logical pixels when
/// the grid's size was known as it was set, which is what the grid draws
/// (a terminal keeps its columns as the window grows), and as its share of
/// the room it is carved from, which lays it out where no size is known
/// (the agents' layout, a Hangar with no window) and stands in for the
/// pixels when there are none. Both are kept within bounds where they are
/// read (`layout::sized_tree`), never in the file, so a window that grows
/// back gives a pane back the size it was pinned at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanePin {
    /// The terminal's or the view's id; a respawn carries the pin over.
    pub id: PaneId,
    pub edge: PinEdge,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub px: Option<f64>,
    /// Its share, 0 to 1, of the room the pins before it leave, along its
    /// axis: of the whole grid for the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fraction: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl PanePin {
    pub fn new(id: impl Into<PaneId>, edge: PinEdge) -> PanePin {
        PanePin {
            id: id.into(),
            edge,
            px: None,
            fraction: None,
            extra: Extra::new(),
        }
    }

    /// Its size in pixels, when it has one that reads as a size.
    pub fn pixels(&self) -> Option<f64> {
        self.px.filter(|px| px.is_finite() && *px > 0.0)
    }

    /// Its share of its room, when it has one that reads as a share (above
    /// 0, below 1); not yet within `layout`'s bounds.
    pub fn share(&self) -> Option<f64> {
        self.fraction
            .filter(|share| share.is_finite() && *share > 0.0 && *share < 1.0)
    }
}

impl Workspace {
    /// Every pane of the grid, terminals first, then the views, in the order
    /// they were made: what the tree is normalized against
    /// (`layout::grid_tree`).
    pub fn leaf_ids(&self) -> Vec<PaneId> {
        self.panes
            .iter()
            .map(|pane| pane.id.clone())
            .chain(self.view_panes.iter().map(|view| view.id.clone()))
            .collect()
    }

    /// How many panes the grid holds, terminals and views: what `MAX_PANES`
    /// bounds.
    pub fn pane_count(&self) -> usize {
        self.panes.len() + self.view_panes.len()
    }

    pub fn has_view(&self, id: &str) -> bool {
        self.view_panes.iter().any(|view| view.id == id)
    }

    /// Whether its node runs it itself, this Hangar mirroring it.
    pub fn is_hosted(&self) -> bool {
        self.hosted == Some(true)
    }

    /// Its own choice of account for `kind`'s agent, as written.
    pub fn account(&self, kind: AgentKind) -> Option<&str> {
        per_kind(kind, &self.claude_account, &self.codex_account).as_deref()
    }

    pub fn account_mut(&mut self, kind: AgentKind) -> &mut Option<String> {
        per_kind(kind, &mut self.claude_account, &mut self.codex_account)
    }

    /// A terminal or a view of this workspace's grid.
    pub fn has_leaf(&self, id: &str) -> bool {
        self.panes.iter().any(|pane| pane.id == id) || self.has_view(id)
    }

    /// The pin of pane `id` of the grid, if it is pinned: its first, should
    /// the file hold two.
    pub fn pin_of(&self, id: &str) -> Option<&PanePin> {
        self.pins
            .iter()
            .find(|pin| pin.id == id)
            .filter(|_| self.has_leaf(id))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorktreeHooks {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_create: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_remove: Option<String>,
}

/// How a workspace's orchestrator works. Missing fields read as
/// `ORCHESTRATION_DEFAULTS` (`lib/orchestration.ts:15`).
///
/// The old app also kept `maxWorkers` and `maxAttempts` here, the most
/// workers at once and the most one task gets. This version sets no such
/// limit, the orchestrator deciding both (owner's decision, 2026-11-04): a
/// file that holds them reads as if it did not, and they are left out of the
/// file at the next save. 1.x then reads its own defaults for them, 3 and 2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OrchestrationSettings {
    pub auto_merge: bool,
    pub check_command: Option<String>,
    pub orchestrator_model: String,
    pub worker_model: String,
    pub reviewer_model: String,
    pub branch: Option<String>,
    /// `auto`, `acceptEdits` or `bypassPermissions`; null: the person's own.
    pub permission_mode: Option<String>,
    /// The workspace's workers build into one cache folder of the
    /// workspace's own (`hangar_core::build_cache`); the delivery checks have
    /// a folder of theirs whatever this says. Off by default: worktrees
    /// building different branches in one folder can mix their builds.
    /// Missing reads false, and false is not written: a file that never turned
    /// it on stays as it was (what 1.x and the round-trip tests expect). Hangar
    /// 1.x does not know the key and, like any other it does not read, leaves
    /// it alone.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub shared_build_cache: bool,
    /// The most waiting deliveries one check is shared by (the merge queue,
    /// docs/native/DELIVERIES.md); none: `hangar_core::delivery::BATCH_SIZE`.
    /// 1 checks each delivery on its own, as before 2.3. Not written while
    /// unset, so a file that never set it stays as it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<u32>,
    /// The agent its orchestrator runs (D20); none: the settings' default,
    /// else the one installed, Claude Code when both are
    /// (`Settings::agent_for`). An agent this version does not know is set
    /// aside and written back (lenient.rs). Like every field below, written
    /// only when set: a file that never set it stays as it was, and an older
    /// Hangar reads it as before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orchestrator_agent: Option<AgentKind>,
    /// The agent its workers run; none: the settings' default, else the
    /// orchestrator's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_agent: Option<AgentKind>,
    /// The agent that reviews its deliveries (D11); none: the settings'
    /// default, else the orchestrator's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer_agent: Option<AgentKind>,
    /// Codex's models, beside Claude Code's: `orchestratorModel`,
    /// `workerModel` and `reviewerModel` stay Claude Code's, so that files of
    /// 1.x and 2.x read as before (docs/native/agents.md §6.6).
    #[serde(skip_serializing_if = "CodexModels::is_empty")]
    pub codex_models: CodexModels,
}

/// What a workspace's orchestration gives an agent to do, each with an agent
/// and a model of its own (`OrchestrationSettings::agent`, `::model`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrchestrationRole {
    Orchestrator,
    Worker,
    Reviewer,
}

/// The models a workspace's Codex agents start with (`codexModels`), each
/// absent meaning Codex's own default. Keys of a newer version are kept.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CodexModels {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orchestrator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl CodexModels {
    /// Nothing chosen: not written.
    pub fn is_empty(&self) -> bool {
        self.orchestrator.is_none()
            && self.worker.is_none()
            && self.reviewer.is_none()
            && self.extra.is_empty()
    }
}

impl OrchestrationSettings {
    /// The agent this workspace chose for `role`, if it chose one.
    pub fn agent(&self, role: OrchestrationRole) -> Option<AgentKind> {
        match role {
            OrchestrationRole::Orchestrator => self.orchestrator_agent,
            OrchestrationRole::Worker => self.worker_agent,
            OrchestrationRole::Reviewer => self.reviewer_agent,
        }
    }

    /// The model `role` starts `kind`'s agent with, trimmed: Claude Code's
    /// from `orchestratorModel`…, Codex's from `codexModels`. None: the
    /// agent's own default. Read on what `roles::orchestration_of` gives,
    /// which puts a blank Claude Code name back to Hangar's default.
    pub fn model(&self, role: OrchestrationRole, kind: AgentKind) -> Option<&str> {
        let claude = match role {
            OrchestrationRole::Orchestrator => &self.orchestrator_model,
            OrchestrationRole::Worker => &self.worker_model,
            OrchestrationRole::Reviewer => &self.reviewer_model,
        };
        let codex = match role {
            OrchestrationRole::Orchestrator => &self.codex_models.orchestrator,
            OrchestrationRole::Worker => &self.codex_models.worker,
            OrchestrationRole::Reviewer => &self.codex_models.reviewer,
        };
        per_kind(kind, Some(claude.as_str()), codex.as_deref())
            .map(str::trim)
            .filter(|model| !model.is_empty())
    }
}

impl Default for OrchestrationSettings {
    fn default() -> Self {
        OrchestrationSettings {
            auto_merge: true,
            check_command: None,
            orchestrator_model: "opus".into(),
            worker_model: "sonnet".into(),
            reviewer_model: "sonnet".into(),
            branch: None,
            permission_mode: None,
            shared_build_cache: false,
            batch_size: None,
            orchestrator_agent: None,
            worker_agent: None,
            reviewer_agent: None,
            codex_models: CodexModels::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpMode {
    #[default]
    Inherit,
    Custom,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceMcp {
    pub mode: McpMode,
    pub enabled: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceKit {
    pub id: String,
    pub granted: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A navigator group of workspaces, one level deep; it owns nothing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceFolder {
    pub id: String,
    pub name: String,
    pub collapsed: bool,
    /// The Claude account of the workspaces filed in this folder that chose
    /// none of their own, read at each launch (`folders::account_of`): one
    /// moved in follows it, one moved out leaves it. Not for a workspace its
    /// node runs. Written only when set, so a file from before stays as it
    /// was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_account: Option<String>,
    /// The Codex account of the same workspaces, read the same way. Written
    /// only when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_account: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl WorkspaceFolder {
    /// The account it names for `kind`'s agent, as written.
    pub fn account(&self, kind: AgentKind) -> Option<&str> {
        per_kind(kind, &self.claude_account, &self.codex_account).as_deref()
    }

    pub fn account_mut(&mut self, kind: AgentKind) -> &mut Option<String> {
        per_kind(kind, &mut self.claude_account, &mut self.codex_account)
    }
}

/// `row` places its children side by side, `col` stacks them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDir {
    #[default]
    Row,
    Col,
}

/// The pane arrangement: each split owns its own ratio. Leaves name a pane
/// id, so closing or moving a pane never renumbers (`lib/layout.ts`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SplitNode {
    Pane {
        id: PaneId,
    },
    Split {
        dir: SplitDir,
        ratio: f64,
        a: Box<SplitNode>,
        b: Box<SplitNode>,
    },
}

/// Where a pane is dropped on another (`Zone`): `Center` swaps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropZone {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    Block,
    #[default]
    Bar,
    Underline,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VoiceEngine {
    #[default]
    Local,
    Groq,
}

/// What a pane types to start its agent where it differs from the plain
/// `claude` or `codex` (`agentCommands`, by agent id). The old app kept one
/// for each of its agents; this version reads Claude Code's and Codex's, and
/// the others' are left out of the file at the next save. A value that is
/// not text reads as not customised.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentCommands {
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "text_or_none"
    )]
    pub claude: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "text_or_none"
    )]
    pub codex: Option<String>,
}

impl AgentCommands {
    /// The command set for `kind`'s agent, as written.
    pub fn of(&self, kind: AgentKind) -> Option<&str> {
        per_kind(kind, &self.claude, &self.codex).as_deref()
    }
}

/// A window opacity that leaves the window as it always was.
fn is_opaque(opacity: &f64) -> bool {
    *opacity == 1.0
}

fn is_on(on: &bool) -> bool {
    *on
}

fn text_or_none<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(Value::deserialize(deserializer)?
        .as_str()
        .map(str::to_owned))
}

/// The settings. `Default` is `DEFAULT_SETTINGS`, and the file's settings
/// are read over it (`{...DEFAULT_SETTINGS, ...loaded.settings}`). The UI
/// controls of each are in inventory §2.3/2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub theme_id: String,
    /// `null` follows the system's language.
    pub locale: Option<Locale>,
    /// `null`: the first shell detected.
    pub shell_id: Option<String>,
    pub font_family: String,
    pub font_size: f64,
    pub line_height: f64,
    pub letter_spacing: f64,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    pub scrollback: u64,
    pub padding: f64,
    /// What a pane types to start its agent, where it differs.
    pub agent_commands: AgentCommands,
    pub auto_resume: bool,
    pub hibernate_idle: bool,
    /// Read as `max(1, value)` minutes.
    pub hibernate_after_min: f64,
    pub claude_bridge: bool,
    /// « Attach Teskad to Codex »: Hangar starts each Codex pane's
    /// app-server and follows it as a second client (D5, on by default, the
    /// person's decision); off, the pane runs Codex's plain TUI, read from
    /// its output and its files (D6). Written only when off, so a file from
    /// before stays as it was.
    #[serde(skip_serializing_if = "is_on")]
    pub codex_attach: bool,
    pub launch_delay_ms: u64,
    pub notify_on_idle: bool,
    pub notify_idle_ms: u64,
    pub notify_only_when_away: bool,
    /// Re-bound shortcuts by command id, overrides only; `[]` disables one.
    /// Ids this version does not know are kept (`prune` in shortcuts.ts).
    pub keybindings: Map<String, Value>,
    pub saved_commands: Vec<SavedCommand>,
    pub voice_enabled: bool,
    pub voice_engine: VoiceEngine,
    pub voice_model: String,
    pub voice_cloud_model: String,
    pub voice_language: String,
    /// Legacy plaintext Groq key; `load_state` moves it to the keychain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_api_key: Option<String>,
    pub voice_cleanup: bool,
    pub voice_cleanup_model: String,
    pub voice_cleanup_hint: String,
    pub voice_submit: bool,
    pub discord_presence: bool,
    pub discord_show_workspace: bool,
    pub discord_show_agents: bool,
    pub discord_app_id: String,
    pub board_dock_open: bool,
    pub board_dock_width: f64,
    /// The width of 1.x's preview panel, read and written back as it was for
    /// 1.x: this version has no such panel, every preview being a pane of the
    /// grid.
    pub model_panel_width: f64,
    pub navigator_open: bool,
    pub inspector_open: bool,
    pub inspector_width: f64,
    pub inspector_collapsed: Vec<String>,
    /// Percent, clamped to 90–130 where it is read.
    pub ui_scale: f64,
    /// The system draws the windows' title bars, not Hangar (Linux only:
    /// shell/title_bar.rs). Written only when on, so a file the app wrote
    /// before it had the setting is written back the same.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub system_title_bar: bool,
    /// How opaque the windows' backgrounds are, from 0.5 to 1 (held there
    /// where it is read), whatever the theme: below 1 the desktop shows
    /// through behind the page, the bars and the terminals (Linux only).
    /// Written only when it is not 1, so a file from before stays as it was.
    #[serde(skip_serializing_if = "is_opaque")]
    pub window_opacity: f64,
    pub agent_pane_control: bool,
    /// `"main"` by default; `null` is kept as the person wrote it.
    pub claude_account: Option<String>,
    /// The default Codex account; none is main, the person's own Codex home
    /// (docs/native/agents.md §6.1). Written only when set, so a file from
    /// before stays as it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_account: Option<String>,
    /// The agent the orchestrators of new workspaces run; none is
    /// « Automatic »: the one installed, Claude Code when both are (D20).
    /// Like the two below, written only when set, and an agent this version
    /// does not know is set aside and written back (lenient.rs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_orchestrator_agent: Option<AgentKind>,
    /// The agent workers run where their workspace chose none; none is
    /// « Same as the orchestrator ».
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_worker_agent: Option<AgentKind>,
    /// The agent that reviews deliveries where their workspace chose none;
    /// none is « Same as the orchestrator » (D11).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_reviewer_agent: Option<AgentKind>,
    /// Ask « restart the running agents now? » when an account is chosen,
    /// as before. Off, each agent moves to the new account by itself, on its
    /// conversation, once Hangar knows it has nothing to lose: its turn over
    /// as its mod reports it, nothing typed, no command of its own running
    /// (`accounts::switch::follow`). Written only when on, so a file from
    /// before stays as it was.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub account_switch_ask: bool,
    pub experiments: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kits_everywhere: Option<Vec<WorkspaceKit>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projects_folder: Option<String>,
    /// The machines the `nodes` experiment connects to (`nodes`). Written
    /// only when there is one, so a file from before stays as it was.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeConfig>,
    /// Account badges wear the tone and icon chosen for each account
    /// (`accountStyles`); off, they are the plain labels they always were.
    /// Written only when on, so a file from before stays as it was.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub account_badges: bool,
    /// How each Claude account is marked where its badge shows, by account
    /// id (`accountStyles`): `{"perso": {"tone": "success", "icon":
    /// "gamepad"}}`. Kept as written, as `keybindings` is, and read through
    /// `account_style`: a tone or a key a newer version added, or an entry
    /// this one cannot read, stays in the file for the version that can.
    /// Written only when an account has a style, so a file from before stays
    /// as it was.
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub account_styles: Map<String, Value>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// The colours an account's badge can take: the theme's own tones, so a badge
/// reads on every theme, light or dark, as the other badges of Hangar do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccountTone {
    Accent,
    Success,
    Warning,
    Danger,
}

impl AccountTone {
    /// In the order the settings offer them.
    pub const ALL: [AccountTone; 4] = [
        AccountTone::Accent,
        AccountTone::Success,
        AccountTone::Warning,
        AccountTone::Danger,
    ];

    /// The name `accountStyles` stores it under.
    pub const fn name(self) -> &'static str {
        match self {
            AccountTone::Accent => "accent",
            AccountTone::Success => "success",
            AccountTone::Warning => "warning",
            AccountTone::Danger => "danger",
        }
    }

    /// The tone stored under `name`; one this version does not know is none.
    pub fn from_name(name: &str) -> Option<AccountTone> {
        AccountTone::ALL
            .into_iter()
            .find(|tone| tone.name() == name)
    }
}

/// What this version reads of an account's entry in `accountStyles`: a tone,
/// an icon (by its name in Hangar's icon set), or neither.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountStyle {
    pub tone: Option<AccountTone>,
    pub icon: Option<String>,
}

/// A machine reached over SSH that runs `teskad-node` (docs/native/nodes.md
/// §4). Only how to reach it is stored: its state is the `nodes` package's
/// and lives as long as the app.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NodeConfig {
    pub id: String,
    /// What the page calls it; the SSH target when empty.
    pub label: String,
    /// What `ssh` is given: `user@host`, or a `Host` of the SSH config.
    pub ssh_target: String,
    /// An SSH config file passed as `-F`; the person's own when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_config: Option<String>,
    /// What sort of machine it is, which picks the icon it goes by.
    /// Written only when it is not a server, so a file from before stays
    /// as it was.
    #[serde(skip_serializing_if = "MachineKind::is_server")]
    pub kind: MachineKind,
    /// `Some(false)`: switched off by the person. It is not connected, and
    /// its workspaces are out of the lists until it is switched on again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// The run of its daemon (`boot` of `GET /node/own/state`) whose list
    /// of workspaces the mirrors here last agreed with. A list that comes
    /// under another run and lacks one of them is a node that started
    /// again without it (its state lost, another home), not a workspace
    /// deleted there: that mirror is kept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl NodeConfig {
    /// Not switched off: the app keeps it connected.
    pub fn is_on(&self) -> bool {
        self.enabled != Some(false)
    }
}

/// The sort of machine a node is. Only how it is drawn depends on it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MachineKind {
    #[default]
    Server,
    Cloud,
    Laptop,
    Desktop,
    MiniPc,
    Tower,
}

impl MachineKind {
    /// Every kind, in the order a picker offers them.
    pub const ALL: [MachineKind; 6] = [
        MachineKind::Server,
        MachineKind::Cloud,
        MachineKind::Laptop,
        MachineKind::Desktop,
        MachineKind::MiniPc,
        MachineKind::Tower,
    ];

    fn is_server(&self) -> bool {
        *self == MachineKind::Server
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme_id: DEFAULT_THEME_ID.into(),
            locale: None,
            shell_id: None,
            font_family: r#""JetBrains Mono", "Cascadia Code", Consolas, "Courier New", monospace"#
                .into(),
            font_size: 13.0,
            line_height: 1.3,
            letter_spacing: 0.0,
            cursor_style: CursorStyle::Bar,
            cursor_blink: true,
            scrollback: 10_000,
            padding: 10.0,
            agent_commands: AgentCommands::default(),
            auto_resume: true,
            hibernate_idle: true,
            hibernate_after_min: 10.0,
            claude_bridge: true,
            codex_attach: true,
            launch_delay_ms: 700,
            notify_on_idle: true,
            notify_idle_ms: 3000,
            notify_only_when_away: true,
            keybindings: Map::new(),
            saved_commands: Vec::new(),
            voice_enabled: false,
            voice_engine: VoiceEngine::Local,
            voice_model: "whisper-small-q5_1".into(),
            voice_cloud_model: "whisper-large-v3-turbo".into(),
            voice_language: String::new(),
            voice_api_key: None,
            voice_cleanup: false,
            voice_cleanup_model: String::new(),
            voice_cleanup_hint: String::new(),
            voice_submit: false,
            discord_presence: false,
            discord_show_workspace: true,
            discord_show_agents: true,
            discord_app_id: String::new(),
            board_dock_open: false,
            board_dock_width: 420.0,
            model_panel_width: 440.0,
            navigator_open: true,
            inspector_open: true,
            inspector_width: 296.0,
            inspector_collapsed: Vec::new(),
            ui_scale: 100.0,
            system_title_bar: false,
            window_opacity: 1.0,
            account_switch_ask: false,
            agent_pane_control: true,
            claude_account: Some("main".into()),
            codex_account: None,
            default_orchestrator_agent: None,
            default_worker_agent: None,
            default_reviewer_agent: None,
            experiments: Vec::new(),
            kits_everywhere: None,
            projects_folder: None,
            nodes: Vec::new(),
            account_badges: false,
            account_styles: Map::new(),
            extra: Map::new(),
        }
    }
}

impl Settings {
    /// `uiScale` as the factor iced scales the window by, clamped as the TS
    /// clamped it.
    pub fn scale_factor(&self) -> f32 {
        let percent = if self.ui_scale.is_finite() {
            self.ui_scale.clamp(UI_SCALE_MIN, UI_SCALE_MAX)
        } else {
            100.0
        };
        (percent / 100.0) as f32
    }

    /// The default choice of account for `kind`'s agent, as written.
    pub fn account(&self, kind: AgentKind) -> Option<&str> {
        per_kind(kind, &self.claude_account, &self.codex_account).as_deref()
    }

    pub fn account_mut(&mut self, kind: AgentKind) -> &mut Option<String> {
        per_kind(kind, &mut self.claude_account, &mut self.codex_account)
    }

    /// The agent the settings give `role` where a workspace chose none.
    pub fn default_agent(&self, role: OrchestrationRole) -> Option<AgentKind> {
        match role {
            OrchestrationRole::Orchestrator => self.default_orchestrator_agent,
            OrchestrationRole::Worker => self.default_worker_agent,
            OrchestrationRole::Reviewer => self.default_reviewer_agent,
        }
    }

    /// The agent `role` runs for a workspace whose orchestration is
    /// `orchestration` (D11, D20): the workspace's choice, else the
    /// settings' default. With neither, an orchestrator runs
    /// [`automatic_agent`], and a worker or a reviewer runs what the
    /// orchestrator runs — `orchestrator`, the agent of the pane pinned as
    /// such when there is one, else the orchestrator's own rule — or, when
    /// that one is not installed, one that is.
    ///
    /// A choice is kept even when its agent is not installed: what then is
    /// the caller's to say. `installed` is `hangar_core::agent::detect::installed`
    /// outside the tests.
    pub fn agent_for(
        &self,
        role: OrchestrationRole,
        orchestration: Option<&OrchestrationSettings>,
        orchestrator: Option<AgentKind>,
        installed: impl Fn(AgentKind) -> bool,
    ) -> AgentKind {
        let chosen = |role: OrchestrationRole| {
            orchestration
                .and_then(|orchestration| orchestration.agent(role))
                .or(self.default_agent(role))
        };
        if let Some(chosen) = chosen(role) {
            return chosen;
        }
        if role == OrchestrationRole::Orchestrator {
            return automatic_agent(installed);
        }
        let same = orchestrator
            .or_else(|| chosen(OrchestrationRole::Orchestrator))
            .unwrap_or_else(|| automatic_agent(&installed));
        if installed(same) {
            return same;
        }
        AgentKind::ALL
            .into_iter()
            .find(|kind| installed(*kind))
            .unwrap_or(same)
    }

    /// The style chosen for account `id`. A tone this version does not know,
    /// a value that is not text, or an entry that is not an object reads as
    /// nothing chosen: the badge is plain, and the file keeps what it holds.
    pub fn account_style(&self, id: &str) -> AccountStyle {
        let entry = self.account_styles.get(id);
        let text = |key: &str| entry.and_then(|entry| entry.get(key)?.as_str());
        AccountStyle {
            tone: text("tone").and_then(AccountTone::from_name),
            icon: text("icon").map(str::to_owned),
        }
    }

    /// Sets the tone of account `id`'s badge, or with `None` takes it off.
    pub fn set_account_tone(&mut self, id: &str, tone: Option<AccountTone>) {
        self.restyle_account(id, "tone", tone.map(|tone| tone.name().into()));
    }

    /// Sets the icon of account `id`'s badge by its name, or with `None`
    /// takes it off.
    pub fn set_account_icon(&mut self, id: &str, icon: Option<&str>) {
        self.restyle_account(id, "icon", icon.map(Value::from));
    }

    /// Writes `key` of account `id`'s entry, keeping the keys this version
    /// does not know. An entry that is not an object is replaced (the person
    /// chose anew), and one left empty is taken out of the file.
    fn restyle_account(&mut self, id: &str, key: &str, value: Option<Value>) {
        let entry = self
            .account_styles
            .entry(id)
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        let Value::Object(fields) = entry else {
            return;
        };
        match value {
            Some(value) => {
                fields.insert(key.to_string(), value);
            }
            None => {
                fields.remove(key);
            }
        }
        if fields.is_empty() {
            self.account_styles.remove(id);
        }
    }
}

/// The agent of a part nobody chose (D20): Claude Code when it is
/// installed, as it has always been, Codex when it alone is. With neither,
/// Claude Code, which the setup offers first.
pub fn automatic_agent(installed: impl Fn(AgentKind) -> bool) -> AgentKind {
    if !installed(AgentKind::Claude) && installed(AgentKind::Codex) {
        AgentKind::Codex
    } else {
        AgentKind::Claude
    }
}

/// A shell discovered on this machine.
pub type ShellInfo = hangar_core::shells::ShellInfo;

#[cfg(test)]
mod tests {
    use super::*;
    use AgentKind::{Claude, Codex};
    use OrchestrationRole::{Orchestrator, Reviewer, Worker};

    /// Which agents a machine has, for `installed`.
    fn has(kinds: &[AgentKind]) -> impl Fn(AgentKind) -> bool + '_ {
        move |kind| kinds.contains(&kind)
    }

    #[test]
    fn a_pane_s_agent_has_a_kind_and_a_shell_has_none() {
        assert_eq!(AgentId::Shell.kind(), None);
        assert_eq!(AgentId::Claude.kind(), Some(Claude));
        assert_eq!(AgentId::Codex.kind(), Some(Codex));
        for kind in AgentKind::ALL {
            let id = AgentId::from(kind);
            assert_eq!(id.kind(), Some(kind));
            assert_eq!(id.as_str(), kind.id(), "one id for both");
            let read: AgentId = serde_json::from_value(Value::from(kind.id())).unwrap();
            assert_eq!(read, id);
        }
        assert_eq!(AgentId::Codex.info().label, "Codex");
        assert_eq!(AgentId::Codex.info().id, AgentId::Codex);
        assert!(
            AGENTS.iter().all(|agent| agent.id != AgentId::Codex),
            "not offered by the pickers yet"
        );
        assert!(RETIRED_AGENTS.iter().all(|(id, _)| *id != "codex"));
    }

    /// D20: nobody chose, so Claude Code when it is there, Codex when it
    /// alone is.
    #[test]
    fn the_automatic_agent_is_claude_code_unless_codex_alone_is_installed() {
        assert_eq!(automatic_agent(has(&[Claude, Codex])), Claude);
        assert_eq!(automatic_agent(has(&[Claude])), Claude);
        assert_eq!(automatic_agent(has(&[Codex])), Codex);
        assert_eq!(automatic_agent(has(&[])), Claude);
    }

    /// The workspace's choice, else the settings' default, else the rule.
    #[test]
    fn an_orchestrator_runs_the_workspace_s_choice_then_the_default_then_the_rule() {
        let mut settings = Settings::default();
        let both = has(&[Claude, Codex]);
        assert_eq!(settings.agent_for(Orchestrator, None, None, &both), Claude);
        assert_eq!(
            settings.agent_for(Orchestrator, None, None, has(&[Codex])),
            Codex
        );
        settings.default_orchestrator_agent = Some(Codex);
        assert_eq!(settings.agent_for(Orchestrator, None, None, &both), Codex);
        let chose_claude = OrchestrationSettings {
            orchestrator_agent: Some(Claude),
            ..OrchestrationSettings::default()
        };
        assert_eq!(
            settings.agent_for(Orchestrator, Some(&chose_claude), None, &both),
            Claude
        );
        // A choice stands, installed or not: the caller says why it cannot.
        assert_eq!(
            settings.agent_for(Orchestrator, Some(&chose_claude), None, has(&[Codex])),
            Claude
        );
    }

    /// D11 and « Same as the orchestrator »: a worker and a reviewer follow
    /// the orchestrator, the one running when there is one, unless chosen.
    #[test]
    fn workers_and_reviewers_follow_the_orchestrator_unless_chosen() {
        let mut settings = Settings::default();
        let both = has(&[Claude, Codex]);
        let codex_led = OrchestrationSettings {
            orchestrator_agent: Some(Codex),
            ..OrchestrationSettings::default()
        };
        for role in [Worker, Reviewer] {
            assert_eq!(settings.agent_for(role, None, None, &both), Claude);
            assert_eq!(
                settings.agent_for(role, Some(&codex_led), None, &both),
                Codex
            );
            // The pane pinned as orchestrator is what it follows.
            assert_eq!(
                settings.agent_for(role, Some(&codex_led), Some(Claude), &both),
                Claude
            );
            // One that is not installed falls back to one that is.
            assert_eq!(
                settings.agent_for(role, None, Some(Claude), has(&[Codex])),
                Codex
            );
            assert_eq!(settings.agent_for(role, None, Some(Codex), has(&[])), Codex);
        }
        settings.default_reviewer_agent = Some(Claude);
        assert_eq!(
            settings.agent_for(Reviewer, Some(&codex_led), None, &both),
            Claude
        );
        assert_eq!(
            settings.agent_for(Worker, Some(&codex_led), None, &both),
            Codex
        );
        let chose = OrchestrationSettings {
            worker_agent: Some(Claude),
            reviewer_agent: Some(Codex),
            ..codex_led
        };
        assert_eq!(
            settings.agent_for(Worker, Some(&chose), None, &both),
            Claude
        );
        assert_eq!(
            settings.agent_for(Reviewer, Some(&chose), None, &both),
            Codex
        );
    }

    #[test]
    fn each_kind_reads_its_own_account_command_and_model() {
        let pane = Pane {
            claude_account: Some("pro".into()),
            codex_account: Some("team".into()),
            ..Pane::default()
        };
        assert_eq!(pane.account(Claude), Some("pro"));
        assert_eq!(pane.account(Codex), Some("team"));
        let mut folder = WorkspaceFolder::default();
        *folder.account_mut(Codex) = Some("perso".into());
        assert_eq!(folder.codex_account.as_deref(), Some("perso"));
        assert_eq!(folder.account(Claude), None);
        let settings = Settings::default();
        assert_eq!(settings.account(Claude), Some("main"));
        assert_eq!(settings.account(Codex), None, "main, unwritten");

        let mut orchestration = OrchestrationSettings::default();
        assert_eq!(orchestration.model(Worker, Claude), Some("sonnet"));
        assert_eq!(orchestration.model(Worker, Codex), None);
        orchestration.codex_models.reviewer = Some(" gpt-5.5 ".into());
        orchestration.codex_models.worker = Some("   ".into());
        assert_eq!(orchestration.model(Reviewer, Codex), Some("gpt-5.5"));
        assert_eq!(
            orchestration.model(Worker, Codex),
            None,
            "blank: the default"
        );
    }
}
