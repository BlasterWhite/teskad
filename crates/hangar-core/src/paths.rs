//! Where Hangar keeps its files: the folders Tauri resolved for the app, to
//! the byte, so an install that ran the Tauri app finds its `state.json`,
//! `memory.json`, `remote.json`, voice models and the rest where they are.
//!
//! Tauri 2 named every app folder after the bundle identifier, under the
//! `dirs` crate's folder of that kind (tauri's `path/desktop.rs`, which
//! resolved them through `dirs` 6, as here):
//!
//! | folder          | Linux                          | macOS                                   | Windows                          |
//! |-----------------|--------------------------------|-----------------------------------------|----------------------------------|
//! | `app_data`      | `~/.local/share/<id>`          | `~/Library/Application Support/<id>`    | `%APPDATA%\<id>`                 |
//! | `app_config`    | `~/.config/<id>`               | `~/Library/Application Support/<id>`    | `%APPDATA%\<id>`                 |
//! | `app_local_data`| `~/.local/share/<id>`          | `~/Library/Application Support/<id>`    | `%LOCALAPPDATA%\<id>`            |
//! | `app_cache`     | `~/.cache/<id>`                | `~/Library/Caches/<id>`                 | `%LOCALAPPDATA%\<id>`            |
//! | `app_log`       | `~/.local/share/<id>/logs`     | `~/Library/Logs/<id>`                   | `%LOCALAPPDATA%\<id>\logs`       |
//!
//! (the XDG variables move the Linux ones, as they did for Tauri). Every file
//! the backend wrote went under `app_data`; the other four are here for the
//! app crate, which inherits whatever the window kept in them.
//!
//! Tauri resolved a folder each time it was asked; `Paths` resolves them once,
//! when the host is made, which comes to the same for a running app.

use std::path::{Path, PathBuf};

/// The bundle identifier of this app (`identifier` in crates/hangar-app's
/// Cargo.toml, under `package.metadata.packager`). It names every folder
/// below, and the Windows notifications' AUMID (notify.rs). It is the fork's
/// own: it shares nothing with the official app's (`OFFICIAL_IDENTIFIER`), and
/// is changed only together with a migration (migrate.rs).
pub const IDENTIFIER: &str = "io.github.blasterwhite.teskad";

/// What the person sees the app called: the packager's `productName`, which is
/// also the name its login item is filed under (autostart.rs).
pub const PRODUCT_NAME: &str = "Teskad";

/// The folder, under the person's home, where Teskad keeps what agents and
/// other programs reach by a fixed path: the endpoint file, the Claude Code
/// mod, the accounts, the kits and their data.
pub const HOME_DIR_NAME: &str = ".teskad";

/// The bundle identifier of the official HangarAI app. Teskad never writes
/// there: it only reads it, once, to copy its data over (migrate.rs).
pub const OFFICIAL_IDENTIFIER: &str = "com.simnjs.iabench";

/// The official app's home folder (`HOME_DIR_NAME`'s counterpart): read once by
/// the migration, never written.
pub const OFFICIAL_HOME_DIR_NAME: &str = ".hangar";

/// `~/.teskad`, the one place the code below the home directory is named
/// from: the endpoint file, the mods, the accounts, the kits, the kit data
/// and the folders a clone may not land in all join onto it.
pub fn home_folder() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(HOME_DIR_NAME))
}

/// A folder the system would not name — no home directory, most likely.
/// Worded as Tauri's `Error::UnknownPath`, which the messages that carry it
/// (`no app data dir: unknown path`) were written around.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownPath;

impl std::fmt::Display for UnknownPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("unknown path")
    }
}

impl std::error::Error for UnknownPath {}

/// The app's folders, resolved. Cheap to clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    data: Option<PathBuf>,
    config: Option<PathBuf>,
    local_data: Option<PathBuf>,
    cache: Option<PathBuf>,
    log: Option<PathBuf>,
}

impl Paths {
    /// The folders Tauri used for this identifier on this system.
    pub fn system() -> Self {
        Self::system_for(IDENTIFIER)
    }

    /// The folders `identifier` would have on this system: the official
    /// app's, for the migration to read (migrate.rs).
    pub fn system_for(identifier: &str) -> Self {
        let app = |base: Option<PathBuf>| base.map(|dir| dir.join(identifier));
        #[cfg(target_os = "macos")]
        let log = dirs::home_dir().map(|home| home.join("Library/Logs").join(identifier));
        #[cfg(not(target_os = "macos"))]
        let log = dirs::data_local_dir().map(|dir| dir.join(identifier).join("logs"));
        Self {
            data: app(dirs::data_dir()),
            config: app(dirs::config_dir()),
            local_data: app(dirs::data_local_dir()),
            cache: app(dirs::cache_dir()),
            log,
        }
    }

