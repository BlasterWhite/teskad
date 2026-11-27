//! First launch: Teskad's folders are its own, and what the official HangarAI
//! app left in its folders is copied over once.
//!
//! Teskad does not share an identity with the official app (`paths`): its data
//! folder is named after `io.github.blasterwhite.teskad` and its home folder
//! is `~/.teskad`. A person who ran HangarAI before finds their workspaces,
//! memory, accounts and kits here anyway, because this module **copies** them
//! from `com.simnjs.iabench` and `~/.hangar` on the first launch.
//!
//! What it promises:
//!
//! - **The official app's folders are only read.** Nothing there is moved,
//!   changed or deleted: going back to the official app finds everything as it
//!   left it.
//! - **A folder is imported when it does not exist yet.** One that exists
//!   (made by an earlier launch of Teskad, or by hand) is never written into,
//!   and a marker file (`MARKER`) in an imported one keeps a later launch from
//!   copying again, also after the person deleted something they did not want.
//! - **A copy is whole or absent.** It is made beside the target
//!   (`<folder>.importing`) and renamed into place with its marker last: a run
//!   that dies half way leaves only that staging folder, which the next launch
//!   removes and starts again.
//! - **What names the official app is rewritten.** The copied `mcp/*.json`
//!   files and the `.mcp.json` of the workspaces in the copied `state.json`
//!   name the official binary for the `hangar` server; they are made to name
//!   this one, which reads this app's endpoint file.
//!
//! What it does not copy: `endpoint.json` (the official app's port and token,
//! that no agent of Teskad may be handed), `logs/`, the voice `models/` (large,
//! downloaded again from the settings) and the `state.json.corrupt.*` rescues.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::paths::{self, Paths};

/// What marks an imported folder as complete. It sits inside the folder, so it
/// goes with it when the person deletes the folder to import again.
pub const MARKER: &str = ".imported-from-official";

/// Appended to a target's name while it is being copied.
const STAGING_SUFFIX: &str = ".importing";

/// One folder to import: `from` is the official app's, `to` is Teskad's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub from: PathBuf,
    pub to: PathBuf,
    /// Names, at the top of `from`, that are left out.
    pub skip: Vec<&'static str>,
}

/// Everything a run imports, and what it rewrites the copies to name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub pairs: Vec<Pair>,
    /// This app's executable: the `hangar` MCP server of every copied config
    /// is made to run it. `None` leaves them as they are.
    pub executable: Option<PathBuf>,
}

/// What became of one pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Copied: this many files.
    Imported { files: usize },
    /// Left alone, and why.
    Skipped(Skip),
    /// Not copied; the target does not exist, so the next launch tries again.
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// The official app has no such folder.
    NoSource,
    /// An earlier run imported it (the marker is there).
    AlreadyImported,
    /// The folder exists without the marker: it is Teskad's own, in use.
    TargetExists,
}

/// What a run did, pair by pair, and the workspaces whose `.mcp.json` it
/// rewrote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub pairs: Vec<(PathBuf, Outcome)>,
    pub rewritten_projects: Vec<PathBuf>,
    /// The workspaces whose `.mcp.json` held Teskad's server under the
    /// upstream's name (`hangar`), moved to `teskad` (install.rs).
    pub renamed_projects: Vec<PathBuf>,
}

impl Report {
    /// Whether anything was copied on this run.
    pub fn imported(&self) -> bool {
        self.pairs
            .iter()
            .any(|(_, outcome)| matches!(outcome, Outcome::Imported { .. }))
    }
}

