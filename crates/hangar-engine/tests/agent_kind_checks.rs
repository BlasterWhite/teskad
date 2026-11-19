//! The rule of docs/native/agents.md §3.3 (D3), held: **features read
//! capabilities, never the agent kind.** Only the agent modules — the kinds'
//! rows and drivers in `hangar-core/src/agent/`, and the files kept per kind
//! (sessions, transcripts, accounts, installers, reviewers, delegate
//! runners) — and the state reader that maps persisted ids name a kind.
//! Everything else reads `hangar_core::agent::info(kind).caps` or the pane's
//! live capabilities (`hangar-engine/src/live.rs`), so that it works for
//! Claude Code, for Codex alone, and for a third agent later.
//!
//! The test reads the Rust sources of the crates that run agents
//! ([`CRATES`]; Mobile's server is `hangar-engine/src/mobile/`) and fails on
//! each line outside [`ALLOWED`] that names a kind: `AgentId::Claude`,
//! `AgentKind::Codex`, `AgentKind::default()` (Claude Code), an import of the
//! variants, an `is_claude()`-style helper, or a comparison with `"claude"`
//! or `"codex"`. Comments, strings and test code — `#[cfg(test)]` items,
//! `#[test]` functions, modules declared under `#[cfg(test)]` — are not
//! read: a test names the kinds it runs.
//!
//! The sources are read as text, comments and strings blanked and test items
//! cut out first: a lint for code rustfmt has laid out, not a parser.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

/// The crates whose sources the rule covers. Mobile's server is
/// `hangar-engine/src/mobile/`; the page it serves is not Rust.
const CRATES: [&str; 5] = [
    "hangar-core",
    "hangar-engine",
    "hangar-app",
    "hangar-node",
    "hangar-discord",
];

/// What a failure cites.
const RULE: &str =
    "docs/native/agents.md §3.3 (D3): features read capabilities, never the agent kind";

/// The ids of the kinds: the strings a comparison with a kind's id holds,
/// and what `<kind>` stands for in [`Allowed::path`].
const KIND_IDS: [&str; 2] = ["claude", "codex"];

/// What names a kind in a line of code, and how a failure says it.
const CHECKS: [(&str, &str); 6] = [
    (
        r"\bAgent(?:Id|Kind)\s*::\s*(?:Claude|Codex)\b",
        "names a kind",
    ),
    (
        r"\bAgentKind\s*::\s*default\s*\(\s*\)",
        "names the default kind (Claude Code)",
    ),
    (
        r"\bAgent(?:Id|Kind)\s*::\s*(?:\*|\{[^}]*\b(?:Claude|Codex)\b)",
        "brings the kinds' names in",
    ),
    (r"\bis_(?:claude|codex)\w*\s*\(", "asks for a kind"),
    (
        r#"(?:==|!=)\s*"(?:claude|codex)"|"(?:claude|codex)"\s*(?:==|!=|=>|\|)"#,
        "compares with a kind's id",
    ),
    (
        r#"\bfrom_id\s*\(\s*"(?:claude|codex)""#,
        "names a kind by its id",
    ),
];

/// A place allowed to name a kind, and why.
struct Allowed {
    /// A file, or a folder when it ends with `/`, from the workspace's root;
    /// `<kind>` stands for each kind's id (`claude`, `codex`).
    path: &'static str,
    /// The lines allowed, as rustfmt writes them (trimmed); empty: the
    /// whole file.
    lines: &'static [&'static str],
    /// One line. A place that should read a capability says
    /// `TODO(capability): …` and is allowed by its lines only, so that a
    /// new kind check beside it still fails.
    reason: &'static str,
}

/// All of `path`.
const fn whole(path: &'static str, reason: &'static str) -> Allowed {
    Allowed {
        path,
        lines: &[],
        reason,
    }
}

