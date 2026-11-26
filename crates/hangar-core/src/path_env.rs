//! PATH repair for a window that was not opened from a shell.
//!
//! On Windows an installer writes to the registry and every process sees the
//! same PATH. On macOS it does not work that way: a bundled app is started by
//! `launchd`, which hands it the bare system PATH — `/usr/bin:/bin:/usr/sbin:
//! /sbin` — and nothing a developer installs lives there. Homebrew, npm, cargo
//! and bun all put their binaries somewhere the shell adds on login, so the app
//! opens with every agent greyed out as "not installed" and half the shells
//! missing, on a machine where all of them are one `which` away.
//!
//! The user's login shell is the authority on what their PATH is, so it gets
//! asked once and its answer goes in front. Well-known install directories are
//! appended as a floor, for the machine whose shell cannot be probed at all —
//! an unsupported shell, or an rc file that hangs. npm's global folder and
//! Homebrew's are in that floor by name: `npm install -g @openai/codex` and
//! `brew install --cask codex` put `codex` there, and Claude Code installed
//! either way puts `claude` there too.
//!
//! Doing this to our own process is what makes it reach everything downstream:
//! shell detection, agent detection, and the panes themselves, which inherit
//! the environment we were fixed with.

#[cfg(unix)]
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Runs the repair once per process. Every entry point that reads PATH calls
/// this first, so the order the frontend happens to issue its commands in
/// cannot decide whether detection works.
pub fn ensure() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(apply);
}

/// Runs the repair again, after something was installed while the app ran:
/// Claude Code's installer puts `claude` in `~/.local/bin`, and a first
/// `npm install -g` makes npm's global folder — folders that may not have
/// existed (so were left out) the first time — and adds them to the rc files
/// the login shell reads. The shell is asked afresh; what PATH already had
/// stays, after it.
pub fn refresh() {
    ensure();
    apply();
}

#[cfg(not(unix))]
fn apply() {
    // Windows has no login shell to ask. An installer writes its folder to
    // the user's PATH in the registry, which a process already running never
    // sees: Claude Code's native one (`~/.local/bin`) and npm's global folder,
    // where `npm install -g` puts `codex.cmd`, are added here when they are
    // there.
    let current: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let mut merged = current.clone();
    for folder in windows_floor() {
        if folder.is_dir() && !merged.contains(&folder) {
            merged.push(folder);
        }
    }
    if merged.len() == current.len() {
        return;
    }
    if let Ok(joined) = std::env::join_paths(&merged) {
        std::env::set_var("PATH", joined);
    }
}

/// The folders Windows' PATH gets when they exist and it lacks them, in this
/// order.
#[cfg(not(unix))]
fn windows_floor() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if let Some(home) = dirs::home_dir() {
        folders.push(home.join(".local").join("bin"));
    }
    folders.extend(npm_bins());
    folders
}

#[cfg(unix)]
fn apply() {
    let current = std::env::var_os("PATH");
    let merged = merged(login_shell_path(), current, well_known());
    if let Ok(joined) = std::env::join_paths(&merged) {
        std::env::set_var("PATH", joined);
    }
}

/// The PATH the process gets: the login shell's answer first — it is the PATH
/// the user actually works in, and the one their agents were installed into —
/// then the current one, then the well-known folders, each folder once and
/// only when it exists.
#[cfg(unix)]
fn merged(
    login: Option<std::ffi::OsString>,
    current: Option<std::ffi::OsString>,
    well_known: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut merged: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    if let Some(answer) = login {
        push_all(std::env::split_paths(&answer), &mut merged, &mut seen);
    }
    if let Some(current) = current {
        push_all(std::env::split_paths(&current), &mut merged, &mut seen);
    }
    push_all(well_known, &mut merged, &mut seen);
    merged
}

/// Keeps directories that exist, in the order first seen. A PATH entry that is
/// not there is noise every later lookup would walk over.
#[cfg(unix)]
fn push_all(
    candidates: impl IntoIterator<Item = PathBuf>,
    merged: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
) {
    for path in candidates {
        if path.as_os_str().is_empty() || !path.is_dir() {
            continue;
        }
        if seen.insert(path.clone()) {
            merged.push(path);
        }
    }
}

