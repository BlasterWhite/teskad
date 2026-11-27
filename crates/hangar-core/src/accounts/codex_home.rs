//! Codex accounts: one `CODEX_HOME` per account, a *shadow* of the main one
//! (docs/native/agents.md §6.1, D16). The login is the account's own, the
//! rest is main's.
//!
//! Read in openai/codex at `rust-v0.170.0` (paths under `codex-rs/`):
//!
//! - **Private to a shadow home**: `auth.json` and `auth.lock`, where
//!   `codex login` keeps the login and locks it
//!   (`login/src/auth/storage.rs`), and `models_cache.json`, the models the
//!   account was offered (`models-manager/src/manager.rs`). A login Codex
//!   keeps in the system keyring instead (`cli_auth_credentials_store`, a
//!   file by default) is filed under the hash of the home's path, so a second
//!   home is a second login there too. Whatever is not linked below stays the
//!   shadow's own, as Codex makes it.
//! - **Linked to main's** (`SHARED_DIRS`, `SHARED_FILES`): the configuration,
//!   the person's tools, and the conversations, so that a session started on
//!   one account is resumed, counted and shown from every other. Codex writes
//!   `config.toml` through a symbolic link: its edits resolve the link first
//!   (`core/src/config/edit.rs` and `config/src/plugin_edit.rs`, through
//!   `utils/path-utils` `resolve_symlink_write_paths`), so the link holds and
//!   main's file is the one changed.
//! - **Not linked: the SQLite state.** A database opened through a link gets
//!   its `-wal` and `-shm` beside the link: two homes would write one
//!   database with two journals. A pane on a shadow home is started with
//!   `CODEX_SQLITE_HOME` naming main's folder instead, which Codex uses when
//!   `config.toml` names no `sqlite_home` (`core/src/config/mod.rs`,
//!   `state/src/lib.rs` `SQLITE_HOME_ENV`). One that does name it is main's
//!   file, so every home uses that folder all the same.
//! - **Not linked: `app-server-control/`**, the daemon's sockets: each home
//!   keeps its own daemon.
//!
//! Main is only ever read: a folder or file main does not have is not linked,
//! nor created on main. Folders are linked as Claude's profiles link theirs
//! (share.rs, a junction on Windows). A file link on Windows wants Developer
//! Mode; without it a configuration file is copied from main before each
//! launch, as Claude's `settings.json` is, and a history is hard-linked
//! (appends reach main's) — see `Fallback`.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use super::link;
use super::share::{self, Issue};
use super::store::write_atomic;

/// Folders a shadow home reaches through a link to main's own.
pub const SHARED_DIRS: &[&str] = &[
    // Conversations: rollouts, archived ones, generated images.
    "sessions",
    "archived_sessions",
    "generated_images",
    // The person's tools.
    "plugins",
    "skills",
    "log",
];

/// Files a shadow home reaches through a link to main's own, and what stands
/// in for the link where a file cannot be linked.
pub const SHARED_FILES: &[(&str, Fallback)] = &[
    ("config.toml", Fallback::Copy),
    ("hooks.json", Fallback::Copy),
    ("AGENTS.md", Fallback::Copy),
    // Thread names, and the prompts `↑` brings back: appended to.
    ("session_index.jsonl", Fallback::HardLink),
    ("history.jsonl", Fallback::HardLink),
];

/// The login and what goes with it: never a link, whoever made one.
pub const PRIVATE: &[&str] = &["auth.json", "auth.lock", "models_cache.json"];

/// What a shared file becomes when it cannot be a symbolic link (Windows
/// without Developer Mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// A copy of main's, made again before each launch when main's differs:
    /// a change made from a shadow pane is undone then, main is where the
    /// setup is changed. A file Codex rewrites whole would break a hard link.
    Copy,
    /// A hard link: what Codex appends from a shadow pane lands in main's. A
    /// rewrite (a thread's name removed) leaves the shadow a file of its own,
    /// which is kept and said.
    HardLink,
}

/// A Codex home: the folder `CODEX_HOME` names.
#[derive(Debug, Clone, PartialEq)]
pub struct CodexHome {
    pub dir: PathBuf,
}

impl CodexHome {
    /// The main account: `CODEX_HOME` as Hangar was started with it,
    /// `~/.codex` otherwise. Hangar never sets the variable on itself, only on
    /// the panes it starts.
    pub fn main() -> Option<Self> {
        crate::sessions::codex::home().map(Self::at)
    }

    pub fn at(dir: PathBuf) -> Self {
        CodexHome { dir }
    }

    /// Where `codex login` keeps the login.
    pub fn auth(&self) -> PathBuf {
        self.dir.join("auth.json")
    }
}

/// The folder Codex keeps its SQLite state in for a pane started without
/// `CODEX_SQLITE_HOME` of its own: `env`, the variable as Hangar has it, when
/// set (Codex trims it and resolves it against the pane's folder, so it is
/// passed on as it is); main's home otherwise.
pub fn sqlite_home(main: &CodexHome, env: Option<OsString>) -> PathBuf {
    env.filter(|value| !value.to_string_lossy().trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| main.dir.clone())
}

