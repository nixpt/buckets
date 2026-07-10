//! Ephemeral git worktrees: `buckets worktree create` gives a task its own
//! working copy (via `git worktree add`, not a full clone — cheap, shares
//! the repo's object store) at a fresh branch, which `buckets build`/`run`/
//! `shell` can then target directly like any other local path — no new
//! build machinery needed here, worktree creation just produces a path.
//!
//! "Destroyed once you merge": [`remove`] shells out to `git branch -d`
//! (not `-D`), which git itself refuses if the branch isn't actually
//! merged into its upstream/HEAD — that refusal IS the safety check, not
//! something reimplemented here. `--force` (→ `-D` + `worktree remove
//! --force`) is the deliberate override for "no, really, discard this."

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Create a worktree for `repo` at a fresh branch `branch`, based on
/// `base` (defaults to the repo's current `HEAD`). Returns the new
/// worktree's path, named `<repo-name>-<branch, slugified>` to keep
/// concurrent worktrees for different repos/branches from colliding.
///
/// `worktree_parent`: `None` (the default — see `Config::worktree_dir`'s
/// doc comment for why) creates the worktree as a SIBLING of `repo`
/// itself, e.g. `/workspace/projects/contextgc-my-branch` next to
/// `/workspace/projects/contextgc` — required for relative sibling
/// path-deps (`../other-repo`, this workspace's own convention) to keep
/// resolving correctly from inside the worktree. `Some(dir)` overrides
/// with an explicit parent directory instead.
pub fn create(repo: &Path, branch: &str, base: Option<&str>, worktree_parent: Option<&Path>) -> Result<PathBuf> {
    let repo = repo
        .canonicalize()
        .with_context(|| format!("'{}' is not a directory that exists", repo.display()))?;
    if !repo.join(".git").exists() {
        bail!("{} is not a git repository (no .git)", repo.display());
    }

    let parent = match worktree_parent {
        Some(dir) => dir.to_path_buf(),
        None => repo.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(".")),
    };
    std::fs::create_dir_all(&parent)
        .with_context(|| format!("Failed to create {}", parent.display()))?;

    let repo_name = repo.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "repo".to_string());
    let dest = parent.join(format!("{repo_name}-{}", slugify(branch)));
    if dest.exists() {
        bail!("{} already exists — remove it first or pick a different branch name", dest.display());
    }

    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(&repo).arg("worktree").arg("add");
    if branch_exists(&repo, branch)? {
        // Existing branch: `git worktree add <path> <branch>` (no -b).
        cmd.arg(&dest).arg(branch);
    } else {
        cmd.arg("-b").arg(branch).arg(&dest);
        if let Some(b) = base {
            cmd.arg(b);
        }
    }

    eprintln!("▶ creating worktree: {} ({branch})", dest.display());
    let status = cmd.status().context("Failed to run git worktree add")?;
    if !status.success() {
        bail!("git worktree add failed");
    }

    Ok(dest)
}

/// Remove a worktree and (unless it's still unmerged and `force` is
/// false) its branch. `force` = `git worktree remove --force` +
/// `git branch -D` (discard even if unmerged/dirty) instead of the safe
/// `remove`/`-d`.
pub fn remove(repo: &Path, worktree_path: &Path, branch: &str, force: bool) -> Result<()> {
    let repo = repo
        .canonicalize()
        .with_context(|| format!("'{}' is not a directory that exists", repo.display()))?;

    let mut rm_cmd = Command::new("git");
    rm_cmd.arg("-C").arg(&repo).arg("worktree").arg("remove");
    if force {
        rm_cmd.arg("--force");
    }
    rm_cmd.arg(worktree_path);
    eprintln!("▶ removing worktree: {}", worktree_path.display());
    let status = rm_cmd.status().context("Failed to run git worktree remove")?;
    if !status.success() {
        bail!(
            "git worktree remove failed — if it has uncommitted changes, use --force \
             (this is the same protection `git worktree remove` always has)"
        );
    }

    let mut branch_cmd = Command::new("git");
    branch_cmd.arg("-C").arg(&repo).arg("branch");
    branch_cmd.arg(if force { "-D" } else { "-d" });
    branch_cmd.arg(branch);
    let status = branch_cmd.status().context("Failed to run git branch -d")?;
    if !status.success() {
        // Deliberately not an error: the worktree is already gone (the
        // point of this function succeeded), and git's own refusal here
        // IS the "destroyed once you merge" safety property working as
        // intended — the branch just isn't merged yet.
        eprintln!(
            "⚠ branch '{branch}' was NOT deleted (git refused — likely not yet merged). \
             The worktree is gone; re-run with --force to also discard the branch, \
             or merge it and delete manually."
        );
    }

    Ok(())
}

