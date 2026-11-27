//! « Add to Codex »: the parts of a kit Codex takes only once installed — its
//! skills and the hooks Codex runs — as a Codex plugin Hangar builds from the
//! kit, which Codex installs with its own commands, once, when the person asks
//! (docs/native/agents.md §6.4, D18). « Remove from Codex » undoes it.
//!
//! Read in openai/codex at `rust-v0.170.0` (paths under `codex-rs/`):
//!
//! - A marketplace is a folder holding `.agents/plugins/marketplace.json`
//!   (`core-plugins/src/marketplace.rs`, `MARKETPLACE_MANIFEST_RELATIVE_PATHS`):
//!   its `name`, and its plugins by `name` and `source`. A local source is a
//!   `./` path that stays inside the marketplace's folder
//!   (`resolve_local_plugin_source_path`), so the plugins Hangar builds live
//!   under the marketplace's own folder, `~/.teskad/codex-plugins/`, rather
//!   than in each kit's folder as §6.4 first said: a linked kit has no folder
//!   of Hangar's, and a cloned one would carry a folder git does not know.
//! - `codex plugin marketplace add <folder>` records a local marketplace where
//!   it is, nothing copied, and answers "already added" the next time
//!   (`core-plugins/src/marketplace_add.rs`); its plugins are read from there
//!   each time, so adding a second kit is writing its entry.
//! - `codex plugin add <plugin>@<marketplace>` copies the plugin into
//!   `$CODEX_HOME/plugins/cache/<marketplace>/<plugin>/<version>/`, replacing
//!   what was there, and turns it on in `config.toml`
//!   (`core-plugins/src/store.rs`, `manager.rs` `install_plugin`). The copy
//!   skips symbolic links (`store.rs`, `copy_dir_recursive`), so the plugin
//!   holds copies of the kit's skills, not links to them. Which version is
//!   there tells whether what Codex has is the kit as it is now: the version
//!   Hangar writes ends with a hash of what it copies.
//! - The plugin's skills are its `skills/<name>/SKILL.md` and its hooks
//!   `hooks/hooks.json`, by default (`core-plugins/src/loader.rs`). Codex
//!   hands a plugin's hooks `CLAUDE_PLUGIN_ROOT` naming its copy in the cache
//!   (`hooks/src/engine/discovery.rs`), where the kit's scripts are not: the
//!   hooks are written with the kit's own folder in its place, as the kit's
//!   MCP servers are for Codex (plugin.rs). Codex runs a plugin's hooks once
//!   the person trusts them in its `/hooks`.
//! - `codex plugin remove <plugin>@<marketplace>` deletes the copy and its
//!   line in `config.toml`; `codex plugin marketplace remove <name>` forgets
//!   the marketplace (`cli/src/plugin_cmd.rs`, `marketplace_cmd.rs`).
//!
//! The commands run with Hangar's own environment, so they change main's
//! `CODEX_HOME`, whose `config.toml` and `plugins/` every Codex account
//! Hangar keeps shares (accounts/codex_home.rs). Nothing here is run against
//! a Codex on this page's machine: the commands are as the source has them.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::hooks::{self, Hook};
use super::plugin;
use super::store::{remove_tree, write_atomic, Loaded};
use crate::agent::{AgentKind, KitPart};

/// The marketplace Hangar keeps for Codex, by the name its file gives it.
pub const MARKETPLACE: &str = "hangar-kits";

/// Where the marketplace's file sits under its folder.
const MARKETPLACE_FILE: &str = ".agents/plugins/marketplace.json";

/// Where Codex keeps the plugins it installed, under its home
/// (`core-plugin-common/src/installed.rs`, `PLUGINS_CACHE_DIR`).
const CODEX_CACHE: &str = "plugins/cache";

/// How long one `codex plugin` command may take: it reads files and writes
/// `config.toml`, nothing over the network for a local marketplace.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Deep enough for a skill's folder, short of a stray `node_modules`.
const MAX_DEPTH: usize = 8;
const MAX_FILES: usize = 2000;

/// Where a kit stands with Codex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodexState {
    /// Nothing of the kit for Codex to install, and nothing installed: its
    /// manifest leaves Codex out, or it has no skill nor hook Codex runs.
    Nothing,
    /// There is something to add, and Codex has none of it.
    NotAdded,
    /// Codex has the kit's plugin as the kit is now.
    Added,
    /// Codex has the kit's plugin as it was: adding again brings it what the
    /// kit holds now, or, when it holds nothing for Codex any more, removing
    /// takes it out.
    Changed,
}

