//! What a kit's Claude Code plugin runs by itself: its hooks, read from
//! `hooks/hooks.json` and from `hooks` in `.claude-plugin/plugin.json`.
//!
//! The person sees this list before enabling a kit, so the reading leans one
//! way: whatever may run gets listed. A hook that is not a `command` (a prompt,
//! a URL, a type a newer Claude Code knows) is written out as its JSON, a
//! `modules` entry (a hooks module, code Claude Code loads) by its path, and a
//! file this cannot read as it stands, under the file's name: Claude Code may
//! read what this does not, and nothing should run unseen.
//!
//! The list's hash is what a workspace accepts (docs/kits.md): the pseudo
//! permission `hooks:<first 16 hex of its SHA-256>`.
//!
//! The same hooks reach Codex in the Codex plugin Hangar builds from the kit
//! (docs/native/agents.md §6.4, D18), as far as Codex runs them:
//! [`not_for_codex`] says which do not, and why, so the kit's activation
//! lists them as Claude Code's only. What Codex runs was read in its source at
//! the release `agent::codex::CHECKED_AT` names (paths under `codex-rs/`). It
//! hands a plugin's hooks `CLAUDE_PLUGIN_ROOT` and `CLAUDE_PLUGIN_DATA` as
//! Claude Code does (`hooks/src/engine/discovery.rs`), so a command written
//! with them runs there as it is.

use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use super::manifest;
use super::setup::sha256_hex;

const PLUGIN_FILE: &str = ".claude-plugin/plugin.json";
const HOOKS_FILE: &str = "hooks/hooks.json";

/// The field order is the canonical form's, and the sorting's.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Hook {
    pub event: String,
    /// `""`: each time the event happens.
    pub matcher: String,
    /// What it runs, exactly as the plugin writes it.
    pub command: String,
}

fn listed(event: &str, matcher: &str, command: &str) -> Hook {
    Hook {
        event: event.to_string(),
        matcher: matcher.to_string(),
        command: command.to_string(),
    }
}

/// A plugin's hooks, in canonical order and each once. No hooks, no list.
pub fn read(plugin_dir: &Path) -> Vec<Hook> {
    let mut found = Vec::new();
    let default = plugin_dir.join("hooks").join("hooks.json");
    if let Some(config) = read_file(&default, HOOKS_FILE, &mut found) {
        from_config(&config, HOOKS_FILE, &mut found);
    }
    let plugin = plugin_dir.join(".claude-plugin").join("plugin.json");
    if let Some(plugin) = read_file(&plugin, PLUGIN_FILE, &mut found) {
        if let Some(declared) = plugin.get("hooks") {
            from_declared(plugin_dir, declared, &mut found);
        }
    }
    found.sort();
    found.dedup();
    found
}

/// What accepting these hooks stores on a workspace; `None` for no hooks, which
/// store nothing.
pub fn grant(hooks: &[Hook]) -> Option<String> {
    if hooks.is_empty() {
        return None;
    }
    let mut canonical = hooks.to_vec();
    canonical.sort();
    canonical.dedup();
    let text = serde_json::to_string(&canonical).ok()?;
    Some(format!("hooks:{}", &sha256_hex(&text)[..16]))
}

/// A JSON file. One that is missing holds nothing; one that cannot be read is
/// listed as it stands under `label`.
fn read_file(path: &Path, label: &str, found: &mut Vec<Hook>) -> Option<Value> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            found.push(listed(label, "", &err.to_string()));
            return None;
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
        Ok(value) => Some(value),
        Err(_) => {
            found.push(listed(label, "", &text));
            None
        }
    }
}

/// `hooks` in `plugin.json`: inline, the relative path of a file, or a list of
/// either.
fn from_declared(plugin_dir: &Path, declared: &Value, found: &mut Vec<Hook>) {
    match declared {
        Value::Null => {}
        Value::Array(items) => {
            for item in items {
                from_declared(plugin_dir, item, found);
            }
        }
        Value::String(relative) => {
            let file = manifest::normalize(relative)
                .ok()
                .and_then(|path| manifest::inside(plugin_dir, &path).ok());
            match file {
                Some(file) => {
                    if let Some(config) = read_file(&file, relative, found) {
                        from_config(&config, relative, found);
                    }
                }
                None => found.push(listed(PLUGIN_FILE, "", relative)),
            }
        }
        inline => from_config(inline, PLUGIN_FILE, found),
    }
}

