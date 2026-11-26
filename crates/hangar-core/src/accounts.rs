//! « Comptes »: several subscriptions of each agent on one machine — one
//! Claude Code profile, or one Codex home, per account — and every agent pane
//! launched on the account chosen for it. The accounts of every kind are
//! listed in one `accounts.json`, by kind (store.rs); a Claude account is
//! told below, a Codex account under « Codex ».
//!
//! # Profiles
//!
//! Claude Code reads its login and its configuration from the folder named
//! by `CLAUDE_CONFIG_DIR` — `~/.claude`, with `~/.claude.json` beside it,
//! when the variable is not set; under the variable, `.claude.json` lives
//! inside the folder. An account is such a folder:
//!
//! - **main** is Claude Code's own configuration, the one Hangar was started
//!   with (`CLAUDE_CONFIG_DIR` if it was set, `~/.claude` otherwise). It is
//!   always there, cannot be removed, and its files are only ever read.
//! - every other account is a profile Hangar made, `~/.hangar/accounts/<id>/`
//!   (beside `HANGAR_ENDPOINT_FILE` for a development build, like the mods of
//!   bridge.rs), listed in `~/.hangar/accounts/accounts.json` as
//!   `{ id, label, dir, addedAt }` (store.rs).
//!
//! A Claude Code pane on a profile is started with `CLAUDE_CONFIG_DIR` set to
//! its folder (pty.rs); a pane on main, and every pane that is not Claude
//! Code, gets the environment Hangar has. Nothing is ever switched without
//! the person: there is no rotation when a limit is reached, here or
//! anywhere else.
//!
//! # Same setup, own login
//!
//! A profile holds its own login — `.credentials.json` (or its own Keychain
//! entry on macOS) and its own `.claude.json`, where Claude Code keeps the
//! account it is logged in as — and takes everything else from main
//! (share.rs), before each launch:
//!
//! - **Folders** are links to main's: a junction on Windows, which needs no
//!   privilege, a symbolic link elsewhere. Claude Code writes through them
//!   as through any folder, so a conversation started on one account is
//!   filed in main's `projects/` and `claude --resume <id>` finds it from
//!   every account. Linked: `projects`, `file-history`, `todos`, `plans` (a
//!   conversation and what it brings with it), `skills`, `commands`,
//!   `agents`, `output-styles`, `rules`, `plugins` (the person's tools). A
//!   folder main does not have is not linked, nor created on main.
//! - **Files** Claude Code rewrites in place are copied, one way, from main:
//!   `settings.json`, `CLAUDE.md`, `keybindings.json`. A link would not do:
//!   Claude Code 2.1.316 refuses to write its settings through a symbolic
//!   link, and its atomic rewrites break a hard link. A change made from a
//!   pane on a profile is therefore undone at the next launch: main is where
//!   the setup is changed.
//! - **`.claude.json`** is the profile's own, but its setup keys are copied
//!   from main's: the user-scope MCP servers (`mcpServers`), and for each
//!   folder main knows, the trust given to it — an agent would otherwise
//!   wait on the trust dialog — its local-scope servers and the choices made
//!   about its `.mcp.json` servers. The login, the account and everything
//!   else Claude Code keeps there are never touched. Hangar does not take
//!   Claude Code's lock on that file: it renames its version over it only if
//!   the file has not moved since it was read, and reads it again otherwise.
//!
//! Nothing of main is modified, logged, or copied anywhere but into the
//! profile, which lives beside it in the person's home. Since every
//! conversation lands in main's `projects/`, whatever reads transcripts —
//! the session list and resume (sessions.rs), the statistics (stats.rs), the
//! context gauge and the phone's chat (context.rs) — sees every account's
//! by reading main's alone, and counts each once. A profile whose folder
//! could not be linked keeps its own; the session list and the statistics
//! read it too (`projects_dirs`).
//!
//! # macOS
//!
//! On macOS Claude Code keeps the login in the Keychain, not in
//! `.credentials.json`. Claude Code 2.1.316 names the entry
//! `Claude Code-credentials` for the default folder and
//! `Claude Code-credentials-<first 8 hex digits of sha256(folder)>` when
//! `CLAUDE_CONFIG_DIR` is set (read in its bundle), so each profile has an
//! entry of its own and logins stay apart. `CLAUDE_SECURESTORAGE_CONFIG_DIR`
//! overrides that name, so it is taken out of a profile pane's environment.
//! An older Claude Code that kept one entry for every folder would log a
//! profile in over main: `accounts_support` checks that each logged-in
//! profile has its own entry and says plainly when one does not
//! (login.rs).
//!
//! # Logging in
//!
//! `account_add` prepares the profile and answers with `claude auth login`,
//! which the person runs in a visible Claude Code pane on that account —
//! `Pane.login` in src/types.ts — and finishes in their browser. Hangar
//! never logs in, out, or switches an account by itself.
//! `Event::AccountsChanged` (the window's `accounts:changed`) is sent after
//! every change made here, and when a profile's login appears, changes or
//! goes (`watch`).
//!
//! # Codex
//!
//! A Codex account is a *shadow* `CODEX_HOME` (codex_home.rs,
//! docs/native/agents.md §6.1): main is the person's own (`CODEX_HOME` if
//! Hangar was started with it, `~/.codex` otherwise), only ever read; every
//! other account is `~/.hangar/accounts/codex/<id>/`, with its own
//! `auth.json` and models cache and links to main's configuration, tools and
//! conversations. A pane on it — the agent and its server — is started with
//! `CODEX_HOME` naming it and `CODEX_SQLITE_HOME` naming main's SQLite folder
//! (`env_for`). Its login is `codex login` in a pane on it, which writes
//! only its own `auth.json`; who it is logged in as is read from that file
//! (login.rs), its tokens never held. Logins stay apart by construction: a
//! file per home, and a keyring entry named after the home's path.