/// What « Add to Codex » would add of a kit, and whether Codex has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPlugin {
    pub state: CodexState,
    /// As Codex names it: `<kit>@hangar-kits`.
    pub plugin: String,
    /// The skills it holds, by folder name.
    pub skills: Vec<String>,
    /// The hooks it holds, as the kit writes them.
    pub hooks: Vec<Hook>,
    /// Whether Codex is installed on this machine, to run the commands.
    pub codex: bool,
    /// What « Add to Codex » runs, one command a line, as it runs it.
    pub add: Vec<String>,
    /// What « Remove from Codex » runs.
    pub remove: Vec<String>,
}

/// What of a kit goes into its Codex plugin.
#[derive(Debug, Default)]
struct Contents {
    /// The kit's plugin folder.
    dir: PathBuf,
    skills: Vec<String>,
    hooks: Vec<Hook>,
}

impl Contents {
    fn is_empty(&self) -> bool {
        self.skills.is_empty() && self.hooks.is_empty()
    }
}

/// The parts of `kit` Codex takes once installed (its `extensions`): its
/// skills and the hooks Codex runs, when the manifest names Codex.
fn contents(kit: &Loaded) -> Contents {
    let Some(dir) = kit.plugin_dir() else {
        return Contents::default();
    };
    if !kit.manifest.meant_for(AgentKind::Codex) {
        return Contents::default();
    }
    let ext = AgentKind::Codex.info().caps.extensions;
    let skills = if ext.skills == KitPart::Installed {
        plugin::skills(&dir)
    } else {
        Vec::new()
    };
    let hooks = if ext.hooks == KitPart::Installed {
        hooks::read(&dir)
            .into_iter()
            .filter(|hook| hooks::not_for_codex(hook).is_none())
            .collect()
    } else {
        Vec::new()
    };
    Contents { dir, skills, hooks }
}