    /// `HANGAR_DATA_ROOT`'s folders when it is set and not empty, the
    /// system's otherwise.
    ///
    /// The native app is being written on machines where the released Hangar
    /// is installed and in use, and both read the same `state.json`: a build
    /// started to try something must not rewrite the person's workspaces. Set
    /// it with `HANGAR_ENDPOINT_FILE` (endpoint.rs), which moves the endpoint,
    /// the mods, the kits and the accounts beside it, and nothing of the
    /// installed app is touched.
    pub fn from_env() -> Self {
        match std::env::var_os("HANGAR_DATA_ROOT").filter(|value| !value.is_empty()) {
            Some(root) => Self::rooted(Path::new(&root)),
            None => Self::system(),
        }
    }

    /// Whether this run is one kept off the installed app's data
    /// (`HANGAR_DATA_ROOT` set): what it must not write either, the files of
    /// other programs that the installed Hangar writes to, such as Claude
    /// Code's own config, it can ask here.
    pub fn isolated() -> bool {
        std::env::var_os("HANGAR_DATA_ROOT").is_some_and(|value| !value.is_empty())
    }

    /// Every folder under `root`, each by its own name: for tests, and for a
    /// build that must not touch the installed app's data.
    pub fn rooted(root: &Path) -> Self {
        Self {
            data: Some(root.join("data")),
            config: Some(root.join("config")),
            local_data: Some(root.join("local-data")),
            cache: Some(root.join("cache")),
            log: Some(root.join("logs")),
        }
    }

    /// `app_data_dir`: state.json, memory.json, remote.json, the voice models,
    /// the MCP scope files, the usage cache, the HQ folder.
    pub fn app_data_dir(&self) -> Result<PathBuf, UnknownPath> {
        self.data.clone().ok_or(UnknownPath)
    }

    pub fn app_config_dir(&self) -> Result<PathBuf, UnknownPath> {
        self.config.clone().ok_or(UnknownPath)
    }

    pub fn app_local_data_dir(&self) -> Result<PathBuf, UnknownPath> {
        self.local_data.clone().ok_or(UnknownPath)
    }

    pub fn app_cache_dir(&self) -> Result<PathBuf, UnknownPath> {
        self.cache.clone().ok_or(UnknownPath)
    }

    pub fn app_log_dir(&self) -> Result<PathBuf, UnknownPath> {
        self.log.clone().ok_or(UnknownPath)
    }
}

/// The name this computer goes by on its network, as the system has it;
/// `None` when it cannot be read or is empty.
pub fn hostname() -> Option<String> {
    #[cfg(unix)]
    let name = {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer's length is passed with it.
        let named = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        named.then(|| String::from_utf8_lossy(&buf[..end]).into_owned())
    };
    #[cfg(not(unix))]
    let name = std::env::var("COMPUTERNAME").ok();
    name.map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_folder_is_named_after_the_tauri_identifier() {
        let paths = Paths::system();
        let expect = |base: Option<PathBuf>, got: Result<PathBuf, UnknownPath>| {
            assert_eq!(
                got.ok(),
                base.map(|dir| dir.join("io.github.blasterwhite.teskad"))
            );
        };
        expect(dirs::data_dir(), paths.app_data_dir());
        expect(dirs::config_dir(), paths.app_config_dir());
        expect(dirs::data_local_dir(), paths.app_local_data_dir());
        expect(dirs::cache_dir(), paths.app_cache_dir());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_logs_sit_in_the_local_data_folder_outside_macos() {
        assert_eq!(
            Paths::system().app_log_dir().ok(),
            dirs::data_local_dir()
                .map(|dir| dir.join("io.github.blasterwhite.teskad").join("logs"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_logs_sit_in_library_logs_on_macos() {
        assert_eq!(
            Paths::system().app_log_dir().ok(),
            dirs::home_dir().map(|home| home.join("Library/Logs/io.github.blasterwhite.teskad"))
        );
    }

    #[test]
    fn nothing_here_is_the_official_apps() {
        assert_ne!(IDENTIFIER, OFFICIAL_IDENTIFIER);
        assert_ne!(HOME_DIR_NAME, OFFICIAL_HOME_DIR_NAME);
        assert_ne!(PRODUCT_NAME, "Hangar.AI");
        let ours = Paths::system();
        let theirs = Paths::system_for(OFFICIAL_IDENTIFIER);
        assert_ne!(ours.app_data_dir().ok(), theirs.app_data_dir().ok());
        assert_ne!(ours.app_log_dir().ok(), theirs.app_log_dir().ok());
        assert_eq!(
            home_folder().and_then(|dir| dir.file_name().map(|n| n.to_owned())),
            Some(std::ffi::OsString::from(".teskad"))
        );
    }

    #[test]
    fn an_unknown_folder_reads_as_tauri_s_error_did() {
        let none = Paths {
            data: None,
            config: None,
            local_data: None,
            cache: None,
            log: None,
        };
        assert_eq!(
            format!("no app data dir: {}", none.app_data_dir().unwrap_err()),
            "no app data dir: unknown path"
        );
    }
}
