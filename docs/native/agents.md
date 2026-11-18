# Agents: Claude Code and Codex

Design, 2026-11-19 (board task 9146bcec). No code ships with this page: it
decides how Hangar runs more than one agent, how it drives Codex, and in what
order the work is done. Each decision says why. Where a fact about Codex is
used, its source is named (§4.0); where something could not be confirmed, it
says so and the task that needs it starts by checking it.

## 0. In short

- **One abstraction, read by capability.** An agent is a *kind* (Claude Code,
  Codex) described once in `hangar-core/src/agent/`, with a set of
  capabilities. Every feature asks what the pane's agent *can do* (report its
  turns, take a message mid-turn, hold an approval open, write a transcript,
  resume, run headless…), never *which* agent it is. A test keeps agent-kind
  checks out of feature code.
- **Codex runs in a pane, like Claude Code, attached.** Every interactive
  role — the person's panes, the orchestrator, workers, the project manager —
  runs Codex's own TUI in a real terminal. Since 0.157 that TUI is a client of
  a `codex app-server`; Hangar starts that server for each Codex pane, points
  the TUI at it (`codex --remote`), and connects to it as a second client. It
  learns turns, approvals (held for the chat view, Mobile and Discord; the
  first answer wins), usage, limits and images from the protocol, and sends
  Hangar's events with `turn/start` / `turn/steer` — what the hangar-bridge
  mod does for Claude Code. The plain TUI, read from its output and rollout
  files, is the floor when attaching is not possible.
- **Headless where there is no pane.** The delivery reviewer (`codex exec`),
  `delegate` sub-tasks and image jobs (app-server threads) have no terminal.
  One typed client of the app-server protocol, generated from a pinned Codex
  release, serves panes and headless runs alike.
- **Either agent anywhere.** A workspace's orchestrator is Claude Code or
  Codex (the person chooses; Claude Code by default when both are there,
  whichever is installed otherwise); workers are either; `worker_spawn
  {agent}` and a new `delegate` tool hand work across.
- **Codex alone is enough.** Setup is satisfied by either agent, and every
  feature either works with Codex alone or says why not (§6.9).
- **Pinned, written from the schema, labelled beta.** Codex ships about
  three releases a week. Hangar pins one (0.170.0 to start) and generates its
  protocol types from that release's schema. No Codex session is recorded
  (the person's decision, 2026-11-19: "code blind, and if it doesn't work
  someone will send a PR"). T3 Code is the reference for every protocol
  behaviour (§4.7): its 28 recorded Codex transcripts and its tests become
  Hangar's fixtures, and only what T3 does not cover is written from the
  schema; each fixture says where it comes from. A weekly job diffs the latest
  release's schema against the pinned one, and Codex support is labelled
  **beta** in the app and the README until real users confirm it.
- **Thirty-odd tasks in three waves** (§7): hangar-core now, the engine
  after the pin-pane task, the app after the focus and theme tasks.

Decisions are numbered D1–D22 in the order they were taken (D22, T3 Code as
the reference, came last and sits in §4.7).

## 1. What is asked

The person, 2026-11-19: Codex supported "as well as" Claude Code; agents that
talk to each other and hand work over (Codex generating images for a Claude
orchestrator); "let people choose which orchestrator they want"; and "some
people don't have Claude at all": Hangar fully usable with Codex only, the
setup offering both agents and satisfied by either.

Hangar 2.0 dropped Codex, Gemini CLI and OpenCode (9fe9d5b, 2026-11-03). What
2.0 removed was shallow — launch `codex` or `codex resume <id>`, list its
rollouts, read their token counts for the statistics and the gauge, write
`[mcp_servers.hangar]` into `~/.codex/config.toml`. Everything Hangar built
since is Claude-only: the hangar-bridge mod (turn state, event delivery,
briefings, held approvals), the chat view, Mobile and Discord answers,
accounts, orchestration and workers, the delivery reviewer, kits, the setup
sheet. Appendix A lists every place, by feature.

## 2. How Hangar runs Claude Code today

A pane is a PTY running the person's shell; Hangar types the agent's command
into it once the shell has settled (`hangar-engine/src/host/launch.rs`,
`hangar-app/src/panes/controller.rs`), so aliases and the shell's
environment apply. Around that command Hangar sets, per pane:

| Channel | Claude Code today |
|---|---|
| Identity | `HANGAR_PANE_ID`, `HANGAR_PANE_NAME`, a per-pane API token (`pty.rs`) |
| Hangar's MCP server | `--mcp-config $HANGAR_MCP_CONFIG [--strict-mcp-config]`, a file Hangar writes per launch (`mcp_scope.rs`) |
| Side channel | the hangar-bridge mod, loaded through `CLAUDE_CODE_PLUGIN_DIRS`; it reports `hello`, `turn`, `permission`/`ask`, `settings`, `usage`, `edit`, `session`, `end` to `/api/bridge/{pane}/event`, pulls queued messages from `/next`, waits on `/ask/{id}` for the person's answer, reads `/briefing` |
| Briefing | the role at the end of the system prompt (`prompt.compose`), the state as `additionalContext` at each session start |
| Delivery of events | `bridge_enqueue`: submitted as a prompt when idle, appended to the running turn otherwise, labelled with who sent it |
| Files | transcripts `~/.claude/projects/<slug>/<session>.jsonl`: sessions, context gauge, statistics, chat view, Mobile |
| Account | `CLAUDE_CONFIG_DIR` = a profile folder that shares everything but the login |
| Kits | Claude Code plugins on `CLAUDE_CODE_PLUGIN_DIRS` |
| Without the mod | activity guessed from the output (bursts, silences, bell, OSC 9/777), answers typed as keys into Claude Code's dialogs, checked on screen |

The design keeps this shape — a real terminal the person can take over, a
side channel for what the screen cannot say, files for history — and makes
each channel a capability.

## 3. The agent abstraction

### 3.1 What an agent is

**Decision D1.** An agent is a *kind* plus the *drivers* Hangar has for it:

- **Pane driver**: the agent's own interactive program (TUI) in a Hangar PTY
  pane, started by typing its command into the pane's shell. The person can
  read and type in it at any time. Hangar learns what it does through a
  *side channel* (Claude Code's mod; for Codex, the app-server its TUI is
  attached to), the files it writes
  (transcripts, rollouts) and, last, the screen.
- **Headless driver**: the agent driven over a protocol in a child process
  Hangar owns, with no terminal: Codex's `app-server` (JSON-RPC over
  stdio), and the one-shot runs both agents have (`claude -p`, `codex exec`).

*Why both, and why the pane first.* Hangar's promise is "every pane is a real
process in a real terminal" (`hangar-app/src/panes/mod.rs`): the person sees
and can take over any agent, a pane survives Hangar's restart by resuming its
session, hibernation and nodes work on PTYs. T3 Code, a chat app, drives
every agent headless; Hangar is not a chat app, so headless is the exception —
used where no pane exists or would be wanted (a review, a delegated sub-task,
an image job) and where only the protocol carries a fact (exact plan limits).

The kinds are `Claude` and `Codex`. A `Shell` pane has no agent; it is not a
kind (today's `AgentId::Shell` stays the pane's "nothing to start").

### 3.2 Capabilities

**Decision D2.** Each kind declares what it can do, as data, in
`hangar-core/src/agent/`. A capability is a *level*, not a yes/no, so a
feature can degrade honestly ("turn end only", "keys checked on screen")
instead of pretending. Sketch (names are indicative; the task that writes it
settles them):

```rust
pub enum AgentKind { Claude, Codex }            // persisted "claude" / "codex"

pub struct AgentInfo {
    pub kind: AgentKind,
    pub label: &'static str,                    // "Claude Code", "Codex"
    pub program: &'static str,                  // "claude", "codex"
    pub instructions_file: &'static str,        // "CLAUDE.md", "AGENTS.md"
    pub caps: Capabilities,                     // the best the tested version does
}

pub struct Capabilities {
    pub launch: Launch,          // flags it takes: model, effort, permission mode, extra roots, session name
    pub turns: TurnSignal,       // Reported (start+end, live) | EndOnly | Inferred (output shape)
    pub inject: Inject,          // idle: Submit (side channel) | Type (paste + Enter);
                                 // running: Append (into the turn) | Steer | AfterTurn | None
    pub briefing: Briefing,      // role: SystemPrompt | DeveloperInstructions | FirstMessage;
                                 // state at (re)start: SessionContext | FirstMessage
    pub approvals: Approvals,    // Held (side channel waits for Hangar's answer) | Protocol (headless) | Keys (checked on screen) | None
    pub questions: Questions,    // structured questions Hangar can answer (Claude's AskUserQuestion, Codex's requestUserInput)
    pub transcript: Option<TranscriptFormat>,   // ClaudeJsonl | CodexRollout: chat, stats, context, sessions
    pub sessions: Sessions,      // resume by id, resume last, fork; id Reported | Hunted (folder diff)
    pub usage: Usage,            // context: Transcript | Reported | Protocol; plan limits: Reported | Transcript | Protocol | None
    pub mcp: McpInjection,       // ConfigFile (Claude --mcp-config) | Overrides (Codex -c); strict: bool
    pub extensions: Extensions,  // plugins, skills, hooks, commands a kit can bring
    pub accounts: Accounts,      // ConfigDir { var: "CLAUDE_CONFIG_DIR" | "CODEX_HOME", shared: [...] }
    pub images: Images,          // generates (Codex image_gen), takes image input (paste, -i)
    pub headless: Option<Headless>,  // AppServer { protocol } | OneShot (claude -p, codex exec)
    pub permission_modes: &'static [PermissionMode],   // only modes it enforces itself
    pub background_jobs: bool,   // runs commands that outlive a turn (pty/jobs.rs)
}
```

Two levels, as T3 does with capabilities emitted at session start:

- **`AgentInfo::caps`**, static per kind, refined once by the detected
  version (`codex --version`, `claude --version`): what Hangar may *offer* —
  the setup sheet, the agent pickers, the pin-as-orchestrator entry, which
  kit parts apply.
- **The pane's live capabilities**, held in `hangar-engine/src/live.rs` beside
  today's `bridged`: what *this* session does now. A Claude Code with the
  mod turned off, or a Codex pane that could not attach, reads `turns:
  Inferred`, `inject: Type`, `approvals: Keys`. Today's `bridged` flag becomes
  "the side channel spoke", for either kind.

### 3.3 The rule

**Decision D3. Features read capabilities, never the agent kind.** Only the
agent modules (`hangar-core/src/agent/**`, the kind's own driver files such as
the Claude mod and the Codex rollout reader) may match on `AgentKind`, plus
the state reader that maps persisted strings. A test in `hangar-engine`
(`agent_kind_checks_stay_in_agent_modules`) scans the workspace's sources and
fails on `AgentId::Claude`, `AgentId::Codex`, `AgentKind::` or
`agent == "claude"` outside an allowlist of paths and `#[cfg(test)]` blocks.

