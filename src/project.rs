//! Source resolution + build-system detection for `buckets build`: turn a
//! git URL or local path into a real directory on disk, then figure out
//! what toolchain it needs and how to build/test/run it — reusing
//! [`crate::resolve::resolve_multi`] for the toolchain exactly like a
//! plain `buckets run` spec would.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Everything the CLI's `build` subcommand needs to drive a project
/// through the resolve → sandboxed-build pipeline.
#[derive(Debug, Clone)]
pub struct ProjectPlan {
    pub source_dir: PathBuf,
    /// Was `source_dir` a git clone into a tempdir? If so, the caller
    /// cleans it up afterward (unless the build failed — left for
    /// inspection then).
    pub is_temp: bool,
    /// Fed straight into `resolve::resolve_multi`.
    pub toolchain_specs: Vec<String>,
    pub build_cmd: Vec<String>,
    pub test_cmd: Option<Vec<String>>,
    /// Best-effort guess (e.g. `cargo run`) — not every build system has
    /// an unambiguous "run the project" convention, so this may be `None`.
    pub run_cmd: Option<Vec<String>>,
}

/// A git URL (`https://...`, `git@host:owner/repo.git`, `ssh://...`, or
/// anything ending `.git`) gets shallow-cloned into a fresh tempdir;
/// anything else is treated as a local path as-is (must already exist).
/// Returns `(dir, is_temp)`.
pub fn resolve_source(input: &str) -> Result<(PathBuf, bool)> {
    if looks_like_git_url(input) {
        let dir = tempfile::Builder::new()
            .prefix("buckets-build-")
            .tempdir()
            .context("Failed to create temp dir for git clone")?
            .keep(); // caller owns cleanup via ProjectPlan::is_temp
        eprintln!("↓ cloning {input} into {}", dir.display());
        let status = std::process::Command::new("git")
            .args(["clone", "--depth", "1", input])
            .arg(&dir)
            .status()
            .context("Failed to run git clone")?;
        if !status.success() {
            anyhow::bail!("git clone failed for {input}");
        }
        Ok((dir, true))
    } else {
        let path = PathBuf::from(input);
        let canonical = path
            .canonicalize()
            .with_context(|| format!("'{input}' is not a git URL and not a local path that exists"))?;
        Ok((canonical, false))
    }
}

fn looks_like_git_url(s: &str) -> bool {
    s.starts_with("https://")
        || s.starts_with("http://")
        || s.starts_with("git@")
        || s.starts_with("ssh://")
        || s.starts_with("git://")
        || s.ends_with(".git")
}

/// Detect the build system at `source_dir` and produce a [`ProjectPlan`].
/// Checked in order (most-specific/most-common first): Cargo.toml,
/// package.json, pyproject.toml/setup.py/requirements.txt, go.mod,
/// Makefile. `Err` if nothing recognized.
pub fn detect(source_dir: &Path) -> Result<ProjectPlan> {
    let base = |toolchain_specs: Vec<String>, build_cmd: Vec<String>,
                test_cmd: Option<Vec<String>>, run_cmd: Option<Vec<String>>| ProjectPlan {
        source_dir: source_dir.to_path_buf(),
        is_temp: false, // caller overwrites from resolve_source's result
        toolchain_specs,
        build_cmd,
        test_cmd,
        run_cmd,
    };

    if source_dir.join("Cargo.toml").exists() {
        let spec = rust_toolchain_spec(source_dir);
        return Ok(base(
            vec![spec],
            vec!["cargo".into(), "build".into()],
            Some(vec!["cargo".into(), "test".into()]),
            Some(vec!["cargo".into(), "run".into()]),
        ));
    }

    if source_dir.join("package.json").exists() {
        let spec = node_toolchain_spec(source_dir);
        let scripts = node_scripts(source_dir);
        return Ok(base(
            vec![spec],
            vec!["npm".into(), "install".into()],
            scripts.contains(&"test".to_string()).then(|| vec!["npm".into(), "test".into()]),
            scripts.contains(&"start".to_string()).then(|| vec!["npm".into(), "start".into()]),
        ));
    }

    if source_dir.join("pyproject.toml").exists() || source_dir.join("setup.py").exists() {
        return Ok(base(
            vec!["python".into()],
            vec!["pip".into(), "install".into(), "-e".into(), ".".into()],
            None, // pytest isn't a declared dependency of the `python` bucket — see module doc
            None,
        ));
    }
    if source_dir.join("requirements.txt").exists() {
        return Ok(base(
            vec!["python".into()],
            vec!["pip".into(), "install".into(), "-r".into(), "requirements.txt".into()],
            None,
            None,
        ));
    }

    if source_dir.join("go.mod").exists() {
        let spec = go_toolchain_spec(source_dir);
        return Ok(base(
            vec![spec],
            vec!["go".into(), "build".into(), "./...".into()],
            Some(vec!["go".into(), "test".into(), "./...".into()]),
            Some(vec!["go".into(), "run".into(), ".".into()]),
        ));
    }

    if source_dir.join("Makefile").exists() || source_dir.join("makefile").exists() {
        return Ok(base(
            vec!["make".into()],
            vec!["make".into()],
            Some(vec!["make".into(), "test".into()]),
            None,
        ));
    }

    anyhow::bail!(
        "couldn't detect a build system in {} (looked for Cargo.toml, package.json, \
         pyproject.toml/setup.py/requirements.txt, go.mod, Makefile)",
        source_dir.display()
    )
}