/// The plan for this machine, or `None` for a run kept off the person's real
/// files (`TESKAD_DATA_ROOT` or `TESKAD_ENDPOINT_FILE`, set by a development
/// build, which must not read the official app's data either).
pub fn system_plan() -> Option<Plan> {
    let set = |value: Option<std::ffi::OsString>| value.is_some_and(|value| !value.is_empty());
    if set(crate::env::var_os("TESKAD_DATA_ROOT")) || set(std::env::var_os("TESKAD_ENDPOINT_FILE"))
    {
        return None;
    }
    let home = dirs::home_dir()?;
    let mut pairs = vec![Pair {
        from: home.join(paths::OFFICIAL_HOME_DIR_NAME),
        to: home.join(paths::HOME_DIR_NAME),
        skip: vec!["endpoint.json"],
    }];
    let (old, new) = (
        Paths::system_for(paths::OFFICIAL_IDENTIFIER),
        Paths::system(),
    );
    type Dir = fn(&Paths) -> Result<PathBuf, paths::UnknownPath>;
    for dir in [
        Paths::app_data_dir as Dir,
        Paths::app_config_dir,
        Paths::app_local_data_dir,
    ] {
        let (Ok(from), Ok(to)) = (dir(&old), dir(&new)) else {
            continue;
        };
        // Linux and macOS give two of the three the same folder.
        if pairs.iter().any(|pair| pair.from == from) {
            continue;
        }
        pairs.push(Pair {
            from,
            to,
            skip: vec!["logs", "models"],
        });
    }
    Some(Plan {
        pairs,
        executable: crate::executable::stable().ok(),
    })
}

/// Runs the plan for this machine, if there is one. Errors are in the report:
/// a migration that fails leaves Teskad to start empty, never to stop.
pub fn run_at_startup() -> Report {
    let Some(plan) = system_plan() else {
        return Report::default();
    };
    let mut report = run(&plan);
    if let Some(home) = paths::home_folder() {
        report.renamed_projects = rename_server_in_projects(&home.join("state.json"));
    }
    report
}

/// The workspaces `state` lists whose `.mcp.json` names Teskad's MCP server
/// `hangar`, as every config written before the rename does: the entry moves to
/// `teskad` (the tools agents see are `mcp__teskad__*`). An entry under that
/// name that runs the official app is left as it is
/// (`install::migrate_project_config`). Returns the workspaces rewritten.
pub fn rename_server_in_projects(state: &Path) -> Vec<PathBuf> {
    workspace_folders(state)
        .into_iter()
        .filter(|cwd| crate::install::migrate_project_config(cwd))
        .collect()
}

/// Imports every pair of `plan` that needs it.
pub fn run(plan: &Plan) -> Report {
    let mut report = Report::default();
    let mut state_files = Vec::new();
    for pair in &plan.pairs {
        let outcome = import(pair, plan.executable.as_deref());
        if matches!(outcome, Outcome::Imported { .. }) {
            state_files.push(pair.to.join("state.json"));
        }
        report.pairs.push((pair.to.clone(), outcome));
    }
    if let Some(exe) = &plan.executable {
        for state in state_files.iter().filter(|state| state.is_file()) {
            for cwd in workspace_folders(state) {
                if rewrite_project_config(&cwd.join(".mcp.json"), exe) {
                    report.rewritten_projects.push(cwd);
                }
            }
        }
    }
    report
}

fn import(pair: &Pair, executable: Option<&Path>) -> Outcome {
    if pair.to.join(MARKER).exists() {
        return Outcome::Skipped(Skip::AlreadyImported);
    }
    if pair.to.exists() {
        return Outcome::Skipped(Skip::TargetExists);
    }
    if !pair.from.is_dir() {
        return Outcome::Skipped(Skip::NoSource);
    }
    let staging = staging_of(&pair.to);
    // What a run that died left: partial, so never to be completed.
    let _ = fs::remove_dir_all(&staging);
    match copy_into(pair, &staging, executable) {
        Ok(files) => Outcome::Imported { files },
        Err(err) => {
            let _ = fs::remove_dir_all(&staging);
            Outcome::Failed(format!(
                "{} -> {}: {err}",
                pair.from.display(),
                pair.to.display()
            ))
        }
    }
}

fn staging_of(to: &Path) -> PathBuf {
    let mut name = to.file_name().unwrap_or_default().to_os_string();
    name.push(STAGING_SUFFIX);
    to.with_file_name(name)
}

fn copy_into(pair: &Pair, staging: &Path, executable: Option<&Path>) -> io::Result<usize> {
    if let Some(parent) = pair.to.parent() {
        fs::create_dir_all(parent)?;
    }
    create_private(staging)?;
    let files = copy_tree(&pair.from, staging, &pair.skip, true)?;
    if let Some(exe) = executable {
        rewrite_scope_files(&staging.join("mcp"), exe);
    }
    let marker = serde_json::json!({
        "from": pair.from.display().to_string(),
        "at": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs()),
    });
    fs::write(staging.join(MARKER), marker.to_string())?;
    fs::rename(staging, &pair.to)?;
    Ok(files)
}

