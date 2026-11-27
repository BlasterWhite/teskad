//! The variables a person sets (development, tests, `TESKAD_WEBVIEW`,
//! `TESKAD_DATA_ROOT`, a node's binary…), read under their name and, failing
//! that, under the name they had before the rename.
//!
//! Teskad's name comes first: `TESKAD_X` is read, and `HANGAR_X` only when
//! `TESKAD_X` is not set, so a script or a shell profile written for the
//! upstream app keeps working. Nothing is ever written under the old name.
//!
//! The variables the app sets for its own children (a pane's id and API
//! token, the MCP configuration, the bridge's URL, the endpoint file…) do not
//! come through here: they are set and read as `TESKAD_*` alone, so a Teskad
//! started inside a pane of the official app never mistakes that pane's for
//! its own.

use std::env::VarError;
use std::ffi::OsString;

/// The pre-rename name of a `TESKAD_*` variable.
fn upstream_name(name: &str) -> Option<String> {
    name.strip_prefix("TESKAD_")
        .map(|rest| format!("HANGAR_{rest}"))
}

/// [`std::env::var_os`] of `name` (a `TESKAD_*` name), then of its
/// pre-rename `HANGAR_*` name.
pub fn var_os(name: &str) -> Option<OsString> {
    std::env::var_os(name).or_else(|| upstream_name(name).and_then(std::env::var_os))
}

/// [`std::env::var`] of `name` (a `TESKAD_*` name), then of its pre-rename
/// `HANGAR_*` name.
pub fn var(name: &str) -> Result<String, VarError> {
    match std::env::var(name) {
        Err(VarError::NotPresent) => match upstream_name(name) {
            Some(old) => std::env::var(old),
            None => Err(VarError::NotPresent),
        },
        found => found,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each test has its own names: the environment is the process's.
    #[test]
    fn the_new_name_wins_and_the_old_one_stands_in() {
        let (new, old) = ("TESKAD_ENV_TEST_A", "HANGAR_ENV_TEST_A");
        std::env::remove_var(new);
        std::env::set_var(old, "old");
        assert_eq!(var(new).as_deref(), Ok("old"));
        assert_eq!(var_os(new), Some("old".into()));
        std::env::set_var(new, "new");
        assert_eq!(var(new).as_deref(), Ok("new"));
        assert_eq!(var_os(new), Some("new".into()));
        std::env::remove_var(new);
        std::env::remove_var(old);
        assert_eq!(var(new), Err(VarError::NotPresent));
        assert_eq!(var_os(new), None);
    }

    #[test]
    fn a_name_that_is_not_teskad_s_has_no_old_name() {
        std::env::set_var("HANGAR_ENV_TEST_B", "x");
        assert_eq!(var("OTHER_ENV_TEST_B"), Err(VarError::NotPresent));
        assert_eq!(var_os("HANGAR_ENV_TEST_B"), Some("x".into()));
        std::env::remove_var("HANGAR_ENV_TEST_B");
    }
}