/// `rust-toolchain.toml`'s `channel = "1.75.0"` or a plain `rust-toolchain`
/// file's content, IF it's a real version number — pkgx resolves semver,
/// not rustup channel names ("stable"/"nightly" aren't valid specs here).
/// Falls back to bare "rust" (latest) otherwise.
fn rust_toolchain_spec(source_dir: &Path) -> String {
    let channel = std::fs::read_to_string(source_dir.join("rust-toolchain.toml"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.trim().strip_prefix("channel").map(str::to_string))
        })
        .and_then(|rest| rest.split('"').nth(1).map(str::to_string))
        .or_else(|| std::fs::read_to_string(source_dir.join("rust-toolchain")).ok().map(|s| s.trim().to_string()));

    match channel {
        Some(c) if c.chars().next().is_some_and(|ch| ch.is_ascii_digit()) => format!("rust@{c}"),
        _ => "rust".to_string(),
    }
}

/// `package.json`'s `engines.node`, if present (e.g. `"^20"` -> `node@^20`).
fn node_toolchain_spec(source_dir: &Path) -> String {
    read_json(&source_dir.join("package.json"))
        .and_then(|v| v.get("engines")?.get("node")?.as_str().map(str::to_string))
        .map(|c| format!("node@{c}"))
        .unwrap_or_else(|| "node".to_string())
}

/// `package.json`'s `scripts` keys (e.g. does it have a "test"/"start" script).
fn node_scripts(source_dir: &Path) -> Vec<String> {
    read_json(&source_dir.join("package.json"))
        .and_then(|v| v.get("scripts")?.as_object().map(|m| m.keys().cloned().collect()))
        .unwrap_or_default()
}