/// List existing worktrees for `repo` (a thin wrapper over `git worktree
/// list` — nothing buckets-specific is tracked beyond what git itself
/// already knows).
pub fn list(repo: &Path) -> Result<String> {
    let repo = repo
        .canonicalize()
        .with_context(|| format!("'{}' is not a directory that exists", repo.display()))?;
    let output = Command::new("git")
        .arg("-C").arg(&repo)
        .arg("worktree").arg("list")
        .output()
        .context("Failed to run git worktree list")?;
    if !output.status.success() {
        bail!("git worktree list failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn branch_exists(repo: &Path, branch: &str) -> Result<bool> {
    let status = Command::new("git")
        .arg("-C").arg(repo)
        .arg("show-ref").arg("--verify").arg("--quiet")
        .arg(format!("refs/heads/{branch}"))
        .status()
        .context("Failed to run git show-ref")?;
    Ok(status.success())
}

/// Filesystem-safe worktree directory name component.
fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let status = Command::new("git").arg("-C").arg(dir).args(args).status().unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test"]);
        std::fs::write(dir.join("README.md"), "hello\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
    }

    #[test]
    fn slugify_replaces_unsafe_chars() {
        assert_eq!(slugify("feature/foo bar"), "feature-foo-bar");
        assert_eq!(slugify("agent/claude/EXO-47"), "agent-claude-EXO-47");
    }

    #[test]
    fn create_rejects_non_git_directory() {
        let dir = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let result = create(dir.path(), "feature", None, Some(parent.path()));
        assert!(result.is_err());
    }

    #[test]
    fn create_and_remove_roundtrip() {
        let repo_dir = tempfile::tempdir().unwrap();
        init_repo(repo_dir.path());
        let parent = tempfile::tempdir().unwrap();

        let wt = create(repo_dir.path(), "feature-x", None, Some(parent.path())).unwrap();
        assert!(wt.exists());
        assert!(wt.join("README.md").exists());

        // Unmerged branch: safe remove leaves the branch (git refuses -d),
        // but the worktree itself is gone either way.
        remove(repo_dir.path(), &wt, "feature-x", false).unwrap();
        assert!(!wt.exists());
    }

    /// Regression test for a real bug: defaulting worktree_parent to a
    /// fixed location (was ~/.buckets/worktrees/) broke every relative
    /// sibling path-dependency a repo had, because the worktree was no
    /// longer sitting next to its siblings. `None` must default to a
    /// SIBLING of the repo, not some unrelated fixed directory.
    #[test]
    fn default_worktree_parent_is_sibling_of_repo() {
        let outer = tempfile::tempdir().unwrap();
        let repo_dir = outer.path().join("myrepo");
        std::fs::create_dir(&repo_dir).unwrap();
        init_repo(&repo_dir);

        let wt = create(&repo_dir, "feature-sibling", None, None).unwrap();
        assert_eq!(wt.parent().unwrap(), outer.path().canonicalize().unwrap());

        remove(&repo_dir, &wt, "feature-sibling", true).unwrap();
    }

    #[test]
    fn create_refuses_existing_destination() {
        let repo_dir = tempfile::tempdir().unwrap();
        init_repo(repo_dir.path());
        let parent = tempfile::tempdir().unwrap();

        let _wt = create(repo_dir.path(), "feature-y", None, Some(parent.path())).unwrap();
        let second = create(repo_dir.path(), "feature-y", None, Some(parent.path()));
        assert!(second.is_err());
    }

    #[test]
    fn list_includes_created_worktree() {
        let repo_dir = tempfile::tempdir().unwrap();
        init_repo(repo_dir.path());
        let parent = tempfile::tempdir().unwrap();

        let wt = create(repo_dir.path(), "feature-z", None, Some(parent.path())).unwrap();
        let output = list(repo_dir.path()).unwrap();
        assert!(output.contains(&wt.file_name().unwrap().to_string_lossy().to_string()));
    }
}