*Why.* T3 Code writes the same rule ("Orchestration and clients read
capabilities, never the driver kind"; `docs/internals/adding-a-provider.md`)
after paying for the opposite. Hangar paid too: removing three agents in 2.0
touched 82 files, and adding one back the old way would add a branch to each
of the ~200 files of Appendix A. Codex-only use makes it sharper: a feature
written as `if agent == Claude` silently does nothing on a Codex-only
machine; one written as `if caps.approvals != None` works for both and for a
third agent later.

### 3.4 Where it lives

**Decision D4.**

| Crate / module | Holds | Why there |
|---|---|---|
| `hangar-core/src/agent/mod.rs` (new) | `AgentKind`, `AgentInfo`, `Capabilities`, `info(kind)`, detection results (`installed`, `version`, `path`) | core is free now, has no UI, and is what the node and the engine both link; `AgentKind` must be shared by core (PTY, accounts, MCP, reviewer) and engine |
| `hangar-core/src/agent/launch.rs` (new) | `LaunchRequest` (kind, session, model, effort, mode, roots, name, MCP set, account, kits, role) → `LaunchSpec` (program, args, env, files to write) | one builder instead of today's two copies (`host/launch.rs` and `panes/controller.rs` both decide "Claude gets the bridge, the account, the MCP scope") |
| `hangar-core/src/agent/claude.rs`, `agent/codex.rs` (new) | each kind's flags, environment, side-channel setup, version range | the only places that know the flag spellings |
| `hangar-core/src/codex/` (Samuel's task c48c7086) | Codex detection, rollout reading (`.jsonl`, `.jsonl.zst`), token counts, prices | already being written; `agent/codex.rs` calls it, does not repeat it |
| `hangar-core/src/codex_app/` (new) | the generated app-server types and the JSON-RPC client | shared by attached panes and headless runs; generated code stays out of the hand-written modules |
| `hangar-core/src/agent/codex_attach.rs` (new) | Codex's side channel: protocol events in, bridge events out; the inbox drained by `turn/start` / `turn/steer`; held asks answered | what the Claude mod is for Claude Code, on Hangar's side of the wire |
| `hangar-core/src/bridge.rs` | the side channel's server side, for both kinds: the inbox, held asks, edits, briefings; reports normalized into one `BridgeEvent` | the API and the queue are already agent-neutral; only the mod's install is Claude's |
| `hangar-engine/src/state/types.rs` | `AgentId { Shell, Claude, Codex }` (the persisted pane field) with `kind() -> Option<AgentKind>`; `agent_commands.codex`; `orchestrator_agent` per workspace | persisted model; owned by Wren's pin-pane task now |
| `hangar-engine/src/panes/*`, `orchestration/*`, `mobile/*`, `control/*` | read `info(kind).caps` and the pane's live caps | the features |
| `hangar-app` | labels, icons, pickers, settings rows from `AgentInfo` | drawing only |

### 3.5 Each feature, by capability

| Feature | Reads | Claude Code | Codex (pane) | Codex headless | Without the capability |
|---|---|---|---|---|---|
| Pane activity (working / waiting / yours / idle) | `turns` | Reported (mod) | Reported (attached, §4.2.3); floor: EndOnly (`notify`) | Protocol (`turn/started`, `turn/completed`) | Inferred from output, as today |
| Events to an agent, `pane_send {when_idle}`, kit `panes.send` | `inject` | Submit / Append (mod) | `turn/start` / `turn/steer` (attached, §4.2.4); floor: Type (Enter idle, Tab queued) | `turn/start`, `turn/steer` | Type when idle; refuse `when_idle` with a reason |
| Orchestrator / worker / PM brief | `briefing` | SystemPrompt + SessionContext | DeveloperInstructions + SessionContext (§4.2.5) | `developerInstructions` / `thread/inject_items` | FirstMessage (the brief as a labelled first prompt) |
| Approvals, questions from chat, Mobile, Discord | `approvals`, `questions` | Held (mod), else Keys | Held: server requests, first answer wins (attached, §4.2.6); floor: Keys | Protocol (server requests) | Read-only: shown, answered at the PC |
| Chat view, Mobile chat | `transcript` | ClaudeJsonl + mod | live items and `thread/items/list` (attached); rollout items otherwise (§4.2.7) | live items | Terminal only, with the reason |
| Sessions, resume, hibernation, session hunt | `sessions` | `--resume <id>`, folder hunt | `--remote … resume <id>`, thread id Reported (attached); floor: `codex resume <id>`, folder hunt | `thread/resume` | Fresh start; hibernation keeps the workspace awake |
| Context gauge | `usage.context` | transcript `usage` | `thread/tokenUsage/updated` (attached); rollout `token_count` otherwise | `thread/tokenUsage/updated` | Hidden |
| Plan limits (navigator, accounts) | `usage.limits` | Reported (mod) | `account/rateLimits/*`, exact (attached); rollout `rate_limits` otherwise | `account/rateLimits/*` | Hidden |
| Statistics, pricing | `transcript` | Claude transcripts | rollouts (Samuel) | — | Not counted, said on the page |
| MCP: Hangar's server, workspace scope | `mcp` | ConfigFile, strict | Overrides (`-c mcp_servers.*`) (§4.2.2) | `-c` on the child's server | Hangar's tools absent: orchestration roles refused |
| Kits | `extensions` | plugins | MCP per launch; skills and hooks as a Codex plugin (§6.4) | same | Kit part skipped and listed |
| Accounts | `accounts` | `CLAUDE_CONFIG_DIR` profiles | `CODEX_HOME` shadow homes (§6.1) | same env | Single account |
| Delivery reviewer | `headless` | `claude -p` | `codex exec` read-only (§4.4) | — | Review skipped is never silent: the delivery waits, with the reason |
| `delegate` | `headless` or pane | pane worker or one-shot | pane worker or app-server thread (§5.2) | thread | Refused with the reason |
| Images | `images` | input only | image_gen items → files (§5.3) | `imageGeneration.savedPath` | Not offered |
| Background jobs between turns | `background_jobs` | yes | yes: the pane server's process tree (§4.1) | the child's process tree | as today |

## 4. How Hangar drives Codex

### 4.0 The Codex this page was checked against

Codex is not installed on the machine this page was written on, so nothing
below comes from memory or from `codex --help` here. Facts come from the
official sources, read on 2026-11-19: the `openai/codex` repository at tag
`rust-v0.170.0` (source under `codex-rs/`), its release notes, the npm
package `@openai/codex`, and the Codex docs (`learn.chatgpt.com/docs/`, where
`developers.openai.com/codex/*` now redirects); and T3 Code at 1cefbc4, which
drives `codex app-server` in production. "Source only" marks what the code
does but the docs do not say.

**Where the `codex` binary runs for Hangar's own work.** Codex is not
installed on the person's PC either, and the person has no Codex or ChatGPT
subscription (2026-11-19). So everything the implementation tasks need the
binary for — generating the protocol schema (T3a) and the weekly schema
diff (T12) — runs on the Vast build box (or a GitHub runner), where Codex is
installed with `npm i -g @openai/codex`. Neither needs a login. Nothing is
ever run against a logged-in Codex: no API key, no recorded session (§7,
decision 1). So wherever this page says a task "checks" something about
Codex, it means reading the pinned release's schema, source and docs.

| Fact | Value at 0.170.0 | Source |
|---|---|---|
| Latest stable | **0.170.0**, 2026-11-18; 40 stable releases from 2026-08-19 to 2026-11-19 (≈ 3 a week) plus ≈ 270 alphas | GitHub releases, npm `latest` |
| Hooks | stable since 0.124.0 (2026-04-23), on by default; events `SessionStart`, `SessionEnd`, `SubagentStart/Stop`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `Pre/PostModelRequest`, `Pre/PostCompact`, `UserPromptSubmit`, `Stop`, `Interrupt`; inline `[[hooks.<Event>]]` in any config layer, `-c` included; non-managed hooks must be **trusted by hash** in `/hooks` (or `--dangerously-bypass-hook-trust`) | release notes 0.114–0.124; docs `hooks`; `hooks/src/engine/discovery.rs` |
| `notify` | argv program, JSON appended as last argument, fires only on `agent-turn-complete` (`thread-id`, `turn-id`, `cwd`, `last-assistant-message`) | `hooks/src/legacy_notify.rs`; docs `config-advanced` |
| TUI signals | title by OSC 0 with a spinner and a status (Starting / Working / Thinking / Waiting / Ready); notifications by OSC 9 / OSC 777 / BEL, condition `unfocused` by default (the TUI assumes focus until told otherwise) | `tui/src/terminal_title.rs`, `tui/src/notifications/*` |
| `-c key=value` | TOML value, dotted keys, highest user layer, deep-merged, **never written to disk** | `utils/cli/src/config_override.rs`, `config/src/merge.rs` |
| MCP per server | `command`, `args`, `env`, **`env_vars`** (names forwarded), `cwd`, `url`, `bearer_token_env_var`, `http_headers`, `enabled`, `required`, `enabled_tools`, `disabled_tools`, `startup_timeout_sec` (10), `tool_timeout_sec` (60); stdio servers start from a **cleared environment** plus a short default list | docs `config-reference`; `rmcp-client/src/utils.rs` |
| Instructions | `AGENTS.override.md` / `AGENTS.md` from the project root down to the cwd (trusted projects only since 0.150), `project_doc_fallback_filenames`; launch keys `developer_instructions`, `model_instructions_file` | docs `agents-md`, `config-reference` |
| Rollouts | `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-…-<uuid>.jsonl`; `.jsonl.zst` read since 0.137, written only behind an off-by-default feature; since 0.158 new threads use `historyMode: "paginated"`: messages are `event_msg` `item_completed` turn items; `token_count` carries `total/last_token_usage`, `model_context_window`, `rate_limits {primary, secondary: {used_percent, window_minutes, resets_at}}`; "the transcript format isn't a stable interface" | `rollout/src/*`, `protocol/src/protocol.rs`; docs `hooks` |
| Resume | `codex resume [ID] [--last]`, `codex fork [ID]`, `codex exec resume`, `codex exec fork` | docs `developer-commands` |
| Auth | per `CODEX_HOME` (`auth.json`, or the keyring under an account named after the hash of the `CODEX_HOME` path); `codex login`, `--device-auth`, `--with-api-key` (stdin), `codex login status` | `login/src/auth/storage.rs`; docs `auth` |
| App-server | `codex app-server --listen stdio:// \| unix://PATH \| ws://…`; `proxy --sock PATH` bridges stdio to a socket; v2 protocol only (v1 deleted in 0.110); `generate-json-schema`, `generate-ts`; documented as **experimental** | `cli/src/main.rs`; docs `app-server` |
| TUI attach | `codex --remote unix://PATH \| ws://…`; since 0.157 a plain `codex` already runs its TUI against a shared local daemon (`features.daemon_auto_start`, stable) — but any non-`features.*` `-c`, `--profile` or `--dangerously-bypass-hook-trust` makes it fall back to an in-process server; `--remote` refuses `--add-dir` | docs `app-server#connect-the-cli-terminal-ui`; `tui/src/daemon_startup.rs` |
| Several clients on one thread | `thread/resume` of a running thread attaches the caller; events go to every subscriber; **approval requests go to all, the first answer wins**, then `serverRequest/resolved` to everyone (source only) | `app-server/src/outgoing_message.rs`, `thread_processor.rs` |
| Approvals | `-a on-request \| never` (`untrusted` retired in 0.149), `-s read-only \| workspace-write \| danger-full-access`, `--approve-for-me`, `--yolo`; `--full-auto` gone | `approval_mode_cli_arg.rs`; release notes |
| Typing into the TUI | bracketed paste on; **Enter steers** the running turn, **Tab queues** for the next one; a fast unbracketed stream is taken as a paste | docs `developer-commands#interactive-shortcuts`; `tui/src/bottom_pane/paste_burst.rs` |
| Images | `features.image_generation` stable, **on by default**; `$imagegen` skill, `image_gen` tool, `gpt-image-2`; files in `$CODEX_HOME/generated_images/<session>/<call>.png`; app-server item `imageGeneration {status, revisedPrompt, savedPath, failure}` | `ext/image-generation/src/*`; docs `image-generation` |
| Skills, plugins | `SKILL.md` in `.agents/skills` (repo), `~/.agents/skills` (user), `/etc/codex/skills`, system skills in `$CODEX_HOME/skills/.system`; plugins (`plugin.json` or `.codex-plugin/plugin.json`: skills, `mcp.json`, hooks) installed with `codex plugin add` from a marketplace; no per-launch plugin folder found | docs `build-skills`, `plugins` |
| Headless one-shot | `codex exec --json` (`thread.started`, `turn.*`, `item.*`), `-o`, `--output-schema`, `--ephemeral`, `-s`; `codex mcp-server` **removed** in 0.154 | `exec/src/exec_events.rs`; docs `non-interactive-mode` |
| Platforms, licence | macOS, Linux, native Windows (own sandbox), WSL2; Apache-2.0 | docs `windows`; `LICENSE` |

Not confirmed, and read in the pinned source by the task that needs it:
that a `--remote` TUI draws a turn another client started (§4.1), that `codex
--remote … resume <id>` resumes a thread, that `unix://` works on native
Windows, the exact rollout envelope of an image in paginated history, and
whether a skill folder can be added per launch. Since nothing is run, what
the source does not settle stays an *assumption*: written as one in a
comment where the code relies on it (`// Assumption (from source, not run):
…`), listed in the beta notes (§9), and the first thing a Codex user's bug
report is read against.

### 4.1 Three ways to run Codex

| | **Attached** (default for panes) | **Plain TUI** (fallback) | **Headless** |
|---|---|---|---|
| The pane runs | `codex --remote unix://<pane socket>` | `codex` | nothing (no pane) |
| Hangar runs | `codex app-server --listen unix://<pane socket>` for this pane, and connects to it as a second client | — | `codex app-server` (stdio) or `codex exec` |
| Turn state | protocol (`turn/started`, `turn/completed`, `thread/status/changed`) | `notify` (turn end) + title status + output shape | protocol |
| Events to the agent | `turn/start` when idle, `turn/steer` while running | typed: bracketed paste, then Enter (idle) or Tab (queue) | `turn/start`, `turn/steer` |
| Approvals, questions | server requests, held by Hangar; first answer wins against the TUI's dialog | keys typed into the dialog, checked on screen | server requests |
| Chat, usage, limits, images | items, `thread/tokenUsage/updated`, `account/rateLimits/*`, `imageGeneration` | rollout files | same as attached |
| Used for | every interactive Codex pane: the person's, orchestrator, workers, project manager | a Codex outside the tested range, `--remote` unavailable, or the setting off | reviewer (`exec`), `delegate` children, image jobs, limit probes |

**Decision D5. A Codex pane runs attached.** Hangar starts, per Codex pane, a
`codex app-server` listening on a socket of that pane (in Hangar's runtime
folder), with the pane's environment and Hangar's per-launch settings as `-c`
arguments, and types `codex --remote unix://<socket>` (plus `resume <id>`)
into the pane's shell. The person gets Codex's own TUI, unchanged; Hangar is
a second client of the same threads. *Decided by the person, 2026-11-19:*
the attached mode is on by default, the plain TUI (D6) is the fallback.

*Why.* It gives Hangar, for Codex, everything the hangar-bridge mod gives it
for Claude Code — live turn state, held approvals answered from anywhere
("whichever answers first wins", the mod's own rule), messages into an idle
session or a running turn without typing, exact token usage and plan limits,
image items, thread ids without a folder hunt — while the pane stays a real
terminal. And it is mostly the code the headless runs need anyway: one
app-server client in core serves panes, delegations and probes.

- **Per pane, not one per account.** The server's process *is* the agent: the
  commands Codex runs are its children, with its environment. Started with
  the pane's environment (`HANGAR_PANE_ID`, the pane's API token, the port
  range, `BOARD_RELAY`…), it gives each pane's commands and MCP servers the
  right identity without per-thread environment tricks, and it lives and dies
  with the pane (close, restart, hibernation, `worker_stop`). Memory is what
  Codex costs today: since 0.157 the TUI is already a client of an app-server,
  in-process or the daemon.
- **Settings ride on the server's command line**, as arguments of a process
  Hangar spawns (never typed into a shell, never on screen): `-c
  mcp_servers.hangar…`, `-c developer_instructions=…` for the role, the
  permission mapping, `-c sandbox_workspace_write.writable_roots=[…]` for a
  multi-root workspace (the TUI's `--remote` refuses `--add-dir`). The
  daemon's `-c` exclusion (above) does not apply: the TUI is pointed at
  Hangar's server explicitly.
- **Same binary twice.** The server and the TUI are the same program (the
  settings' Codex command), so their protocol versions always match; Hangar's
  client is the one that must match, which the version gate (§4.6) checks.
- **Finding the thread.** The pane's server holds only that pane's threads:
  Hangar lists them (`thread/loaded/list`), attaches (`thread/resume`), and
  follows `/new` and `/resume` in the TUI the same way. The thread id is the
  pane's session id.
- **Commands are no longer children of the PTY.** `pty/jobs.rs` (a job running
  between turns) walks the pane server's process tree for a Codex pane, not
  the shell's; the protocol's background terminals say the same thing where
  the version has them.
- **On a node**, the node's core spawns the server and is the client; it emits
  the same bridge events over `/node/events` as a Claude pane's mod does
  there. **In a WSL pane**, the server runs inside WSL and Hangar reaches it
  through `codex app-server proxy --sock <socket>` run through `wsl.exe`,
  speaking stdio.

**Decision D6. The plain TUI is the floor.** When attaching is not possible —
a Codex older than the attached minimum or a release listed as broken (by
a user's report or the schema diff, §4.6), a native Windows where the socket transport does not work, a
custom command that cannot take `--remote`, or the person turning
« Attach Hangar to Codex » off — the pane runs plain `codex` with `-c`
settings typed on its command line, and Hangar works the way it works today
with a Claude Code that has no mod: turn end from `notify` (`-c
notify=["<hangar>","--codex-notify"]`), "working" from the title status and
the output, "waiting" from Codex's notification (`-c
tui.notification_method="osc9"`, `-c tui.notification_condition="always"`,
read by the existing OSC 9 handling), messages typed, answers typed as keys
and checked on screen, history from the rollout files. Every feature that
needs more (held approvals, exact limits, image items) says it needs the
attached mode, with the reason.

**Decision D7. Hooks are not used in the first version.** Codex's hooks are
stable and could carry turn state, briefings, approvals (`PermissionRequest`)
and messages at a turn's end (`Stop` with `decision: "block"` continues the
turn with `reason` as a prompt). They are left out because the attached mode
gives all of it and more, and because hooks Hangar passes with `-c` are
untrusted until the person trusts their hash in `/hooks` — a first-run
chore — or until Hangar passes `--dangerously-bypass-hook-trust`, which also
skips the trust of the person's own hooks and is not Hangar's to decide. They
stay plan B: if a Codex release breaks the attached mode for long, the floor
can be raised with a `hangar-ai --codex-hook <event>` handler mapping
`SessionStart` → hello + briefing, `UserPromptSubmit`/`Stop` → turn state,
`Stop` → queued messages, `PermissionRequest` → held ask. A kit's own hooks
are a different matter (§6.4).

### 4.2 The pane, channel by channel

#### 4.2.1 Launch

`agent/codex.rs` builds, from the neutral `LaunchRequest` (§3.4):

- the **server**: `<codex> app-server --listen unix://<runtime>/codex-<pane>.sock`
  then the `-c` list below, cwd the pane's folder, environment the pane's
  (built by the same `pty.rs` functions as the shell's, plus `CODEX_HOME` for
  an account, §6.1, and, as `CLAUDE_*` are today, the variables a Codex sets
  for the commands it runs scrubbed, so a Hangar started from a Codex pane
  does not pass them on);
- the **typed command**: `<codex> --remote unix://<socket>` and `resume <id>`
  when the pane resumes. The model (`-c model="…"`) and the effort (`-c
  model_reasoning_effort="…"`) go on the server, so the typed line stays
  short and shows no path beyond the socket's.
- **trust**: Codex asks before it works in a folder it does not trust, and
  reads a project's `AGENTS.md` only in a trusted one. A worker's worktree is
  a new folder every time; when the workspace's own folder is trusted in the
  person's Codex configuration (read, never written), the worker's server
  gets `-c projects."<worktree>".trust_level="trusted"` for that launch only,
  as Hangar copies Claude Code's folder trust into its profiles today (key
  spelling checked against the pinned release by T4a).
- permission modes, from the workspace's orchestration policy
  (`OrchestrationSettings::permission_mode`), set on the server as
  configuration — and on the typed line too if the pinned TUI turns out to
  send its own policy with each turn (T4a reads which side decides in the
  pinned source):

| Hangar policy | Claude Code | Codex |
|---|---|---|
| none | the person's own | the person's own (`config.toml`) |
| `acceptEdits` | `--permission-mode acceptEdits` | `-c approval_policy="on-request"`, `-c sandbox_mode="workspace-write"` (Codex has no "edits only": commands in the sandbox run, escalations ask) |
| `auto` | `--permission-mode auto` | the above plus `--approve-for-me` (Codex's automatic reviewer) |
| `bypassPermissions` | `--permission-mode bypassPermissions` | `--dangerously-bypass-approvals-and-sandbox` |

`allowedTools` (Claude rules let through without asking) maps to Codex only
for Hangar's own MCP tools, through `mcp_servers.hangar.default_tools_approval_mode`
(the value that means "do not ask", as the pinned release spells it).

#### 4.2.2 MCP injection

**Decision D8.** Hangar's server reaches every Codex pane through `-c` on the
pane's app-server, never through `~/.codex/config.toml`:

```text
-c mcp_servers.hangar.command="<hangar-ai executable>"
-c mcp_servers.hangar.args=["--mcp"]
-c mcp_servers.hangar.env_vars=["HANGAR_PANE_ID","HANGAR_PANE_NAME","HANGAR_API_URL",
                                "HANGAR_API_TOKEN","HANGAR_WORKSPACE","BOARD_RELAY", …]
-c mcp_servers.hangar.tool_timeout_sec=120
```

`env_vars` is required: Codex starts stdio servers from a cleared
environment, so without it `hangar-ai --mcp` would not know its pane. Since
these are arguments of a process Hangar spawns, no path or token is typed
into a shell. In the floor mode the same keys are typed on the `codex`
command line (quoted for the pane's shell, as dropped file paths already
are), with the executable path through an environment variable the shell
expands, as `$HANGAR_MCP_CONFIG` does for Claude Code.

A workspace in « custom » MCP mode (`mcp_scope.rs`) gets the same effect as
Claude's `--strict-mcp-config`: the servers it did not choose are turned off
with `-c mcp_servers.<name>.enabled=false` (their names read, never written,
from the person's Codex configuration: `codex mcp list`, or `config/read` over
the protocol), and the kits' servers added as `mcp_servers.<kit>_<server>`.

#### 4.2.3 Turn state

From the protocol: `turn/started` → working; `item/*/requestApproval`,
`item/tool/requestUserInput`, `thread/status/changed` with
`waitingOnApproval` → waiting; `turn/completed` (`completed`, `interrupted`,
`failed`) → yours, with the last agent message as the headline the
notification shows. Core turns these into the bridge events the engine
already reads (`turn` start/end, `permission`), so `pane_agent.rs` sees a
Codex pane as "bridged" the moment Hangar's client attaches (`hello`, with
the thread id, model and Codex version).

#### 4.2.4 Events and messages into the session

**Decision D9.** `bridge_enqueue` (core's inbox, unchanged) is drained for a
Codex pane by Hangar's client instead of a mod:

- **idle**: `turn/start` with the message as input text, labelled exactly as
  the mod labels it ("[Hangar] From agent "Leo" — not typed by the person…");
  the TUI draws it like any turn (an assumption from the source, §4.0).
- **running**: `turn/steer {expectedTurnId}` — the equivalent of the mod's
  `session.append`, read by the model at its next step; a steer refused
  (the turn ended in between) falls back to `turn/start`.
- what the *person* queued (`from: None`) is sent unlabelled, as the mod's
  `asUser` prompt is.

In the floor mode, the same queue is typed: bracketed paste of the labelled
text, then a separate Enter when the session is idle (Enter steers a running
turn in Codex) or Tab to queue it for the next turn — but only when the
screen shows an empty composer, so a draft the person is typing is never
sent with it.

This is how Hangar's events reach a **Codex orchestrator**: worker events,
deliveries, the board, the person's messages from the chat view, Mobile and
Discord all go through `bridge_enqueue` today, and keep doing so.

#### 4.2.5 Briefing

- **Role** (orchestrator, worker, project manager): `-c
  developer_instructions="<role>"` on the pane's server, so it is in every
  thread the pane starts. A role given to a pane already running reaches it
  as a labelled message now and as developer instructions at the pane's next
  start, as the mod does for Claude ("reaches it at the next compaction;
  Hangar tells the session in the meantime").
- **State** (board, plan, latest events), handed at every start, resume and
  compaction: `thread/inject_items` with a developer message when the thread
  is attached, and again after a `contextCompaction` item (T3 found Codex
  drops injected context at compaction, `CodexAdapterV2.ts` A:1956-1990).
  Kept short: T3 measured `additionalContext` truncated past about 1 000
  tokens.
- In the floor mode: the role as the first labelled message, the state with
  it.

#### 4.2.6 Approvals and questions

Each server request (`item/commandExecution/requestApproval`,
`item/fileChange/requestApproval`, `item/permissions/requestApproval`,
`item/tool/requestUserInput`, `mcpServer/elicitation/request`) becomes a
`HeldAsk` in core (`bridge.rs`, "Asks the mod holds open"), with what it asks
(the command, the patch, the questions). The chat view, Mobile and Discord
offer the same choices they offer for Claude Code — allow, allow for the
session, deny (with a message), the question's options or the person's own
words — mapped to Codex's answers (`accept`, `acceptForSession`, `decline`,
`cancel`; `answers` by question id). Codex sends the request to the TUI too:
whichever answers first wins and `serverRequest/resolved` closes the other.
Only the person answers: an agent cannot, as today.

#### 4.2.7 Sessions, history, context, statistics, images

- **Resume**: `codex --remote … resume <thread id>` (attached) or `codex
  resume <id>` (floor); hibernation, the session picker and pane restarts read
  `sessions.resume` and keep working.
- **Session list, statistics, the gauge of a pane not attached**: the rollout
  files, read by Samuel's task (c48c7086) — `.jsonl` and `.jsonl.zst`,
  legacy and paginated lines.
- **Live gauge and plan limits**: `thread/tokenUsage/updated`,
  `account/rateLimits/updated` (exact, with reset times).
- **Chat view and Mobile chat**: `thread/items/list` pages and live `item/*`
  notifications for an attached pane; for a pane that is not running, the
  rollout's `item_completed` turn items, which are the protocol's own item
  types, so the generated decoders read both.
- **Images**: `imageGeneration` items (attached), the rollout for the floor
  (§5.3).

### 4.3 Headless

**Decision D10.** A typed client of `codex app-server` lives in
`hangar-core/src/codex_app/`, used by attached panes (§4.1) and headless
children:

- **Generated, pinned.** Types are generated from
  `codex app-server generate-json-schema` of the pinned release (as T3
  generates from `codex-rs/app-server-protocol/schema` at a pinned commit,
  `packages/effect-codex-app-server/scripts/generate.ts`), by a script in
  `scripts/codex-protocol/`, reduced to the methods Hangar uses, and checked
  in. Regenerated only when the pin moves.
- **Lenient.** Unknown notifications and fields are ignored; a notification
  that fails to decode is dropped and reported once (T3 does the same); a
  server request Hangar does not handle gets `methodNotFound`, never silence.
- **Handshake**: `initialize {clientInfo: {name: "Hangar"}, capabilities:
  {experimentalApi: false}}` — Hangar uses only the non-experimental surface
  unless a method it needs is gated, which the pin records — then
  `initialized`. The server's version is read from the answer and checked
  against the gate (§4.6).
- **Headless children** (`delegate`, image jobs): `codex app-server --listen
  stdio://` with the workspace's account and settings, one process per
  child, `thread/start {cwd, model, developerInstructions}`, `turn/start`,
  events into the same bridge model as a pane's (a child has an id like a
  pane's, without a terminal), approvals held the same way.
- **Probes**: plan limits for the accounts page (`account/rateLimits/read`),
  the model list for pickers (`model/list`), MCP status. Never opens a
  thread: a status check must not start MCP servers or hooks (T3's rule).

### 4.4 The delivery reviewer

**Decision D11.** The reviewer is chosen per workspace (« Reviewer: same agent
as the orchestrator | Claude Code | Codex »), defaulting to the
orchestrator's agent, falling back to whichever is installed. A Codex review
is `codex exec` — a stable command — in the temporary worktree: `--sandbox
read-only`, `--ephemeral`, `--output-schema <hangar's review schema>`, `-o
<file>` for the answer, `--json` to follow it, the workspace's Codex account
and reviewer model, the same 15-minute limit and the same scrubbed
environment. `delivery.rs` already has the `Reviewer` trait; this adds an
implementation, and its usage-limit wording reads Codex's limit errors.

### 4.5 Which feature uses which mode

| Feature | Attached pane | Floor pane | Headless |
|---|---|---|---|
| Person's Codex panes | yes | fallback | — |
| Orchestrator, workers, project manager | yes | fallback (events typed, approvals by keys) | — |
| Chat view | live items | rollout + typing | — |
| Mobile, Discord: answers, approvals | held asks | keys checked on screen | held asks of a child |
| Context gauge, plan limits | protocol | rollout | probe |
| Statistics, session picker | rollouts (all modes) | rollouts | rollouts |
| Delivery reviewer | — | — | `codex exec` |
| `delegate`, image jobs | — (or `visible: true`: an attached pane) | — | app-server |

**The orchestrator** works in the attached mode and in the floor; it is not
offered headless-only. *Why:* the chat view already gives the person a chat
over the orchestrator's pane, and keeping a terminal behind it keeps "take
over at any time", resume and nodes as they are for Claude Code.

### 4.6 Versions

**Decision D12.** `agent/codex.rs` holds `CODEX_PINNED` (the release whose
schema the types were generated from: 0.170.0 when the first Codex task
lands) and `CODEX_ATTACH_MIN` (the oldest release whose schema has every
method and field the attached mode uses, found by generating the schemas of
older releases, which needs no login, and by their release notes). Older than
the minimum: the floor, with a notice naming the version needed. Newer than
pinned: attached, with lenient decoding. The weekly schema diff (T12) says
when the subset Hangar uses changes; a release that users report broken, or
whose diff removes something Hangar needs, is listed so Hangar falls back to
the floor for it without waiting for an update.

### 4.7 T3 Code is the reference for the protocol

**Decision D22.** Hangar does not guess how Codex's app-server behaves: T3
Code (MIT, cloned at `/home/simnjs/dev/research/t3code`, commit 1cefbc4)
drives it in production, and every Hangar task that speaks to Codex mirrors
the T3 files named below before writing anything of its own. *Why:* the
person's decision is to code without running Codex (§7), and T3's code and
tests are the closest thing to observed behaviour Hangar has. Where T3 and
the pinned release's schema disagree (T3 targets Codex ≥ 0.156, Hangar pins
0.170.0), the schema wins and the difference is written down in the code.

T3 commits more than its recorder: next to
`apps/server/scripts/record-codex-app-server-replay-fixture.ts` (the scenarios
and the transcript format) it keeps **28 recorded app-server transcripts**,
`apps/server/src/orchestration-v2/testkit/fixtures/<scenario>/codex_transcript.ndjson`
— 25 recorded with Codex 0.156.1 and 1 with 0.157.0 by that recorder, 1 a
normalized live probe of 0.124.0-alpha.3, 1 constructed by hand for 0.120.0
(each says so in its first line). They cover `initialize`, `thread/start`,
`thread/resume`, `thread/revert`, `turn/start`, `turn/steer`,
`turn/interrupt`, the item and turn notifications, `thread/tokenUsage/updated`,
`account/rateLimits/updated`, `serverRequest/resolved`, command and file
approvals and `item/tool/requestUserInput`.

| Hangar task | T3 files it mirrors |
|---|---|
| T3a (types) | `packages/effect-codex-app-server/scripts/generate.ts` (what is generated, the hand-added experimental schemas, the patched `turn/start`), `packages/effect-codex-app-server/src/{schema,_generated}` |
| T3b (client) | `packages/effect-codex-app-server/src/{client,protocol,rpc,replay,errors}.ts` and their tests (`client.test.ts`, `protocol.test.ts`, `schema.test.ts`), `test/fixtures/codex-app-server-mock-peer.ts`, `test/examples/codex-app-server-probe.ts`; the handshake and version parsing of `apps/server/src/provider/CodexProvider.ts` |
| T4a (launch spec) | `apps/server/src/provider/codexLaunchArgs.ts`, `CodexManagedRuntime.ts`, the per-thread `config` overrides of `CodexAdapterV2.ts` (MCP servers, `shell_environment_policy.set.*`) and its permission-mode mapping |
| T4b (server lifecycle) | the spawn, kill and restart paths of `CodexAdapterV2.ts` (process exit drained before failing, SIGTERM then kill), `CodexManagedRuntime.ts` |
| T4c (attach) | `apps/server/src/orchestration-v2/Adapters/CodexAdapterV2.ts` — notifications to events, server requests and their answers, `turn/steer` with `expectedTurnId`, stop before `turn/started`, `thread/inject_items` after compaction — with `CodexAdapterV2.test.ts`; `apps/server/src/provider/CodexMcpElicitation.ts` for elicitations |
| T5 (accounts) | `apps/server/src/provider/Drivers/CodexHomeLayout.ts`, `CodexManagedHome.ts` |
| T6 (reviewer) | the usage-limit classification of `CodexAdapterV2.ts` (`usage_limit` with its reset time); T3 has no `codex exec` reviewer, so the exec side is from Codex's docs and schema |
| T9 (chat items) | the item mapping of `CodexAdapterV2.ts` (`item/started`, `item/completed` per item type) |
| T10 (images) | the `imageGeneration` handling of `CodexAdapterV2.ts` and `packages/shared/src/generatedImageMarkdown.ts` |
| T11a (delegate) | `apps/server/src/mcp/toolkits/orchestrator/tools.ts`, `packages/contracts/src/orchestratorMcp.ts`, `apps/server/src/mcp/OrchestratorMcpService.ts` |
| T12 (schema watch) | `generate.ts` for producing the schema; the recorder's scenario list as the list of behaviours the watch reports on |
| also | `apps/server/src/provider/CodexThreadRevert.ts` (not needed now: Hangar offers no rollback; read before adding one) |

**Fixtures.** Hangar's Codex tests are ported, not invented, wherever T3 has
the case: its recorded transcripts first, then its mock-peer exchanges and
the cases of `CodexAdapterV2.test.ts` and the client's tests; only what T3
does not cover is written from the schema. Each fixture file says where it
comes from in its first line:

- `from T3 Code's recordings (Codex 0.156.1, <scenario>)`;
- `from T3 Code's tests (<file>)`;
- `from schema, not recorded (Codex 0.170.0)`, or `from docs` / `from source`
  with the reference.

A ported transcript older than the pin is replayed as it is, and a copy
adapted to a field the 0.170.0 schema changed is marked as adapted. The MIT
licence of T3 Code (Copyright (c) 2026 T3 Tools Inc.) goes beside the ported
files as `T3CODE-LICENSE.txt`, the way Hangar keeps third-party licences next
to what they cover (`hangar-theme/fonts/Inter-LICENSE.txt`,
`site/vendor/GSAP-LICENSE.md`), and the fixtures' README names it.

**What T3 does not cover**, so stays "from source/schema" and beta: the TUI
attached with `--remote` and a second client on the same thread (T3 is the
only client of a stdio server it owns, one for all threads), Hangar's server
per pane, the plain-TUI floor and its screens, `codex exec` for the reviewer,
and rollout files (T3 reads the protocol, not rollouts; that side is
Samuel's, c48c7086).

## 5. Cross-agent work

### 5.1 Workers of either agent, under an orchestrator of either agent

**Decision D13.** `worker_spawn` gains `agent: "claude" | "codex"`, and
`pane_create` / `pane_restart` take `codex` in their `agent` enum. A worker's
agent defaults to the workspace's *worker agent* setting, which defaults to
the orchestrator's agent. `model` is checked against the chosen kind's
aliases (Claude: sonnet/opus/haiku or a `claude-*` id; Codex: a `gpt-*` id
from its `model/list`), with the refusal naming what the kind accepts.

What a worker needs of its agent, read as capabilities, not kinds:

| Need | Capability | Claude Code | Codex |
|---|---|---|---|
| Receive its brief | `briefing`, `inject` | role in the system prompt, brief submitted by the mod | role as developer instructions on its server, brief through the inbox as its first `turn/start` (§4.2.4, §4.2.5) |
| Reach Hangar's tools (`delivery_submit`, `board_*`) | `mcp` | `--mcp-config` | `-c mcp_servers.hangar…` |
| Be seen working / done | `turns` | mod | attached client |
| Keep a job running between turns | `background_jobs` | yes | yes |

Today's refusal "Hangar's Claude Code bridge is turned off, so a worker would
never receive its brief" (`orchestration/worker.rs:894`) becomes a capability
check: a worker can start when its kind's `briefing` is not `None` *for the
settings in force* — the Claude bridge switch for Claude; for Codex a
worker always starts, attached or, with « Attach Hangar to Codex » off, on
the floor with its brief typed (§6.6) — and the refusal names the switch.

The brief texts (`orchestration/brief_text.rs`) stop saying "this Claude Code
session" and say "this {label} session"; the tool names they cite are
Hangar's MCP tools, which both agents call as `mcp__hangar__…` (Claude) or
`hangar.…` (Codex) — the text names the tool, not the prefix.

*Why the orchestrator's agent by default.* A person who chose Codex as their
orchestrator, or has only Codex, gets Codex workers without asking; mixing is
a choice made per worker or per workspace.

### 5.2 `delegate`: a sub-task to another agent, its result back

Workers carry board tasks to a delivery (branch, check, review, landing).
Some work is not a board task: "generate the five icons", "have the other
agent review this function", "answer this question about the repo". For
that, a new MCP tool family, modelled on T3's `delegate_task`
(`apps/server/src/mcp/toolkits/orchestrator/tools.ts`,
`packages/contracts/src/orchestratorMcp.ts`).

**Decision D14.**

```text
delegate {
  task:       string            // the whole brief: the child sees nothing else
  agent?:     "claude"|"codex"  // default: the caller's kind
  model?:     string
  isolation?: "shared"|"worktree"   // default "shared" (read-mostly work);
                                    // "worktree": a worktree from the caller's HEAD
  mode?:      "async"|"wait"        // default "wait"
  wait_s?:    number                // ≤ 45: a wait never outlives an MCP call
  images?:    boolean               // let a Codex child generate images
  visible?:   boolean               // run it in a pane of the caller's workspace
}
→ { delegation, status: "running"|"done"|"failed"|"cancelled"|"needs_you",
    result?: string,                 // the child's final message
    files?:  [path], images?: [path],
    branch?: string,                 // worktree isolation: where its commits are
    waited_out?: boolean }
delegation_status { delegation, wait_s? }
delegation_cancel { delegation }
delegation_list   {}
```

- **Where it runs.** Headless by default: a Codex child is an app-server
  thread (§4.3), a Claude child a `claude -p` run with the stream-json
  output, its permission prompts sent to a Hangar MCP tool that holds them
  for the person (`--permission-prompt-tool`, checked against the current
  Claude Code by T11b). `visible: true` runs it in a pane instead (the same
  launch as a worker, no board task, no delivery), for work the person wants
  to watch.
  *Why headless by default:* a sub-task that returns a result is not
  something to lay out in the grid; T3 makes the same call.
- **Coming back.** `wait` blocks up to `wait_s` and returns the state;
  `async` returns at once. In both cases, when the child ends, Hangar queues a
  labelled event to the caller's pane through `inject` ("[Hangar] Delegation
  d-3 (codex) finished: …"), exactly as worker events reach an orchestrator
  today. *Why ≤ 45 s:* Codex's MCP calls time out (`tool_timeout_sec`,
  60 s by default) and so may Claude Code's; T3 caps at 45 s for the same
  reason.
- **Asks.** A headless child's approvals and questions go where every other
  ask goes: the needs-you list, the chat view, Mobile, Discord — answered by
  the person only; `status: "needs_you"` tells the caller it waits. An agent
  never approves for the person (the bridge's rule).
- **Permissions.** The child runs under the workspace's orchestration policy
  (what its agents may do without asking), mapped to the child kind's own
  modes; a mode the child kind does not enforce steps down to the next
  stricter one it does (T3's rule: never imitate a mode by answering
  approvals in Hangar).
- **Isolation.** `shared` runs in the caller's folder; `worktree` makes a
  worktree and branch from the caller's committed HEAD (uncommitted edits are
  not copied, said in the tool's description), returned as `branch`; Hangar
  removes it with `delegation_cancel` or when the caller's workspace closes,
  unless it holds commits.
- **Accounting.** Each delegation is recorded with its parent pane, kind,
  model, duration and tokens, shown in the inspector of the caller and
  counted in the statistics under its kind.

### 5.3 Generated images

**Decision D15.** Images a Codex session generates are first-class Hangar
artifacts:

- **Detected** from the app-server's `imageGeneration` item (`savedPath`,
  `status`, `revisedPrompt`) for an attached pane and a headless child, and
  from the rollout for a floor pane (`image_generation_end` with
  `saved_path` in legacy history; the paginated envelope is checked by T10).
  Only a `completed` item without `failure` counts (T3,
  `CodexAdapterV2.ts` A:5722). Hangar never moves Codex's file; it reads it
  where Codex saved it.
- **Shown** automatically: inline in the chat view's message (as T3's
  `generatedImageMarkdown.ts` appends `![](path)` to the final answer when the
  message did not show it already), in the inspector of the pane, and in a
  preview pane when the person clicks it — the `preview_show` viewer that
  exists.
- **Shared** to Hangar Mobile as an image chat item (the phone already shows
  `phone_share` files) and to Discord as an attachment of the turn's card.
- **Handed back** in `delegate`'s `images` so a Claude orchestrator can move
  them into the project.

Image generation is on by default in Codex (`features.image_generation`,
stable). Hangar leaves the person's panes as their configuration says, and
sets it per child: `-c features.image_generation=true` for `delegate
{images: true}`, `false` otherwise, so a child sent to review code cannot
spend image quota. The files stay where Codex writes them,
`$CODEX_HOME/generated_images/<session>/<call>.png` (main's folder for every
account, §6.1).

## 6. The rest of Hangar, per agent

### 6.1 Accounts: Codex shadow homes

**Decision D16.** « Comptes Claude » becomes « Comptes » with one section per
kind. A Codex account is a *shadow* `CODEX_HOME`, made the way T3 makes it
(`apps/server/src/provider/Drivers/CodexHomeLayout.ts`, "auth overlay"):

- **main** is the person's own Codex home (`$CODEX_HOME` if set, `~/.codex`
  otherwise): always there, only ever read.
- every other account is `~/.hangar/accounts/codex/<id>/`, listed in the same
  `accounts.json` with `kind: "codex"`. Private to it: `auth.json`,
  `auth.lock`, `models_cache.json` — and, when the person's Codex keeps
  credentials in the system keyring, the keyring entry, which Codex already
  names after the hash of the `CODEX_HOME` path, so a second home is a second
  login with nothing to do (`login/src/auth/storage.rs`). Linked to main:
  `config.toml`, `hooks.json`, `AGENTS.md`, `sessions`, `archived_sessions`,
  `session_index.jsonl`, `history.jsonl`, `generated_images`, `plugins`,
  `skills`, `log` — so a session started on one account is resumable,
  counted and shown from every other, as Claude's profiles do with
  `projects/`.
- **Not linked: the SQLite state.** A database opened through a symbolic link
  gets its `-wal` and `-shm` files beside the link, so two homes would write
  one database with two journals. A pane on a shadow home is started with
  `CODEX_SQLITE_HOME` naming main's folder instead (the variable Codex reads
  for exactly this, docs `environment-variables`), one real path for all.
- **Not linked: `app-server-control/`**, the daemon's socket folder: each home
  keeps its own daemon for the person's use outside Hangar, and Hangar's
  panes use sockets of their own anyway (§4.1).
- a pane on a Codex account starts — its server and its TUI — with
  `CODEX_HOME` set to the shadow home; login is `codex login` run in a pane on
  that home (the existing "pane opened to log an account in", `Pane::login`),
  which writes only the shadow's `auth.json`.
- who an account is logged in as is read from its `auth.json` (the ChatGPT
  account's e-mail and plan, or "API key"), locally, never sent anywhere; the
  plan limits come from `account/rateLimits/read` over a probe (§4.3).

*Why links and not copies.* A copied `config.toml` drifts; a linked one is
the person's single source of truth, and Hangar never writes it (its
per-launch settings ride on `-c`, §4.2.2). If a Codex release turns out to
rewrite `config.toml` by replacing the file (which would replace the link),
T5 copies it one way from main instead, as `share.rs` does for Claude's
`settings.json`; T5 reads it in the pinned release's source.

Where an account applies: as today, pane → workspace → folder → settings,
with one field per kind (`claude_account`, `codex_account`). The per-kind
field is a decision, not a generalisation: a workspace whose orchestrator is
Claude Code on « Pro » and workers on Codex « Team » is a real case.

The reviewer and headless children run on the account of the workspace they
work for, as the Claude reviewer does today.

### 6.2 Statistics and pricing

Samuel's task (c48c7086) brings back reading Codex rollouts and pricing their
tokens in hangar-core. What this design adds on top, in later tasks:

- the statistics read both kinds and the by-agent card comes back
  (`hangar-engine/src/stats/model.rs`, `hangar-app/src/stats/*`), with model
  names pretty-printed per kind (`gpt-5.x-codex` → "GPT-5.x Codex");
- the Hangar account's statistics sync carries the kind (`hangar-cloud-api`
  rows are keyed by model, which already tells the kind; no schema change);
- headless children (reviewer, delegations) are counted under their kind:
  their rollouts land in the same `sessions/` (Codex) or `projects/` (Claude),
  so nothing extra is read.

The plan block of the navigator shows a Codex account's windows (5-hour and
weekly, or the plan's own) from the attached client's
`account/rateLimits/updated` when one of its panes is attached, from a probe
(`account/rateLimits/read`) otherwise — both exact, with reset times — and
from the `rate_limits` of the last rollout line as a last resort.

### 6.3 Setup and install

**Decision D17.** The setup sheet offers both agents and is satisfied by
either.

- It shows two cards, Claude Code and Codex, each with « Installed (version)
  », « Install », or « Not found ». It stands in the way only when *neither*
  is installed (and `shellsOnly` is off). With one installed, it is a
  dismissable notice offering the other.
- **Codex install**, by the ways OpenAI documents only (which ones, and how
  to detect each, is Samuel's `install.rs` work in c48c7086), run as a child
  whose output is shown as Anthropic's installer's is today
  (`shell/setup/install.rs`): npm (`npm install -g @openai/codex`) when `npm`
  is on PATH, Homebrew when `brew` is, otherwise a link to the official
  instructions. Hangar does not download binaries itself. Codex runs natively
  on Windows (with its own sandbox) and in WSL2, so the card is offered on
  every platform Hangar ships on. On NixOS, the nixpkgs package, as for
  Claude Code.
- **Login** next: « Sign in » opens a pane running `codex login` (ChatGPT
  sign-in in the browser; « Use an API key » runs `codex login
  --with-api-key` reading the key from the pane, never from Hangar).
- Detection (`codex --version`, path) is Samuel's task; the sheet reads it.
- Updates stay manual, as T3 decides: Hangar updates only through the
  installer that provably owns the binary, and otherwise says a newer tested
  version exists (§8).

### 6.4 Kits: Claude plugins and Codex plugins

A kit today may carry a Claude Code plugin (skills, commands, agents, MCP
servers, hooks), tabs, and setup commands (`docs/kits.md`). Tabs and setup
commands are Hangar's own and work whatever the agent. Codex has its own
plugins (§4.0): a `plugin.json` (or `.codex-plugin/plugin.json`)
bundling `skills/`, `mcp.json` and `hooks/hooks.json`, installed with `codex
plugin add` from a marketplace (Codex even reads the legacy
`.claude-plugin/marketplace.json`). Skills use the same `SKILL.md` format as
Claude Code's. What 0.170 does not have, as far as the sources show, is a way
to load a plugin or a skill folder for one launch, as Claude Code's
`CLAUDE_CODE_PLUGIN_DIRS` does.

**Decision D18.** Hangar keeps one kit format and adapts it per kind:

| Plugin part | Claude Code | Codex |
|---|---|---|
| MCP servers (`.mcp.json`) | plugin servers | per launch, `-c mcp_servers.<kit>_<server>.*` on the pane's server, `${CLAUDE_PLUGIN_ROOT}` written out as the kit's folder (as `kits/plugin.rs` already does for Claude's strict mode). Works on every launch, nothing installed |
| Skills (`skills/<name>/SKILL.md`) | loaded with the plugin | in a Codex plugin Hangar builds from the kit (`~/.hangar/kits/<id>/codex-plugin/`: a `.codex-plugin/plugin.json`, links to the kit's `skills/`, its hooks), published in a local marketplace Hangar keeps; « Add to Codex » on the kit runs `codex plugin marketplace add` and `codex plugin add` once, with the person's consent, and « Remove from Codex » undoes it |
| Hooks (`hooks/hooks.json`) | plugin hooks | in that Codex plugin, for the events Codex has with the same name; a hook whose matcher names a Claude tool (`Edit`, `Bash`…) is listed as "Claude Code only" |
| Commands, agents, output styles | yes | listed as "Claude Code only" |

*Why « Add to Codex » is the one place Hangar has Codex change its own
configuration:* without a per-launch way, the alternatives are writing into
the person's `~/.agents/skills` or the project's `.agents/skills` behind
their back, or not offering kit skills to Codex at all. An explicit,
reversible install through Codex's own command is the honest middle. If a
Codex release adds a per-launch plugin folder, T7 switches to it and the
button goes. T7 reads in the pinned source first whether `[[skills.config]]` with a `path` can
already do it per launch.

The kit's activation view lists, per kind, what will load and what will not;
the consent hash covers the Codex mapping too. A kit may declare
`"agents": ["claude"]` in `hangar-kit.json` to say it is meant for one kind;
it is then not offered to a Codex pane at all.

### 6.5 Project instructions

**Decision D19.** Hangar's playbook block (`hangar-core/src/instructions.rs`)
is written into the file each agent reads: `CLAUDE.md` for Claude Code,
`AGENTS.md` for Codex — in a workspace that runs that agent, between the same
markers, never touching the rest of the file. When `CLAUDE.md` is a link to
`AGENTS.md` (a common setup, `store.rs` keeps it a link), the block is written
once. Hangar never sets `project_doc_fallback_filenames` to make Codex read
`CLAUDE.md`: that is the person's Codex configuration to choose, it only
applies in folders with no `AGENTS.md`, and Codex has `/import` to convert
Claude Code's files if the person wants. Codex reads a project's `AGENTS.md`
only in a trusted folder (since 0.150), which is one more reason for the
worktree trust of §4.2.1.

The *roles* (orchestrator, worker, PM) are not in these files: they are
per-pane and arrive by `briefing` (§4.2.5).

### 6.6 Settings

- « Sessions »: the « Hangar bridge » switch becomes one switch per kind
  (« Claude Code: hangar-bridge mod », « Codex: attach Hangar to Codex »).
- « Agents »: one launch command per kind (`agentCommands.claude`,
  `agentCommands.codex`), default `claude` / `codex`.
- « Workspaces »: default orchestrator agent for new workspaces (« Automatic
  », Claude Code, Codex), default worker agent (« Same as the orchestrator »),
  default reviewer agent (§4.4).
- « Orchestration » of a workspace: models per kind. Today's
  `orchestratorModel`, `workerModel`, `reviewerModel` (`opus`, `sonnet`,
  `sonnet`) stay Claude Code's, so files written by 1.x and 2.x read as
  before; `codexModels { orchestrator, worker, reviewer }` holds Codex's,
  each absent meaning Codex's own default model, picked from `model/list`.

### 6.7 Choosing the orchestrator per workspace

**Decision D20.** A workspace has an orchestrator *agent*. The new-workspace
dialog's « Start with an orchestrator » shows a two-choice control (Claude
Code / Codex), each greyed with its reason when not installed. The default:
the settings' default if set; otherwise Claude Code when both are installed,
the installed one when only one is. Pinning an existing pane as orchestrator
(pane menu, `orchestration_ui/actions.rs`) works on a pane of either kind
whose capabilities allow it (`briefing`, `inject`, `mcp` not `None`); the
entry says why when they do not.

*Why Claude Code stays the default when both are there:* the person's
request ("default stays Claude Code"), and today's users see no change.

### 6.8 Nodes

A node runs whatever agents are installed on it. `/node/info` reports both
(`claude`, `codex`: path and version), the desktop greys the kinds a node
lacks in its pickers, and the node's transcript routes (`/node/chat`,
`/node/context`, `/node/sessions`) take a `kind`. A node's Codex panes use the
node's `~/.codex` and shadow homes there. Headless Codex (reviewer,
delegations) runs on the node that owns the workspace, like today's reviewer.

### 6.9 Codex only: every place that assumed Claude Code

The person's requirement: "some people don't have Claude at all". With Codex
installed and Claude Code absent:

| Place (Appendix A) | Today | After |
|---|---|---|
| Setup sheet (`shell/setup/*`) | blocks until `claude` is installed | satisfied by Codex (D17) |
| New workspace (`workspace_dialog/folder.rs`), `workspace_create` (`host/mod.rs:550`), node workspaces (`hangar-node/src/own.rs`), Discord `/workspace` | a Claude Code pane | the default orchestrator agent (D20) |
| Pin as orchestrator, project manager (`state/roles.rs`, `orchestration_ui/actions.rs`) | Claude Code panes only | any kind with the capabilities |
| Workers (`orchestration/worker.rs`) | Claude Code, refused when the Claude bridge is off | either kind (D13) |
| Brief texts (`brief_text.rs`) | "this Claude Code session" | the kind's label |
| Bridge mod install (`bridge.rs`) | written on every Claude launch | unchanged, simply never used without Claude |
| Delivery reviewer (`delivery.rs`) | `claude -p`, fails "Claude Code was not found" | the reviewer agent setting, Codex when Claude is absent (§4.4) |
| Chat view, Mobile, Discord follow | Claude transcript + mod | attached client; rollout in the floor (D5, D9) |
| Accounts | Claude profiles | Codex shadow homes (D16) |
| Kits | Claude plugins only | MCP per launch, skills and hooks as a Codex plugin (D18) |
| Statistics | Claude transcripts only | rollouts too (Samuel) |
| MCP install panel (`mcp/install_panel.rs`), servers panel, `mcp_scope.rs` | `.mcp.json`, Claude's scopes | per kind; for Codex nothing to register: Hangar's server and the workspace's choice ride on `-c` at each launch (§4.2.2) |
| Instructions (`instructions.rs`) | `CLAUDE.md` | `AGENTS.md` too (D19) |
| `pane_create {prompt}`, `pane_send {when_idle}`, kit `panes.send` | Claude Code only | any kind whose `inject` allows it |
| Context gauge, plan limits | Claude only | the attached client, rollouts otherwise (D5, D10) |
| Hibernation (`hibernate.rs`) | resumable = Claude | `sessions.resume` |
| PATH repair (`path_env.rs`) | `~/.local/bin` | also npm's global bin and Homebrew's, where `codex` lands |
| Node info (`hangar-node/src/api.rs`) | `claude` on PATH | both |

What does **not** work with Codex alone, said where it shows:
- Claude Code plugin *commands*, *agents* and *output styles* of a kit (no
  Codex equivalent): listed as "Claude Code only" in the kit's activation.
- Anything that needs a feature of the pinned Codex the person's Codex is too
  old for: the pane says which version is needed (§8).

## 7. Task breakdown

Each task below is one worker's: an outcome, the files it owns, what it waits
for, and a size (changed lines, tests included, generated code excluded).
The person's rules apply to every one: nothing compiled on the PC (checks on
Vast through `scripts/remote`), nothing that needs the `codex` binary run on
the PC either (it is not installed there: the Vast build box, §4.0), a PR per
task, no CHANGELOG entry, no Claude attribution. Every task that speaks to
Codex starts from the T3 Code files §4.7 names for it.