mod codex_home;
mod link;
mod login;
mod share;
mod store;
#[cfg(test)]
mod testing;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::agent::AgentKind;
use crate::host::{Event, Host};

pub use codex_home::CodexHome;
pub use share::Issue;
use share::{stamp, Stamp};
use store::Store;

/// The account that is the agent's own configuration, of every kind.
pub const MAIN: &str = "main";
/// The id of Claude Code, the agent `account_add` and the other calls
/// without a kind are for.
pub const CLAUDE_AGENT: &str = "claude";
/// What logs a profile in, typed in a pane whose Claude Code runs on it
/// (checked against Claude Code 2.1.316, which also has `claude` then
/// `/login`).
pub const LOGIN_COMMAND: &str = "claude auth login";
/// What logs a Codex account in, typed in a pane on its home: it writes that
/// home's `auth.json` (`--device-auth` where no browser can be opened).
pub const CODEX_LOGIN_COMMAND: &str = "codex login";
/// How often the logins are looked at for `Event::AccountsChanged`.
const WATCH_EVERY: Duration = Duration::from_secs(3);
/// Claude Code keeps tokens in the Keychain on macOS, in
/// `.credentials.json` elsewhere.
const KEYCHAIN: bool = cfg!(target_os = "macos");

/// A Claude Code configuration: the folder `CLAUDE_CONFIG_DIR` names, and
/// the `.claude.json` that goes with it.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeHome {
    pub dir: PathBuf,
    pub config: PathBuf,
}

impl ClaudeHome {
    /// The main account: Claude Code's default, or the `CLAUDE_CONFIG_DIR`
    /// Hangar was started with. Hangar never sets the variable on itself,
    /// only on the panes it starts.
    pub fn main() -> Option<Self> {
        let custom = std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from);
        Some(Self::main_at(&dirs::home_dir()?, custom))
    }

    fn main_at(home: &Path, custom: Option<PathBuf>) -> Self {
        let dir = custom.clone().unwrap_or_else(|| home.join(".claude"));
        let parent = custom.unwrap_or_else(|| home.to_path_buf());
        Self::with_config(dir, &parent)
    }

    /// A profile: `.claude.json` inside its folder, as under
    /// `CLAUDE_CONFIG_DIR`.
    pub fn profile(dir: PathBuf) -> Self {
        let parent = dir.clone();
        Self::with_config(dir, &parent)
    }

    /// A configuration written by old versions, `.config.json` in the folder,
    /// is the one Claude Code still prefers when it is there.
    fn with_config(dir: PathBuf, parent: &Path) -> Self {
        let legacy = dir.join(".config.json");
        let config = if legacy.is_file() {
            legacy
        } else {
            parent.join(".claude.json")
        };
        ClaudeHome { dir, config }
    }

    pub fn settings(&self) -> PathBuf {
        self.dir.join("settings.json")
    }

    pub fn credentials(&self) -> PathBuf {
        self.dir.join(".credentials.json")
    }

    pub fn projects(&self) -> PathBuf {
        self.dir.join("projects")
    }
}

/// `~/.teskad/accounts/`, or beside the endpoint file a development build
/// run next to an installed Hangar was given (endpoint.rs), so that it does
/// not adopt the installed app's profiles.
fn store_root() -> Option<PathBuf> {
    if let Some(file) = std::env::var_os("HANGAR_ENDPOINT_FILE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(file).parent()?.join("accounts"));
    }
    Some(crate::paths::home_folder()?.join("accounts"))
}

/// The main account of each kind.
#[derive(Debug, Clone)]
struct Mains {
    claude: ClaudeHome,
    codex: CodexHome,
}

impl Mains {
    fn dir(&self, kind: AgentKind) -> &Path {
        match kind {
            AgentKind::Claude => &self.claude.dir,
            AgentKind::Codex => &self.codex.dir,
        }
    }
}

fn homes() -> Result<(Mains, Store), String> {
    match (ClaudeHome::main(), CodexHome::main(), store_root()) {
        (Some(claude), Some(codex), Some(root)) => Ok((Mains { claude, codex }, Store::at(root))),
        _ => Err("No home directory on this system.".into()),
    }
}

/// What logs an account of `kind` in, typed in a pane on it.
pub fn login_command(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => LOGIN_COMMAND,
        AgentKind::Codex => CODEX_LOGIN_COMMAND,
    }
}

