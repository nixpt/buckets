use anyhow::Result;
use semver::Version;
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

/// Check if the cache has metadata suggesting we've already looked up
/// a project (to avoid repeated network requests for nonexistent projects).
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
