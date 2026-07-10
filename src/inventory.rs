use anyhow::{Context, Result};
use semver::{Version, VersionReq};
use std::io::BufRead;

use crate::config::Config;

/// Fetch the list of available versions for a project from the dist server.
pub fn list_remote_versions(config: &Config, project: &str) -> Result<Vec<Version>> {
    let url = config.versions_url(project);
    let response = ureq::get(&url)
        .call()
        .with_context(|| format!("Failed to fetch versions from {url}"))?;

    let reader = response.into_reader();
    let mut versions = Vec::new();

    for line in std::io::BufReader::new(reader).lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Lines can be plain versions ("20.11.0") or have arch/os suffixes
        // like "20.11.0+linux+x86_64". Parse the first segment as the version.
        let ver_str = line.split('+').next().unwrap_or(line);
        if let Ok(v) = Version::parse(ver_str) {
            versions.push(v);
        }
    }

    // Sort newest first
    versions.sort_by(|a, b| b.cmp(a));
    versions.dedup();

    Ok(versions)
}

/// Find the best matching remote version for a constraint.
/// Returns `None` if no version matches.
pub fn best_match(config: &Config, project: &str, constraint: &VersionReq) -> Result<Option<Version>> {
    let versions = list_remote_versions(config, project)?;
    // versions are sorted newest-first, so find() returns the newest match
    // (semver pre-release versions are excluded unless explicitly requested)
    Ok(versions.into_iter().find(|v| {
        if v.pre.is_empty() {
            constraint.matches(v)
        } else {
            false
        }
    }))
}

/// Download an versions.txt and return the latest version.
pub fn latest_version(config: &Config, project: &str) -> Result<Option<Version>> {
    let versions = list_remote_versions(config, project)?;
    Ok(versions.into_iter().find(|v| v.pre.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_url() {
        let config = Config {
            dist_url: "https://dist.pkgx.dev".to_string(),
            cache_dir: std::path::PathBuf::from("/tmp/test"),
            platform: "linux/x86_64".to_string(),
        };
        assert_eq!(
            config.versions_url("nodejs.org"),
            "https://dist.pkgx.dev/linux/x86_64/nodejs.org/versions.txt"
        );
        assert_eq!(
            config.bottle_url("nodejs.org", "20.11.0"),
            "https://dist.pkgx.dev/linux/x86_64/nodejs.org/v20.11.0.tar.xz"
        );
    }
}
