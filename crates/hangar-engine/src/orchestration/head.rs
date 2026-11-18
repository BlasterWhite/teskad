//! How far a checkout's HEAD has moved: what tells the orchestration engine
//! that a worker it called idle committed since (`EngineIo::head_moves`).
//!
//! Every commit, checkout or reset made in a checkout adds a line to its HEAD
//! reflog, a linked worktree's own included: the file's length is a counter
//! of them, read without starting git.

use std::path::{Path, PathBuf};

/// The git folder of the checkout at `cwd`: `.git` itself, or the folder a
/// linked worktree's `.git` file points to.
fn git_dir(cwd: &Path) -> Option<PathBuf> {
    let dot = cwd.join(".git");
    if dot.is_dir() {
        return Some(dot);
    }
    let text = std::fs::read_to_string(&dot).ok()?;
    let target = text
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))?
        .trim();
    let target = Path::new(target);
    Some(if target.is_absolute() {
        target.to_path_buf()
    } else {
        cwd.join(target)
    })
}

/// The length of the HEAD reflog of the checkout at `cwd`. `None`: not the
/// root of a git checkout, or one that keeps no reflog.
pub fn moves(cwd: &str) -> Option<u64> {
    if cwd.is_empty() {
        return None;
    }
    let dir = git_dir(Path::new(cwd))?;
    std::fs::metadata(dir.join("logs").join("HEAD"))
        .ok()
        .map(|meta| meta.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(dir: &Path, lines: usize) {
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        std::fs::write(dir.join("logs").join("HEAD"), "a b c\n".repeat(lines)).unwrap();
    }

    #[test]
    fn counts_a_checkouts_moves_and_a_linked_worktrees_own() {
        let root = crate::testdir::scratch("head");
        let main = root.join("main");
        log(&main.join(".git"), 2);
        assert_eq!(moves(&main.to_string_lossy()), Some(12));
        log(&main.join(".git"), 3);
        assert_eq!(moves(&main.to_string_lossy()), Some(18));

        // A worktree's `.git` file, absolute or relative.
        let tree = root.join("tree");
        let own = main.join(".git").join("worktrees").join("tree");
        log(&own, 1);
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join(".git"), format!("gitdir: {}\n", own.display())).unwrap();
        assert_eq!(moves(&tree.to_string_lossy()), Some(6));
        std::fs::write(tree.join(".git"), "gitdir: ../main/.git/worktrees/tree\n").unwrap();
        assert_eq!(moves(&tree.to_string_lossy()), Some(6));
    }

    #[test]
    fn a_folder_that_is_no_checkout_has_nothing_to_count() {
        let root = crate::testdir::scratch("head-none");
        assert_eq!(moves(&root.to_string_lossy()), None);
        assert_eq!(moves(""), None);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        assert_eq!(moves(&root.to_string_lossy()), None);
    }
}