/// `{ "hooks": { <event>: […] }, "modules": […] }`, or the events at the top,
/// as `plugin.json` may have them inline.
fn from_config(config: &Value, label: &str, found: &mut Vec<Hook>) {
    let Value::Object(top) = config else {
        found.push(listed(label, "", &config.to_string()));
        return;
    };
    match top.get("hooks") {
        Some(Value::Object(events)) => {
            for (event, value) in events {
                from_event(event, value, found);
            }
            if let Some(modules) = top.get("modules") {
                from_event("modules", modules, found);
            }
        }
        _ => {
            for (event, value) in top {
                from_event(event, value, found);
            }
        }
    }
}

fn from_event(event: &str, value: &Value, found: &mut Vec<Hook>) {
    match value {
        Value::Array(items) => {
            for item in items {
                from_group(event, item, found);
            }
        }
        Value::Object(_) => from_group(event, value, found),
        // One module, without a list.
        Value::String(_) if event == "modules" => from_group(event, value, found),
        // `description`, `$schema`: no event holds a bare value.
        _ => {}
    }
}

/// `{ "matcher": "…", "hooks": [ … ] }`, or a hook given without a group.
fn from_group(event: &str, group: &Value, found: &mut Vec<Hook>) {
    let matcher = match group.get("matcher") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    };
    match group.get("hooks") {
        Some(Value::Array(hooks)) => {
            for hook in hooks {
                found.push(listed(event, &matcher, &command_of(hook)));
            }
        }
        _ => found.push(listed(event, &matcher, &command_of(group))),
    }
}

/// Why one of a plugin's hooks does not run in Codex: it is listed as Claude
/// Code's only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NotForCodex {
    /// An event Codex has no hooks for (`Notification`,
    /// `PostToolUseFailure`…), or a file this could not read as hooks.
    Event,
    /// Not a command: a prompt or an agent, which Codex skips ("not supported
    /// yet", `hooks/src/engine/discovery.rs`), a type it does not know (an
    /// HTTP call), or a hooks module, Claude Code's own JavaScript.
    Handler,
    /// A tool event whose matcher selects none of the tools Codex tells its
    /// hooks about: one of Claude Code's own (`Read`, `Grep`, `WebFetch`…),
    /// or a kit's MCP tool by the name Claude Code gives a plugin's server.
    Tool,
}

/// The events Codex runs hooks for, by the names its `hooks.json` takes
/// (`protocol/src/hooks.rs`, `HookEventsToml`).
const CODEX_EVENTS: [&str; 14] = [
    "PreModelRequest",
    "PostModelRequest",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "PreCompact",
    "PostCompact",
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "SubagentStart",
    "SubagentStop",
    "Stop",
    "Interrupt",
];

/// The events whose matcher is tested against a tool's name
/// (`hooks/src/events/{pre_tool_use,permission_request,post_tool_use}.rs`).
const TOOL_EVENTS: [&str; 3] = ["PreToolUse", "PermissionRequest", "PostToolUse"];

/// The names Codex tests a tool event's matcher against
/// (`core/src/tools/hook_names.rs`): its shell, `Bash`; its file edits,
/// `apply_patch`, also matched as `Write` and `Edit`; its sub-agents,
/// `spawn_agent`, also matched as `Agent`.
const CODEX_TOOLS: [&str; 6] = [
    "Bash",
    "apply_patch",
    "Write",
    "Edit",
    "spawn_agent",
    "Agent",
];

/// An MCP tool is matched as `mcp__<server>__<tool>`
/// (`core/src/tools/handlers/mcp.rs`, `hook_tool_name_for`): the same name as
/// in Claude Code for a server both have, Hangar's included.
const MCP_TOOL: &str = "mcp__";

/// How Claude Code names a plugin's MCP server's tools
/// (`mcp__plugin_<plugin>_<server>__<tool>`, plugin.rs): Codex gets a kit's
/// servers under other names (`mcp_scope.rs`), so a matcher of these selects
/// nothing there.
const PLUGIN_MCP_TOOL: &str = "mcp__plugin_";