**Who holds what now** (from the board, 2026-11-19), and what it blocks:

| Holder | Owns | Blocks |
|---|---|---|
| Samuel, c48c7086 | `hangar-core/src/{install,sessions,stats,pricing,context,codex}*`, core `Cargo.toml`, `Cargo.lock` | anything that reads rollouts, detects Codex or adds a core dependency |
| Wren, afdb9027 (pin panes) | `hangar-engine/src/state/{layout,store,types,lenient,hosted}.rs`, `orchestration/`, `control/` | the persisted `AgentId`, orchestration, the control tools |
| Rex, 6d653883 (focus bug) | `hangar-app/src/{panes,status_bar,keymap,chat_view,board,navigator,workspace_dialog,inspector,needs_you}/`, `keys*`, `focus*`, `state/ui.rs` | the pane controller, the dialog, the chat view |
| Cleo, eec8da2b (theme) | `hangar-theme`, `hangar-term/src`, `hangar-app/src/settings/`, `shell/{windows,frame}.rs`, **`hangar-i18n`** | every task with new strings |

**Coordination with Samuel's task.** This design expects from c48c7086, and
does not redo: `codex` detection (path, version), the rollout listing and
reader (`.jsonl` and `.jsonl.zst`, legacy and paginated lines), token counts
with `model_context_window` and `rate_limits`, OpenAI prices, and session,
context and statistics functions that take the agent kind. If c48c7086 lands
with its own kind type (a string or an enum), T1 adopts it as
`hangar_core::agent::AgentKind` rather than adding a second one.