/// An account's folder, by kind: what its login is read from.
enum Home {
    Claude(ClaudeHome),
    Codex(CodexHome),
}

impl Home {
    fn main(mains: &Mains, kind: AgentKind) -> Self {
        match kind {
            AgentKind::Claude => Home::Claude(mains.claude.clone()),
            AgentKind::Codex => Home::Codex(mains.codex.clone()),
        }
    }

    /// The account `id` of `kind` Hangar made.
    fn of(store: &Store, kind: AgentKind, id: &str) -> Self {
        let dir = store.dir_of(kind, id);
        match kind {
            AgentKind::Claude => Home::Claude(ClaudeHome::profile(dir)),
            AgentKind::Codex => Home::Codex(CodexHome::at(dir)),
        }
    }

    fn dir(&self) -> &Path {
        match self {
            Home::Claude(home) => &home.dir,
            Home::Codex(home) => &home.dir,
        }
    }
}

/// Every change to the index or to a profile's folder goes through here, so
/// two panes starting on one profile prepare it one after the other.
fn lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------------

/// One account, as the window lists it. The email and the plan come from the
/// account's own files on this machine and go nowhere else.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub label: String,
    pub is_main: bool,
    pub logged_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
}

/// Logins already read, by folder, with the stamps of the files they were
/// read from (`.claude.json` and `.credentials.json`, or `auth.json`):
/// polling every few seconds reads nothing that did not move.
fn read_login(home: &Home) -> login::Login {
    type Cache = HashMap<PathBuf, (Option<Stamp>, Option<Stamp>, login::Login)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let stamps = match home {
        Home::Claude(home) => (stamp(&home.config), stamp(&home.credentials())),
        Home::Codex(home) => (stamp(&home.auth()), None),
    };
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((first, second, login)) = cache.get(home.dir()) {
        if (*first, *second) == stamps {
            return login.clone();
        }
    }
    let login = match home {
        Home::Claude(home) => login::read_login(home, KEYCHAIN),
        Home::Codex(home) => login::read_codex_login(&home.auth()),
    };
    cache.insert(
        home.dir().to_path_buf(),
        (stamps.0, stamps.1, login.clone()),
    );
    login
}

fn account(id: &str, label: &str, home: &Home) -> Account {
    let login = read_login(home);
    Account {
        id: id.to_string(),
        label: label.to_string(),
        is_main: id == MAIN,
        logged_in: login.logged_in,
        email: login.email,
        plan: login.plan,
    }
}

/// The accounts of `kind`: main first, then the others in the order they
/// were added. An index nobody can read lists main alone rather than
/// nothing.
fn list_with(mains: &Mains, store: &Store, kind: AgentKind) -> Vec<Account> {
    let index = store.read().unwrap_or_else(|err| {
        eprintln!("accounts: {err}");
        store::Index::default()
    });
    std::iter::once(account(
        MAIN,
        index.main_label(kind),
        &Home::main(mains, kind),
    ))
    .chain(
        index
            .of(kind)
            .map(|entry| account(&entry.id, &entry.label, &Home::of(store, kind, &entry.id))),
    )
    .collect()
}

fn list(kind: AgentKind) -> Vec<Account> {
    match homes() {
        Ok((mains, store)) => list_with(&mains, &store, kind),
        Err(_) => Vec::new(),
    }
}

/// Every kind's list, in `AgentKind::ALL`'s order: what the windows are told
/// changed.
fn list_all() -> Vec<Vec<Account>> {
    AgentKind::ALL.iter().map(|kind| list(*kind)).collect()
}

// ---------------------------------------------------------------------------
// What the other modules ask
// ---------------------------------------------------------------------------

/// The account a pane is launched on.
#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    /// The account it really runs on: main when the one asked for is gone.
    pub id: String,
    /// The account's folder, what `CLAUDE_CONFIG_DIR` or `CODEX_HOME` is set
    /// to (`env_for`); `None` on main, which keeps the environment Hangar
    /// has.
    pub dir: Option<PathBuf>,
    /// What the account could not take from main this time.
    pub issues: Vec<Issue>,
}

impl Launch {
    fn main() -> Self {
        Launch {
            id: MAIN.to_string(),
            dir: None,
            issues: Vec::new(),
        }
    }
}

fn chosen(account: Option<&str>) -> Option<&str> {
    account
        .map(str::trim)
        .filter(|id| !id.is_empty() && *id != MAIN)
}

/// The account a pane of `agent` (an `AgentKind` id) starts on, its folder
/// prepared: `None` for a pane no account applies to, a shell's. An id that
/// is not an account of that agent — removed since the window chose it, or
/// another agent's — runs on main.
pub fn for_launch(agent: Option<&str>, account: Option<&str>) -> Option<Launch> {
    let kind = agent.and_then(AgentKind::from_id)?;
    let Some(id) = chosen(account) else {
        return Some(Launch::main());
    };
    let Ok((mains, store)) = homes() else {
        return Some(Launch::main());
    };
    Some(launch_with(&mains, &store, kind, id))
}