/// The variables a pane on the shadow home `dir` is started with: the home,
/// and main's SQLite folder (`sqlite_home`) when there is a main to name.
pub fn env(dir: &Path, sqlite_home: Option<PathBuf>) -> Vec<(&'static str, Option<PathBuf>)> {
    let mut vars = vec![("CODEX_HOME", Some(dir.to_path_buf()))];
    if let Some(sqlite) = sqlite_home {
        vars.push(("CODEX_SQLITE_HOME", Some(sqlite)));
    }
    vars
}

/// Brings `shadow` level with `main`: its links (or what stands in for
/// them), and no link where its login is. Nothing of `main` is written, and
/// nothing of the shadow's own is deleted but a copy of main's configuration
/// that drifted.
pub fn prepare(main: &CodexHome, shadow: &CodexHome) -> Vec<Issue> {
    let mut issues = Vec::new();
    if let Err(err) = fs::create_dir_all(&shadow.dir) {
        issues.push(Issue::new("profile", err));
        return issues;
    }
    // A Hangar started with `CODEX_HOME` on a shadow home has it as main.
    if link::same_place(&main.dir, &shadow.dir) {
        return issues;
    }
    for name in PRIVATE {
        let path = shadow.dir.join(name);
        if link::is_link(&path) {
            if let Err(err) = link::remove_link(&path) {
                issues.push(Issue::new(name, err));
            }
        }
    }
    for name in SHARED_DIRS {
        if let Err(reason) = share::share_dir(&main.dir.join(name), &shadow.dir.join(name)) {
            issues.push(Issue::new(name, reason));
        }
    }
    for (name, fallback) in SHARED_FILES {
        if let Err(reason) = share_file(&main.dir.join(name), &shadow.dir.join(name), *fallback) {
            issues.push(Issue::new(name, reason));
        }
    }
    issues
}

/// `here` made a link to the file `source` when main has it. A link that
/// leads elsewhere, or nowhere, goes; a file of the shadow's own stays, but
/// for a drifted copy of main's configuration (`Fallback::Copy`).
fn share_file(source: &Path, here: &Path, fallback: Fallback) -> Result<(), String> {
    let wanted = source.is_file();
    if link::is_link(here) {
        if wanted && link::same_place(here, source) {
            return Ok(());
        }
        link::remove_link(here).map_err(|e| e.to_string())?;
    } else if link::exists(here) {
        if !wanted || same_file(here, source) {
            return Ok(());
        }
        match fallback {
            Fallback::HardLink => {
                return Err("the account has a file of its own here; it is kept".into())
            }
            Fallback::Copy => {
                if fs::read(here).ok() == fs::read(source).ok() {
                    return Ok(());
                }
                fs::remove_file(here).map_err(|e| e.to_string())?;
            }
        }
    }
    if !wanted || link::link_file(source, here).is_ok() {
        return Ok(());
    }
    match fallback {
        Fallback::Copy => {
            let body = fs::read(source).map_err(|e| e.to_string())?;
            write_atomic(here, &body)
        }
        Fallback::HardLink => fs::hard_link(source, here).map_err(|e| e.to_string()),
    }
}