/// The floor: the folders developers' tools install into, then Homebrew's on
/// Linux, then npm's global folder wherever the person put it.
#[cfg(unix)]
fn well_known() -> Vec<PathBuf> {
    let home = dirs::home_dir();
    let mut paths = install_dirs(home.as_deref());
    paths.extend(homebrew_bins(
        home.as_deref(),
        std::env::var_os("HOMEBREW_PREFIX"),
    ));
    paths.extend(npm_bins());
    paths
}

#[cfg(unix)]
fn install_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    // Homebrew moved to /opt/homebrew on Apple Silicon and kept /usr/local on
    // Intel, so both are worth having.
    let mut paths: Vec<PathBuf> = ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin"]
        .iter()
        .map(PathBuf::from)
        .collect();

    if let Some(home) = home {
        for relative in [
            ".local/bin",
            ".cargo/bin",
            ".bun/bin",
            ".deno/bin",
            ".volta/bin",
            ".npm-global/bin",
            "go/bin",
        ] {
            paths.push(home.join(relative));
        }
    }

    paths
}

/// Homebrew's folders beyond macOS's: on Linux it lives in
/// `/home/linuxbrew/.linuxbrew`, or in `~/.linuxbrew` for a person who could
/// not create that user; and wherever it lives, its `shellenv` exports
/// `HOMEBREW_PREFIX`, which Hangar has when a shell started it.
#[cfg(unix)]
fn homebrew_bins(home: Option<&Path>, prefix: Option<std::ffi::OsString>) -> Vec<PathBuf> {
    let mut prefixes: Vec<PathBuf> = Vec::new();
    if let Some(prefix) = prefix.filter(|prefix| !prefix.is_empty()) {
        prefixes.push(PathBuf::from(prefix));
    }
    prefixes.push(PathBuf::from("/home/linuxbrew/.linuxbrew"));
    if let Some(home) = home {
        prefixes.push(home.join(".linuxbrew"));
    }
    prefixes
        .iter()
        .flat_map(|prefix| [prefix.join("bin"), prefix.join("sbin")])
        .collect()
}

/// npm's global bin folders, where `npm install -g` puts a command:
/// `<prefix>/bin` on macOS and Linux, `<prefix>` itself on Windows.
///
/// The prefix is the person's when they chose one: `npm_config_prefix` in the
/// environment, `prefix=` in their `.npmrc` (the file `npm_config_userconfig`
/// names, if it names one). Otherwise it is where Node was installed:
/// `/usr/local` or Homebrew's on macOS, both well known already, `/usr` on
/// Linux, on every PATH, and `%APPDATA%\npm` on Windows. A version manager's
/// (nvm, fnm) moves with the Node version in use, which only the login shell
/// knows: that one is left to it.
fn npm_bins() -> Vec<PathBuf> {
    let home = dirs::home_dir();
    npm_bins_from(
        &|name| std::env::var(name).ok(),
        home.as_deref(),
        cfg!(windows),
    )
}

/// [`npm_bins`] with the environment, the home folder and the platform
/// passed in.
fn npm_bins_from(
    env: &dyn Fn(&str) -> Option<String>,
    home: Option<&Path>,
    windows: bool,
) -> Vec<PathBuf> {
    // npm reads its variables in either case.
    let var = |name: &str| {
        env(name)
            .or_else(|| env(&name.to_ascii_uppercase()))
            .filter(|value| !value.is_empty())
    };
    let mut prefixes: Vec<PathBuf> = Vec::new();
    if let Some(prefix) = var("npm_config_prefix").and_then(|value| npm_path(&value, home, env)) {
        prefixes.push(prefix);
    }
    let userconfig = match var("npm_config_userconfig") {
        Some(file) => npm_path(&file, home, env),
        None => home.map(|home| home.join(".npmrc")),
    };
    if let Some(prefix) = userconfig
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|text| npmrc_prefix(&text, home, env))
    {
        prefixes.push(prefix);
    }
    if windows {
        if let Some(appdata) = var("APPDATA") {
            prefixes.push(PathBuf::from(appdata).join("npm"));
        }
    }
    prefixes
        .into_iter()
        .map(|prefix| if windows { prefix } else { prefix.join("bin") })
        .collect()
}

/// The `prefix` an `.npmrc` sets: the last one, when it sets it twice.
fn npmrc_prefix(
    text: &str,
    home: Option<&Path>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    let value = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with([';', '#']))
        .filter_map(|line| line.split_once('='))
        .filter(|(key, _)| key.trim() == "prefix")
        .map(|(_, value)| value.trim())
        .next_back()?;
    let unquoted = ['"', '\'']
        .iter()
        .find_map(|quote| value.strip_prefix(*quote)?.strip_suffix(*quote))
        .unwrap_or(value);
    npm_path(unquoted, home, env)
}