fn launch_with(mains: &Mains, store: &Store, kind: AgentKind, id: &str) -> Launch {
    let _guard = lock();
    let listed = store
        .read()
        .is_ok_and(|index| index.find(kind, id).is_some());
    if !listed {
        return Launch::main();
    }
    let dir = store.dir_of(kind, id);
    let issues = prepare(mains, kind, &dir);
    Launch {
        id: id.to_string(),
        dir: Some(dir),
        issues,
    }
}

/// Brings the account at `dir` level with main: a Claude profile's links,
/// copies and settings (share.rs), a Codex home's links (codex_home.rs).
fn prepare(mains: &Mains, kind: AgentKind, dir: &Path) -> Vec<Issue> {
    match kind {
        AgentKind::Claude => share::prepare(&mains.claude, &ClaudeHome::profile(dir.to_path_buf())),
        AgentKind::Codex => codex_home::prepare(&mains.codex, &CodexHome::at(dir.to_path_buf())),
    }
}

/// The configuration a Claude Code pane on `account` reads, for the MCP
/// servers its workspace chose (mcp_scope.rs): main's for main, an unknown
/// id or none; the profile's otherwise, `prepared` first when the file
/// written from it is about to be used.
pub fn home_for(account: Option<&str>, prepared: bool) -> Option<ClaudeHome> {
    let main = ClaudeHome::main()?;
    let Some(id) = chosen(account) else {
        return Some(main);
    };
    let Ok((mains, store)) = homes() else {
        return Some(main);
    };
    if prepared {
        return Some(
            match launch_with(&mains, &store, AgentKind::Claude, id).dir {
                Some(dir) => ClaudeHome::profile(dir),
                None => main,
            },
        );
    }
    let listed = store
        .read()
        .is_ok_and(|index| index.find(AgentKind::Claude, id).is_some());
    Some(if listed {
        ClaudeHome::profile(store.dir_of(AgentKind::Claude, id))
    } else {
        main
    })
}

/// Every folder Claude Code files conversations in, each once: main's
/// `projects/`, which every profile's links to, and the `projects/` of a
/// profile that could not be linked and kept its own.
pub fn projects_dirs() -> Vec<PathBuf> {
    match homes() {
        Ok((mains, store)) => projects_with(&mains.claude, &store),
        Err(_) => Vec::new(),
    }
}

