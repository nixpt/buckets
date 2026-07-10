//! Local cache inspection and the `v*` / `v<major>` / `v<major.minor>`
//! symlink scheme: is a version already installed, what versions are
//! cached, and keeping those "latest in range" symlinks current after an
//! install. [`crate::resolve` module](mod@crate::resolve) checks here first, before ever hitting the
//! network via [`crate::inventory`].

use anyhow::{Context, Result};
use semver::Version;
use std::fs;
use std::path::PathBuf;

use crate::config::Config;
use crate::types::{Installation, Package};

/// Check if a specific version of a project is already installed in cache.
pub fn is_installed(config: &Config, project: &str, version: &Version) -> bool {
    config.version_dir(project, &version.to_string()).join("bin").exists()
}

/// List all installed versions of a project, sorted newest-first.
pub fn list_installed(config: &Config, project: &str) -> Vec<Version> {
    let project_dir = config.project_dir(project);
    if !project_dir.exists() {
        return Vec::new();
    }

    let mut versions = Vec::new();
    let entries = match std::fs::read_dir(&project_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Skip symlinks (v*, v20, v20.11) — only real version dirs
        if entry.path().is_symlink() {
            continue;
        }
        if let Some(ver_str) = name_str.strip_prefix('v') {
            if let Ok(ver) = Version::parse(ver_str) {
                versions.push(ver);
            }
        }
    }

    versions.sort_by(|a, b| b.cmp(a)); // newest first
    versions
}

/// Find the best installed version matching a constraint.
pub fn best_installed(config: &Config, project: &str, constraint: &semver::VersionReq) -> Option<Version> {
    list_installed(config, project)
        .into_iter()
        .find(|v| constraint.matches(v))
}

/// Get the full Installation struct for an installed package.
pub fn get_installation(config: &Config, pkg: &Package) -> Installation {
    let path = config.version_dir(&pkg.project, &pkg.version.to_string());
    Installation {
        pkg: pkg.clone(),
        path,
    }
}

/// Create version symlinks after a new installation (pkgx-style).
///
/// Creates relative symlinks in the project directory:
/// - `v*` → `v<full-version>` (latest overall)
/// - `v<major>` → `v<full-version>` (latest in major series)
/// - `v<major.minor>` → `v<full-version>` (latest in minor series)
///
/// Symlinks are updated to point to the NEWEST installed version
/// that matches the respective prefix.
pub fn update_version_symlinks(config: &Config, project: &str, installed: &Version) -> Result<()> {
    let project_dir = config.project_dir(project);
    fs::create_dir_all(&project_dir)
        .with_context(|| format!("Failed to create project dir: {}", project_dir.display()))?;

    let all_versions = list_installed(config, project);

    // Find the latest version overall for v*
    if let Some(latest) = all_versions.first() {
        create_symlink(&project_dir, "v*", latest)?;
    }

    // Find the latest version matching this major
    let major = installed.major;
    if let Some(latest_major) = all_versions.iter().find(|v| v.major == major) {
        create_symlink(&project_dir, &format!("v{major}"), latest_major)?;
    }

    // Find the latest version matching this major.minor
    let minor = installed.minor;
    if let Some(latest_minor) = all_versions.iter().find(|v| v.major == major && v.minor == minor) {
        create_symlink(&project_dir, &format!("v{major}.{minor}"), latest_minor)?;
    }

    Ok(())
}

/// Create or update a relative symlink at `project_dir/<name>` pointing
/// to `v<version>`.
fn create_symlink(project_dir: &PathBuf, name: &str, version: &Version) -> Result<()> {
    let link_path = project_dir.join(name);
    let target = format!("v{}", version);

    // Remove existing symlink or directory if present
    if link_path.exists() || link_path.is_symlink() {
        fs::remove_file(&link_path).or_else(|_| fs::remove_dir_all(&link_path))?;
    }

    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link_path)
        .with_context(|| format!("Failed to create symlink {} → {target}", link_path.display()))?;

    #[cfg(windows)]
    {
        // On Windows, use directory junction or fall back
        let _ = std::os::windows::fs::symlink_dir(&target, &link_path)
            .or_else(|_| std::os::windows::fs::symlink_file(&target, &link_path));
    }

    Ok(())
}

/// Resolve a version alias (v*, v20, v20.11) to the concrete version it points to.
pub fn resolve_symlink(config: &Config, project: &str, alias: &str) -> Option<Version> {
    let path = config.project_dir(project).join(alias);
    if !path.is_symlink() {
        return None;
    }
    let target = std::fs::read_link(&path).ok()?;
    let name = target.file_name()?.to_string_lossy();
    if let Some(ver_str) = name.strip_prefix('v') {
        Version::parse(ver_str).ok()
    } else {
        None
    }
}

/// Check if the cache has metadata suggesting we've already looked up
/// a project (to avoid repeated network requests).
#[allow(dead_code)]
pub fn has_lookup_tombstone(config: &Config, project: &str) -> bool {
    config.project_dir(project).join(".no-such-project").exists()
}

/// Mark a project as nonexistent in the distribution server.
#[allow(dead_code)]
pub fn mark_lookup_tombstone(config: &Config, project: &str) -> Result<()> {
    let dir = config.project_dir(project);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(".no-such-project"), "")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use semver::Version;

    #[test]
    fn test_version_sorting() {
        let v1 = Version::parse("20.11.0").unwrap();
        let v2 = Version::parse("20.10.0").unwrap();
        let v3 = Version::parse("21.0.0").unwrap();
        let mut versions = vec![v2.clone(), v3.clone(), v1.clone()];
        versions.sort_by(|a, b| b.cmp(a));
        assert_eq!(versions, vec![v3, v1, v2]);
    }

    #[test]
    fn test_symlink_target_format() {
        let v = Version::parse("20.11.0").unwrap();
        assert_eq!(format!("v{}", v), "v20.11.0");
    }
}