/// Why `hook` does not run in Codex; `None` when it does.
pub fn not_for_codex(hook: &Hook) -> Option<NotForCodex> {
    if hook.event == "modules" {
        return Some(NotForCodex::Handler);
    }
    if !CODEX_EVENTS.contains(&hook.event.as_str()) {
        return Some(NotForCodex::Event);
    }
    // A hook that is not a command is listed as its JSON (`command_of`).
    if matches!(
        serde_json::from_str::<Value>(&hook.command),
        Ok(Value::Object(_))
    ) {
        return Some(NotForCodex::Handler);
    }
    if TOOL_EVENTS.contains(&hook.event.as_str()) && !selects_a_codex_tool(&hook.matcher) {
        return Some(NotForCodex::Tool);
    }
    None
}

/// Whether a tool event's matcher selects one of Codex's tools, read as
/// Codex reads it (`hooks/src/engine/matcher.rs`): empty or `*`, every tool;
/// letters, digits, `_` and `|`, exact names; anything else, a regular
/// expression, which matches anywhere in the name.
fn selects_a_codex_tool(matcher: &str) -> bool {
    let mcp = |name: &str| name.starts_with(MCP_TOOL) && !name.starts_with(PLUGIN_MCP_TOOL);
    if matcher.is_empty() || matcher == "*" {
        return true;
    }
    if matcher
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '|')
    {
        return matcher
            .split('|')
            .any(|name| CODEX_TOOLS.contains(&name) || mcp(name));
    }
    match regex::Regex::new(matcher) {
        Ok(regex) => {
            CODEX_TOOLS.iter().any(|name| regex.is_match(name))
                || (matcher.contains(MCP_TOOL) && !matcher.contains(PLUGIN_MCP_TOOL))
        }
        // Codex leaves a hook whose matcher does not compile out.
        Err(_) => false,
    }
}