/// A folder only its owner reads: `~/.teskad` holds the accounts' credentials.
fn create_private(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(dir)
    }
}

/// Copies `from` into `to` (which exists), file by file, never writing under
/// `from`. A link is copied as a link where the system allows it (Unix) and
/// left out elsewhere: following it could copy what is not the official app's.
fn copy_tree(from: &Path, to: &Path, skip: &[&str], top: bool) -> io::Result<usize> {
    let mut files = 0;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let text = name.to_string_lossy();
        if top && (skip.iter().any(|s| *s == text) || text.contains(".corrupt.")) {
            continue;
        }
        let target = to.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            fs::create_dir(&target)?;
            fs::set_permissions(&target, entry.metadata()?.permissions())?;
            files += copy_tree(&entry.path(), &target, skip, false)?;
        } else if kind.is_symlink() {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(fs::read_link(entry.path())?, &target)?;
                files += 1;
            }
        } else {
            // `fs::copy` carries the permissions over: a credentials file
            // stays 0600.
            fs::copy(entry.path(), &target)?;
            files += 1;
        }
    }
    Ok(files)
}

/// Points the `teskad` entry of an `mcpServers` document at `exe`; whether the
/// document changed. The entry keeps its `args` and `env` (its `HANGAR_*`
/// variables renamed `TESKAD_*`); one under an older name, `hangar` (the
/// official app's, as copied) or `iabench`, becomes `teskad`, as an install
/// does (install.rs).
///
/// A `shared` document is one the official app reads as well (a project's
/// `.mcp.json`): there, an entry under `hangar` is taken only when it runs
/// Teskad, and the official app's own is left in place.
fn point_at(doc: &mut Value, exe: &Path, shared: bool) -> bool {
    let exe = exe.to_string_lossy();
    let Some(servers) = doc.get_mut("mcpServers").and_then(Value::as_object_mut) else {
        return false;
    };
    let before = servers.clone();
    let upstream = crate::install::UPSTREAM_SERVER_KEY;
    let takes_upstream = servers
        .get(upstream)
        .is_some_and(|entry| !shared || crate::install::names_teskad(entry));
    let key = if servers
        .get(crate::install::SERVER_KEY)
        .is_some_and(Value::is_object)
    {
        crate::install::SERVER_KEY
    } else if takes_upstream && servers.get(upstream).is_some_and(Value::is_object) {
        upstream
    } else if servers.get("iabench").is_some_and(Value::is_object) {
        "iabench"
    } else {
        return false;
    };
    let mut entry = servers[key].clone();
    entry["command"] = Value::String(exe.into_owned());
    if entry.get("args").is_none() {
        entry["args"] = serde_json::json!(["--mcp"]);
    }
    crate::install::rename_env(&mut entry);
    servers.remove("iabench");
    if takes_upstream {
        servers.remove(upstream);
    }
    servers.insert(crate::install::SERVER_KEY.into(), entry);
    *servers != before
}

/// The copied `mcp/*.json` files a pane's Claude Code is pointed at. They are
/// rewritten at every pane start too (mcp_scope.rs); doing it here keeps one
/// that is read before then from naming the official binary.
fn rewrite_scope_files(dir: &Path, exe: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            rewrite_json(&path, exe, false);
        }
    }
}

/// A workspace's `.mcp.json`, when it holds the `teskad` server (or an
/// `iabench` one, or a `hangar` one that runs Teskad) pointing elsewhere than
/// at `exe`; the official app's `hangar` entry is not touched. Makes the one-time copy `<file>.hangar-bak` first,
/// as an install does. A file that does not parse is left as it is.
fn rewrite_project_config(path: &Path, exe: &Path) -> bool {
    path.is_file() && rewrite_json(path, exe, true)
}

fn rewrite_json(path: &Path, exe: &Path, backup: bool) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(mut doc) = serde_json::from_str::<Value>(raw.trim_start_matches('\u{feff}')) else {
        return false;
    };
    // The copied `mcp/*.json` are Teskad's own; a project's file is shared.
    if !point_at(&mut doc, exe, backup) {
        return false;
    }
    if backup {
        let copy = path.with_extension(format!(
            "{}.hangar-bak",
            path.extension().and_then(|e| e.to_str()).unwrap_or("")
        ));
        if !copy.exists() {
            let _ = fs::copy(path, copy);
        }
    }
    let Ok(body) = serde_json::to_string_pretty(&doc) else {
        return false;
    };
    crate::store::write_atomic(path, body.as_bytes(), crate::store::Access::AsBefore).is_ok()
}