fn projects_with(main: &ClaudeHome, store: &Store) -> Vec<PathBuf> {
    let mut dirs = vec![main.projects()];
    let index = store.read().unwrap_or_default();
    for entry in index.of(AgentKind::Claude) {
        let projects = ClaudeHome::profile(store.dir_of(AgentKind::Claude, &entry.id)).projects();
        if projects.is_dir() && !dirs.iter().any(|known| link::same_place(known, &projects)) {
            dirs.push(projects);
        }
    }
    dirs
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct LoginCommand {
    pub command: String,
}

#[derive(Debug, Serialize)]
pub struct Added {
    pub account: Account,
    pub login: LoginCommand,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Support {
    pub isolated_logins: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The last lists told to the windows, so that the watch does not tell a
/// change made here a second time.
fn last_told() -> &'static Mutex<Option<Vec<Vec<Account>>>> {
    static LAST: Mutex<Option<Vec<Vec<Account>>>> = Mutex::new(None);
    &LAST
}

fn tell(host: &Host) {
    let now = list_all();
    if let Ok(mut last) = last_told().lock() {
        *last = Some(now);
    }
    let _ = host.emit(Event::AccountsChanged);
}

/// The Claude accounts.
pub fn accounts_list() -> Vec<Account> {
    list(AgentKind::Claude)
}

/// The accounts of `kind`, main first.
pub fn accounts_of(kind: AgentKind) -> Vec<Account> {
    list(kind)
}

/// A new Claude account named `label` (`account_add_of`).
pub fn account_add(host: &Host, label: String) -> Result<Added, String> {
    account_add_of(host, AgentKind::Claude, label)
}

/// A new account of `kind` named `label`, ready for its login. An account
/// that could not share main's conversations is not made: a conversation
/// could not move to it or from it.
pub fn account_add_of(host: &Host, kind: AgentKind, label: String) -> Result<Added, String> {
    let (mains, store) = homes()?;
    let added = add_with(&mains, &store, kind, &label, now_ms())?;
    tell(host);
    Ok(added)
}

fn add_with(
    mains: &Mains,
    store: &Store,
    kind: AgentKind,
    label: &str,
    now: u64,
) -> Result<Added, String> {
    let _guard = lock();
    let entry = store.add(kind, label, now)?;
    let dir = store.dir_of(kind, &entry.id);
    let issues = prepare(mains, kind, &dir);
    // Where each kind keeps its conversations.
    let conversations = match kind {
        AgentKind::Claude => "projects",
        AgentKind::Codex => "sessions",
    };
    if let Some(issue) = issues
        .iter()
        .find(|issue| issue.item == conversations || issue.item == "profile")
    {
        let _ = store.forget(&entry.id);
        delete_tree(&dir);
        return Err(format!(
            "The account could not share the conversations of the main one: {}",
            issue.reason
        ));
    }
    Ok(Added {
        account: account(&entry.id, &entry.label, &Home::of(store, kind, &entry.id)),
        login: LoginCommand {
            command: login_command(kind).to_string(),
        },
    })
}

/// Renames a Claude account, its main one included (`account_rename_of`).
pub fn account_rename(host: &Host, id: String, label: String) -> Result<Account, String> {
    account_rename_of(host, AgentKind::Claude, id, label)
}

/// Renames an account of `kind`, its main one included.
pub fn account_rename_of(
    host: &Host,
    kind: AgentKind,
    id: String,
    label: String,
) -> Result<Account, String> {
    let (mains, store) = homes()?;
    let renamed = rename_with(&mains, &store, kind, &id, &label)?;
    tell(host);
    Ok(renamed)
}

fn rename_with(
    mains: &Mains,
    store: &Store,
    kind: AgentKind,
    id: &str,
    label: &str,
) -> Result<Account, String> {
    let label = {
        let _guard = lock();
        store.rename(kind, id, label)?
    };
    let home = if id == MAIN {
        Home::main(mains, kind)
    } else {
        Home::of(store, kind, id)
    };
    Ok(account(id, &label, &home))
}

/// Deletes an account Hangar made, of any kind — its folder, its login —
/// once the window has asked the person. Never main, nothing the links lead
/// to, and not while a pane still runs on it.
pub fn account_remove(
    host: &Host,
    manager: &crate::pty::PtyManager,
    id: String,
) -> Result<(), String> {
    let running = crate::pty::panes_on(manager, &id);
    if running > 0 {
        return Err(format!(
            "{running} pane(s) still run on this account: switch or close them first."
        ));
    }
    let (mains, store) = homes()?;
    remove_with(&mains, &store, &id, now_ms())?;
    tell(host);
    Ok(())
}

fn remove_with(mains: &Mains, store: &Store, id: &str, now: u64) -> Result<(), String> {
    if id == MAIN {
        return Err("The main account cannot be removed.".into());
    }
    let _guard = lock();
    let index = store.read()?;
    let Some(entry) = index
        .accounts
        .iter()
        .find(|entry| entry.id == id && store::valid_id(id))
    else {
        return Err("No such account.".into());
    };
    let dir = store.dir_of(entry.kind, id);
    let main = mains.dir(entry.kind);
    if link::same_place(&dir, main) || main.starts_with(&dir) {
        return Err("This account's folder is the main one's: it is not removed.".into());
    }
    // A Claude account made before Codex accounts with the id `codex` holds
    // their folder.
    if index
        .accounts
        .iter()
        .any(|other| other.id != id && store.dir_of(other.kind, &other.id).starts_with(&dir))
    {
        return Err("Other accounts' folders are inside this one's: it is not removed.".into());
    }
    // Moved aside in one step first: a folder still in use stays whole.
    let trash = store.trash(id, now);
    if link::exists(&dir) {
        fs::rename(&dir, &trash).map_err(|err| {
            format!("The account's folder is in use ({err}): close what uses it and try again.")
        })?;
    }
    if let Err(err) = store.forget(id) {
        let _ = fs::rename(&trash, &dir);
        return Err(err);
    }
    delete_tree(&trash);
    Ok(())
}

/// An account's folder, deleted: its links taken out first, so nothing is
/// ever removed through one. Only ever called on a folder of the store.
fn delete_tree(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if link::is_link(&path) {
                let _ = link::remove_link(&path);
            }
        }
    }
    let _ = fs::remove_dir_all(dir);
}

/// Whether logging in to one Claude account here leaves the others logged
/// in. Codex's always do: a file per home, a keyring entry per home's path.
pub fn accounts_support() -> Support {
    if !KEYCHAIN {
        return Support {
            isolated_logins: true,
            note: None,
        };
    }
    let profiles: Vec<ClaudeHome> = match homes() {
        Ok((_, store)) => store
            .read()
            .unwrap_or_default()
            .of(AgentKind::Claude)
            .map(|entry| ClaudeHome::profile(store.dir_of(AgentKind::Claude, &entry.id)))
            .collect(),
        Err(_) => Vec::new(),
    };
    support_from(login::logins_isolated(&profiles, login::keychain_has))
}

fn support_from(isolated: bool) -> Support {
    Support {
        isolated_logins: isolated,
        note: (!isolated).then(|| {
            "This Claude Code keeps a single Keychain login for every account. \
             Update Claude Code (2.1.316 keeps one per account), then log in to each account again."
                .to_string()
        }),
    }
}

/// Tells the windows when an account's login appears, changes or goes —
/// the person logged in from a pane, or out — by looking at the accounts'
/// files of every kind every few seconds. Also deletes what a removal could
/// not.
pub fn watch(host: Host) {
    std::thread::spawn(move || {
        if let Some(root) = store_root() {
            for trash in Store::at(root).trashed() {
                delete_tree(&trash);
            }
        }
        loop {
            let now = list_all();
            let changed = {
                let mut last = last_told()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let changed = last.as_ref().is_some_and(|last| *last != now);
                *last = Some(now);
                changed
            };
            if changed {
                let _ = host.emit(Event::AccountsChanged);
            }
            std::thread::sleep(WATCH_EVERY);
        }
    });
}