### Wave A — hangar-core, now

| Id | Outcome | Owns | Depends on | Size | In parallel with |
|---|---|---|---|---|---|
| **T1** | `AgentKind`, `AgentInfo`, `Capabilities` (static levels per kind), `info(kind)`, detection facade (Claude's existing check, Codex's from c48c7086), PATH repair for npm's and Homebrew's global bins | `hangar-core/src/agent/{mod,caps}.rs`, `lib.rs` (one line), `path_env.rs` | c48c7086 for the Codex detection call | 300 | — (first) |
| **T2** | One launch builder: `LaunchRequest` → `LaunchSpec`; Claude Code's flags moved here from `hangar-engine/src/panes/agents.rs` with their tests (the engine switches in T14a) | `hangar-core/src/agent/{launch,claude}.rs` | T1 | 400 | T3a, T5, T8 |
| **T3a** | Protocol types: a generator script over `codex app-server generate-json-schema` at the pinned release, run on the Vast build box (no login needed), the checked-in subset Hangar uses, a README of how to bump | `scripts/codex-protocol/`, `hangar-core/src/codex_app/generated.rs` | T1 | 200 + generated | T2, T5, T8 |
| **T3b** | The JSON-RPC client: spawn (stdio, or `proxy --sock`), `initialize`, requests, notification stream, server requests with responses, version gate, lenient decoding; tests on T3 Code's recorded transcripts and mock-peer exchanges, ported (§4.7), and fixtures from the schema only where T3 has none; `T3CODE-LICENSE.txt` beside them | `hangar-core/src/codex_app/{mod,client,gate}.rs`, `hangar-core/tests/fixtures/codex/app-server/` | T3a | 400 | T4a, T5, T6 |
| **T4a** | Codex launch spec: the pane server's arguments (`-c` MCP, role, permissions, roots, model, effort, trust of Hangar's worktrees), the typed `--remote` line, the floor's typed line, per-shell quoting; tests per shell | `hangar-core/src/agent/codex.rs` | T1, T2 | 350 | T3b, T5 |
| **T4b** | The pane's Codex server lives with the pane: spawned with the pane's environment, socket in Hangar's runtime folder, killed and restarted with the pane (close, restart, hibernation, `worker_stop`), WSL through `proxy`; `pty/jobs.rs` walks its tree; `apply_account` for `CODEX_HOME` | `hangar-core/src/pty.rs` (Codex parts), `pty/jobs.rs`, `pty/codex_server.rs` (new) | T4a | 350 | T3b, T6 |
| **T4c** | Attach: per pane, Hangar's client finds and follows the pane's threads and turns protocol events into the bridge events the engine reads (hello, turn, permission/ask, settings, usage, edit, session, end); the inbox drained by `turn/start` / `turn/steer`; held asks answered as protocol responses; briefing by `thread/inject_items`, again after compaction; tests ported from T3's recordings and `CodexAdapterV2.test.ts` (§4.7); the second-client and `--remote` cases, which T3 has not, from the source and marked so | `hangar-core/src/agent/codex_attach.rs` (new), `bridge.rs` (a delivery seam for panes without a mod), `server.rs` if a route changes | T3b, T4b | 450 | T5, T6, T9 |
| **T5** | Codex accounts: shadow `CODEX_HOME`s (T3's auth overlay, `CODEX_SQLITE_HOME` pointed at main's), `kind` in `accounts.json`, who an account is from `auth.json`, `env_for(kind, account)` | `hangar-core/src/accounts.rs`, `accounts/{store,login,link}.rs`, `accounts/codex_home.rs` (new) | T1 | 400 | T2–T4 |
| **T6** | The reviewer per agent: `codex exec` read-only with an output schema, chosen by the workspace's reviewer setting; Codex limit wording | `hangar-core/src/delivery.rs`, `delivery/review_codex.rs` (new) | T1, T5 | 300 | T3b, T4b |
| **T7** | MCP and kits for Codex in core: custom MCP mode as `enabled=false` overrides (names from `codex mcp list` / `config/read`), kits' servers as `-c`, kit hooks and parts listed per kind, `agents` in the kit manifest, MCP tool schemas (`agent` enum with `codex`, neutral wording) | `hangar-core/src/{mcp_scope,mcp}.rs`, `kits/{plugin,hooks,manifest}.rs` | T4a | 400 | T4b, T5 |
| **T8** | The playbook block in `AGENTS.md` for Codex workspaces, once when `CLAUDE.md` links to it | `hangar-core/src/instructions.rs` | T1 | 150 | everything |
| **T9** | Codex conversation as chat items: protocol item pages and the rollout's `item_completed` items → `chat` items; tool titles for command, patch, MCP call, web search, image | `hangar-core/src/chat/{mod,codex}.rs`, `chat/tools.rs` | T3a, c48c7086 | 400 | T4c, T5 |
| **T10** | Generated images: `Event::ImageGenerated` from attached panes, headless children and the floor's rollout; an image chat item for Mobile | `hangar-core/src/agent/images.rs` (new), `host.rs` (event), `chat/` (item kind) after T9 | T4c, T9 | 250 | T11a |
| **T11a** | `delegate` runner, Codex side: children as app-server threads, the delegation registry, `shared`/`worktree` isolation, results (message, files, images, branch), events to the caller's inbox | `hangar-core/src/delegate.rs`, `delegate/` (new) | T3b, T4c, T5 | 350 | T10 |
| **T11b** | `delegate` runner, Claude side: `claude -p` stream-json children, their asks held for the person | `hangar-core/src/delegate/claude.rs` | T11a | 200 | T12 |
| **T12** | The schema watch and the bug path: a weekly job (Vast build box or a GitHub runner, no login) that runs `codex app-server generate-json-schema` on the latest Codex release, diffs it against the pinned one restricted to the subset Hangar uses, and opens an issue when that subset changes; a GitHub issue template for Codex users (what happened, `codex --version`, Hangar's version, attached or floor, the pane's Hangar log) | `scripts/codex/`, `.github/workflows/codex-schema-watch.yml`, `.github/ISSUE_TEMPLATE/codex-bug.yml` | T3a | 250 | T3b–T11 |