/// A path as npm reads it from its configuration: `${VAR}` replaced by the
/// variable (`${VAR?}` by nothing when it is unset; unset without the `?`,
/// npm refuses the value, and so does this), a leading `~` by the home
/// folder. Only an absolute path is kept: a relative one would be relative to
/// whatever folder npm was run from.
fn npm_path(
    value: &str,
    home: Option<&Path>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    let mut expanded = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        let (before, after) = rest.split_at(start);
        let end = after.find('}')?;
        let name = &after[2..end];
        let (name, optional) = match name.strip_suffix('?') {
            Some(name) => (name, true),
            None => (name, false),
        };
        expanded.push_str(before);
        match env(name) {
            Some(found) => expanded.push_str(&found),
            None if optional => {}
            None => return None,
        }
        rest = &after[end + 1..];
    }
    expanded.push_str(rest);

    let path = match expanded.strip_prefix('~') {
        Some("") => home?.to_path_buf(),
        Some(tail) if tail.starts_with(['/', '\\']) => home?.join(&tail[1..]),
        _ => PathBuf::from(expanded),
    };
    path.is_absolute().then_some(path)
}

/// Wraps the answer so whatever an rc file prints on the way — a banner, an
/// instant prompt, a shell integration escape sequence — can be cut away.
#[cfg(unix)]
const MARKER: &str = "__HANGAR_PATH__";

/// How long the shell gets before we take the well-known directories instead.
/// An interactive rc that blocks would otherwise hold the first detection call
/// for as long as it felt like.
#[cfg(unix)]
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);

