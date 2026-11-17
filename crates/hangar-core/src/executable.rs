//! The path Hangar gives others to start it again: the agents' MCP configs
//! (install.rs) and the login item (autostart.rs) keep it after this run ends.
//!
//! That is the running executable, except on Nix. A Nix package runs from
//! `/nix/store/<hash>-hangar-<version>/bin/hangar-ai`, a folder the next
//! upgrade replaces and the garbage collector then deletes, so a config naming
//! it stops working a few days after an update. What the system puts on the
//! `PATH` (`/run/current-system/sw/bin`, a profile's `bin`) is a link that
//! follows each upgrade: when one of them leads to the binary running now,
//! that link is the path given.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Where packages are installed on Nix and NixOS.
const NIX_STORE: &str = "/nix/store";

/// The path to start this program again by: the running executable, or on
/// Nix the link to it that outlives the next upgrade (see the module).
pub fn stable() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(stable_path(&exe, std::env::var_os("PATH").as_deref()))
}

/// [`stable`] with the executable and the `PATH` passed in, for the tests.
///
/// A folder of the `PATH` gives its entry of the same name when that entry,
/// links followed, is the executable itself, and it is not in the store: a
/// link from the store to the store is no steadier than the binary.
pub(crate) fn stable_path(exe: &Path, path: Option<&OsStr>) -> PathBuf {
    if !exe.starts_with(NIX_STORE) {
        return exe.to_path_buf();
    }
    let (Some(name), Ok(target)) = (exe.file_name(), exe.canonicalize()) else {
        return exe.to_path_buf();
    };
    path.into_iter()
        .flat_map(std::env::split_paths)
        .filter(|folder| folder.is_absolute() && !folder.starts_with(NIX_STORE))
        .map(|folder| folder.join(name))
        .find(|candidate| candidate.canonicalize().is_ok_and(|real| real == target))
        .unwrap_or_else(|| exe.to_path_buf())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::fs::symlink;

    fn path_of(folders: &[&Path]) -> OsString {
        std::env::join_paths(folders).unwrap()
    }

    #[test]
    fn an_executable_outside_the_store_is_given_as_it_is() {
        let dir = crate::testdir::scratch("executable-plain");
        let exe = dir.join("hangar-ai");
        std::fs::write(&exe, "").unwrap();
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        symlink(&exe, bin.join("hangar-ai")).unwrap();

        assert_eq!(stable_path(&exe, Some(&path_of(&[&bin]))), exe);
        std::fs::remove_dir_all(dir).ok();
    }

    // The store itself cannot be written from a test: these look for a binary
    // under /nix/store that is on this machine's PATH, which is a NixOS (or Nix)
    // machine's case, and say nothing elsewhere.
    fn store_binary_on_path() -> Option<(PathBuf, PathBuf)> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .filter(|folder| !folder.starts_with(NIX_STORE))
            .filter_map(|folder| std::fs::read_dir(folder).ok())
            .flatten()
            .filter_map(Result::ok)
            .find_map(|entry| {
                let real = entry.path().canonicalize().ok()?;
                (real.starts_with(NIX_STORE) && real.file_name() == Some(&entry.file_name()))
                    .then(|| (entry.path(), real))
            })
    }

    #[test]
    fn a_store_executable_is_given_as_the_link_on_the_path() {
        let Some((link, real)) = store_binary_on_path() else {
            return;
        };
        let folder = link.parent().unwrap();
        assert_eq!(stable_path(&real, Some(&path_of(&[folder]))), link);
    }

    #[test]
    fn a_store_executable_no_link_leads_to_is_given_as_it_is() {
        let Some((_, real)) = store_binary_on_path() else {
            return;
        };
        let dir = crate::testdir::scratch("executable-unrelated");
        let decoy = real.file_name().unwrap();
        std::fs::write(dir.join(decoy), "").unwrap();

        assert_eq!(stable_path(&real, Some(&path_of(&[&dir]))), real);
        assert_eq!(stable_path(&real, None), real);
        std::fs::remove_dir_all(dir).ok();
    }
}