### Wave B — hangar-engine, after Wren's pin-pane task lands

| Id | Outcome | Owns | Depends on | Size | In parallel with |
|---|---|---|---|---|---|
| **T13** | Persisted model: `AgentId::Codex` and `kind()`; `codex` read as Codex again (`RETIRED_AGENTS` keeps Gemini/OpenCode only); `agentCommands.codex`, `codex_account` on pane/workspace/folder/settings; workspace `orchestratorAgent`, `workerAgent`, `reviewerAgent`; Codex models beside the Claude ones (`codexModels`, absent = Codex's default); settings `codexAttach`, default agents; round-trip and lenient tests | `hangar-engine/src/state/{types,lenient}.rs`, their fixtures and tests | **Wren landed**, T1 | 400 | — (first of B) |
| **T14a** | One launch path: `panes/agents.rs` and `host/{launch,terminals,mod}.rs` call core's builder; the windowless `workspace_create` uses the default orchestrator agent | `hangar-engine/src/panes/agents.rs`, `host/{launch,mod,terminals}.rs` | T13, T2, T4a | 300 | T14b, T15, T18, T19 |
| **T14b** | Activity by capability: `bridged` becomes "side channel spoke" for either kind; live capabilities per pane; trust-question reading per kind (Claude's screen, Codex's) | `hangar-engine/src/panes/{pane_agent,agent_state,bridge_events,pane_insight}.rs`, `live.rs` | T13, T4c | 300 | T14a, T15 |
| **T15** | Sessions and sleep by capability: no folder hunt when the session id is reported; resume by kind; hibernation reads `sessions` | `hangar-engine/src/panes/{session_watch,hibernate,hibernation}.rs` | T13, c48c7086 | 300 | T14a, T14b |
| **T16a** | Orchestration for either kind: briefs by capability, texts with the kind's label, `worker_spawn {agent}`, the worker refusal by capability, pin rule by capability | `hangar-engine/src/orchestration/{worker,brief,brief_text,projects}.rs`, `state/roles.rs` | **Wren landed**, T13, T14b | 350 | T16b, T18 |
| **T16b** | Control tools by capability: `pane_create {agent: codex, prompt}`, `pane_send {when_idle}`, `pane_restart {agent}`, `workspace_create` default agent, the manager | `hangar-engine/src/control/{panes,workspace,manager}.rs` | **Wren landed**, T13, T14a | 250 | T16a, T18 |
| **T17** | `delegate`, `delegation_status`, `delegation_cancel`, `delegation_list` as control operations and MCP tools | `hangar-engine/src/control/delegate.rs` (new), `hangar-core/src/mcp.rs` (schemas) after T7 | T11a, T16b | 350 | T18, T19 |
| **T18** | Mobile and desk by capability: answers and permissions through held asks for either kind, keys for the floor's dialogs (Codex's own, checked on screen), chat pages from T9 | `hangar-engine/src/mobile/{answer,held,desk,ops,chat_live,calls}.rs` | T13, T9, T4c | 400 | T16a/b, T19 |
| **T19** | Statistics model: the by-agent card back, model names per kind | `hangar-engine/src/stats/model.rs` | T13, c48c7086 | 200 | T18 |
| **T20** | The rule's test: agent-kind checks fail outside the allowlist (§3.3) | `hangar-engine/tests/agent_kind_checks.rs` (new) | T14–T19 (lands last of B) | 150 | — |

