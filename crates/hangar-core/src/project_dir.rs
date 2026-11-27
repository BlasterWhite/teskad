//! The folder Teskad keeps inside every project it opens: `<project>/.teskad/`
//! (the board, the phone's uploads, annotations).
//!
//! Projects opened before the rename hold the same things in `.hangar/`. The
//! first time one is opened, that folder is *copied* to `.teskad/`: never
//! moved, because the official app may still use it.

use std::path::{Path, PathBuf};

/// The project folder's name, the one place it is written.
pub const PROJECT_DIR_NAME: &str = ".teskad";

/// What it was called before: read once by [`adopt`], never written.
pub const LEGACY_PROJECT_DIR_NAME: &str = ".hangar";

/// Left in `.teskad/` by [`adopt`]: the copy was made, do not make it again.
pub const ADOPTED_MARKER: &str = ".adopted-from-hangar";

/// `<root>/.teskad`.
pub fn project_dir(root: &Path) -> PathBuf {
    root.join(PROJECT_DIR_NAME)
}

/// Copies `<root>/.hangar` to `<root>/.teskad` when only the former exists.
///
/// Nothing happens when `.teskad` exists (adopted before, or new), or when
/// there is no `.hangar`. The copy is made beside, then renamed into place, so
/// an interrupted copy is never taken for a finished one. Best effort: on
/// failure `.teskad` is simply not there and the next call tries again.
/// Returns whether a copy was made.
pub fn adopt(root: &Path) -> bool {
    let target = project_dir(root);
    let legacy = root.join(LEGACY_PROJECT_DIR_NAME);
    if target.exists() || !legacy.is_dir() {
        return false;
    }
    let staging = root.join(format!("{PROJECT_DIR_NAME}-adopting"));
    let _ = std::fs::remove_dir_all(&staging);
    let copied = copy_tree(&legacy, &staging)
        .and_then(|()| std::fs::write(staging.join(ADOPTED_MARKER), b""))
        .and_then(|()| std::fs::rename(&staging, &target));
    if copied.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
        return false;
    }
    true
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "teskad-projdir-{tag}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed(root: &Path, dir: &str) {
        std::fs::create_dir_all(root.join(dir).join("uploads")).unwrap();
        std::fs::write(root.join(dir).join("board.json"), b"{\"v\":1}").unwrap();
        std::fs::write(root.join(dir).join("uploads/a.png"), b"png").unwrap();
    }

    #[test]
    fn old_alone_is_copied_and_left_intact() {
        let root = tmp("old");
        seed(&root, ".hangar");
        assert!(adopt(&root));
        assert_eq!(
            std::fs::read(root.join(".teskad/board.json")).unwrap(),
            b"{\"v\":1}"
        );
        assert_eq!(
            std::fs::read(root.join(".teskad/uploads/a.png")).unwrap(),
            b"png"
        );
        assert!(root.join(".teskad").join(ADOPTED_MARKER).is_file());
        assert_eq!(
            std::fs::read(root.join(".hangar/board.json")).unwrap(),
            b"{\"v\":1}"
        );
        assert!(root.join(".hangar/uploads/a.png").is_file());
        assert!(!root.join(".teskad-adopting").exists());
        // Not copied again, even if the old one changed.
        std::fs::write(root.join(".hangar/board.json"), b"{\"v\":2}").unwrap();
        assert!(!adopt(&root));
        assert_eq!(
            std::fs::read(root.join(".teskad/board.json")).unwrap(),
            b"{\"v\":1}"
        );
    }

    #[test]
    fn new_alone_does_nothing() {
        let root = tmp("new");
        seed(&root, ".teskad");
        assert!(!adopt(&root));
        assert!(!root.join(".hangar").exists());
        assert!(!root.join(".teskad").join(ADOPTED_MARKER).exists());
    }

    #[test]
    fn both_do_nothing() {
        let root = tmp("both");
        seed(&root, ".hangar");
        std::fs::create_dir_all(root.join(".teskad")).unwrap();
        std::fs::write(root.join(".teskad/board.json"), b"mine").unwrap();
        assert!(!adopt(&root));
        assert_eq!(
            std::fs::read(root.join(".teskad/board.json")).unwrap(),
            b"mine"
        );
        assert!(!root.join(".teskad/uploads").exists());
    }

    #[test]
    fn neither_does_nothing() {
        let root = tmp("none");
        assert!(!adopt(&root));
        assert!(!root.join(".teskad").exists());
    }
}