/// The variables a launch of `agent` (an `AgentKind` id) on the account
/// whose folder is `dir` (`Launch::dir`) needs: `Some` sets one, `None`
/// takes it out of the environment.
///
/// - Claude Code on a profile: `CLAUDE_CONFIG_DIR`, without
///   `CLAUDE_SECURESTORAGE_CONFIG_DIR`, which would make every profile share
///   one Keychain login on macOS.
/// - Codex on a shadow home: `CODEX_HOME`, and `CODEX_SQLITE_HOME` naming
///   main's SQLite folder (codex_home.rs).
///
/// Nothing for main, nor for a pane no account applies to: those keep the
/// environment Hangar has, its own `CLAUDE_CONFIG_DIR` or `CODEX_HOME`
/// included.
pub fn env_for(agent: Option<&str>, dir: Option<&Path>) -> Vec<(&'static str, Option<PathBuf>)> {
    let Some(dir) = dir else {
        return Vec::new();
    };
    match agent.and_then(AgentKind::from_id) {
        Some(AgentKind::Claude) => vec![
            ("CLAUDE_CONFIG_DIR", Some(dir.to_path_buf())),
            ("CLAUDE_SECURESTORAGE_CONFIG_DIR", None),
        ],
        Some(AgentKind::Codex) => codex_home::env(
            dir,
            CodexHome::main()
                .map(|main| codex_home::sqlite_home(&main, std::env::var_os("CODEX_SQLITE_HOME"))),
        ),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentKind::{Claude, Codex};
    use testing::Scratch;

    fn store(scratch: &Scratch) -> Store {
        Store::at(scratch.root.join("accounts"))
    }

    /// Main accounts in use: Claude Code's logged in on « pro », Codex's on
    /// a ChatGPT « team » account (fixtures/codex/chatgpt.json).
    fn furnished(scratch: &Scratch) -> Mains {
        let claude = scratch.main_home();
        scratch.file("home/.claude/projects/C--repo/conv-1.jsonl", "{}\n");
        scratch.file("home/.claude/skills/review/SKILL.md", "skill");
        scratch.file("home/.claude/settings.json", "{}");
        scratch.file(
            "home/.claude/.credentials.json",
            r#"{"claudeAiOauth":{"accessToken":"main","subscriptionType":"pro"}}"#,
        );
        scratch.file(
            "home/.claude.json",
            r#"{"oauthAccount":{"emailAddress":"main@example.com"}}"#,
        );
        let codex = scratch.codex_home();
        scratch.file("home/.codex/config.toml", "model = \"gpt-5.5\"\n");
        scratch.file(
            "home/.codex/sessions/2026/11/18/rollout-2026-11-18T09-00-00-t1.jsonl",
            "{}\n",
        );
        scratch.file(
            "home/.codex/auth.json",
            include_str!("accounts/fixtures/codex/chatgpt.json"),
        );
        Mains { claude, codex }
    }

    #[test]
    fn main_is_claude_codes_default_or_the_folder_hangar_was_given() {
        let home = Path::new("/home/me");
        let main = ClaudeHome::main_at(home, None);
        assert_eq!(main.dir, home.join(".claude"));
        assert_eq!(main.config, home.join(".claude.json"));
        let custom = ClaudeHome::main_at(home, Some(PathBuf::from("/cfg")));
        assert_eq!(custom.dir, PathBuf::from("/cfg"));
        assert_eq!(custom.config, PathBuf::from("/cfg/.claude.json"));
        let profile = ClaudeHome::profile(PathBuf::from("/p/work"));
        assert_eq!(profile.config, PathBuf::from("/p/work/.claude.json"));
    }

    #[test]
    fn an_added_account_is_listed_after_main_and_logged_out() {
        let scratch = Scratch::new("add");
        let mains = furnished(&scratch);
        let store = store(&scratch);
        let added = add_with(&mains, &store, Claude, "Work", 7).unwrap();
        assert_eq!(added.login.command, "claude auth login");
        assert_eq!(added.account.id, "work");
        assert!(!added.account.logged_in);

        let listed = list_with(&mains, &store, Claude);
        assert_eq!(listed.len(), 2);
        assert_eq!(
            listed[0],
            Account {
                id: "main".into(),
                label: "Main".into(),
                is_main: true,
                // With the Keychain (macOS), the account in .claude.json is
                // the login, and the plan is not known.
                logged_in: true,
                email: Some("main@example.com".to_string()),
                plan: (!KEYCHAIN).then(|| "pro".to_string()),
            }
        );
        assert_eq!(listed[1].id, "work");
        assert!(!listed[1].is_main);
        // Its folder is ready: the conversations and tools of main are there.
        let dir = store.dir_of(Claude, "work");
        assert!(dir.join("projects/C--repo/conv-1.jsonl").is_file());
        assert!(dir.join("skills/review/SKILL.md").is_file());
        // No Codex account came with it.
        assert_eq!(list_with(&mains, &store, Codex).len(), 1);

        // The serialized shape the window reads (src/lib/accounts.ts).
        let json = serde_json::to_value(&added).unwrap();
        assert_eq!(json["login"]["command"], "claude auth login");
        assert_eq!(json["account"]["isMain"], false);
        assert_eq!(json["account"]["loggedIn"], false);
        assert!(json["account"].get("email").is_none());
    }

    #[test]
    fn a_codex_account_is_a_shadow_home_listed_with_codexs_main() {
        let scratch = Scratch::new("add-codex");
        let mains = furnished(&scratch);
        let store = store(&scratch);
        let added = add_with(&mains, &store, Codex, "Team", 7).unwrap();
        assert_eq!(added.login.command, "codex login");
        assert_eq!(added.account.id, "team");
        assert!(!added.account.logged_in);
        let home = store.dir_of(Codex, "team");
        assert_eq!(home, scratch.root.join("accounts/codex/team"));
        // Main's conversations and configuration, not its login.
        assert!(home
            .join("sessions/2026/11/18/rollout-2026-11-18T09-00-00-t1.jsonl")
            .is_file());
        assert!(home.join("config.toml").is_file());
        assert!(!link::exists(&home.join("auth.json")));

        // Logged in from a pane on it: who it is comes from its own file.
        scratch.file(
            "accounts/codex/team/auth.json",
            include_str!("accounts/fixtures/codex/api-key.json"),
        );
        let listed = list_with(&mains, &store, Codex);
        assert_eq!(
            listed,
            vec![
                Account {
                    id: "main".into(),
                    label: "Main".into(),
                    is_main: true,
                    logged_in: true,
                    email: Some("team@example.com".into()),
                    plan: Some("team".into()),
                },
                Account {
                    id: "team".into(),
                    label: "Team".into(),
                    is_main: false,
                    logged_in: true,
                    email: None,
                    plan: Some("apikey".into()),
                },
            ]
        );
        // Claude's list is as it was.
        assert_eq!(list_with(&mains, &store, Claude).len(), 1);
        // And nothing secret is in what the window reads.
        let shown = serde_json::to_string(&listed).unwrap();
        assert!(!shown.contains("placeholder") && !shown.contains("eyJ"));

        // Renamed per kind: Codex's main, not Claude's.
        let renamed = rename_with(&mains, &store, Codex, "main", "Perso").unwrap();
        assert_eq!(renamed.label, "Perso");
        assert_eq!(list_with(&mains, &store, Claude)[0].label, "Main");
    }

    #[test]
    fn removing_a_profile_leaves_main_and_the_shared_data_intact() {
        let scratch = Scratch::new("remove");
        let mains = furnished(&scratch);
        let main = &mains.claude;
        let store = store(&scratch);
        add_with(&mains, &store, Claude, "Work", 1).unwrap();
        add_with(&mains, &store, Claude, "Perso", 2).unwrap();
        let work = store.dir_of(Claude, "work");
        // A conversation written on the account, and its own login.
        scratch.file("accounts/work/projects/C--repo/conv-2.jsonl", "{}\n");
        scratch.file(
            "accounts/work/.credentials.json",
            r#"{"claudeAiOauth":{"accessToken":"w"}}"#,
        );

        assert!(remove_with(&mains, &store, "main", 3).is_err());
        assert!(remove_with(&mains, &store, "../home", 3).is_err());
        assert!(remove_with(&mains, &store, "nobody", 3).is_err());
        remove_with(&mains, &store, "work", 3).unwrap();

        assert!(!link::exists(&work));
        assert!(store.trashed().is_empty());
        let ids: Vec<String> = list_with(&mains, &store, Claude)
            .into_iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(ids, ["main", "perso"]);
        // Everything shared is still main's, the conversation made on the
        // removed account included, and the other profile still reaches it.
        assert!(main.dir.join("projects/C--repo/conv-1.jsonl").is_file());
        assert!(main.dir.join("projects/C--repo/conv-2.jsonl").is_file());
        assert!(main.dir.join("skills/review/SKILL.md").is_file());
        assert!(main.dir.join(".credentials.json").is_file());
        assert!(main.config.is_file());
        assert!(store
            .dir_of(Claude, "perso")
            .join("projects/C--repo/conv-2.jsonl")
            .is_file());
    }

    #[test]
    fn removing_a_codex_account_leaves_mains_conversations_and_login() {
        let scratch = Scratch::new("remove-codex");
        let mains = furnished(&scratch);
        let store = store(&scratch);
        add_with(&mains, &store, Codex, "Team", 1).unwrap();
        let team = store.dir_of(Codex, "team");
        scratch.file(
            "accounts/codex/team/auth.json",
            "{\"OPENAI_API_KEY\":\"k\"}",
        );
        scratch.file(
            "accounts/codex/team/sessions/2026/11/19/rollout-2026-11-19T09-00-00-t2.jsonl",
            "{}\n",
        );

        remove_with(&mains, &store, "team", 3).unwrap();
        assert!(!link::exists(&team));
        assert!(store.trashed().is_empty());
        assert_eq!(list_with(&mains, &store, Codex).len(), 1);
        let main = &mains.codex.dir;
        assert!(main.join("auth.json").is_file());
        assert!(main.join("config.toml").is_file());
        assert!(main
            .join("sessions/2026/11/19/rollout-2026-11-19T09-00-00-t2.jsonl")
            .is_file());
    }

    #[test]
    fn a_launch_prepares_the_profile_and_falls_back_to_main_for_a_gone_account() {
        let scratch = Scratch::new("launch");
        let mains = furnished(&scratch);
        let store = store(&scratch);
        add_with(&mains, &store, Claude, "Work", 1).unwrap();
        add_with(&mains, &store, Codex, "Team", 2).unwrap();
        // Main's setup changed since the account was added.
        scratch.file("home/.claude/CLAUDE.md", "New rule.");
        scratch.file("home/.codex/AGENTS.md", "New rule.");

        let launch = launch_with(&mains, &store, Claude, "work");
        assert_eq!(launch.id, "work");
        assert_eq!(
            launch.dir.as_deref(),
            Some(store.dir_of(Claude, "work").as_path())
        );
        assert!(launch.issues.is_empty(), "{:?}", launch.issues);
        assert!(store.dir_of(Claude, "work").join("CLAUDE.md").is_file());

        let launch = launch_with(&mains, &store, Codex, "team");
        assert_eq!(
            launch.dir.as_deref(),
            Some(store.dir_of(Codex, "team").as_path())
        );
        assert!(launch.issues.is_empty(), "{:?}", launch.issues);
        assert!(store.dir_of(Codex, "team").join("AGENTS.md").is_file());

        assert_eq!(launch_with(&mains, &store, Claude, "gone"), Launch::main());
        // An account of the other agent is not this one's.
        assert_eq!(launch_with(&mains, &store, Claude, "team"), Launch::main());
        assert_eq!(launch_with(&mains, &store, Codex, "work"), Launch::main());
    }

    #[test]
    fn panes_get_their_agents_folder_only_on_an_account_hangar_made() {
        let dir = PathBuf::from("/accounts/work");
        assert_eq!(
            env_for(Some("claude"), Some(&dir)),
            vec![
                ("CLAUDE_CONFIG_DIR", Some(dir.clone())),
                ("CLAUDE_SECURESTORAGE_CONFIG_DIR", None),
            ]
        );
        // Codex: its home, and main's SQLite folder beside it (its value is
        // codex_home.rs's to test: it comes from this process's
        // environment).
        let codex = env_for(Some("codex"), Some(&dir));
        let names: Vec<&str> = codex.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["CODEX_HOME", "CODEX_SQLITE_HOME"]);
        assert_eq!(codex[0].1.as_deref(), Some(dir.as_path()));
        // Main keeps Hangar's environment.
        assert!(env_for(Some("claude"), None).is_empty());
        assert!(env_for(Some("codex"), None).is_empty());
        // No account applies to a shell.
        assert!(env_for(Some("shell"), Some(&dir)).is_empty());
        assert!(env_for(None, Some(&dir)).is_empty());
        // And no launch is ever made for them.
        assert_eq!(for_launch(Some("shell"), Some("work")), None);
        assert_eq!(for_launch(None, Some("work")), None);
        assert_eq!(for_launch(Some("claude"), None), Some(Launch::main()));
        assert_eq!(for_launch(Some("codex"), None), Some(Launch::main()));
        assert_eq!(
            for_launch(Some("claude"), Some(" main ")),
            Some(Launch::main())
        );
    }

    #[test]
    fn conversations_are_read_once_and_a_profile_of_its_own_too() {
        let scratch = Scratch::new("projects");
        let mains = furnished(&scratch);
        let store = store(&scratch);
        add_with(&mains, &store, Claude, "Work", 1).unwrap();
        // A profile whose link was replaced by a folder of its own.
        add_with(&mains, &store, Claude, "Perso", 2).unwrap();
        // A Codex account is no Claude profile.
        add_with(&mains, &store, Codex, "Team", 3).unwrap();
        let perso = store.dir_of(Claude, "perso").join("projects");
        link::remove_link(&perso).unwrap();
        scratch.file("accounts/perso/projects/C--x/conv-9.jsonl", "{}\n");

        let dirs = projects_with(&mains.claude, &store);
        assert_eq!(dirs, vec![mains.claude.projects(), perso]);
    }

    #[test]
    fn the_support_note_is_only_there_when_logins_are_shared() {
        assert_eq!(
            support_from(true),
            Support {
                isolated_logins: true,
                note: None
            }
        );
        let shared = serde_json::to_value(support_from(false)).unwrap();
        assert_eq!(shared["isolatedLogins"], false);
        assert!(shared["note"]
            .as_str()
            .is_some_and(|note| note.contains("Keychain")));
    }
}