### Wave C — hangar-app and the edges, after Rex's and Cleo's tasks land

Every task of `hangar-app` here adds strings, so it waits for Cleo
(`hangar-i18n`); each adds its keys in a block of its own so merges stay
mechanical.

| Id | Outcome | Owns | Depends on | Size | In parallel with |
|---|---|---|---|---|---|
| **T21** | Setup sheet for both agents, satisfied by either; Codex install by the official installers; « Sign in » panes; Codex labelled « beta » on its card | `hangar-app/src/shell/setup/`, `shell/overlay.rs`, `services/core.rs` | **Cleo landed**, T1, c48c7086 | 400 | T22–T28 |
| **T22** | New-workspace dialog: orchestrator agent choice, per-pane agent picker with Codex, greyed with reasons, Codex labelled « beta » | `hangar-app/src/workspace_dialog/` | **Rex, Cleo landed**, T13 | 300 | T21, T23–T28 |
| **T23** | Settings: launch command per kind, « Attach Hangar to Codex », bridge per kind, default orchestrator / worker / reviewer agents, Codex models | `hangar-app/src/settings/` | **Cleo landed**, T13 | 300 | T21, T22, T24–T28 |
| **T24a** | The pane controller on the core builder and live capabilities; header and look per kind (Shift+Enter for Codex) | `hangar-app/src/panes/{controller,header,look,launch,waves,mod}.rs` | **Rex landed**, T14a, T14b | 350 | T24b |
| **T24b** | Pane menu, session picker, toolbar, orchestration panel by capability; models per kind | `hangar-app/src/{pane_menu,session_picker,toolbar,orchestration_ui}/` | **Cleo landed**, T13, T16a | 300 | T24a |
| **T25** | « Comptes » per kind: Codex accounts, login panes, badges, plan blocks; a worktree keeps both accounts | `hangar-app/src/accounts/`, `git_ui/worktrees/mod.rs`, `hangar-engine/src/accounts.rs` | **Cleo landed**, T5, T13 | 400 | T21–T24 |
| **T26** | Chat view, needs-you, inspector and navigator plan by capability; generated images inline | `hangar-app/src/{chat_view,needs_you,inspector}/`, `navigator/plan.rs` | **Rex landed**, T18, T10 | 400 | T27, T28 |
| **T27** | Kits per kind: what loads in Claude Code and in Codex, « Add to Codex » for a kit's skills (§6.4) | `hangar-app/src/kits/`, `kit_tabs/api.rs`, `hangar-kit-host/src/{api,gate}.rs` | **Cleo landed**, T7 | 300 | T26 |
| **T28** | Statistics page by agent | `hangar-app/src/stats/` | **Cleo landed**, T19 | 200 | T26, T27 |
| **T29** | Nodes run Codex: `/node/info` both agents, node transcript routes take the kind, node workspaces use the default agent, node panes spawn their Codex server | `hangar-node/src/{api,own,panes,transcripts}.rs`, `hangar-app/src/nodes/` | **Cleo landed** (for the app part), T13, T4b, T4c, c48c7086 | 400 | Wave C |
| **T30** | Discord follows either kind, `/workspace` uses the default agent, images attached to turn cards | `hangar-discord/src/{bot,follow,cards,hangar}.rs` | T13, T10 | 250 | Wave C |
| **T31** | README, guide, site and `docs/kits.md` per D21, the capability table generated from `AgentInfo`, Codex marked beta with a link to the Codex bug template (T12) | `README.md`, `docs/guide/`, `docs/kits.md`, the site's pages | the features (last) | 300 | — |