/// The `lines` of `path`.
const fn only(path: &'static str, lines: &'static [&'static str], reason: &'static str) -> Allowed {
    Allowed {
        path,
        lines,
        reason,
    }
}

/// Where a kind may be named. A new entry is a kind's own module, or a line
/// with a reason no capability can give; anything else reads a capability.
const ALLOWED: &[Allowed] = &[
    // The agent modules (§3.4).
    whole(
        "crates/hangar-core/src/agent.rs",
        "the agent module: AgentKind and each kind's row (D2)",
    ),
    whole(
        "crates/hangar-core/src/agent/",
        "the agent modules: each kind's flags, launch, detection and side channel (D4)",
    ),
    whole(
        "crates/hangar-core/src/accounts.rs",
        "accounts per kind (D16): Claude Code's profiles, Codex's shadow homes",
    ),
    whole(
        "crates/hangar-core/src/accounts/",
        "accounts per kind (D16): where each kind's accounts are kept and logged in",
    ),
    whole(
        "crates/hangar-core/src/sessions.rs",
        "sessions per kind: the folder and the format a kind's sessions are in",
    ),
    whole(
        "crates/hangar-core/src/sessions/<kind>.rs",
        "a kind's session reader",
    ),
    whole(
        "crates/hangar-core/src/context.rs",
        "the context gauge per kind: a Claude Code transcript line or a Codex rollout line",
    ),
    whole(
        "crates/hangar-core/src/stats.rs",
        "statistics per kind: each kind's transcripts and the figures in them",
    ),
    whole(
        "crates/hangar-core/src/stats/",
        "statistics per kind: each kind's transcript parser",
    ),
    whole(
        "crates/hangar-core/src/chat/<kind>.rs",
        "a kind's conversation, read as chat items (T9)",
    ),
    whole(
        "crates/hangar-core/src/delegate/<kind>.rs",
        "a kind's delegate runner (T11a, T11b)",
    ),
    whole(
        "crates/hangar-core/src/delivery/review_agent.rs",
        "the reviewer per kind: which program reviews a delivery (T6)",
    ),
    whole(
        "crates/hangar-core/src/delivery/review_<kind>.rs",
        "a kind's reviewer (T6)",
    ),
    whole(
        "crates/hangar-core/src/install/agent.rs",
        "each kind's official installer and the folders it installs to",
    ),
    whole(
        "crates/hangar-core/src/kits/plugin.rs",
        "kit parts per kind (T7): what Claude Code's plugins and Codex take of a kit",
    ),
    whole(
        "crates/hangar-core/src/kits/<kind>_plugin.rs",
        "a kit in a kind's own plugin system (§6.4)",
    ),
    whole(
        "crates/hangar-engine/src/state/types.rs",
        "the persisted model: AgentId and its kind, the fields kept per kind, D20's automatic agent",
    ),
    whole(
        "crates/hangar-engine/src/state/lenient.rs",
        "the state reader: maps the persisted agent ids (§3.3)",
    ),
    whole(
        "crates/hangar-app/src/shell/setup/",
        "the setup sheet installs and signs in each kind (D17): installers, sign-in panes, slots",
    ),
    whole(
        "crates/hangar-node/src/transcripts.rs",
        "a node's transcript routes per kind (§6.8): each kind's reader, Claude Code's when none is named",
    ),
    // Lines that map a kind to what is its own, with no capability to read.
    only(
        "crates/hangar-core/src/kits.rs",
        &[
            ".filter(|kit| kit.manifest.meant_for(AgentKind::Claude))",
            "if let Some(program) = crate::install::agent::find(AgentKind::Codex) {",
            "crate::install::agent::find(AgentKind::Codex)",
            "crate::install::agent::installed(AgentKind::Codex),",
        ],
        "a kit goes into each kind's own plugin system: Claude Code's plugin dirs, Codex's plugin (T7)",
    ),
    only(
        "crates/hangar-core/src/kits/manifest.rs",
        &["return vec![AgentKind::Claude];"],
        "a manifest without `agents` was written for Claude Code alone",
    ),
    only(
        "crates/hangar-core/src/mcp_scope.rs",
        &[
            "let kind = Some(AgentKind::Codex.id());",
            "let program = crate::install::agent::find(AgentKind::Codex)",
        ],
        "asks Codex itself which MCP servers it starts (`codex mcp list`, T7)",
    ),
    only(
        "crates/hangar-engine/src/mobile/answer.rs",
        &[
            "AgentKind::Claude => Dialogs::ClaudeCode,",
            "AgentKind::Codex => Dialogs::Codex,",
        ],
        "each kind's TUI draws its own dialogs: the screens Mobile checks before a key (T18)",
    ),
    only(
        "crates/hangar-engine/src/mobile/ops.rs",
        &[
            "AgentKind::Claude => SideChannelSetting {",
            "AgentKind::Codex => SideChannelSetting {",
        ],
        "the side channel's switch is a setting kept once per kind, read here once (T18)",
    ),
    only(
        "crates/hangar-engine/src/stats/model.rs",
        &["AgentKind::Codex", "AgentKind::default()"],
        "a model id names its maker's agent, for figures that carry none: OpenAI's run in Codex",
    ),
    only(
        "crates/hangar-app/src/accounts/mod.rs",
        &["pub const BETA: &[AgentKind] = &[AgentKind::Codex];"],
        "Codex's accounts are labelled « beta » (§7, decision 5)",
    ),
    only(
        "crates/hangar-app/src/accounts/mod.rs",
        &["if kind == AgentKind::default() {"],
        "Claude Code's accounts keep their place from before Codex: unprefixed keys, a section always shown",
    ),
    only(
        "crates/hangar-app/src/accounts/mod.rs",
        &[
            "AgentKind::Claude => self.accounts.as_deref(),",
            "AgentKind::Codex => self.codex.as_deref(),",
            "AgentKind::Claude => &mut self.accounts,",
            "AgentKind::Codex => &mut self.codex,",
        ],
        "the account lists kept once per kind",
    ),
    only(
        "crates/hangar-app/src/accounts/plan.rs",
        &[
            "*kind == AgentKind::default() || account.logged_in || figures(*kind, account).is_some()",
        ],
        "Claude Code's accounts count as they did before Codex; another kind's once used (T25)",
    ),
    only(
        "crates/hangar-app/src/accounts/switcher.rs",
        &[".filter(|(kind, accounts)| *kind == AgentKind::default() || accounts.len() > 1)"],
        "Claude Code's accounts are listed as before Codex; another kind's once it has two (T25)",
    ),
    only(
        "crates/hangar-app/src/accounts/menu_items.rs",
        &[".filter(|(kind, accounts)| *kind == AgentKind::default() || accounts.len() > 1)"],
        "Claude Code's accounts are listed as before Codex; another kind's once it has two (T25)",
    ),
    only(
        "crates/hangar-app/src/orchestration_ui/panel.rs",
        &[
            "let names = suggestions(ctx, AgentKind::Claude, move |name| {",
            "let names = suggestions(ctx, AgentKind::Codex, move |name| {",
        ],
        "the model fields kept once per kind, each with its kind's model names",
    ),
    only(
        "crates/hangar-app/src/services/core.rs",
        &["codex: hangar_core::agent::detect::installed(hangar_core::agent::AgentKind::Codex),"],
        "the detection at start fills the setup sheet's slot of each kind",
    ),
    only(
        "crates/hangar-app/src/settings/schema.rs",
        &[
            "beta(t, AgentKind::Codex.info().label)",
            "agent_choice_id(Some(AgentKind::Claude)),",
            "AgentKind::Claude.info().label,",
            "Choice::new(agent_choice_id(Some(AgentKind::Codex)), codex_label(t)),",
            "Some(AgentId::Claude.info().label.to_string()),",
            "placeholder: AgentKind::Codex.info().program.into(),",
            "AgentId::Claude.info().label.to_string(),",
        ],
        "settings rows kept once per kind (commands, default agents, setup), Codex's « beta » (D21)",
    ),
    only(
        "crates/hangar-node/src/api.rs",
        &[
            "agent(&node.versions, AgentKind::Claude),",
            "agent(&node.versions, AgentKind::Codex)",
        ],
        "`/node/info` reports each kind under its own key, `claude` and `codex` (§6.8)",
    ),
    only(
        "crates/hangar-app/src/nodes/chat.rs",
        &["if kind != AgentKind::Claude {"],
        "a node's routes read Claude Code's when a request names no kind, as before Codex (§6.8)",
    ),
    only(
        "crates/hangar-app/src/nodes/model.rs",
        &[
            "AgentKind::Claude => &self.agents.claude,",
            "AgentKind::Codex => &self.agents.codex,",
            "installed: kind == AgentKind::Claude && self.claude,",
        ],
        "a node's agents kept once per kind; one from before Codex said whether Claude Code is there",
    ),
    only(
        "crates/hangar-app/src/nodes/page.rs",
        &[
            "AgentKind::Claude => (Key::NodesClaudeFound, Key::NodesClaudeMissing),",
            "AgentKind::Codex => (Key::NodesCodexFound, Key::NodesCodexMissing),",
            "matches!(kind, AgentKind::Codex)",
        ],
        "each kind's own strings on a node's row, Codex's « beta » (§7, decision 5)",
    ),
    // Features that name a kind where they should read a capability.
    only(
        "crates/hangar-core/src/pty.rs",
        &["let (server, state) = if attach && request.kind == AgentKind::Codex {"],
        "TODO(capability): the pane's own app-server starts by kind; read `caps.headless.app_server`",
    ),
    only(
        "crates/hangar-engine/src/accounts.rs",
        &[
            "Self::of_kind(state, ws, pane, AgentKind::Claude)",
            "self.launch_of(AgentKind::Claude)",
        ],
        "TODO(capability): `of` and `launch` are Claude Code's whatever the pane runs; pass its kind",
    ),
    only(
        "crates/hangar-engine/src/host/mod.rs",
        &[
            "let says_itself = info.agent == AgentId::Claude && model.store.settings().claude_bridge;",
        ],
        "TODO(capability): whether the side channel tells the session is `caps.sessions.id` (Reported)",
    ),
    only(
        "crates/hangar-app/src/panes_logic/mod.rs",
        &["let says_itself = info.agent == AgentId::Claude && m.settings().claude_bridge;"],
        "TODO(capability): whether the side channel tells the session is `caps.sessions.id` (Reported)",
    ),
    only(
        "crates/hangar-app/src/panes_logic/mod.rs",
        &[".is_some_and(|(_, found)| found.agent == AgentId::Claude);"],
        "TODO(capability): the context gauge is kept for Claude Code panes only; read `caps.usage.context`",
    ),
    only(
        "crates/hangar-app/src/git_flow/deps.rs",
        &["agents: vec![AgentId::Claude],"],
        "TODO(capability): a clone's workspace starts Claude Code; start the default orchestrator agent (D20)",
    ),
    only(
        "crates/hangar-app/src/mcp/servers_panel.rs",
        &[".filter(|pane| pane.agent == AgentId::Claude)"],
        "TODO(capability): only Claude Code panes are counted and restarted for an MCP change; read `caps.mcp`",
    ),
    only(
        "crates/hangar-app/src/mobile/live.rs",
        &["hangar_core::agent::AgentKind::Claude,"],
        "TODO(capability): Mobile reads a node pane's chat as Claude Code's whatever it runs; pass its kind",
    ),
    only(
        "crates/hangar-app/src/panes_logic/remote_transcripts.rs",
        &["remote::context(&client, AgentKind::Claude, &session, &cwd).await"],
        "TODO(capability): a node pane's context is read as Claude Code's whatever it runs; pass its kind",
    ),
    only(
        "crates/hangar-app/src/accounts/mod.rs",
        &[
            "picker::view(ctx, AgentKind::Claude, value, folder, |id| {",
            "has_any_of(ctx, AgentKind::Claude)",
        ],
        "TODO(capability): the new-workspace dialog's account picker offers Claude Code's accounts only",
    ),
    only(
        "crates/hangar-app/src/settings/discord.rs",
        &["let claude = AgentId::Claude;"],
        "TODO(capability): the Discord presence names Claude Code whatever runs; name the agents running",
    ),
];

impl Allowed {
    /// Whether `finding` is in this place.
    fn covers(&self, finding: &Finding) -> bool {
        let here = KIND_IDS.iter().any(|kind| {
            let path = self.path.replace("<kind>", kind);
            if path.ends_with('/') {
                finding.path.starts_with(&path)
            } else {
                finding.path == path
            }
        });
        here && (self.lines.is_empty() || self.lines.iter().any(|line| *line == finding.text))
    }
}

/// A line of code that names a kind.
struct Finding {
    /// From the workspace's root, with `/`.
    path: String,
    line: usize,
    what: &'static str,
    /// The line, trimmed.
    text: String,
}

/// The expressions the scan reads with, built once.
struct Patterns {
    /// An outer attribute's opening, `#[`.
    attribute: Regex,
    /// A file that is a test as a whole: `#![cfg(test)]`.
    test_file: Regex,
    /// A module declared from a file: `mod name;`.
    module: Regex,
    /// A module whose file a `#[path]` names; group 1 the path, group 2 its
    /// `mod`.
    path_module: Regex,
    /// [`CHECKS`], compiled.
    checks: Vec<(Regex, &'static str)>,
}

impl Patterns {
    fn new() -> Patterns {
        let compile = |pattern: &str| Regex::new(pattern).expect("a valid expression");
        Patterns {
            attribute: compile(r"#\s*\["),
            test_file: compile(r"#!\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]"),
            module: compile(r"\bmod\s+(\w+)\s*;"),
            path_module: compile(
                r#"#\[\s*path\s*=\s*"([^"]+)"\s*\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\s*\([^)]*\))?\s+)?(mod)\s+\w+\s*;"#,
            ),
            checks: CHECKS
                .iter()
                .map(|&(pattern, what)| (compile(pattern), what))
                .collect(),
        }
    }
}

/// A source file as the scan reads it.
struct Source {
    /// Its text, lines ending in `\n`.
    text: String,
    /// The text with its comments and strings blanked ([`blank`]).
    code: String,
    /// `code` with its test code blanked too: what the rule reads.
    live: String,
    /// Whether the file says it is a test as a whole (`#![cfg(test)]`).
    test_file: bool,
    /// The modules it declares from files: name, and where their `mod` is.
    modules: Vec<(String, usize)>,
}

impl Source {
    fn read(text: &str, patterns: &Patterns) -> Source {
        let text = text.replace("\r\n", "\n");
        let code = blank(&text);
        let test_file = patterns.test_file.is_match(&code);
        let live = if test_file {
            let mut all = code.clone().into_bytes();
            let end = all.len();
            spaces(&mut all, 0, end);
            String::from_utf8(all).expect("ASCII")
        } else {
            strip_tests(&code, &patterns.attribute)
        };
        let modules = patterns
            .module
            .captures_iter(&code)
            .filter_map(|found| Some((found.get(1)?.as_str().to_string(), found.get(0)?.start())))
            .collect();
        Source {
            text,
            code,
            live,
            test_file,
            modules,
        }
    }

    /// The lines of its live code that name a kind, first to last: number
    /// and what names it.
    fn kind_checks(&self, patterns: &Patterns) -> Vec<(usize, &'static str)> {
        let mut found = Vec::new();
        for (check, what) in &patterns.checks {
            for at in check.find_iter(&self.live) {
                let line = 1 + self.live.as_bytes()[..at.start()]
                    .iter()
                    .filter(|byte| **byte == b'\n')
                    .count();
                found.push((line, at.start(), *what));
            }
        }
        found.sort();
        found.dedup_by_key(|(line, _, _)| *line);
        found
            .into_iter()
            .map(|(line, _, what)| (line, what))
            .collect()
    }
}

/// Whether `byte` may be part of an identifier.
fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Blanks `bytes[from..to]`: a space for each byte, line ends kept, so that
/// offsets and line numbers stay where they were.
fn spaces(bytes: &mut [u8], from: usize, to: usize) {
    let to = to.min(bytes.len());
    for byte in bytes.iter_mut().take(to).skip(from) {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

/// Where the raw string whose quote opens at `quote`, with `hashes` `#`s,
/// closes: its closing quote, or the end.
fn raw_string_end(bytes: &[u8], quote: usize, hashes: usize) -> usize {
    (quote + 1..bytes.len())
        .find(|&at| {
            bytes[at] == b'"'
                && bytes
                    .get(at + 1..at + 1 + hashes)
                    .is_some_and(|tail| tail.iter().all(|byte| *byte == b'#'))
        })
        .unwrap_or(bytes.len())
}

/// `src` with its comments and the contents of its strings and characters
/// blanked ([`spaces`]), but for a string that is a kind's id: what a
/// comparison with one reads.
fn blank(src: &str) -> String {
    let bytes = src.as_bytes();
    let n = bytes.len();
    let mut out = bytes.to_vec();
    let kept = |from: usize, to: usize| {
        KIND_IDS
            .iter()
            .any(|id| bytes.get(from..to) == Some(id.as_bytes()))
    };
    let mut i = 0;
    while i < n {
        let rest = &bytes[i..];
        let after_ident = i > 0 && is_ident(bytes[i - 1]);
        if rest.starts_with(b"//") {
            let end = rest
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(n, |at| i + at);
            spaces(&mut out, i, end);
            i = end;
            continue;
        }
        if rest.starts_with(b"/*") {
            let (mut depth, mut j) = (1, i + 2);
            while j < n && depth > 0 {
                if bytes[j..].starts_with(b"/*") {
                    depth += 1;
                    j += 2;
                } else if bytes[j..].starts_with(b"*/") {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            spaces(&mut out, i, j);
            i = j;
            continue;
        }
        // A raw string: r"…", r#"…"#, br"…".
        let prefix = if rest.starts_with(b"br") {
            2
        } else if rest.starts_with(b"r") {
            1
        } else {
            0
        };
        if prefix > 0 && !after_ident {
            let hashes = rest[prefix..]
                .iter()
                .take_while(|byte| **byte == b'#')
                .count();
            let quote = i + prefix + hashes;
            if bytes.get(quote) == Some(&b'"') {
                let end = raw_string_end(bytes, quote, hashes);
                if !kept(quote + 1, end) {
                    spaces(&mut out, quote + 1, end);
                }
                i = (end + 1 + hashes).min(n);
                continue;
            }
        }
        match bytes[i] {
            b'"' => {
                let mut j = i + 1;
                while j < n && bytes[j] != b'"' {
                    j += if bytes[j] == b'\\' { 2 } else { 1 };
                }
                let j = j.min(n);
                if !kept(i + 1, j) {
                    spaces(&mut out, i + 1, j);
                }
                i = j + 1;
            }
            b'\'' if bytes.get(i + 1) == Some(&b'\\') => {
                // An escaped character: '\n', '\'', '\u{…}'.
                let end = (i + 3..n).find(|&at| bytes[at] == b'\'').unwrap_or(n);
                spaces(&mut out, i + 1, end);
                i = end + 1;
            }
            b'\'' => {
                // A character, or else a lifetime ('a, 'static).
                let width = src[i + 1..].chars().next().map_or(0, char::len_utf8);
                if width > 0 && bytes.get(i + 1 + width) == Some(&b'\'') {
                    spaces(&mut out, i + 1, i + 1 + width);
                    i += 2 + width;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).expect("whole characters blanked")
}

/// Whether an attribute (its text between `#[` and `]`) makes its item test
/// code: `#[test]`, `#[tokio::test]`, or a `cfg` that holds only with
/// `test` (`cfg(test)`, `cfg(all(test, unix))`, not `cfg(not(test))`).
fn is_test_attribute(body: &str) -> bool {
    let body: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let head = body.split_once('(').map_or(body.as_str(), |(head, _)| head);
    if head == "test" || head.ends_with("::test") {
        return true;
    }
    if head != "cfg" {
        return false;
    }
    let body = body.replace("not(test)", "");
    let bytes = body.as_bytes();
    body.match_indices("test").any(|(at, _)| {
        (at == 0 || !is_ident(bytes[at - 1]))
            && bytes.get(at + 4).is_none_or(|byte| !is_ident(*byte))
    })
}

/// Where the attribute whose `[` is at `open` ends: after its `]`.
fn attribute_end(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0;
    for (at, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return at + 1;
                }
            }
            _ => {}
        }
    }
    bytes.len()
}

/// Where the item that starts at `from` ends: after its `;` or its block.
fn item_end(bytes: &[u8], from: usize) -> usize {
    let mut depth = 0i32;
    for (at, byte) in bytes.iter().enumerate().skip(from) {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if *byte == b'}' && depth <= 0 {
                    return at + 1;
                }
                if depth < 0 {
                    return at;
                }
            }
            b';' if depth == 0 => return at + 1,
            _ => {}
        }
    }
    bytes.len()
}

/// `code` with its test items blanked ([`is_test_attribute`]), their other
/// attributes with them.
fn strip_tests(code: &str, attribute: &Regex) -> String {
    let bytes = code.as_bytes();
    let mut out = bytes.to_vec();
    let mut at = 0;
    while let Some(found) = attribute.find_at(code, at) {
        let end = attribute_end(bytes, found.end() - 1);
        let body = code.get(found.end()..end.saturating_sub(1)).unwrap_or("");
        if !is_test_attribute(body) {
            at = end;
            continue;
        }
        let mut item = end;
        loop {
            let next = item
                + bytes[item..]
                    .iter()
                    .take_while(|byte| byte.is_ascii_whitespace())
                    .count();
            match attribute.find_at(code, next) {
                Some(more) if more.start() == next => item = attribute_end(bytes, more.end() - 1),
                _ => break,
            }
        }
        let stop = item_end(bytes, item);
        spaces(&mut out, found.start(), stop);
        at = stop;
    }
    String::from_utf8(out).expect("ASCII blanks")
}

/// `path` from `dir`, both with `/`.
fn joined(dir: &str, path: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|part| !part.is_empty()).collect();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// The sources the rule covers, read.
struct Scan {
    /// By their path from the workspace's root, with `/`.
    sources: BTreeMap<String, Source>,
    /// The files a `#[path]` declares: the declaring file, and where its
    /// `mod` is.
    by_path: BTreeMap<String, (String, usize)>,
}

impl Scan {
    /// `files` (path from the workspace's root, text), read.
    fn new(files: Vec<(String, String)>, patterns: &Patterns) -> Scan {
        let sources: BTreeMap<String, Source> = files
            .into_iter()
            .map(|(path, text)| (path, Source::read(&text, patterns)))
            .collect();
        let mut by_path = BTreeMap::new();
        for (path, source) in &sources {
            let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
            for found in patterns.path_module.captures_iter(&source.text) {
                let (Some(whole), Some(file), Some(keyword)) =
                    (found.get(0), found.get(1), found.get(2))
                else {
                    continue;
                };
                // Code, not a comment that shows one.
                if source.code.as_bytes()[whole.start()] == b'#' {
                    by_path.insert(joined(dir, file.as_str()), (path.clone(), keyword.start()));
                }
            }
        }
        Scan { sources, by_path }
    }

    /// The crates of [`CRATES`], from this checkout.
    fn workspace(patterns: &Patterns) -> Scan {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("hangar-engine lies in crates/")
            .to_path_buf();
        let mut files = Vec::new();
        for name in CRATES {
            let mut paths = Vec::new();
            rust_files(&root.join("crates").join(name).join("src"), &mut paths);
            for path in paths {
                let text = fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                let relative: Vec<String> = path
                    .strip_prefix(&root)
                    .expect("under the root")
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect();
                files.push((relative.join("/"), text));
            }
        }
        Scan::new(files, patterns)
    }

    /// Where the module `path` is declared, when it is: the declaring file
    /// and where its `mod` is. By a `#[path]`, else in the file of its
    /// parent module, else in the one above when the parent is a block
    /// there (`mod tests { mod batches; }`).
    fn declaration(&self, path: &str) -> Option<(String, usize)> {
        if let Some(found) = self.by_path.get(path) {
            return Some(found.clone());
        }
        let parts: Vec<&str> = path.split('/').collect();
        // crates/<name>/src/…
        if parts.len() < 4 {
            return None;
        }
        let (base, module) = parts.split_at(3);
        let (name, dirs) = match module {
            ["lib.rs" | "main.rs"] | ["bin", ..] => return None,
            [dirs @ .., name, "mod.rs"] => (*name, dirs),
            [dirs @ .., file] => (file.strip_suffix(".rs")?, dirs),
            [] => return None,
        };
        let base = base.join("/");
        let ups = usize::from(!dirs.is_empty());
        for up in 0..=ups {
            let dirs = &dirs[..dirs.len() - up];
            let parents = if dirs.is_empty() {
                [format!("{base}/lib.rs"), format!("{base}/main.rs")]
            } else {
                let dir = dirs.join("/");
                [format!("{base}/{dir}.rs"), format!("{base}/{dir}/mod.rs")]
            };
            for parent in parents {
                if let Some(source) = self.sources.get(&parent)
                    && let Some((_, at)) =
                        source.modules.iter().find(|(declared, _)| declared == name)
                {
                    return Some((parent, *at));
                }
            }
        }
        None
    }

    /// Whether the file at `path` is test code: a test as a whole, or a
    /// module declared under `#[cfg(test)]`, or in one that is.
    fn test_only(&self, path: &str) -> bool {
        if self.sources[path].test_file {
            return true;
        }
        match self.declaration(path) {
            Some((parent, at)) => {
                self.test_only(&parent) || self.sources[&parent].live.as_bytes()[at] != b'm'
            }
            None => false,
        }
    }

    /// Every line of code outside the tests that names a kind.
    fn findings(&self, patterns: &Patterns) -> Vec<Finding> {
        let mut out = Vec::new();
        for (path, source) in &self.sources {
            if self.test_only(path) {
                continue;
            }
            for (line, what) in source.kind_checks(patterns) {
                let text = source.text.lines().nth(line - 1).unwrap_or_default();
                out.push(Finding {
                    path: path.clone(),
                    line,
                    what,
                    text: text.trim().to_string(),
                });
            }
        }
        out
    }
}

/// Every `.rs` file under `dir`.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("a folder's entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn agent_kind_checks_stay_in_agent_modules() {
    let patterns = Patterns::new();
    let outside: Vec<String> = Scan::workspace(&patterns)
        .findings(&patterns)
        .into_iter()
        .filter(|finding| !ALLOWED.iter().any(|allowed| allowed.covers(finding)))
        .map(|finding| {
            format!(
                "  {}:{}: {}: {}",
                finding.path, finding.line, finding.what, finding.text
            )
        })
        .collect();
    assert!(
        outside.is_empty(),
        "{} line(s) name an agent kind outside the agent modules — {RULE}.\n{}\n\
         Read the capability instead: `hangar_core::agent::info(kind).caps`, or the pane's live \
         capabilities (hangar-engine/src/live.rs). A kind's own module goes in ALLOWED \
         (crates/hangar-engine/tests/agent_kind_checks.rs), with a one-line reason.",
        outside.len(),
        outside.join("\n"),
    );
}

#[test]
fn each_allowed_place_says_why_in_one_line() {
    for allowed in ALLOWED {
        let (path, reason) = (allowed.path, allowed.reason);
        assert!(
            path.starts_with("crates/"),
            "{path}: from the workspace's root"
        );
        assert!(
            !reason.trim().is_empty() && !reason.contains('\n'),
            "{path}: one line of reason"
        );
        if reason.starts_with("TODO") {
            assert!(
                reason.starts_with("TODO(capability): "),
                "{path}: a place to fix says `TODO(capability): …`"
            );
            assert!(
                !allowed.lines.is_empty(),
                "{path}: a place to fix is allowed by its lines, not as a whole file"
            );
        }
    }
}

#[test]
fn the_scan_reads_every_crate_and_finds_the_kinds_where_they_are() {
    let patterns = Patterns::new();
    let scan = Scan::workspace(&patterns);
    for name in CRATES {
        let prefix = format!("crates/{name}/src/");
        assert!(
            scan.sources.keys().any(|path| path.starts_with(&prefix)),
            "no source read in {prefix}"
        );
    }
    let tests = scan
        .sources
        .keys()
        .filter(|path| scan.test_only(path))
        .count();
    assert!(
        tests > 0 && tests < scan.sources.len() / 2,
        "{tests} of {} files read as tests",
        scan.sources.len()
    );
    // The persisted model maps AgentId to its kind: an allowed place the scan
    // must see, or it sees nothing.
    let types = "crates/hangar-engine/src/state/types.rs";
    assert!(!scan.test_only(types));
    assert!(
        scan.findings(&patterns)
            .iter()
            .any(|finding| finding.path == types),
        "no kind named in {types}"
    );
}

#[test]
fn the_scan_reads_code_and_not_comments_strings_or_tests() {
    // Each line that ends with "// found" names a kind, and only those.
    let sample = [
        "//! AgentId::Claude in a doc comment",
        "/* AgentKind::Codex in a /* nested */ block comment */",
        "fn feature(pane: &Pane, kind: AgentKind) -> bool {",
        "    let note = \"AgentId::Claude in a string\";",
        "    let raw = r#\"AgentKind::Codex == \"codex\" in a raw string\"#;",
        "    let quote = '\"';",
        "    let tick: &'static str = \"'\";",
        "    if pane.agent == AgentId::Claude { // found",
        "        return true;",
        "    }",
        "    match kind {",
        "        AgentKind::Codex => false, // found",
        "        _ => kind == AgentKind::default(), // found",
        "    }",
        "}",
        "fn helper(kind: AgentKind) -> bool { kind.is_claude() } // found",
        "fn by_id(id: &str) -> bool { id == \"codex\" } // found",
        "fn named(id: &str) -> bool { id == \"claude code\" }",
        "fn parsed() -> Option<AgentKind> { AgentKind::from_id(\"codex\") } // found",
        "fn every() -> [AgentKind; 2] { AgentKind::ALL }",
        "use hangar_core::agent::AgentKind::{ // found",
        "    Claude,",
        "    Codex,",
        "};",
        "#[cfg(not(test))]",
        "fn not_a_test() -> AgentId { AgentId::Codex } // found",
        "#[cfg(test)]",
        "mod tests {",
        "    fn sample() -> AgentId { AgentId::Claude }",
        "}",
        "#[test]",
        "#[should_panic]",
        "fn a_test() { assert_eq!(kind(), AgentKind::Claude); }",
        "#[cfg(all(test, unix))]",
        "fn a_unix_test() -> AgentId { AgentId::Codex }",
        "#[cfg_attr(test, derive(Debug))]",
        "struct Shown(AgentId);",
        "const DEFAULT: AgentId = AgentId::Claude; // found",
    ];
    let expected: Vec<usize> = sample
        .iter()
        .enumerate()
        .filter(|(_, line)| line.ends_with("// found"))
        .map(|(at, _)| at + 1)
        .collect();
    let patterns = Patterns::new();
    let found: Vec<usize> = Source::read(&sample.join("\n"), &patterns)
        .kind_checks(&patterns)
        .into_iter()
        .map(|(line, _)| line)
        .collect();
    assert_eq!(found, expected);
}

#[test]
fn modules_declared_under_cfg_test_are_tests_and_the_others_are_not() {
    let files = [
        (
            "lib.rs",
            "mod feature;\n#[cfg(test)]\nmod tests;\nmod nodes;\n",
        ),
        (
            "feature.rs",
            "fn f(a: AgentId) -> bool { a == AgentId::Claude }\n#[cfg(test)]\nmod tests {\n    mod inner;\n}\n",
        ),
        ("tests.rs", "fn t() -> AgentId { AgentId::Claude }\n"),
        (
            "feature/tests/inner.rs",
            "fn t() -> AgentId { AgentId::Codex }\n",
        ),
        (
            "nodes.rs",
            "#[cfg(test)]\n#[path = \"nodes_tests.rs\"]\nmod tests;\n#[path = \"chat.rs\"]\nmod chat;\n",
        ),
        ("nodes_tests.rs", "fn t() -> AgentId { AgentId::Codex }\n"),
        (
            "chat.rs",
            "// windows line ends\r\nfn c(a: AgentId) -> bool {\r\n    a != AgentId::Codex\r\n}\r\n",
        ),
    ];
    let patterns = Patterns::new();
    let scan = Scan::new(
        files
            .iter()
            .map(|(path, text)| (format!("crates/x/src/{path}"), text.to_string()))
            .collect(),
        &patterns,
    );
    let tests: Vec<&str> = scan
        .sources
        .keys()
        .filter(|path| scan.test_only(path))
        .map(String::as_str)
        .collect();
    assert_eq!(
        tests,
        [
            "crates/x/src/feature/tests/inner.rs",
            "crates/x/src/nodes_tests.rs",
            "crates/x/src/tests.rs",
        ]
    );
    let found: Vec<(String, usize)> = scan
        .findings(&patterns)
        .into_iter()
        .map(|finding| (finding.path, finding.line))
        .collect();
    assert_eq!(
        found,
        [
            ("crates/x/src/chat.rs".to_string(), 3),
            ("crates/x/src/feature.rs".to_string(), 1),
        ]
    );
}

#[test]
fn an_allowed_place_covers_its_file_its_folder_or_its_lines() {
    let finding = |path: &str, text: &str| Finding {
        path: path.to_string(),
        line: 1,
        what: "names a kind",
        text: text.to_string(),
    };
    let folder = whole("crates/a/src/agent/", "a reason");
    assert!(folder.covers(&finding("crates/a/src/agent/codex.rs", "x")));
    assert!(!folder.covers(&finding("crates/a/src/agents.rs", "x")));
    let per_kind = whole("crates/a/src/chat/<kind>.rs", "a reason");
    assert!(per_kind.covers(&finding("crates/a/src/chat/codex.rs", "x")));
    assert!(per_kind.covers(&finding("crates/a/src/chat/claude.rs", "x")));
    assert!(!per_kind.covers(&finding("crates/a/src/chat/tools.rs", "x")));
    let lines = only(
        "crates/a/src/look.rs",
        &["AgentKind::Codex => 1,"],
        "a reason",
    );
    assert!(lines.covers(&finding("crates/a/src/look.rs", "AgentKind::Codex => 1,")));
    assert!(!lines.covers(&finding("crates/a/src/look.rs", "AgentKind::Codex => 2,")));
}
