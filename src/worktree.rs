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

/// Where agent worktrees live inside a repo that uses the fleet layout
/// (squadron SQ-204): `<repo>/.jagent/worktrees/<name>`.
pub const JAGENT_WORKTREES: &str = ".jagent/worktrees";

/// Create a worktree for `repo` at a fresh branch `branch` (BUCKETS-17).
/// Where it goes, first match wins:
///
/// 1. `path` — exactly there (`buckets worktree create --path <dir>`).
/// 2. `worktree_parent` (`BUCKETS_WORKTREE_DIR`) — `<parent>/<repo>-<branch>`.
/// 3. The repo has a `.jagent/` dir (the fleet layout, squadron SQ-204) —
///    `<main checkout>/.jagent/worktrees/<branch>`, INSIDE the repo, so an
///    agent pointed at the repo never needs out-of-path permissions. Relative
///    sibling path-deps (`../other-repo`) keep resolving through
///    `.jagent/worktrees/<sibling> -> ../../../<sibling>` links, created here
///    exactly as squadron's `worktree_link_siblings` does.
/// 4. Otherwise a SIBLING of the repo, `<repo>-<branch>` next to it (see
///    `Config::worktree_dir`'s doc comment for why not a fixed directory).
pub fn create_at(
    repo: &Path,
    branch: &str,
    base: Option<&str>,
    worktree_parent: Option<&Path>,
    path: Option<&Path>,
) -> Result<PathBuf> {
    let repo = repo
        .canonicalize()
        .with_context(|| format!("'{}' is not a directory that exists", repo.display()))?;
    if !repo.join(".git").exists() {
        bail!("{} is not a git repository (no .git)", repo.display());
    }
    let main = main_checkout(&repo);
    let repo_name = repo
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());

    let dest = match (path, worktree_parent) {
        (Some(p), _) => {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                std::env::current_dir()?.join(p)
            }
        }
        (None, Some(dir)) => dir.join(format!("{repo_name}-{}", slugify(branch))),
        (None, None) if main.join(".jagent").is_dir() => {
            let dir = main.join(JAGENT_WORKTREES);
            if !is_ignored(&main, &format!("{JAGENT_WORKTREES}/probe")) {
                eprintln!(
                    "⚠ {JAGENT_WORKTREES}/ is not gitignored in {} — add it (and commit it) \
                     or the worktree shows up in `git status` there",
                    main.display()
                );
            }
            link_siblings(&main, &dir);
            dir.join(slugify(branch))
        }
        (None, None) => repo
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(format!("{repo_name}-{}", slugify(branch))),
    };
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    if dest.exists() {
        bail!(
            "{} already exists — remove it first or pick a different branch name",
            dest.display()
        );
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
    let status = rm_cmd
        .status()
        .context("Failed to run git worktree remove")?;
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
        .arg("-C")
        .arg(&repo)
        .arg("worktree")
        .arg("list")
        .output()
        .context("Failed to run git worktree list")?;
    if !output.status.success() {
        bail!(
            "git worktree list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// The main checkout behind `repo` (itself, unless `repo` is a linked
/// worktree): the parent of `git rev-parse --git-common-dir`.
fn main_checkout(repo: &Path) -> PathBuf {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .filter(|common| common.file_name().is_some_and(|n| n == ".git"))
        .and_then(|common| common.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| repo.to_path_buf())
}

fn is_ignored(repo: &Path, rel: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["check-ignore", "-q", "--no-index", rel])
        .status()
        .is_ok_and(|s| s.success())
}

/// Siblings `repo` reaches by climbing exactly one level out of itself:
/// `path = "../…"` deps in tracked Cargo.toml files and tracked symlinks
/// whose target starts with `../`, normalized against the file's own dir.
/// A port of squadron's `worktree_scan_siblings` (lib/worktree-path.sh).
pub fn scan_siblings(repo: &Path) -> Vec<String> {
    let tracked = |spec: &[&str]| -> Vec<(String, String)> {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["ls-files", "-z", "-s", "--"])
            .args(spec)
            .output();
        let Ok(out) = out else { return Vec::new() };
        out.stdout
            .split(|b| *b == 0)
            .filter_map(|ent| {
                let ent = String::from_utf8_lossy(ent);
                let (meta, path) = ent.split_once('\t')?;
                Some((
                    meta.split_whitespace().next()?.to_string(),
                    path.to_string(),
                ))
            })
            .collect()
    };
    let dep = regex::Regex::new(r#"path\s*=\s*"(\.\./[^"]*)""#).expect("static regex");
    let mut refs: Vec<(String, String)> = Vec::new(); // (dir of source, relative target)
    for (_, path) in tracked(&["Cargo.toml", "*/Cargo.toml"]) {
        let Ok(text) = std::fs::read_to_string(repo.join(&path)) else {
            continue;
        };
        let dir = Path::new(&path)
            .parent()
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_default();
        for m in dep.captures_iter(&text) {
            refs.push((dir.clone(), m[1].to_string()));
        }
    }
    for (mode, path) in tracked(&["."]) {
        if mode != "120000" {
            continue;
        }
        let Ok(target) = std::fs::read_link(repo.join(&path)) else {
            continue;
        };
        let target = target.to_string_lossy().to_string();
        if target.starts_with("../") {
            let dir = Path::new(&path)
                .parent()
                .map(|d| d.to_string_lossy().to_string())
                .unwrap_or_default();
            refs.push((dir, target));
        }
    }
    let mut siblings: Vec<String> = Vec::new();
    for (dir, target) in refs {
        // normalize dir/target lexically (like os.path.normpath)
        let mut parts: Vec<&str> = Vec::new();
        let mut ups = 0usize;
        for comp in dir.split('/').chain(target.split('/')) {
            match comp {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        ups += 1;
                    }
                }
                other => parts.push(other),
            }
        }
        if ups == 1 {
            if let Some(first) = parts.first() {
                if !siblings.iter().any(|s| s == first) {
                    siblings.push(first.to_string());
                }
            }
        } else if ups > 1 {
            eprintln!("⚠ {target} (from {dir}) climbs more than one level out of the repo — no sibling link can satisfy it");
        }
    }
    siblings.sort();
    siblings
}