**Critical path to "a Codex orchestrator with Codex workers"**: T1 → T2 / T3a →
T3b → T4a → T4b → T4c → (Wren) T13 → T14a, T14b → T16a, T16b → (Rex, Cleo) T22,
T24a. Wave A runs entirely before the holders of the engine and the app
finish, so nothing waits idle: sixteen core tasks, four or five of them in
parallel at its widest.

### Decisions taken by the person (2026-11-19)

1. **Fixtures — decided: no API key, no recordings by Hangar.** "Code
   blind, and if it doesn't work someone will send a PR" — and "you have T3
   Code to make sure the mechanism is right". Fixtures are ported from T3
   Code first (its recorded transcripts, its mock peer, its tests: §4.7) and
   marked `from T3 Code's recordings` / `from T3 Code's tests`; what T3 does
   not cover (the attached TUI, the floor's screens, rollout lines) is
   written from the pinned schema, docs and source and marked `from schema,
   not recorded`. T3b, T4c and T12 do not stop for recordings. Where no T3
   recording exists, this knowingly sets aside T3's own rule that
   hand-written frames are a guess: the guess is contained by the beta
   label, the schema watch and the users' reports (§8).
2. **Canary — decided, as a schema watch:** weekly, the latest Codex
   release's `generate-json-schema` diffed against the pinned one (T12), on
   the Vast build box or a GitHub runner, no login; an issue opens when the
   subset Hangar uses changes.
3. **README — decided:** neutral (D21).
4. **Attached mode — decided:** on by default (D5), the plain TUI as the
   fallback (D6).
5. **Beta — decided:** Codex support is labelled « beta » in the app (the
   setup card, the agent pickers) and in the README until real users confirm
   it, with a GitHub issue template asking Codex users for a bug report and
   their `codex --version` (T12, T21, T22, T31).

## 8. Risks, and how they are contained

| Risk | Containment |
|---|---|
| **Codex changes often**: 40 stable releases in three months; recent ones removed `codex mcp-server` (0.154), `-a untrusted` (0.149), `--full-auto`, profile tables (0.134), and changed what a rollout holds (paginated history, 0.158) | One version range in `agent/codex.rs` (D12): `CODEX_PINNED` and `CODEX_ATTACH_MIN`, both read from release schemas. The protocol types are generated from the pinned release, checked in, and regenerated only by a deliberate bump. A release known to break is listed, and Hangar runs the floor for it. |
| **The app-server is "experimental"** in Codex's docs, and two clients answering one thread's approvals is in the source, not the docs | Hangar uses the non-experimental surface (`experimentalApi: false`) unless a method it needs is gated, which the pin records. The TUI itself has been an app-server client since 0.157 (the daemon is a stable feature), so the protocol is exercised by OpenAI's own TUI on every release; the server and the TUI Hangar runs are always the same binary. If the attached mode breaks, every pane still runs on the floor (D6), and hooks are plan B (D7). |
| **Coded blind** (the person's decision: no Codex session is ever run): behaviour the schema does not show — a `--remote` TUI drawing another client's turn, two clients answering one approval, the paginated rollout envelopes, the floor's dialogs — may be wrong | T3 Code as the reference for the protocol (D22): its 28 recorded transcripts (Codex 0.156.1–0.157.0) and its tests ported as fixtures, so most protocol behaviour *was* observed, by T3; what T3 does not cover written from the schema, docs and source and marked `from schema, not recorded`, so nobody mistakes it for observed behaviour; every assumption the code relies on commented as one (§4.0); Codex labelled **beta** in the app and the README; an issue template that asks for `codex --version`, attached or floor, and Hangar's log; « Attach Hangar to Codex » can be turned off, and a release reported broken is listed so Hangar runs the floor for it. |
| A change Hangar did not see breaks a feature silently | The **schema watch** (T12): every week the latest release's schema is generated (no login) and diffed against the pinned one on the subset Hangar uses; a change opens an issue rather than failing a release. Lenient decoding keeps an unknown field or notification from breaking a pane meanwhile. |
| Rollout drift ("the transcript format isn't a stable interface", Codex's hooks docs) | Lenient readers: unknown line types skipped, unknown fields ignored, a line that fails to parse counted and reported once, never a crash or a silent zero. Live views read the protocol, not the files; the files serve history and statistics. |
| Screen-reading fallbacks break on a redraw | Used only on the floor; every key is checked against the screen first and nothing is submitted that the screen does not show, as `mobile/answer.rs` does for Claude Code. |
| **Two agents double the maintenance** | The capability rule (D3) keeps per-kind code in a few files: `agent/claude.rs`, `agent/codex.rs`, `agent/codex_attach.rs`, the Claude mod, the rollout reader, the app-server client. Features stay single-path. The Codex-only tasks of §7 add up to roughly 3 500–4 000 hand-written lines, plus the generated types. |
| The person's `~/.codex/config.toml` is precious | Hangar never writes it: everything rides on `-c`, shadow homes link it. The one change Hangar causes is « Add to Codex » for a kit (D18), through Codex's own command, on request, reversible. A test asserts no other code path opens it for writing. |
| Accounts sharing `sessions/` and state across homes | Sessions and writer locks linked as T3 does; SQLite shared through `CODEX_SQLITE_HOME`, never through a link (D16). |
| Quota surprises (images, headless children) | Images off for children unless asked (D15); every delegation shows its kind and tokens; usage-limit errors classified as such with their reset time (T3's rule), never as a generic failure. |
| Codex on Windows | Native Windows is supported by Codex with its own sandbox. Whether `unix://` sockets work there is checked by T4b; if not, the attached mode uses a loopback WebSocket with a token (`--ws-auth`), or the pane runs on the floor, and the setup card says which. |
| Licence | Codex CLI is Apache-2.0; Hangar generates types from its published schema and links nothing of it. T3 Code is MIT; this page borrows designs, not code. |

## 9. What the README and the site say

**Decision D21.** Neutral, with an honest table. *Decided by the person,
2026-11-19.*

> Hangar runs your coding agents — **Claude Code** and **Codex** — side by
> side in real terminals, and lets them work together: an orchestrator of
> either kind plans on the board and hands tasks to workers of either kind.

with Codex marked **beta** until real users confirm it — the README links
the Codex bug template (T12) and lists the assumptions of §4.0 the code
relies on — followed by a "What works with which agent" table generated
from the same capabilities (D2), so it cannot drift from the code: rows for orchestration,
workers, chat view, Mobile/Discord answers, accounts, kits (with "commands and
agents: Claude Code only"), statistics, images (Codex).

*Why neutral rather than "Claude Code first, Codex supported".* The person
wants Codex "as well as" Claude Code and Codex-only users fully served; a
"first / supported" hierarchy tells a Codex-only user they are second-class
before they try. The table is where differences belong, and it states them
exactly. The default (Claude Code when both are installed) is a product
default, not a ranking, and is said where the choice is made.

## Appendix A. Every place that assumes Claude Code, by feature

From `git grep -n -i claude -- crates` on `main` at 261f8ce (2 858 lines in 297
files; 1 118 of them non-test code once comments and translated strings are
left out). Tests,
fixtures, benches, snapshots and the i18n dictionaries are left out of the
lists below; they follow the code they test. "Cap." names the capability of
§3.2 the place reads once it is migrated; **must work without Claude** marks
the places that today stop a Codex-only machine.

### A.1 Agent identity, launch, setup (cap.: `launch`, descriptor)

| Place | What it assumes today |
|---|---|
| `hangar-engine/src/state/types.rs` | `AgentId { Shell, Claude }`, `AGENTS` labels, `AgentCommands.claude`, `Settings.claude_bridge`, `claude_account` on panes, workspaces, folders, settings |
| `hangar-engine/src/state/lenient.rs` | `RETIRED_AGENTS`: a pane saved as `codex` opens as a shell — **must read `codex` as Codex again** |
| `hangar-engine/src/state/roles.rs` | "an orchestration is one Claude Code pane pinned as its orchestrator" |
| `hangar-engine/src/panes/agents.rs` | `DEFAULT_CLAUDE_COMMAND`, `launch_command` (Claude's flags: `--resume`, `--name`, `--model`, `--permission-mode`, `--allowedTools`, `--mcp-config`/`--strict-mcp-config`, `--add-dir`), `is_resumable`, `is_trust_prompt` (Claude's trust screen) |
| `hangar-engine/src/host/{launch,mod,terminals}.rs` | the windowless launch: bridge, account, MCP scope for `AgentId::Claude` only; `workspace_create` makes one Claude Code pane (`host/mod.rs:550`) — **must work without Claude** |
| `hangar-app/src/panes/{controller,launch,mod,header,look,waves}.rs` | the desktop launch (same decisions as `host/launch.rs`, a second copy), Shift+Enter mapping for Claude, resume waves sized for Claude |
| `hangar-core/src/pty.rs` | scrubs `CLAUDE_CODE_*`/`CLAUDECODE`/`CLAUDE_PID` from panes, `HANGAR_MCP_CONFIG`, `CLAUDE_CODE_PLUGIN_DIRS`, `CLAUDE_CONFIG_DIR` per account |
| `hangar-core/src/pty/jobs.rs` | "a job of its own between two turns": shells started by the agent (Claude's `bash -c`) |
| `hangar-core/src/path_env.rs` | PATH repair for `~/.local/bin` (Claude's installer) |
| `hangar-app/src/shell/setup/{mod,install,nixos,view}.rs`, `shell/overlay.rs` (`Overlay::ClaudeSetup`), `services/core.rs`, `app.rs` | the setup sheet that stands in the way until `claude` is installed; Anthropic's installer; nixpkgs; `shellsOnly` — **must work without Claude** |
| `hangar-app/src/workspace_dialog/{folder,view,mod}.rs` | `Start::Orchestrator => [AgentId::Claude]`, the agent picker offers Claude Code or a shell — **must work without Claude** |
| `hangar-app/src/settings/{schema,change,discord}.rs` | one launch command row (Claude's), the bridge switch, the Claude setup row; the Rich Presence counts Claude Code panes |
| `hangar-app/src/pane_menu/`, `toolbar/{menus,per_window}.rs`, `session_picker/`, `navigator/{menu,interaction}.rs` | "Claude Code" entries, pin-as-orchestrator only on a Claude Code pane |
| `hangar-node/src/api.rs` (`/node/info` reports `claude` on PATH), `hangar-node/src/own.rs` (a node workspace starts one Claude Code) | — **must work without Claude** |
| `hangar-discord/src/{bot,hangar}.rs` | `/workspace` opens one Claude Code pane; the bot follows "the first Claude Code pane" — **must work without Claude** |

### A.2 Turn state, event delivery, steering (cap.: `turns`, `inject`, `briefing`)

| Place | What it assumes today |
|---|---|
| `hangar-core/claude-mod/hangar-bridge/**` | the Claude Code mod: hooks module (`session.start`, `turn.*`, `prompt.compose`, `classic.*`), inbox poll every 1.5 s, `$.prompt.submit`, `$.session.append` mid-turn, briefing in the system prompt |
| `hangar-core/src/bridge.rs` | installs the mod under `~/.hangar/mods/`, `CLAUDE_CODE_PLUGIN_DIRS`, Claude's managed-settings sideloading policy, the inbox (`Queued`), held asks, edits |
| `hangar-core/src/server.rs` | `/api/bridge/{pane}/{event,next,conflict,briefing,ask/{id}}` routes, called by the mod |
| `hangar-engine/src/panes/{pane_agent,agent_state,bridge_events,activity,pane_insight}.rs`, `live.rs` | `bridged` = the mod said hello; guesses from output stand down then |
| `hangar-engine/src/control/panes.rs` | `pane_create {prompt}` and `pane_send {when_idle}` only for Claude Code; the paste-then-Enter delay is tuned for Claude's input |
| `hangar-app/src/kit_tabs/api.rs` | `panes.send` reaches a bridged Claude Code only |
| `hangar-app/src/panes_logic/mod.rs`, `control/live.rs`, `orchestration/live.rs`, `saved_commands/mod.rs`, `git_flow/deps.rs` | `bridge_enqueue` for a bridged pane, keys typed otherwise |
| `hangar-term/src/osc.rs`, `terminal.rs` | OSC 777 `notify;Claude;…`, synchronized updates as Claude draws them (generic code, Claude in tests/comments) |

### A.3 Sessions, resume, transcripts, hibernation (cap.: `sessions`, `transcript`)

| Place | What it assumes today |
|---|---|
| `hangar-core/src/sessions.rs` | `claude_installed`, Claude's `~/.claude/projects/<slug>/*.jsonl` only (Samuel's task brings Codex rollouts back) |
| `hangar-engine/src/panes/session_watch.rs` | the session hunt: diff of Claude's transcript folder |
| `hangar-engine/src/panes/{hibernate,hibernation}.rs` | resumable agent = Claude Code; "at rest" read from the mod |
| `hangar-app/src/session_picker/`, `panes/waves.rs`, `panes_logic/remote_transcripts.rs`, `nodes/chat.rs`, `hangar-node/src/transcripts.rs` | Claude transcripts, local and on a node |

### A.4 Context gauge, plan limits (cap.: `usage`)

| Place | What it assumes today |
|---|---|
| `hangar-core/src/context.rs` | `usage` of Claude's assistant lines (Samuel's task adds Codex `token_count`) |
| `hangar-engine/src/panes/context_gauge.rs` | Claude models' windows (200k / 1M) |
| `hangar-app/src/navigator/plan.rs`, `accounts/plan.rs`, `inspector/{format,sections,view}.rs` | the plan's 5-hour and weekly windows as the mod reports them |

### A.5 Statistics and pricing (cap.: `transcript`)

`hangar-core/src/{stats.rs,stats/parse.rs,stats/aggregate.rs,pricing.rs}`
(Claude transcripts, Anthropic prices — Samuel's task),
`hangar-engine/src/stats/model.rs` (one "Claude Code" row, Claude model names),
`hangar-app/src/stats/{view,sessions}.rs`, `hangar-cloud-api/src/stats.rs`,
`hangar-core/src/cloud/sync.rs`.

### A.6 Chat view, Hangar Mobile, Discord (cap.: `transcript`, `approvals`, `inject`)

| Place | What it assumes today |
|---|---|
| `hangar-core/src/chat/{mod,thread,tools}.rs` | Claude's transcript lines made chat items; `AskUserQuestion` |
| `hangar-engine/src/mobile/{answer,held,desk,ops,chat_live,calls,contract,engine}.rs` | answers through the mod's held ask, or keys typed into Claude Code 2.1's dialogs, checked on screen |
| `hangar-app/src/chat_view/conversation/*` | "a Claude Code pane running hidden behind the view"; `Blocked::NotClaude` (and `ChatReason::NotClaude` in `mobile/{desk,chat_live}.rs`) |
| `hangar-app/src/needs_you/` | waits reported by the mod |
| `hangar-discord/src/{bot,follow,cards,hangar}.rs` | follows Claude Code turns |

### A.7 Orchestration (cap.: `briefing`, `inject`, `turns`)

`hangar-engine/src/orchestration/{brief,brief_text,worker,projects}.rs` (roles
written for "this Claude Code session", a worker is "a Claude Code pane",
workers refused when `claude_bridge` is off — **must work without Claude**),
`hangar-core/src/orchestrator.rs`, `hangar-engine/src/control/{manager,workspace}.rs`,
`hangar-app/src/orchestration_ui/{actions,form,panel}.rs` (pin only a Claude
Code pane; model aliases sonnet/opus/haiku), `hangar-core/src/mcp.rs` (tool
schemas: `agent` enum `claude|shell`, Claude wording).

### A.8 Deliveries (cap.: `headless`)

`hangar-core/src/delivery.rs`: the reviewer is `claude -p` with
`Read,Grep,Glob`, Claude's environment scrubbed, Claude's usage-limit wording
— **must work without Claude**. `hangar-engine/src/control/delivery.rs`.

### A.9 Accounts (cap.: `accounts`)

`hangar-core/src/accounts.rs`, `accounts/{share,login,link,store}.rs`
(`CLAUDE_CONFIG_DIR` profiles, `.claude.json`, Keychain on macOS),
`hangar-engine/src/accounts.rs`, `hangar-app/src/accounts/*` (« Comptes
Claude »), `hangar-app/src/git_ui/worktrees/mod.rs` (a worktree's workspace
keeps its Claude account).

### A.10 MCP (cap.: `mcp`)

`hangar-core/src/mcp_scope.rs` (reads Claude's three scopes, writes the
`--mcp-config` file), `hangar-core/src/install.rs` (`.mcp.json` only),
`hangar-app/src/mcp/{install_panel,scope,servers_panel}.rs`.

### A.11 Instructions (`AgentInfo::instructions_file`)

`hangar-core/src/instructions.rs` (the playbook in `CLAUDE.md` only),
`hangar-core/src/store.rs` (keeps a `CLAUDE.md` → `AGENTS.md` link a link).

### A.12 Kits (cap.: `extensions`)

`hangar-core/src/kits.rs`, `kits/{plugin,hooks,code,manifest,store,data}.rs`
(a kit carries a Claude Code plugin, loaded through `CLAUDE_CODE_PLUGIN_DIRS`,
`${CLAUDE_PLUGIN_ROOT}`), `hangar-app/src/kits/*`, `hangar-kit-host/src/{api,gate}.rs`,
`hangar-kit-ui/src/api.rs`, `docs/kits.md`.