/// `go.mod`'s `go 1.22` directive line, if present.
fn go_toolchain_spec(source_dir: &Path) -> String {
    std::fs::read_to_string(source_dir.join("go.mod"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.trim().strip_prefix("go ").map(|v| v.trim().to_string()))
        })
        .map(|v| format!("go@{v}"))
        .unwrap_or_else(|| "go".to_string())
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn detects_rust_project() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Cargo.toml", "[package]\nname = \"x\"\n");
        let plan = detect(dir.path()).unwrap();
        assert_eq!(plan.toolchain_specs, vec!["rust"]);
        assert_eq!(plan.build_cmd, vec!["cargo", "build"]);
        assert!(plan.test_cmd.is_some());
        assert!(plan.run_cmd.is_some());
    }

    #[test]
    fn detects_pinned_rust_toolchain() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Cargo.toml", "[package]\nname = \"x\"\n");
        write(dir.path(), "rust-toolchain.toml", "[toolchain]\nchannel = \"1.75.0\"\n");
        let plan = detect(dir.path()).unwrap();
        assert_eq!(plan.toolchain_specs, vec!["rust@1.75.0"]);
    }

    #[test]
    fn ignores_non_numeric_rust_channel() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Cargo.toml", "[package]\nname = \"x\"\n");
        write(dir.path(), "rust-toolchain.toml", "[toolchain]\nchannel = \"stable\"\n");
        let plan = detect(dir.path()).unwrap();
        assert_eq!(plan.toolchain_specs, vec!["rust"]);
    }

    #[test]
    fn detects_node_project_with_engines_and_scripts() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", r#"{"engines":{"node":"^20"},"scripts":{"test":"jest","start":"node index.js"}}"#);
        let plan = detect(dir.path()).unwrap();
        assert_eq!(plan.toolchain_specs, vec!["node@^20"]);
        assert_eq!(plan.test_cmd, Some(vec!["npm".to_string(), "test".to_string()]));
        assert_eq!(plan.run_cmd, Some(vec!["npm".to_string(), "start".to_string()]));
    }

    #[test]
    fn node_without_engines_or_scripts_defaults() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", r#"{"name":"x"}"#);
        let plan = detect(dir.path()).unwrap();
        assert_eq!(plan.toolchain_specs, vec!["node"]);
        assert_eq!(plan.test_cmd, None);
        assert_eq!(plan.run_cmd, None);
    }

    #[test]
    fn detects_python_pyproject() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "pyproject.toml", "[project]\nname = \"x\"\n");
        let plan = detect(dir.path()).unwrap();
        assert_eq!(plan.toolchain_specs, vec!["python"]);
        assert_eq!(plan.build_cmd, vec!["pip", "install", "-e", "."]);
    }

    #[test]
    fn detects_python_requirements_only() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "requirements.txt", "requests\n");
        let plan = detect(dir.path()).unwrap();
        assert_eq!(plan.build_cmd, vec!["pip", "install", "-r", "requirements.txt"]);
    }

    #[test]
    fn detects_go_project_with_pinned_version() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "go.mod", "module example.com/x\n\ngo 1.22\n");
        let plan = detect(dir.path()).unwrap();
        assert_eq!(plan.toolchain_specs, vec!["go@1.22"]);
        assert_eq!(plan.build_cmd, vec!["go", "build", "./..."]);
    }

    #[test]
    fn detects_makefile_project() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Makefile", "all:\n\techo hi\n");
        let plan = detect(dir.path()).unwrap();
        assert_eq!(plan.toolchain_specs, vec!["make"]);
        assert_eq!(plan.build_cmd, vec!["make"]);
    }

    #[test]
    fn cargo_toml_takes_priority_over_makefile() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Cargo.toml", "[package]\nname = \"x\"\n");
        write(dir.path(), "Makefile", "all:\n\techo hi\n");
        let plan = detect(dir.path()).unwrap();
        assert_eq!(plan.toolchain_specs, vec!["rust"]);
    }

    #[test]
    fn no_recognized_build_system_is_err() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "README.md", "just docs\n");
        assert!(detect(dir.path()).is_err());
    }

    #[test]
    fn git_url_detection() {
        assert!(looks_like_git_url("https://github.com/nixpt/buckets"));
        assert!(looks_like_git_url("https://github.com/nixpt/buckets.git"));
        assert!(looks_like_git_url("git@github.com:nixpt/buckets.git"));
        assert!(looks_like_git_url("ssh://git@github.com/nixpt/buckets.git"));
        assert!(!looks_like_git_url("/workspace/projects/buckets"));
        assert!(!looks_like_git_url("../buckets"));
        assert!(!looks_like_git_url("buckets"));
    }

    #[test]
    fn resolve_source_local_path_must_exist() {
        assert!(resolve_source("/definitely/does/not/exist/anywhere").is_err());
    }

    #[test]
    fn resolve_source_local_path_canonicalizes() {
        let dir = tempfile::tempdir().unwrap();
        let (resolved, is_temp) = resolve_source(dir.path().to_str().unwrap()).unwrap();
        assert!(!is_temp);
        assert_eq!(resolved, dir.path().canonicalize().unwrap());
    }
}