/// `a` and `b` are one file under two names (a hard link).
#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Without a stable file id in std, two names whose size and time agree are
/// taken for one file: a hard link does, a copy written later does not.
#[cfg(not(unix))]
fn same_file(a: &Path, b: &Path) -> bool {
    let a = share::stamp(a);
    a.is_some() && a == share::stamp(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::testing::Scratch;

    /// A main Codex home in use: configuration, tools, conversations, a login
    /// and its SQLite state.
    fn furnished(scratch: &Scratch) -> CodexHome {
        let main = scratch.codex_home();
        scratch.file("home/.codex/config.toml", "model = \"gpt-5.5\"\n");
        scratch.file("home/.codex/AGENTS.md", "Be brief.\n");
        scratch.file("home/.codex/history.jsonl", "{\"text\":\"hi\"}\n");
        scratch.file(
            "home/.codex/sessions/2026/11/18/rollout-2026-11-18T09-00-00-t1.jsonl",
            "{}\n",
        );
        scratch.file("home/.codex/skills/review/SKILL.md", "skill");
        scratch.file("home/.codex/auth.json", "{\"auth_mode\":\"chatgpt\"}");
        scratch.file("home/.codex/models_cache.json", "{}");
        scratch.file("home/.codex/state_5.sqlite", "db");
        main
    }

    #[cfg(unix)]
    fn shadow(scratch: &Scratch) -> CodexHome {
        CodexHome::at(scratch.root.join("accounts/codex/team"))
    }

    // File links: see `link::link_file` for Windows.
    #[cfg(unix)]
    #[test]
    fn a_shadow_home_shares_everything_but_the_login_and_the_state() {
        let scratch = Scratch::new("codex-shadow");
        let main = furnished(&scratch);
        let team = shadow(&scratch);

        let issues = prepare(&main, &team);
        assert!(issues.is_empty(), "{issues:?}");
        for name in [
            "config.toml",
            "AGENTS.md",
            "history.jsonl",
            "sessions",
            "skills",
        ] {
            let here = team.dir.join(name);
            assert!(link::is_link(&here), "{name}");
            assert!(link::same_place(&here, &main.dir.join(name)), "{name}");
        }
        assert!(team
            .dir
            .join("sessions/2026/11/18/rollout-2026-11-18T09-00-00-t1.jsonl")
            .is_file());
        // What main does not have is neither linked nor made on main.
        for name in ["hooks.json", "session_index.jsonl", "plugins", "log"] {
            assert!(!link::exists(&team.dir.join(name)), "{name}");
            assert!(!link::exists(&main.dir.join(name)), "{name}");
        }
        // The login, the models offered and the database stay apart.
        for name in [
            "auth.json",
            "auth.lock",
            "models_cache.json",
            "state_5.sqlite",
        ] {
            assert!(!link::exists(&team.dir.join(name)), "{name}");
        }

        // Written from a pane on the account, it lands in main's.
        fs::write(team.dir.join("config.toml"), "model = \"o5\"\n").unwrap();
        assert_eq!(
            fs::read_to_string(main.dir.join("config.toml")).unwrap(),
            "model = \"o5\"\n"
        );
        // A second launch finds it level.
        assert!(prepare(&main, &team).is_empty());
    }

    // File links: see `link::link_file` for Windows.
    #[cfg(unix)]
    #[test]
    fn a_login_is_never_reached_through_a_link() {
        let scratch = Scratch::new("codex-private");
        let main = furnished(&scratch);
        let team = shadow(&scratch);
        fs::create_dir_all(&team.dir).unwrap();
        // Linked by hand: logging in on the account would log main in.
        link::link_file(&main.auth(), &team.auth()).unwrap();

        assert!(prepare(&main, &team).is_empty());
        assert!(!link::exists(&team.auth()));
        assert!(main.auth().is_file());
    }

    // File links: see `link::link_file` for Windows.
    #[cfg(unix)]
    #[test]
    fn what_the_account_made_of_its_own_is_kept_and_said() {
        let scratch = Scratch::new("codex-own");
        let main = furnished(&scratch);
        let team = shadow(&scratch);
        // Before main had a session index or a log, the account made its own.
        scratch.file(
            "accounts/codex/team/session_index.jsonl",
            "{\"id\":\"t2\"}\n",
        );
        scratch.file("accounts/codex/team/log/codex-tui.log", "line\n");
        scratch.file("home/.codex/session_index.jsonl", "{\"id\":\"t1\"}\n");
        scratch.file("home/.codex/log/codex-tui.log", "main\n");
        // A copy of main's configuration that drifted is main's again.
        scratch.file("accounts/codex/team/AGENTS.md", "Be verbose.\n");

        let issues = prepare(&main, &team);
        let items: Vec<&str> = issues.iter().map(|issue| issue.item.as_str()).collect();
        assert_eq!(items, ["log", "session_index.jsonl"]);
        assert_eq!(
            fs::read_to_string(team.dir.join("session_index.jsonl")).unwrap(),
            "{\"id\":\"t2\"}\n"
        );
        assert!(team.dir.join("log/codex-tui.log").is_file());
        assert!(link::same_place(
            &team.dir.join("AGENTS.md"),
            &main.dir.join("AGENTS.md")
        ));
        assert_eq!(
            fs::read_to_string(main.dir.join("AGENTS.md")).unwrap(),
            "Be brief.\n"
        );
    }

    // File links: see `link::link_file` for Windows.
    #[cfg(unix)]
    #[test]
    fn a_link_main_no_longer_backs_goes() {
        let scratch = Scratch::new("codex-gone");
        let main = furnished(&scratch);
        let team = shadow(&scratch);
        assert!(prepare(&main, &team).is_empty());
        fs::remove_file(main.dir.join("AGENTS.md")).unwrap();

        assert!(prepare(&main, &team).is_empty());
        assert!(!link::exists(&team.dir.join("AGENTS.md")));
    }

    #[test]
    fn main_on_a_shadow_home_is_left_alone() {
        let scratch = Scratch::new("codex-self");
        let main = furnished(&scratch);
        assert!(prepare(&main, &main.clone()).is_empty());
        assert!(!link::is_link(&main.dir.join("config.toml")));
    }

    #[test]
    fn a_pane_on_a_shadow_home_names_it_and_mains_state() {
        let main = CodexHome::at(PathBuf::from("/home/me/.codex"));
        let team = PathBuf::from("/home/me/.teskad/accounts/codex/team");

        assert_eq!(sqlite_home(&main, None), main.dir);
        assert_eq!(sqlite_home(&main, Some(" ".into())), main.dir);
        assert_eq!(
            sqlite_home(&main, Some("/var/codex".into())),
            PathBuf::from("/var/codex")
        );
        assert_eq!(
            env(&team, Some(main.dir.clone())),
            vec![
                ("CODEX_HOME", Some(team.clone())),
                ("CODEX_SQLITE_HOME", Some(main.dir.clone())),
            ]
        );
        assert_eq!(env(&team, None), vec![("CODEX_HOME", Some(team))]);
    }
}