/// The folders of the workspaces `state.json` lists (`cwd` and the other roots
/// of a `.code-workspace`), without reading more of its schema than that.
fn workspace_folders(state: &Path) -> Vec<PathBuf> {
    let Ok(raw) = fs::read_to_string(state) else {
        return Vec::new();
    };
    let Ok(doc) = serde_json::from_str::<Value>(raw.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let mut folders = Vec::new();
    for workspace in doc
        .get("workspaces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let cwd = workspace.get("cwd").and_then(Value::as_str);
        let extra = workspace
            .get("extraRoots")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str);
        for folder in cwd.into_iter().chain(extra).filter(|f| !f.is_empty()) {
            let folder = PathBuf::from(folder);
            if folder.is_absolute() && !folders.contains(&folder) {
                folders.push(folder);
            }
        }
    }
    folders
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Fixture {
        root: PathBuf,
        exe: PathBuf,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let root = crate::testdir::scratch(tag);
            Self {
                exe: root.join("bin/teskad"),
                root,
            }
        }

        fn old(&self) -> PathBuf {
            self.root.join("official")
        }

        fn new_dir(&self) -> PathBuf {
            self.root.join("teskad")
        }

        fn plan(&self) -> Plan {
            Plan {
                pairs: vec![Pair {
                    from: self.old(),
                    to: self.new_dir(),
                    skip: vec!["logs", "endpoint.json"],
                }],
                executable: Some(self.exe.clone()),
            }
        }

        fn put(&self, dir: &Path, relative: &str, body: &str) {
            let path = dir.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }

        fn official(&self) {
            let old = self.old();
            self.put(&old, "state.json", r#"{"workspaces":[]}"#);
            self.put(&old, "memory.json", "{}");
            self.put(&old, "activity/a.json", "[1]");
            self.put(&old, "accounts/work/.credentials.json", "secret");
            self.put(&old, "kits/ci/kit.json", "{}");
            self.put(&old, "mods/hangar-bridge/hooks/hooks.json", "{}");
            self.put(&old, "endpoint.json", r#"{"port":1,"token":"official"}"#);
            self.put(&old, "logs/hangar.log", "log");
        }

        fn tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
            fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
                for entry in fs::read_dir(dir).unwrap().flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        walk(base, &path, out);
                    } else {
                        let name = path.strip_prefix(base).unwrap().display().to_string();
                        out.push((name, fs::read(&path).unwrap()));
                    }
                }
            }
            let mut out = Vec::new();
            walk(dir, dir, &mut out);
            out.sort();
            out
        }
    }

    fn only(report: &Report) -> &Outcome {
        assert_eq!(report.pairs.len(), 1);
        &report.pairs[0].1
    }

    #[test]
    fn the_official_folder_alone_is_copied_and_left_intact() {
        let f = Fixture::new("migrate-old-only");
        f.official();
        let before = Fixture::tree(&f.old());

        let report = run(&f.plan());

        assert!(matches!(only(&report), Outcome::Imported { files } if *files == 6));
        assert_eq!(
            Fixture::tree(&f.old()),
            before,
            "the official folder is only read"
        );
        let new = f.new_dir();
        for kept in [
            "state.json",
            "memory.json",
            "activity/a.json",
            "accounts/work/.credentials.json",
            "kits/ci/kit.json",
            "mods/hangar-bridge/hooks/hooks.json",
            MARKER,
        ] {
            assert!(new.join(kept).is_file(), "{kept}");
        }
        assert!(
            !new.join("endpoint.json").exists(),
            "the official token stays out"
        );
        assert!(!new.join("logs").exists());
        assert!(!staging_of(&new).exists());
    }

    #[test]
    fn nothing_is_made_when_the_official_app_was_never_there() {
        let f = Fixture::new("migrate-none");
        let report = run(&f.plan());
        assert_eq!(*only(&report), Outcome::Skipped(Skip::NoSource));
        assert!(!f.new_dir().exists());
        assert!(!staging_of(&f.new_dir()).exists());
    }

    #[test]
    fn a_folder_of_teskad_s_own_is_never_written_into() {
        let f = Fixture::new("migrate-new-only");
        let new = f.new_dir();
        f.put(&new, "state.json", "mine");
        let report = run(&f.plan());
        assert_eq!(*only(&report), Outcome::Skipped(Skip::TargetExists));
        assert_eq!(
            Fixture::tree(&new),
            vec![("state.json".into(), b"mine".to_vec())]
        );
    }

    #[test]
    fn with_both_the_new_one_wins_and_the_old_one_is_untouched() {
        let f = Fixture::new("migrate-both");
        f.official();
        let new = f.new_dir();
        f.put(&new, "state.json", "mine");
        let before = Fixture::tree(&f.old());

        let report = run(&f.plan());

        assert_eq!(*only(&report), Outcome::Skipped(Skip::TargetExists));
        assert_eq!(fs::read_to_string(new.join("state.json")).unwrap(), "mine");
        assert!(!new.join("memory.json").exists());
        assert_eq!(Fixture::tree(&f.old()), before);
    }

    #[test]
    fn a_second_launch_does_not_copy_again_even_after_a_deletion() {
        let f = Fixture::new("migrate-marker");
        f.official();
        assert!(run(&f.plan()).imported());
        // The person cleared their memory in Teskad, and the official app
        // changed its own since.
        fs::remove_file(f.new_dir().join("memory.json")).unwrap();
        f.put(&f.old(), "memory.json", "{\"later\":1}");

        let again = run(&f.plan());

        assert_eq!(*only(&again), Outcome::Skipped(Skip::AlreadyImported));
        assert!(!f.new_dir().join("memory.json").exists());
    }

    #[test]
    fn an_interrupted_copy_leaves_no_folder_and_the_next_launch_completes_it() {
        let f = Fixture::new("migrate-interrupted");
        f.official();
        // What a run killed half way leaves: the staging folder, partial.
        let staging = staging_of(&f.new_dir());
        f.put(&staging, "state.json", "half");
        f.put(&staging, "junk/only-in-the-partial-copy", "x");

        assert!(
            !f.new_dir().exists(),
            "no half-imported folder to be mistaken for Teskad's own"
        );
        let report = run(&f.plan());

        assert!(matches!(only(&report), Outcome::Imported { .. }));
        assert!(!staging.exists());
        let new = f.new_dir();
        assert_eq!(
            fs::read_to_string(new.join("state.json")).unwrap(),
            r#"{"workspaces":[]}"#
        );
        assert!(!new.join("junk").exists());
        assert!(new.join(MARKER).is_file());
    }

    #[test]
    fn a_copy_that_fails_leaves_nothing_and_is_tried_again() {
        let f = Fixture::new("migrate-failure");
        // `from` is a file, not a folder of the official app: nothing to read.
        fs::write(f.old(), "not a folder").unwrap();
        let report = run(&f.plan());
        assert_eq!(*only(&report), Outcome::Skipped(Skip::NoSource));
        fs::remove_file(f.old()).unwrap();

        f.official();
        // A file that cannot be read stops the copy half way.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = f.old().join("zz-locked");
            fs::write(&locked, "x").unwrap();
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
            if fs::read(&locked).is_err() {
                let report = run(&f.plan());
                assert!(matches!(only(&report), Outcome::Failed(_)), "{report:?}");
                assert!(!f.new_dir().exists());
                assert!(!staging_of(&f.new_dir()).exists());
            }
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(run(&f.plan()).imported());
    }

    #[cfg(unix)]
    #[test]
    fn the_home_folder_and_the_credentials_stay_private() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new("migrate-modes");
        f.official();
        let secret = f.old().join("accounts/work/.credentials.json");
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(run(&f.plan()).imported());
        let mode = |p: PathBuf| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(f.new_dir()), 0o700);
        assert_eq!(
            mode(f.new_dir().join("accounts/work/.credentials.json")),
            0o600
        );
    }

    #[test]
    fn the_copied_mcp_files_run_this_app_not_the_official_one() {
        let f = Fixture::new("migrate-scope");
        f.official();
        let entry = |command: &str| {
            json!({
                "mcpServers": {
                    "hangar": {
                        "command": command,
                        "args": ["--mcp"],
                        "env": { "HANGAR_WORKSPACE": "/w" }
                    },
                    "linear": { "command": "npx", "args": ["linear"] }
                }
            })
            .to_string()
        };
        f.put(&f.old(), "mcp/ws-1.json", &entry("/opt/hangar/hangar-ai"));

        assert!(run(&f.plan()).imported());

        let copy: Value =
            serde_json::from_str(&fs::read_to_string(f.new_dir().join("mcp/ws-1.json")).unwrap())
                .unwrap();
        assert_eq!(
            copy["mcpServers"]["teskad"]["command"],
            f.exe.to_string_lossy().as_ref()
        );
        assert_eq!(
            copy["mcpServers"]["teskad"]["env"]["TESKAD_WORKSPACE"], "/w",
            "the variable the server reads is renamed with it"
        );
        assert!(copy["mcpServers"].get("hangar").is_none());
        assert_eq!(copy["mcpServers"]["linear"]["command"], "npx");
        let original = fs::read_to_string(f.old().join("mcp/ws-1.json")).unwrap();
        assert_eq!(
            original,
            entry("/opt/hangar/hangar-ai"),
            "the official copy is as it was"
        );
    }

    #[test]
    fn the_projects_of_the_copied_state_are_pointed_at_this_app() {
        let f = Fixture::new("migrate-projects");
        let (stale, current, plain) = (
            f.root.join("proj-stale"),
            f.root.join("proj-current"),
            f.root.join("proj-none"),
        );
        let state = json!({
            "workspaces": [
                { "cwd": stale, "extraRoots": [plain] },
                { "cwd": current },
                { "cwd": f.root.join("proj-gone") },
            ]
        });
        f.official();
        f.put(&f.old(), "state.json", &state.to_string());
        f.put(
            &stale,
            ".mcp.json",
            &json!({"mcpServers": {
                "iabench": { "command": "/old/iabench", "args": ["--mcp"] },
                "other": { "command": "x" }
            }})
            .to_string(),
        );
        let exe = f.exe.to_string_lossy().into_owned();
        let up_to_date = json!({"mcpServers": {"teskad": {"command": exe, "args": ["--mcp"]}}});
        f.put(&current, ".mcp.json", &up_to_date.to_string());
        f.put(&plain, "README.md", "no mcp here");

        let report = run(&f.plan());

        assert_eq!(report.rewritten_projects, vec![stale.clone()]);
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(stale.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(doc["mcpServers"]["teskad"]["command"], exe.as_str());
        assert!(
            doc["mcpServers"].get("iabench").is_none(),
            "the old name goes"
        );
        assert_eq!(doc["mcpServers"]["other"]["command"], "x");
        assert!(
            stale.join(".mcp.json.hangar-bak").is_file(),
            "the previous file is kept"
        );
        assert_eq!(
            fs::read_to_string(current.join(".mcp.json")).unwrap(),
            up_to_date.to_string(),
            "a config that is right is not rewritten"
        );
        assert!(!plain.join(".mcp.json").exists());
    }

    /// A project shared with the official app: its `hangar` entry is the
    /// official app's, and the migration copies nothing of it into Teskad's.
    #[test]
    fn a_project_config_keeps_the_official_apps_server_when_it_is_shared() {
        let f = Fixture::new("migrate-shared");
        let project = f.root.join("proj");
        f.official();
        f.put(
            &f.old(),
            "state.json",
            &json!({"workspaces":[{"cwd": project}]}).to_string(),
        );
        let body = json!({"mcpServers": {
            "hangar": { "command": "/opt/hangar/hangar-ai", "args": ["--mcp"] }
        }})
        .to_string();
        f.put(&project, ".mcp.json", &body);

        let report = run(&f.plan());

        assert!(report.rewritten_projects.is_empty());
        assert_eq!(fs::read_to_string(project.join(".mcp.json")).unwrap(), body);
        assert!(!project.join(".mcp.json.hangar-bak").exists());
    }

    fn config(dir: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(dir.join(".mcp.json")).unwrap()).unwrap()
    }

    #[test]
    fn teskad_s_server_under_the_upstream_name_moves_to_teskad() {
        let f = Fixture::new("migrate-rename");
        let (ours, official, both, none) = (
            f.root.join("ours"),
            f.root.join("official-project"),
            f.root.join("both"),
            f.root.join("none"),
        );
        let state = f.root.join("state.json");
        let folders: Vec<&PathBuf> = vec![&ours, &official, &both, &none];
        f.put(
            &f.root,
            "state.json",
            &json!({"workspaces": folders.iter().map(|cwd| json!({"cwd": cwd})).collect::<Vec<_>>()})
                .to_string(),
        );
        f.put(
            &ours,
            ".mcp.json",
            &json!({"mcpServers": {
                "hangar": {
                    "command": "/home/a/.local/bin/teskad",
                    "args": ["--mcp"],
                    "env": { "HANGAR_WORKSPACE": "/w", "KEEP": "1" }
                },
                "linear": { "command": "npx" }
            }})
            .to_string(),
        );
        let official_body = json!({"mcpServers": {
            "hangar": { "command": "/opt/hangar/hangar-ai", "args": ["--mcp"] }
        }})
        .to_string();
        f.put(&official, ".mcp.json", &official_body);
        // Already moved, and the upstream's entry left from before: ours stays.
        f.put(
            &both,
            ".mcp.json",
            &json!({"mcpServers": {
                "teskad": { "command": "/new/teskad", "args": ["--mcp"] },
                "hangar": { "command": "/old/teskad", "args": ["--mcp"] }
            }})
            .to_string(),
        );

        let renamed = rename_server_in_projects(&state);

        assert_eq!(renamed, vec![ours.clone(), both.clone()]);
        let doc = config(&ours);
        let servers = &doc["mcpServers"];
        assert!(servers.get("hangar").is_none());
        assert_eq!(servers["teskad"]["command"], "/home/a/.local/bin/teskad");
        assert_eq!(servers["teskad"]["env"]["TESKAD_WORKSPACE"], "/w");
        assert_eq!(servers["teskad"]["env"]["KEEP"], "1");
        assert!(servers["teskad"]["env"].get("HANGAR_WORKSPACE").is_none());
        assert_eq!(servers["linear"]["command"], "npx");
        assert!(ours.join(".mcp.json.hangar-bak").is_file());
        assert_eq!(
            fs::read_to_string(official.join(".mcp.json")).unwrap(),
            official_body,
            "the official app's entry is not touched"
        );
        assert!(!official.join(".mcp.json.hangar-bak").exists());
        let doc = config(&both);
        assert_eq!(doc["mcpServers"]["teskad"]["command"], "/new/teskad");
        assert!(doc["mcpServers"].get("hangar").is_none());
        assert!(!none.join(".mcp.json").exists());
        // Run again: nothing left to do.
        assert!(rename_server_in_projects(&state).is_empty());
    }

    #[test]
    fn a_project_config_that_does_not_parse_is_left_alone() {
        let f = Fixture::new("migrate-bad-config");
        let project = f.root.join("proj");
        f.official();
        f.put(
            &f.old(),
            "state.json",
            &json!({"workspaces":[{"cwd": project}]}).to_string(),
        );
        f.put(&project, ".mcp.json", "{ \"mcpServers\": ,}");
        let report = run(&f.plan());
        assert!(report.rewritten_projects.is_empty());
        assert_eq!(
            fs::read_to_string(project.join(".mcp.json")).unwrap(),
            "{ \"mcpServers\": ,}"
        );
    }

    #[test]
    fn a_development_run_does_not_read_the_official_app_s_files() {
        // The variables a development build sets (scripts/native-dev.sh).
        if crate::env::var_os("TESKAD_DATA_ROOT").is_some()
            || std::env::var_os("TESKAD_ENDPOINT_FILE").is_some()
        {
            assert!(system_plan().is_none());
        } else if let Some(plan) = system_plan() {
            let official = Paths::system_for(paths::OFFICIAL_IDENTIFIER);
            assert!(plan.pairs.iter().any(|pair| pair.from.ends_with(".hangar")));
            assert!(plan
                .pairs
                .iter()
                .any(|pair| Some(&pair.from) == official.app_data_dir().ok().as_ref()));
            assert!(plan.pairs.iter().all(|pair| pair.from != pair.to));
            assert!(plan.pairs.iter().all(|pair| !pair.to.ends_with(".hangar")));
        }
    }
}