/// The one-liner that prints the PATH, in the dialect of the shell running it.
///
/// `fish` keeps `$PATH` as a list rather than a colon-joined string, so it
/// needs its own spelling. A shell we do not know how to ask — nushell, elvish
/// — returns `None` and the caller falls back rather than running a line that
/// would mean something else there.
///
/// The braces around `PATH` are not decoration. A marker made of word
/// characters runs straight into the name, and `$PATH__HANGAR_PATH__` is a
/// perfectly valid reference to a variable nobody ever set — which expands to
/// nothing, quietly, on every machine.
#[cfg(unix)]
fn script_for(shell: &Path) -> Option<String> {
    match shell.file_name()?.to_str()? {
        "fish" => Some(format!(
            r#"printf '%s' "{MARKER}"(string join : $PATH)"{MARKER}""#
        )),
        "sh" | "bash" | "zsh" | "ksh" | "ksh93" | "mksh" | "dash" | "ash" => {
            Some(format!(r#"printf '%s' "{MARKER}${{PATH}}{MARKER}""#))
        }
        _ => None,
    }
}

/// Asks the login shell for its PATH, or `None` if it could not be asked.
///
/// `-l -i` because the two halves of a shell's configuration hide in different
/// files: Homebrew's `shellenv` is conventionally in `~/.zprofile`, which only
/// a login shell reads, while version managers like nvm and rbenv initialise
/// from `~/.zshrc`, which only an interactive one does. Asking for both is what
/// makes the answer match the terminal the user would have typed in.
///
/// The shell started by [`start_early`], when there is one, is the one asked:
/// it has been running since the top of `main`.
#[cfg(unix)]
fn login_shell_path() -> Option<std::ffi::OsString> {
    let probe = take_early().or_else(Probe::spawn)?;
    probe.collect()
}

/// The login shell, started at once, its answer collected later.
///
/// The window spends its first quarter second on the GPU probe and iced's
/// instance before core asks for the PATH, and the pane shells wait on that
/// answer. Starting the shell first thing lets its rc files run meanwhile.
///
/// Only a child process is started, never a thread: the GPU probe that runs
/// next may `set_var`, which is sound only while the process has one thread.
/// The child gets the environment as it is now, which is the one it would
/// have got later anyway — nothing it reads is changed before then.
#[cfg(unix)]
pub fn start_early() {
    let Some(probe) = Probe::spawn() else {
        return;
    };
    park(early(), probe);
}

#[cfg(not(unix))]
pub fn start_early() {}

/// Where [`start_early`] parks its shell until [`ensure`] collects it.
#[cfg(unix)]
fn early() -> &'static std::sync::Mutex<Option<Probe>> {
    static EARLY: std::sync::Mutex<Option<Probe>> = std::sync::Mutex::new(None);
    &EARLY
}

#[cfg(unix)]
fn take_early() -> Option<Probe> {
    early().lock().unwrap().take()
}

/// Puts `probe` in `slot`, ending any shell already parked there.
#[cfg(unix)]
fn park(slot: &std::sync::Mutex<Option<Probe>>, probe: Probe) {
    let stale = slot.lock().unwrap().replace(probe);
    if let Some(stale) = stale {
        stale.abandon();
    }
}

/// A login shell asked for its PATH, running.
#[cfg(unix)]
struct Probe {
    child: std::process::Child,
    stdout: std::process::ChildStdout,
    /// When it was started: the timeout counts from here, so a shell started
    /// early has already used part of its time by the time it is collected.
    started: std::time::Instant,
}

#[cfg(unix)]
impl Probe {
    /// Starts `$SHELL`, or `None` when there is no shell we know how to ask.
    fn spawn() -> Option<Self> {
        let shell = PathBuf::from(std::env::var_os("SHELL")?);
        Self::spawn_with(&shell, &["-l", "-i"])
    }

    fn spawn_with(shell: &Path, flags: &[&str]) -> Option<Self> {
        use std::process::{Command, Stdio};

        let script = script_for(shell)?;
        let mut child = Command::new(shell)
            .args(flags)
            .args(["-c", &script])
            // A shell that reads from stdin on startup gets EOF rather than
            // waiting on a terminal that is not there.
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdout = child.stdout.take()?;
        Some(Self {
            child,
            stdout,
            started: std::time::Instant::now(),
        })
    }

    /// Waits for the answer, up to [`PROBE_TIMEOUT`] after the start, and
    /// kills the shell when it has not given one by then.
    fn collect(self) -> Option<std::ffi::OsString> {
        use std::io::Read;
        use std::sync::{mpsc, Arc, Mutex};

        let Self {
            child,
            mut stdout,
            started,
        } = self;
        // The child is parked where the timeout below can reach it. Reading
        // its output happens on the worker, so the shell can be killed while
        // the worker is still blocked on a read that will never finish.
        let child = Arc::new(Mutex::new(Some(child)));
        let worker_child = child.clone();
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let mut output = String::new();
            let read = stdout.read_to_string(&mut output).is_ok();

            // Taken back out before waiting, so a slow exit does not hold the
            // lock the timeout path needs to kill it.
            let taken = worker_child.lock().unwrap().take();
            if let Some(mut spawned) = taken {
                let _ = spawned.wait();
            }
            let _ = tx.send(read.then_some(output));
        });

        let left = PROBE_TIMEOUT.saturating_sub(started.elapsed());
        let answer = rx.recv_timeout(left).ok().flatten();
        if answer.is_none() {
            if let Some(spawned) = child.lock().unwrap().as_mut() {
                let _ = spawned.kill();
            }
        }

        let output = answer?;
        let value = between_markers(&output)?;
        Some(std::ffi::OsString::from(value))
    }

    /// Kills and reaps a shell whose answer is no longer wanted.
    fn abandon(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The text between the two markers, or `None` when the shell printed neither.
#[cfg(unix)]
fn between_markers(output: &str) -> Option<&str> {
    let (_, rest) = output.split_once(MARKER)?;
    let (value, _) = rest.split_once(MARKER)?;
    (!value.trim().is_empty()).then(|| value.trim())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn reads_the_path_out_of_a_noisy_answer() {
        let output =
            format!("\u{1b}]633;A\u{7}welcome!\n{MARKER}/opt/homebrew/bin:/usr/bin{MARKER}");
        assert_eq!(between_markers(&output), Some("/opt/homebrew/bin:/usr/bin"));
    }

    #[test]
    fn ignores_an_answer_that_never_arrived() {
        assert_eq!(between_markers("command not found"), None);
        assert_eq!(between_markers(&format!("{MARKER}   {MARKER}")), None);
    }

    /// Runs the script the way the probe does, against a PATH we set, and
    /// checks the answer comes back whole. A shell quietly expanding the
    /// wrong variable name reads exactly like a machine with nothing
    /// installed, so this is worth proving rather than eyeballing.
    #[test]
    fn the_posix_script_prints_the_path_it_was_given() {
        let shell = Path::new("/bin/sh");
        let script = script_for(shell).expect("/bin/sh is a shell we know");
        let output = std::process::Command::new(shell)
            .args(["-c", &script])
            .env("PATH", "/somewhere/bin:/else/bin")
            .output()
            .expect("/bin/sh runs");

        let text = String::from_utf8_lossy(&output.stdout);
        assert_eq!(between_markers(&text), Some("/somewhere/bin:/else/bin"));
    }

    /// The PATH the shells in these tests answer with: the test process's
    /// own, which `bash -c` hands back unchanged.
    fn own_path() -> String {
        std::env::var("PATH").expect("the tests run with a PATH")
    }

    /// A bash whose rc file takes `delay` seconds, the way nvm makes a real
    /// one slow, asked the way the probe asks.
    fn slow_bash(dir: &Path, delay: &str) -> Probe {
        let rc = dir.join("bashrc");
        std::fs::write(&rc, format!("sleep {delay}\n")).unwrap();
        let rc = rc.to_str().unwrap().to_owned();
        Probe::spawn_with(Path::new("/bin/bash"), &["--rcfile", &rc, "-i"])
            .expect("/bin/bash starts")
    }

    /// The answer of a shell started early and collected later is the one
    /// a shell started and collected at once gives.
    #[test]
    fn an_early_probe_answers_what_a_late_one_does() {
        let dir = tempfile::tempdir().unwrap();
        let early = slow_bash(dir.path(), "0");
        std::thread::sleep(std::time::Duration::from_millis(50));
        let late = slow_bash(dir.path(), "0").collect();

        assert_eq!(early.collect(), late);
        assert_eq!(late, Some(std::ffi::OsString::from(own_path())));
    }

    /// Started before the window, parked, collected once the window is up:
    /// the rc file's time is spent while the window gets ready, not after.
    #[test]
    fn a_parked_probe_runs_while_the_window_starts() {
        let dir = tempfile::tempdir().unwrap();
        let slot = std::sync::Mutex::new(None);

        park(&slot, slow_bash(dir.path(), "0.3"));
        // The GPU probe and iced's instance, which take longer than the rc.
        std::thread::sleep(std::time::Duration::from_millis(600));

        let probe = slot.lock().unwrap().take().expect("the probe was parked");
        assert!(slot.lock().unwrap().is_none(), "collected once");
        let waited = std::time::Instant::now();
        let answer = probe.collect();
        assert!(
            waited.elapsed() < std::time::Duration::from_millis(250),
            "the shell had finished by the time it was collected ({:?})",
            waited.elapsed()
        );
        assert_eq!(answer, Some(std::ffi::OsString::from(own_path())));
    }

    /// A second early start ends the shell already parked instead of
    /// leaving it running unread, and its own answer is the one collected.
    #[test]
    fn a_parked_probe_is_ended_when_another_takes_its_place() {
        let dir = tempfile::tempdir().unwrap();
        let slot = std::sync::Mutex::new(None);
        park(&slot, slow_bash(dir.path(), "30"));
        park(&slot, slow_bash(dir.path(), "0"));

        let probe = slot.lock().unwrap().take().unwrap();
        assert_eq!(probe.collect(), Some(std::ffi::OsString::from(own_path())));
    }

    /// What `refresh` is for: a folder an installer made after the first
    /// repair (`~/.local/bin`, `claude` in it) is on the PATH the second
    /// time, after what the PATH already had.
    #[test]
    fn a_folder_made_after_the_first_repair_joins_on_the_next() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("usr-bin");
        std::fs::create_dir_all(&first).unwrap();
        let local_bin = dir.path().join(".local/bin");
        let current = std::env::join_paths([&first]).unwrap();

        let before = merged(None, Some(current.clone()), vec![local_bin.clone()]);
        assert_eq!(before, vec![first.clone()]);

        std::fs::create_dir_all(&local_bin).unwrap();
        let joined = std::env::join_paths(&before).unwrap();
        let after = merged(None, Some(joined), vec![local_bin.clone()]);
        assert_eq!(after, vec![first, local_bin]);
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    /// `codex` installed by npm under a prefix of the person's, or by
    /// Homebrew on Linux, is found on a machine whose shell could not be
    /// asked, after what PATH already had.
    #[test]
    fn npm_and_homebrew_folders_join_the_floor() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let usr_bin = home.join("usr-bin");
        let npm_bin = home.join(".npm-packages/bin");
        let brew_bin = home.join(".linuxbrew/bin");
        for folder in [&usr_bin, &npm_bin, &brew_bin] {
            std::fs::create_dir_all(folder).unwrap();
        }
        std::fs::write(home.join(".npmrc"), "prefix=~/.npm-packages\n").unwrap();
        let current = std::env::join_paths([&usr_bin]).unwrap();

        let mut floor = install_dirs(Some(home));
        floor.extend(homebrew_bins(Some(home), None));
        floor.extend(npm_bins_from(&no_env, Some(home), false));
        let merged = merged(None, Some(current), floor);

        assert_eq!(merged.first(), Some(&usr_bin));
        assert!(merged.contains(&brew_bin), "{merged:?}");
        assert!(merged.contains(&npm_bin), "{merged:?}");
        // A folder that is not there is left out, as before.
        assert!(!merged.contains(&home.join(".linuxbrew/sbin")));
    }

    /// The prefix `brew shellenv` exported, when a shell started Hangar.
    #[test]
    fn homebrew_s_own_prefix_comes_first() {
        let home = Path::new("/home/someone");
        let bins = homebrew_bins(Some(home), Some("/srv/brew".into()));
        assert_eq!(bins[0], PathBuf::from("/srv/brew/bin"));
        assert_eq!(bins[1], PathBuf::from("/srv/brew/sbin"));
        assert!(bins.contains(&PathBuf::from("/home/linuxbrew/.linuxbrew/bin")));
        assert!(bins.contains(&home.join(".linuxbrew/bin")));
        assert_eq!(homebrew_bins(None, Some("".into())).len(), 2);
    }

    /// Where npm puts global commands: the prefix from the environment,
    /// then the one of the `.npmrc` it is told to read, and on Windows
    /// `%APPDATA%\npm`; `bin` under each but on Windows.
    #[test]
    fn npm_s_prefix_is_the_person_s_choice() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let rc = home.join("custom.npmrc");
        std::fs::write(
            &rc,
            "; mine\nregistry=https://registry.npmjs.org/\nprefix = \"${HOME}/npm-global\"\n",
        )
        .unwrap();
        let home_text = home.to_str().unwrap().to_owned();
        let rc_text = rc.to_str().unwrap().to_owned();
        let env = move |name: &str| match name {
            "NPM_CONFIG_PREFIX" => Some("/opt/npm".to_owned()),
            "npm_config_userconfig" => Some(rc_text.clone()),
            "HOME" => Some(home_text.clone()),
            "APPDATA" => Some("/appdata".to_owned()),
            _ => None,
        };

        assert_eq!(
            npm_bins_from(&env, Some(&home), false),
            [PathBuf::from("/opt/npm/bin"), home.join("npm-global/bin")]
        );
        assert_eq!(
            npm_bins_from(&env, Some(&home), true),
            [
                PathBuf::from("/opt/npm"),
                home.join("npm-global"),
                PathBuf::from("/appdata/npm"),
            ]
        );
        assert!(npm_bins_from(&no_env, Some(&home), false).is_empty());
    }

    #[test]
    fn an_npmrc_prefix_is_read_as_npm_reads_it() {
        let home = Path::new("/home/someone");
        let env = |name: &str| (name == "NODE_ROOT").then(|| "/srv/node".to_owned());
        let read = |text: &str| npmrc_prefix(text, Some(home), &env);

        assert_eq!(read("prefix=~/.npm-global"), Some(home.join(".npm-global")));
        assert_eq!(read("prefix=~"), Some(home.to_path_buf()));
        assert_eq!(read("prefix = '/opt/node'\n"), Some("/opt/node".into()));
        assert_eq!(read("prefix=${NODE_ROOT}/g"), Some("/srv/node/g".into()));
        assert_eq!(read("prefix=/opt${UNSET?}/g"), Some("/opt/g".into()));
        assert_eq!(
            read("prefix=/first\nprefix=/second"),
            Some("/second".into())
        );
        // npm refuses a value naming a variable that is not set.
        assert_eq!(read("prefix=${UNSET}/g"), None);
        assert_eq!(read("# prefix=/commented\n;prefix=/too"), None);
        assert_eq!(read("cache=/var/npm\n"), None);
        assert_eq!(read("prefix=relative/dir"), None);
        assert_eq!(read("prefix=~someone/dir"), None);
    }

    #[test]
    fn only_speaks_to_shells_it_knows() {
        assert!(script_for(Path::new("/bin/zsh")).is_some());
        assert!(script_for(Path::new("/opt/homebrew/bin/fish")).is_some());
        assert!(script_for(Path::new("/opt/homebrew/bin/nu")).is_none());
    }
}
