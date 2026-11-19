//! A delegation's own worktree (docs/native/agents.md §5.2, "Isolation"): a branch made from the
//! caller's last commit, checked out where Hangar makes its worktrees
//! (`<parent of the main checkout>/<name>-worktrees/`), locked while the child works in it so
//! that nothing sweeps it away meanwhile (`git::sweep_orphan_worktrees` leaves a locked one).
//! What the caller has not committed is not in it.

use std::path::Path;

use crate::git;

/// A worktree made for a delegation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Made {
    /// The main checkout of the repository.
    pub repo: String,
    /// The worktree's folder.
    pub path: String,
    /// Its branch.
    pub branch: String,
    /// The commit it was made from: the caller's HEAD then.
    pub base: String,
}

/// Makes delegation `id`'s worktree from the last commit of the repository holding `cwd`.
pub(super) async fn make(cwd: &str, id: &str) -> Result<Made, String> {
    let info = git::git_repo_info(cwd.to_string()).await;
    if !info.installed {
        return Err(
            "git was not found, so no worktree can be made: use the shared isolation".into(),
        );
    }
    if !info.is_repo {
        return Err(format!(
            "{cwd} is not in a git repository, so no worktree can be made from it: use the shared \
             isolation"
        ));
    }
    let base = info.head.ok_or_else(|| {
        format!(
            "{cwd} has no commit yet, so no worktree can be made from it: use the shared isolation"
        )
    })?;
    let repo = info
        .main_root
        .or(info.root)
        .ok_or_else(|| format!("the repository of {cwd} has no working tree"))?;
    let folder =
        worktrees_folder(&repo).ok_or_else(|| format!("no folder for worktrees beside {repo}"))?;
    // The ids start again with each Hangar: a part of its own keeps a new branch from meeting
    // one an earlier delegation left.
    let unique = &uuid::Uuid::new_v4().simple().to_string()[..6];
    let branch = format!("delegate/{id}-{unique}");
    let path = Path::new(&folder)
        .join(format!("delegate-{id}-{unique}"))
        .to_string_lossy()
        .into_owned();
    git::git_worktree_add(
        cwd.to_string(),
        path.clone(),
        branch.clone(),
        true,
        Some(base.clone()),
    )
    .await
    .map_err(|why| format!("could not make the delegation's worktree: {why}"))?;
    let made = Made {
        repo,
        path,
        branch,
        base,
    };
    // A worktree that could not be locked still works.
    if let Err(why) = git::git_worktree_lock(made.repo.clone(), made.path.clone()).await {
        eprintln!("[delegate] {id}: could not lock {}: {why}", made.path);
    }
    Ok(made)
}

/// `<parent of the main checkout>/<name>-worktrees`, where Hangar makes its worktrees.
fn worktrees_folder(repo: &str) -> Option<String> {
    let repo = Path::new(repo.trim_end_matches(['/', '\\']));
    let name = repo.file_name()?.to_string_lossy();
    if name.is_empty() {
        return None;
    }
    let parent = repo.parent()?;
    Some(
        parent
            .join(format!("{name}-worktrees"))
            .to_string_lossy()
            .into_owned(),
    )
}

/// The child is done with it: unlocked, left for the caller.
pub(super) async fn release(made: &Made) {
    let _ = git::git_worktree_unlock(made.repo.clone(), made.path.clone()).await;
}

/// The files the child changed there since the worktree was made — committed, staged, or not —
/// as absolute paths.
pub(super) async fn changed(made: &Made) -> Vec<String> {
    match git::git_changes(made.path.clone(), Some(made.base.clone())).await {
        // Relative to the worktree's root: its folder, as the child was given it.
        Ok(changes) => changes
            .files
            .iter()
            .map(|file| {
                Path::new(&made.path)
                    .join(&file.path)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Removes the worktree and its branch, unless the branch holds commits its base does not: true
/// when it went, false when it stays for its commits. Uncommitted changes go with it.
pub(super) async fn remove(made: &Made) -> Result<bool, String> {
    let ahead =
        git::git_commits_ahead(made.repo.clone(), made.base.clone(), made.branch.clone()).await?;
    if ahead > 0 {
        release(made).await;
        return Ok(false);
    }
    release(made).await;
    let removed =
        git::git_worktree_remove(made.repo.clone(), made.path.clone(), true, None).await?;
    if !removed.removed {
        return Err(removed
            .message
            .unwrap_or_else(|| "git kept the worktree".into()));
    }
    git::git_branch_delete(made.repo.clone(), made.branch.clone(), true).await?;
    Ok(true)
}