/// Create `<dir>/<sibling> -> ../../../<sibling>` for every sibling `repo`
/// references and that exists next to it (squadron `worktree_link_siblings`).
fn link_siblings(repo: &Path, dir: &Path) {
    let siblings = scan_siblings(repo);
    if siblings.is_empty() {
        return;
    }
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    for sib in siblings {
        let link = dir.join(&sib);
        let Some(outside) = repo.parent().map(|p| p.join(&sib)) else {
            continue;
        };
        if !outside.exists() {
            eprintln!(
                "⚠ sibling '{sib}' does not exist at {} — not linked",
                outside.display()
            );
            continue;
        }
        match std::fs::symlink_metadata(&link) {
            Ok(meta) if !meta.file_type().is_symlink() => {
                eprintln!(
                    "⚠ {} exists and is not a symlink — cannot link sibling '{sib}'",
                    link.display()
                );
                continue;
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&link);
            }
            Err(_) => {}
        }
        #[cfg(unix)]
        if let Err(e) = std::os::unix::fs::symlink(format!("../../../{sib}"), &link) {
            eprintln!("⚠ could not link sibling '{sib}': {e}");
        }
    }
}

fn branch_exists(repo: &Path, branch: &str) -> Result<bool> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("show-ref")
        .arg("--verify")
        .arg("--quiet")
        .arg(format!("refs/heads/{branch}"))
        .status()
        .context("Failed to run git show-ref")?;
    Ok(status.success())
}

