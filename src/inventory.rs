//! Remote version discovery: fetch a project's `versions.txt` from the dist
//! server and pick the best version matching a semver constraint. Consulted
//! by the [`crate::resolve` module](mod@crate::resolve) only when [`crate::cellar`] has no cached version
//! that already satisfies the request.

use anyhow::{Context, Result};
use semver::{Version, VersionReq};
use serde::Deserialize;
use std::io::BufRead;

use crate::config::Config;

/// Parse a dist-server version string, tolerating OpenSSL's pre-3.0
/// letter-suffixed scheme (`1.1.1w`, not valid strict semver — a bare
/// `Version::parse` silently drops every one of these, which is fatal for
/// any package (like node) that dynamically links a specific `1.1.x`
/// build: none of that whole line ever becomes visible to resolve
/// against). Reparsed as semver build metadata (`1.1.1+w`), which is
/// spec-valid and preserves the distinguishing suffix — with the known
/// tradeoff that semver ignores build metadata for ordering, so among
/// several `1.1.1<letter>` releases which one `best_match` picks isn't
/// guaranteed to be the alphabetically-latest patch letter. Acceptable
/// here: the goal is "some working 1.1.1x build exists and resolves",
/// not exact patch-letter precision.
pub(crate) fn parse_dist_version(ver_str: &str) -> Option<Version> {
    if let Ok(v) = Version::parse(ver_str) {
        return Some(v);
    }
    let re = regex::Regex::new(r"^(\d+\.\d+\.\d+)([a-z]+)$").ok()?;
    let caps = re.captures(ver_str)?;
    Version::parse(&format!("{}+{}", &caps[1], &caps[2])).ok()
}

#[derive(Deserialize)]
struct CargoCrateVersion {
    num: String,
    yanked: bool,
}

#[derive(Deserialize)]
struct CargoCrateResponse {
    versions: Vec<CargoCrateVersion>,
}

fn list_cargo_versions(project: &str) -> Result<Vec<Version>> {
    let crate_name = project.strip_prefix("cargo:")
        .context("Missing cargo: prefix")?;
    let url = format!("https://crates.io/api/v1/crates/{crate_name}");
    
    let response = ureq::get(&url)
        .set("User-Agent", "crush-buckets/0.1.0 (contact@nixpt.dev)")
        .call()
        .with_context(|| format!("Failed to fetch cargo versions from {url}"))?;
        
    let data: CargoCrateResponse = serde_json::from_reader(response.into_reader())
        .with_context(|| format!("Failed to parse crates.io response for {crate_name}"))?;
        
    let mut versions = Vec::new();
    for cv in data.versions {
        if !cv.yanked {
            if let Ok(v) = Version::parse(&cv.num) {
                versions.push(v);
            }
        }
    }
    
    // Sort newest first
    versions.sort_by(|a, b| b.cmp(a));
    versions.dedup();
    
    Ok(versions)
}

/// Fetch the list of available versions for a project from the dist server.
pub fn list_remote_versions(config: &Config, project: &str) -> Result<Vec<Version>> {
    if project.starts_with("path:") {
        return Ok(vec![Version::new(0, 0, 0)]);
    }
    if project.starts_with("cargo:") {
        return list_cargo_versions(project);
    }
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
        if let Some(v) = parse_dist_version(ver_str) {
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
    fn test_parse_dist_version_plain_semver() {
        assert_eq!(parse_dist_version("20.11.0"), Version::parse("20.11.0").ok());
    }

    #[test]
    fn test_parse_dist_version_openssl_letter_suffix() {
        let v = parse_dist_version("1.1.1w").expect("should parse letter-suffixed version");
        assert_eq!((v.major, v.minor, v.patch), (1, 1, 1));
        assert_eq!(v.build.as_str(), "w");
    }

    #[test]
    fn test_parse_dist_version_rejects_garbage() {
        assert!(parse_dist_version("not-a-version").is_none());
    }

    #[test]
    fn test_platform_url() {
        // project-then-platform order, "x86-64" (hyphen) — matches pkgx's
        // real dist-server layout, verified against a live 200 response.
        let config = Config {
            dist_url: "https://dist.pkgx.dev".to_string(),
            cache_dir: std::path::PathBuf::from("/tmp/test"),
            worktree_dir: None,
            platform: "linux/x86-64".to_string(),
        };
        assert_eq!(
            config.versions_url("nodejs.org"),
            "https://dist.pkgx.dev/nodejs.org/linux/x86-64/versions.txt"
        );
        assert_eq!(
            config.bottle_url("nodejs.org", "20.11.0"),
            "https://dist.pkgx.dev/nodejs.org/linux/x86-64/v20.11.0.tar.xz"
        );
    }

    #[test]
    fn test_list_path_versions() {
        let config = Config::default();
        let versions = list_remote_versions(&config, "path:/tmp/does-not-matter").unwrap();
        assert_eq!(versions, vec![Version::parse("0.0.0").unwrap()]);
    }

    #[test]
    fn test_list_cargo_versions() {
        let config = Config::default();
        let versions = list_remote_versions(&config, "cargo:crush-buckets").unwrap();
        assert!(!versions.is_empty());
        let has_v0_1_0 = versions.iter().any(|v| v.major == 0 && v.minor == 1);
        assert!(has_v0_1_0);
    }
}