fn command_of(hook: &Value) -> String {
    match hook {
        Value::String(text) => text.clone(),
        other => match other.get("command") {
            Some(Value::String(command)) => command.clone(),
            _ => other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kits::testing::Folder;
    use serde_json::json;

    fn plugin(tag: &str, plugin_json: Value) -> Folder {
        let dir = Folder::new(tag);
        dir.file(PLUGIN_FILE, &plugin_json.to_string());
        dir
    }

    fn hook_file(matcher: &str, command: &str) -> String {
        json!({
            "description": "The kit's hooks",
            "hooks": {
                "PostToolUse": [{
                    "matcher": matcher,
                    "hooks": [{ "type": "command", "command": command, "timeout": 30 }]
                }],
                "SessionStart": [{
                    "hooks": [{ "type": "command", "command": "node ${CLAUDE_PLUGIN_ROOT}/start.js" }]
                }]
            }
        })
        .to_string()
    }

    #[test]
    fn hooks_are_read_from_their_file_verbatim() {
        let dir = plugin("hooks-file", json!({ "name": "p" }));
        dir.file(
            HOOKS_FILE,
            &hook_file("Write|Edit", "echo \"edited\" >> log.txt"),
        );
        assert_eq!(
            read(&dir.root),
            vec![
                listed("PostToolUse", "Write|Edit", "echo \"edited\" >> log.txt"),
                listed("SessionStart", "", "node ${CLAUDE_PLUGIN_ROOT}/start.js"),
            ]
        );
    }

    #[test]
    fn hooks_are_read_from_the_plugin_manifest_inline_or_by_path() {
        let events =
            json!({ "Stop": [{ "hooks": [{ "type": "command", "command": "inline.sh" }] }] });
        let flat = plugin("hooks-inline", json!({ "name": "p", "hooks": events }));
        assert_eq!(read(&flat.root), vec![listed("Stop", "", "inline.sh")]);

        let wrapped = plugin(
            "hooks-wrapped",
            json!({ "name": "p", "hooks": { "hooks": events } }),
        );
        assert_eq!(read(&wrapped.root), vec![listed("Stop", "", "inline.sh")]);

        let by_path = plugin(
            "hooks-path",
            json!({ "name": "p", "hooks": ["./config/more.json", { "Stop": events["Stop"] }] }),
        );
        by_path.file("config/more.json", &hook_file("Bash", "more.sh"));
        let found = read(&by_path.root);
        assert!(found.contains(&listed("PostToolUse", "Bash", "more.sh")));
        assert!(found.contains(&listed("Stop", "", "inline.sh")));
    }

    #[test]
    fn both_places_add_up_and_a_hook_listed_twice_counts_once() {
        let dir = plugin(
            "hooks-both",
            json!({ "name": "p", "hooks": {
                "Stop": [{ "hooks": [{ "type": "command", "command": "inline.sh" }] }],
                "SessionStart": [{ "hooks": [{ "command": "node ${CLAUDE_PLUGIN_ROOT}/start.js" }] }]
            } }),
        );
        dir.file(HOOKS_FILE, &hook_file("", "file.sh"));
        assert_eq!(
            read(&dir.root),
            vec![
                listed("PostToolUse", "", "file.sh"),
                listed("SessionStart", "", "node ${CLAUDE_PLUGIN_ROOT}/start.js"),
                listed("Stop", "", "inline.sh"),
            ]
        );
    }

    #[test]
    fn a_plugin_without_hooks_has_an_empty_list_and_no_grant() {
        let dir = plugin("hooks-none", json!({ "name": "p" }));
        assert!(read(&dir.root).is_empty());
        assert_eq!(grant(&read(&dir.root)), None);
        dir.file(
            HOOKS_FILE,
            &json!({ "description": "nothing yet", "hooks": {} }).to_string(),
        );
        assert_eq!(grant(&read(&dir.root)), None);
    }

    #[test]
    fn whatever_else_may_run_is_listed_too() {
        let dir = plugin("hooks-other", json!({ "name": "p" }));
        dir.file(
            HOOKS_FILE,
            &json!({
                "hooks": { "Stop": [{ "hooks": [
                    { "type": "http", "url": "https://example.com/report" },
                    { "type": "prompt", "prompt": "Check the build." }
                ] }] },
                "modules": ["./register.js"]
            })
            .to_string(),
        );
        let found = read(&dir.root);
        assert!(
            found.contains(&listed("modules", "", "./register.js")),
            "{found:?}"
        );
        assert!(found
            .iter()
            .any(|hook| hook.command.contains("https://example.com/report")));
        assert!(found
            .iter()
            .any(|hook| hook.command.contains("Check the build.")));
    }

    #[test]
    fn a_hooks_file_with_only_modules_lists_them() {
        let dir = plugin("hooks-modules", json!({ "name": "p" }));
        dir.file(
            HOOKS_FILE,
            &json!({ "modules": ["./register.tsx"] }).to_string(),
        );
        assert_eq!(
            read(&dir.root),
            vec![listed("modules", "", "./register.tsx")]
        );
    }

    #[test]
    fn what_cannot_be_read_is_shown_rather_than_passed_over() {
        let dir = plugin(
            "hooks-broken",
            json!({ "name": "p", "hooks": "../outside.json" }),
        );
        dir.file(HOOKS_FILE, "{ not json: rm -rf");
        let found = read(&dir.root);
        assert!(
            found.contains(&listed(HOOKS_FILE, "", "{ not json: rm -rf")),
            "{found:?}"
        );
        assert!(
            found.contains(&listed(PLUGIN_FILE, "", "../outside.json")),
            "{found:?}"
        );
    }

    #[test]
    fn the_hash_is_stable_and_changes_with_a_command() {
        let dir = plugin("hooks-hash", json!({ "name": "p" }));
        dir.file(HOOKS_FILE, &hook_file("Bash", "one.sh"));
        let first = grant(&read(&dir.root)).unwrap();
        assert!(first.starts_with("hooks:"), "{first}");
        assert_eq!(first.len(), "hooks:".len() + 16, "{first}");
        assert!(first["hooks:".len()..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));

        // Read again, reformatted, its keys in another order: the same list.
        assert_eq!(grant(&read(&dir.root)).unwrap(), first);
        dir.file(
            HOOKS_FILE,
            &serde_json::to_string_pretty(&json!({
                "hooks": {
                    "SessionStart": [{
                        "hooks": [{ "command": "node ${CLAUDE_PLUGIN_ROOT}/start.js", "type": "command" }]
                    }],
                    "PostToolUse": [{
                        "hooks": [{ "command": "one.sh", "type": "command", "timeout": 5 }],
                        "matcher": "Bash"
                    }]
                }
            }))
            .unwrap(),
        );
        assert_eq!(grant(&read(&dir.root)).unwrap(), first);

        dir.file(HOOKS_FILE, &hook_file("Bash", "two.sh"));
        assert_ne!(grant(&read(&dir.root)).unwrap(), first, "another command");
        dir.file(HOOKS_FILE, &hook_file("Write", "one.sh"));
        assert_ne!(grant(&read(&dir.root)).unwrap(), first, "another matcher");
    }

    /// Every event Codex has runs a command hook there, whatever its
    /// matcher when the event is not a tool's.
    #[test]
    fn a_command_on_an_event_codex_has_runs_there() {
        for event in CODEX_EVENTS {
            assert_eq!(
                not_for_codex(&listed(event, "", "node ${CLAUDE_PLUGIN_ROOT}/x.js")),
                None,
                "{event}"
            );
        }
        assert_eq!(
            not_for_codex(&listed("SessionStart", "startup|resume", "x.sh")),
            None
        );
        assert_eq!(not_for_codex(&listed("PreCompact", "manual", "x.sh")), None);
    }

    /// What Codex has no event for, or does not run, is Claude Code's only:
    /// read from the very list `read` gives.
    #[test]
    fn what_codex_does_not_run_is_said_why() {
        let dir = plugin("hooks-codex", json!({ "name": "p" }));
        dir.file(
            HOOKS_FILE,
            &json!({
                "hooks": {
                    "Notification": [{ "hooks": [{ "type": "command", "command": "notify.sh" }] }],
                    "PostToolUseFailure": [{ "hooks": [{ "type": "command", "command": "fail.sh" }] }],
                    "Stop": [{ "hooks": [
                        { "type": "prompt", "prompt": "Check the build." },
                        { "type": "http", "url": "https://example.com/report" },
                        { "type": "command", "command": "stop.sh" }
                    ] }]
                },
                "modules": ["./register.js"]
            })
            .to_string(),
        );
        let why: Vec<(String, Option<NotForCodex>)> = read(&dir.root)
            .iter()
            .map(|hook| (hook.command.clone(), not_for_codex(hook)))
            .collect();
        let of = |text: &str| {
            why.iter()
                .find(|(command, _)| command.contains(text))
                .unwrap_or_else(|| panic!("{text}: {why:?}"))
                .1
        };
        assert_eq!(of("notify.sh"), Some(NotForCodex::Event));
        assert_eq!(of("fail.sh"), Some(NotForCodex::Event));
        assert_eq!(of("Check the build."), Some(NotForCodex::Handler));
        assert_eq!(of("example.com/report"), Some(NotForCodex::Handler));
        assert_eq!(of("./register.js"), Some(NotForCodex::Handler));
        assert_eq!(of("stop.sh"), None);

        let broken = plugin("hooks-codex-broken", json!({ "name": "p" }));
        broken.file(HOOKS_FILE, "{ not json");
        assert_eq!(
            read(&broken.root)
                .iter()
                .map(not_for_codex)
                .collect::<Vec<_>>(),
            vec![Some(NotForCodex::Event)]
        );
    }

    /// A tool event's matcher is read as Codex reads it, against the tools it
    /// reports: `Bash`, `apply_patch` (also `Write`, `Edit`), `spawn_agent`
    /// (also `Agent`) and MCP tools.
    #[test]
    fn a_tool_hook_runs_in_codex_when_its_matcher_selects_a_codex_tool() {
        let reach = |matcher: &str| not_for_codex(&listed("PreToolUse", matcher, "check.sh"));
        for runs in [
            "",
            "*",
            "Bash",
            "Write|Edit",
            "Read|Write",
            "Agent",
            "apply_patch",
            ".*",
            "^(Edit|MultiEdit)$",
            "mcp__teskad__board_list_tasks",
            "mcp__github__.*",
        ] {
            assert_eq!(reach(runs), None, "{runs}");
        }
        for claude_only in [
            "Read",
            "Read|Grep|Glob",
            "Notebook.*",
            "Web(Fetch|Search)",
            "Task",
            "mcp__plugin_roblox_studio__run",
            "mcp__plugin_roblox_.*",
            "(",
        ] {
            assert_eq!(reach(claude_only), Some(NotForCodex::Tool), "{claude_only}");
        }
        for event in TOOL_EVENTS {
            assert_eq!(
                not_for_codex(&listed(event, "Grep", "x.sh")),
                Some(NotForCodex::Tool),
                "{event}"
            );
        }
    }

    #[test]
    fn the_hash_is_the_documented_one() {
        let hooks = [listed("Stop", "", "x.sh")];
        let canonical = r#"[{"event":"Stop","matcher":"","command":"x.sh"}]"#;
        assert_eq!(
            grant(&hooks).unwrap(),
            format!("hooks:{}", &sha256_hex(canonical)[..16])
        );
    }
}