/// Filesystem-safe worktree directory name component.
fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pre-BUCKETS-17 entry point, kept for these tests: no explicit path.
    fn create(
        repo: &Path,
        branch: &str,
        base: Option<&str>,
        worktree_parent: Option<&Path>,
    ) -> Result<PathBuf> {
        create_at(repo, branch, base, worktree_parent, None)
    }

    fn init_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .status()
                .unwrap();
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
    #[test]
    fn explicit_path_wins() {
        let repo_dir = tempfile::tempdir().unwrap();
        init_repo(repo_dir.path());
        std::fs::create_dir(repo_dir.path().join(".jagent")).unwrap();
        let spot = tempfile::tempdir().unwrap().path().join("exactly-here");
        let wt = create_at(repo_dir.path(), "b17", None, None, Some(&spot)).unwrap();
        assert_eq!(wt, spot);
        assert!(wt.join("README.md").exists());
    }

    /// BUCKETS-17 / squadron SQ-204: a fleet repo (has `.jagent/`) gets its
    /// worktrees INSIDE it, resolved to the main checkout even when handed
    /// a linked worktree; `BUCKETS_WORKTREE_DIR` still overrides.
    #[test]
    fn fleet_repo_worktrees_go_inside_the_main_checkout() {
        let outer = tempfile::tempdir().unwrap();
        let repo = outer.path().join("fleetrepo");
        std::fs::create_dir(&repo).unwrap();
        init_repo(&repo);
        std::fs::create_dir(repo.join(".jagent")).unwrap();
        let main = repo.canonicalize().unwrap();

        let wt = create_at(&repo, "agent/x/T-1", None, None, None).unwrap();
        assert_eq!(wt, main.join(".jagent/worktrees/agent-x-T-1"));
        // handed the linked worktree, it still nests under the MAIN checkout
        let nested = create_at(&wt, "agent/x/T-2", None, None, None).unwrap();
        assert_eq!(nested, main.join(".jagent/worktrees/agent-x-T-2"));
        // an explicit parent (BUCKETS_WORKTREE_DIR) still wins over the default
        let parent = tempfile::tempdir().unwrap();
        let env = create_at(&repo, "agent/x/T-3", None, Some(parent.path()), None).unwrap();
        assert_eq!(env.parent().unwrap(), parent.path());
    }

    #[test]
    fn scan_normalizes_nested_manifests_and_symlinks() {
        let repo_dir = tempfile::tempdir().unwrap();
        let repo = repo_dir.path();
        init_repo(repo);
        std::fs::create_dir_all(repo.join("crates/ai")).unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[workspace]\n[dependencies]\na = { path = \"../alpha\" }\n",
        )
        .unwrap();
        // ../../../uno from crates/ai = one level out of the repo -> sibling "uno"
        std::fs::write(repo.join("crates/ai/Cargo.toml"), "[dependencies]\nuno = { path = \"../../../uno\" }\nfar = { path = \"../../../../far\" }\n").unwrap();
        std::os::unix::fs::symlink("../zeta/lib", repo.join("zlink")).unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(args)
                .status()
                .unwrap()
                .success())
        };
        git(&["add", "."]);
        git(&["commit", "-qm", "deps"]);
        assert_eq!(
            scan_siblings(repo),
            ["alpha", "uno", "zeta"],
            "the deeper climb is not a sibling"
        );
    }

    #[test]
    fn fleet_worktree_links_existing_siblings() {
        let outer = tempfile::tempdir().unwrap();
        let repo = outer.path().join("fleetrepo");
        std::fs::create_dir(&repo).unwrap();
        init_repo(&repo);
        std::fs::create_dir(repo.join(".jagent")).unwrap();
        std::fs::create_dir(outer.path().join("peer")).unwrap();
        std::fs::write(outer.path().join("peer/marker"), "peer").unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[dependencies]\np = { path = \"../peer\" }\nq = { path = \"../missing\" }\n",
        )
        .unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .unwrap()
                .success())
        };
        git(&["add", "."]);
        git(&["commit", "-qm", "deps"]);

        let wt = create_at(&repo, "linked", None, None, None).unwrap();
        let link = repo.join(".jagent/worktrees/peer");
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            Path::new("../../../peer")
        );
        // from inside the worktree, ../peer resolves to the real sibling
        assert_eq!(
            std::fs::read_to_string(wt.join("../peer/marker")).unwrap(),
            "peer"
        );
        assert!(
            !repo.join(".jagent/worktrees/missing").exists(),
            "absent siblings are not linked"
        );
    }
}