/// `${CLAUDE_PLUGIN_ROOT}` and `$CLAUDE_PLUGIN_ROOT` written out as `root`.
fn written_out(command: &str, root: &str) -> String {
    const VAR: &str = "CLAUDE_PLUGIN_ROOT";
    let braced = command.replace(&format!("${{{VAR}}}"), root);
    let mut out = String::with_capacity(braced.len());
    let mut rest = braced.as_str();
    while let Some(at) = rest.find(&format!("${VAR}")) {
        let after = &rest[at + 1 + VAR.len()..];
        let ends = after
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
        out.push_str(&rest[..at]);
        out.push_str(if ends {
            root
        } else {
            &rest[at..at + 1 + VAR.len()]
        });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// `hooks/hooks.json` of the plugin: the hooks in Codex's shape, one group
/// per event and matcher, the kit's folder written out.
fn hooks_file(hooks: &[Hook], root: &Path) -> Value {
    let root = root.to_string_lossy();
    let mut events: BTreeMap<&str, Vec<(&str, Vec<Value>)>> = BTreeMap::new();
    for hook in hooks {
        let groups = events.entry(hook.event.as_str()).or_default();
        let command = json!({ "type": "command", "command": written_out(&hook.command, &root) });
        match groups
            .iter_mut()
            .find(|(matcher, _)| *matcher == hook.matcher)
        {
            Some((_, list)) => list.push(command),
            None => groups.push((hook.matcher.as_str(), vec![command])),
        }
    }
    let events: Map<String, Value> = events
        .into_iter()
        .map(|(event, groups)| {
            let groups = groups
                .into_iter()
                .map(|(matcher, list)| {
                    let mut group = Map::new();
                    if !matcher.is_empty() {
                        group.insert("matcher".into(), Value::String(matcher.to_string()));
                    }
                    group.insert("hooks".into(), Value::Array(list));
                    Value::Object(group)
                })
                .collect();
            (event.to_string(), Value::Array(groups))
        })
        .collect();
    json!({ "hooks": events })
}

/// The files under `dir`, by path from it, links and what is too deep or too
/// many left out: what Codex's copy would keep.
fn files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, prefix: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if depth > MAX_DEPTH || out.len() >= MAX_FILES {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let relative = prefix.join(entry.file_name());
            if kind.is_dir() {
                walk(&entry.path(), &relative, depth + 1, out);
            } else if kind.is_file() && out.len() < MAX_FILES {
                out.push(relative);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, Path::new(""), 0, &mut out);
    out
}

/// What the plugin is built from, file by file: its skills' files and its
/// `hooks/hooks.json`, by their path in the plugin.
fn sources(found: &Contents) -> Vec<(PathBuf, Source)> {
    let mut out = Vec::new();
    for skill in &found.skills {
        let base = Path::new("skills").join(skill);
        for file in files(&found.dir.join(&base)) {
            out.push((
                base.join(&file),
                Source::File(found.dir.join(&base).join(&file)),
            ));
        }
    }
    if !found.hooks.is_empty() {
        let text =
            serde_json::to_string_pretty(&hooks_file(&found.hooks, &found.dir)).unwrap_or_default();
        out.push((Path::new("hooks").join("hooks.json"), Source::Text(text)));
    }
    out
}

enum Source {
    File(PathBuf),
    Text(String),
}

/// The plugin's version: the kit's, then a hash of every file the plugin
/// holds, so that Codex holding another one says the kit changed since.
fn version(kit: &Loaded, sources: &[(PathBuf, Source)]) -> String {
    let mut hash = Sha256::new();
    for (path, source) in sources {
        hash.update(path.to_string_lossy().replace('\\', "/").as_bytes());
        hash.update([0]);
        match source {
            Source::File(file) => hash.update(fs::read(file).unwrap_or_default()),
            Source::Text(text) => hash.update(text.as_bytes()),
        }
        hash.update([0]);
    }
    let digest: String = hash
        .finalize()
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    // Codex takes `[A-Za-z0-9._+-]` (`validate_plugin_version_segment`), which
    // a semver version already is; its build part gives way to the hash.
    let base = kit.manifest.version.split('+').next().unwrap_or_default();
    format!("{base}+{digest}")
}

/// `<kit>@hangar-kits`.
fn plugin_key(id: &str) -> String {
    format!("{id}@{MARKETPLACE}")
}

/// The versions of the kit's plugin Codex holds under `codex_home`.
fn installed(codex_home: Option<&Path>, id: &str) -> Vec<String> {
    let Some(home) = codex_home else {
        return Vec::new();
    };
    let mut versions: Vec<String> = fs::read_dir(home.join(CODEX_CACHE).join(MARKETPLACE).join(id))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    versions.sort();
    versions
}

/// The plugins the marketplace in `root` holds, by name.
fn listed(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root.join("plugins"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry.path().join(".codex-plugin/plugin.json").is_file()
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Where a kit of `id` stands with Codex, its marketplace in `root` and
/// Codex's home in `codex_home`; `codex`: whether Codex is installed.
pub fn status(kit: &Loaded, root: &Path, codex_home: Option<&Path>, codex: bool) -> CodexPlugin {
    let found = contents(kit);
    let id = kit.manifest.id.as_str();
    let held = installed(codex_home, id);
    let state = if held.is_empty() {
        if found.is_empty() {
            CodexState::Nothing
        } else {
            CodexState::NotAdded
        }
    } else if !found.is_empty() && held.contains(&version(kit, &sources(&found))) {
        CodexState::Added
    } else {
        CodexState::Changed
    };
    let key = plugin_key(id);
    let root_text = root.to_string_lossy();
    let mut remove = vec![format!("codex plugin remove {key}")];
    // As `remove` does: the marketplace goes with the last kit it holds.
    if root.join(MARKETPLACE_FILE).is_file() && listed(root).iter().all(|name| name == id) {
        remove.push(format!("codex plugin marketplace remove {MARKETPLACE}"));
    }
    CodexPlugin {
        state,
        add: vec![
            format!("codex plugin marketplace add {}", quoted(&root_text)),
            format!("codex plugin add {key}"),
        ],
        remove,
        plugin: key,
        skills: found.skills,
        hooks: found.hooks,
        codex,
    }
}

/// A path as a shell line shows it: quoted when it holds a space.
fn quoted(text: &str) -> String {
    if text.contains(char::is_whitespace) {
        format!("\"{text}\"")
    } else {
        text.to_string()
    }
}

/// Builds the kit's plugin in the marketplace in `root`, and lists it there.
fn build(kit: &Loaded, found: &Contents, root: &Path) -> Result<(), String> {
    let id = kit.manifest.id.as_str();
    let dir = root.join("plugins").join(id);
    remove_tree(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let sources = sources(found);
    for (path, source) in &sources {
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        match source {
            Source::File(file) => fs::copy(file, &target)
                .map(drop)
                .map_err(|err| format!("{}: {err}", file.display()))?,
            Source::Text(text) => write_atomic(&target, text)?,
        }
    }
    let mut manifest = Map::new();
    manifest.insert("name".into(), json!(id));
    manifest.insert("version".into(), json!(version(kit, &sources)));
    if !kit.manifest.description.is_empty() {
        manifest.insert("description".into(), json!(kit.manifest.description));
    }
    manifest.insert(
        "interface".into(),
        json!({ "displayName": kit.manifest.name }),
    );
    write_atomic(
        &dir.join(".codex-plugin").join("plugin.json"),
        &serde_json::to_string_pretty(&Value::Object(manifest)).unwrap_or_default(),
    )?;
    write_marketplace(root)
}

/// `marketplace.json`, listing every plugin built in `root`.
fn write_marketplace(root: &Path) -> Result<(), String> {
    let plugins: Vec<Value> = listed(root)
        .into_iter()
        .map(|name| {
            json!({
                "name": name,
                "source": { "source": "local", "path": format!("./plugins/{name}") },
            })
        })
        .collect();
    let file = json!({
        "name": MARKETPLACE,
        "interface": { "displayName": "Hangar kits" },
        "plugins": plugins,
    });
    write_atomic(
        &root.join(MARKETPLACE_FILE),
        &serde_json::to_string_pretty(&file).unwrap_or_default(),
    )
}

/// « Add to Codex »: builds the kit's plugin in `root` and has `program`
/// (Codex) record the marketplace and install the plugin.
pub fn add(kit: &Loaded, root: &Path, program: &Path) -> Result<(), String> {
    let found = contents(kit);
    if found.is_empty() {
        return Err("this kit has nothing for Codex to add".into());
    }
    build(kit, &found, root)?;
    let root_arg = root.to_string_lossy().into_owned();
    run(
        program,
        &["plugin", "marketplace", "add", &root_arg, "--json"],
    )?;
    run(
        program,
        &["plugin", "add", &plugin_key(&kit.manifest.id), "--json"],
    )
    .map(drop)
}

/// « Remove from Codex »: has `program` uninstall the kit `id`'s plugin, then
/// takes it out of the marketplace in `root`, and the marketplace out of
/// Codex once it holds no other kit. A plugin Codex no longer has
/// (`codex_home` holds no copy of it) is not asked about again.
pub fn remove(
    id: &str,
    root: &Path,
    codex_home: Option<&Path>,
    program: &Path,
) -> Result<(), String> {
    let key = plugin_key(id);
    if let Err(err) = run(program, &["plugin", "remove", &key, "--json"]) {
        if !installed(codex_home, id).is_empty() {
            return Err(err);
        }
    }
    let dir = root.join("plugins").join(id);
    remove_tree(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    if listed(root).is_empty() {
        if root.join(MARKETPLACE_FILE).is_file() {
            run(
                program,
                &["plugin", "marketplace", "remove", MARKETPLACE, "--json"],
            )?;
        }
        remove_tree(root).map_err(|err| format!("{}: {err}", root.display()))
    } else {
        write_marketplace(root)
    }
}

/// Hides the console window Windows would open for every child of a GUI app.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Runs `program args`, waiting at most `TIMEOUT`; what it printed when it
/// worked, its first line of complaint when it did not.
fn run(program: &Path, args: &[&str]) -> Result<String, String> {
    let shown = format!("codex {}", args.join(" "));
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("`{shown}` could not be started: {err}"))?;
    // Read on threads of their own, so a full pipe never stalls the wait.
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_string(&mut text);
            }
            text
        })
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "`{shown}` did not answer within {} seconds",
                    TIMEOUT.as_secs()
                ));
            }
            Err(err) => return Err(err.to_string()),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if status.success() {
        return Ok(stdout);
    }
    let said = stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map_or_else(|| status.to_string(), str::to_string);
    Err(format!("`{shown}` failed: {said}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kits::store::Store;
    use crate::kits::testing::Folder;

    fn loaded(folder: &Folder, store: &Store) -> Loaded {
        store
            .add_folder(&folder.root.to_string_lossy(), 1)
            .and_then(|entry| store.loaded(&entry))
            .unwrap()
    }

    /// A kit for both agents, with two skills, one held through a link, and
    /// hooks of which Codex runs one.
    fn studio(tag: &str, agents: Value) -> Folder {
        let kit = Folder::new(tag);
        kit.kit(".", "studio", "1.2.0");
        kit.file(
            crate::kits::manifest::FILE,
            &json!({
                "schema": 1, "id": "studio", "name": "Studio", "version": "1.2.0+build.7",
                "description": "Places.", "agents": agents
            })
            .to_string(),
        )
        .file("skills/hello/notes/more.md", "More.")
        .file("skills/place/SKILL.md", "---\nname: place\n---\nPlace.\n")
        .file("scripts/log.sh", "echo")
        .file(
            "hooks/hooks.json",
            &json!({ "hooks": {
                "PostToolUse": [{ "matcher": "Bash", "hooks": [
                    { "type": "command", "command": "\"${CLAUDE_PLUGIN_ROOT}/scripts/log.sh\" $CLAUDE_PLUGIN_ROOT_X" },
                    { "type": "command", "command": "sh $CLAUDE_PLUGIN_ROOT/scripts/log.sh" }
                ] }],
                "Notification": [{ "hooks": [{ "type": "command", "command": "ping.sh" }] }]
            } })
            .to_string(),
        );
        kit
    }

    struct Scene {
        _home: Folder,
        _kit: Folder,
        kit: Loaded,
        root: PathBuf,
        codex_home: PathBuf,
    }

    fn scene(tag: &str, agents: Value) -> Scene {
        let home = Folder::new(tag);
        let store = Store::at(home.root.join("kits"));
        let source = studio(&format!("{tag}-src"), agents);
        let kit = loaded(&source, &store);
        Scene {
            root: home.root.join("codex-plugins"),
            codex_home: home.root.join("codex"),
            _home: home,
            _kit: source,
            kit,
        }
    }

    impl Scene {
        fn status(&self) -> CodexPlugin {
            status(&self.kit, &self.root, Some(&self.codex_home), true)
        }

        /// What `codex plugin add` leaves: a copy under the version given.
        fn codex_holds(&self, version: &str) {
            let dir = self
                .codex_home
                .join(CODEX_CACHE)
                .join(MARKETPLACE)
                .join("studio")
                .join(version);
            fs::create_dir_all(dir).unwrap();
        }

        fn built_version(&self) -> String {
            let plugin: Value = serde_json::from_str(
                &fs::read_to_string(self.root.join("plugins/studio/.codex-plugin/plugin.json"))
                    .unwrap(),
            )
            .unwrap();
            plugin["version"].as_str().unwrap().to_string()
        }
    }

    #[test]
    fn a_kit_meant_for_codex_lists_its_skills_and_the_hooks_codex_runs() {
        let scene = scene("codex-status", json!(["claude", "codex"]));
        let found = scene.status();
        assert_eq!(found.state, CodexState::NotAdded);
        assert_eq!(found.plugin, "studio@hangar-kits");
        assert_eq!(found.skills, ["hello", "place"]);
        assert_eq!(
            found
                .hooks
                .iter()
                .map(|hook| hook.event.as_str())
                .collect::<Vec<_>>(),
            ["PostToolUse", "PostToolUse"],
            "Notification is Claude Code's only"
        );
        assert_eq!(
            found.add,
            [
                format!(
                    "codex plugin marketplace add {}",
                    quoted(&scene.root.to_string_lossy())
                ),
                "codex plugin add studio@hangar-kits".to_string(),
            ]
        );
        // No marketplace yet: nothing of it to forget.
        assert_eq!(found.remove, ["codex plugin remove studio@hangar-kits"]);
        build(&scene.kit, &contents(&scene.kit), &scene.root).unwrap();
        assert_eq!(
            scene.status().remove,
            [
                "codex plugin remove studio@hangar-kits",
                "codex plugin marketplace remove hangar-kits"
            ]
        );
    }

    #[test]
    fn a_kit_left_for_claude_code_has_nothing_for_codex() {
        let scene = scene("codex-nothing", json!(["claude"]));
        let found = scene.status();
        assert_eq!(found.state, CodexState::Nothing);
        assert!(found.skills.is_empty() && found.hooks.is_empty());
        // Added before its manifest left Codex out: there is still a copy to
        // take out.
        scene.codex_holds("1.2.0+abc");
        assert_eq!(scene.status().state, CodexState::Changed);
    }

    #[test]
    fn the_plugin_holds_copies_of_the_skills_and_codex_s_hooks() {
        let scene = scene("codex-build", json!(["codex"]));
        let found = contents(&scene.kit);
        build(&scene.kit, &found, &scene.root).unwrap();
        let plugin = scene.root.join("plugins/studio");
        assert_eq!(
            fs::read_to_string(plugin.join("skills/hello/SKILL.md")).unwrap(),
            "---\nname: hello\ndescription: Says hello.\n---\nHello.\n"
        );
        assert!(plugin.join("skills/hello/notes/more.md").is_file());
        assert!(plugin.join("skills/place/SKILL.md").is_file());
        assert!(!plugin.join("scripts").exists(), "only skills and hooks");
        assert!(
            !plugin.join(".mcp.json").exists(),
            "servers come with each launch"
        );

        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(plugin.join(".codex-plugin/plugin.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["name"], "studio");
        assert_eq!(manifest["description"], "Places.");
        assert_eq!(manifest["interface"]["displayName"], "Studio");
        let version = manifest["version"].as_str().unwrap();
        assert!(version.starts_with("1.2.0+"), "{version}");
        assert_eq!(version.len(), "1.2.0+".len() + 12);

        let root = scene
            .kit
            .plugin_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let hooks: Value =
            serde_json::from_str(&fs::read_to_string(plugin.join("hooks/hooks.json")).unwrap())
                .unwrap();
        assert_eq!(
            hooks,
            json!({ "hooks": { "PostToolUse": [{ "matcher": "Bash", "hooks": [
                { "type": "command", "command": format!("\"{root}/scripts/log.sh\" $CLAUDE_PLUGIN_ROOT_X") },
                { "type": "command", "command": format!("sh {root}/scripts/log.sh") }
            ] }] } })
        );

        let market: Value =
            serde_json::from_str(&fs::read_to_string(scene.root.join(MARKETPLACE_FILE)).unwrap())
                .unwrap();
        assert_eq!(
            market,
            json!({
                "name": "hangar-kits",
                "interface": { "displayName": "Hangar kits" },
                "plugins": [{ "name": "studio", "source": { "source": "local", "path": "./plugins/studio" } }]
            })
        );
    }

    /// Codex's copy keeps no link: neither does the plugin.
    #[cfg(unix)]
    #[test]
    fn links_in_a_skill_are_left_out() {
        let scene = scene("codex-links", json!(["codex"]));
        let secret = Folder::new("codex-links-secret");
        secret.file("key", "secret");
        let skill = scene.kit.plugin_dir().unwrap().join("skills/hello");
        std::os::unix::fs::symlink(secret.root.join("key"), skill.join("key")).unwrap();
        std::os::unix::fs::symlink(&secret.root, skill.join("dir")).unwrap();
        build(&scene.kit, &contents(&scene.kit), &scene.root).unwrap();
        let plugin = scene.root.join("plugins/studio/skills/hello");
        assert!(plugin.join("SKILL.md").is_file());
        assert!(!plugin.join("key").exists());
        assert!(!plugin.join("dir").exists());
    }

    #[test]
    fn the_version_codex_holds_says_whether_it_is_the_kit_as_it_is() {
        let scene = scene("codex-version", json!(["codex"]));
        build(&scene.kit, &contents(&scene.kit), &scene.root).unwrap();
        let built = scene.built_version();
        scene.codex_holds(&built);
        assert_eq!(scene.status().state, CodexState::Added);

        // A skill edited: Codex holds the kit as it was.
        fs::write(
            scene
                .kit
                .plugin_dir()
                .unwrap()
                .join("skills/place/SKILL.md"),
            "Placed.",
        )
        .unwrap();
        assert_eq!(scene.status().state, CodexState::Changed);
        build(&scene.kit, &contents(&scene.kit), &scene.root).unwrap();
        assert_ne!(scene.built_version(), built);
    }

    #[test]
    fn plugin_root_is_written_out_only_as_a_whole_name() {
        assert_eq!(written_out("a ${CLAUDE_PLUGIN_ROOT}/x", "/k"), "a /k/x");
        assert_eq!(written_out("$CLAUDE_PLUGIN_ROOT", "/k"), "/k");
        assert_eq!(
            written_out("$CLAUDE_PLUGIN_ROOTS", "/k"),
            "$CLAUDE_PLUGIN_ROOTS"
        );
        assert_eq!(
            written_out("$CLAUDE_PLUGIN_ROOT/a $CLAUDE_PLUGIN_DATA", "/k"),
            "/k/a $CLAUDE_PLUGIN_DATA"
        );
    }

    /// A stand-in for `codex` that writes down what it is asked, and refuses
    /// what `fail` names.
    #[cfg(unix)]
    fn fake_codex(folder: &Folder, fail: &str) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let log = folder.root.join("asked.log");
        let program = folder.root.join("codex");
        fs::write(
            &program,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$*\" in\n  *'{fail}'*) echo 'Error: refused' >&2; exit 1;;\nesac\necho '{{}}'\n",
                log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        (program, log)
    }

    #[cfg(unix)]
    #[test]
    fn adding_and_removing_run_codex_s_own_commands() {
        let scene = scene("codex-run", json!(["claude", "codex"]));
        let bin = Folder::new("codex-run-bin");
        let (program, log) = fake_codex(&bin, "never");
        add(&scene.kit, &scene.root, &program).unwrap();
        let root = scene.root.to_string_lossy().into_owned();
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            format!("plugin marketplace add {root} --json\nplugin add studio@hangar-kits --json\n")
        );
        assert!(scene
            .root
            .join("plugins/studio/skills/hello/SKILL.md")
            .is_file());

        fs::remove_file(&log).unwrap();
        remove("studio", &scene.root, Some(&scene.codex_home), &program).unwrap();
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            "plugin remove studio@hangar-kits --json\nplugin marketplace remove hangar-kits --json\n"
        );
        assert!(
            !scene.root.exists(),
            "the last kit takes the marketplace along"
        );
    }

    #[cfg(unix)]
    #[test]
    fn removing_one_kit_keeps_the_others() {
        let scene = scene("codex-keep", json!(["codex"]));
        let bin = Folder::new("codex-keep-bin");
        let (program, log) = fake_codex(&bin, "never");
        build(&scene.kit, &contents(&scene.kit), &scene.root).unwrap();
        // Another kit built beside it.
        let other = scene.root.join("plugins/other/.codex-plugin");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("plugin.json"), "{\"name\":\"other\"}").unwrap();
        assert_eq!(
            scene.status().remove,
            ["codex plugin remove studio@hangar-kits"]
        );
        remove("studio", &scene.root, Some(&scene.codex_home), &program).unwrap();
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            "plugin remove studio@hangar-kits --json\n"
        );
        let market: Value =
            serde_json::from_str(&fs::read_to_string(scene.root.join(MARKETPLACE_FILE)).unwrap())
                .unwrap();
        assert_eq!(market["plugins"][0]["name"], "other");
        assert_eq!(market["plugins"].as_array().unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn codex_refusing_says_why_and_a_copy_it_still_holds_stays_listed() {
        let scene = scene("codex-refuse", json!(["codex"]));
        let bin = Folder::new("codex-refuse-bin");
        let (program, _) = fake_codex(&bin, "plugin add");
        let err = add(&scene.kit, &scene.root, &program).unwrap_err();
        assert_eq!(
            err,
            "`codex plugin add studio@hangar-kits --json` failed: Error: refused"
        );

        let (program, _) = fake_codex(&bin, "plugin remove");
        scene.codex_holds("1.2.0+abc");
        assert!(remove("studio", &scene.root, Some(&scene.codex_home), &program).is_err());
        assert!(scene.root.join("plugins/studio").is_dir());
        // Codex no longer has it (removed from Codex itself): Hangar's side
        // goes all the same.
        fs::remove_dir_all(scene.codex_home.join(CODEX_CACHE)).unwrap();
        remove("studio", &scene.root, Some(&scene.codex_home), &program).unwrap();
        assert!(!scene.root.join("plugins/studio").exists());
    }

    #[test]
    fn a_kit_without_anything_for_codex_is_not_added() {
        let scene = scene("codex-empty", json!(["claude"]));
        let err = add(&scene.kit, &scene.root, Path::new("codex")).unwrap_err();
        assert_eq!(err, "this kit has nothing for Codex to add");
        assert!(!scene.root.exists());
    }
}
