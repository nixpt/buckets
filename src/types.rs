//! Shared types threaded through the whole pipeline: [`PackageReq`] (a
//! parsed, not-yet-resolved spec like `node@^20`) → [`Package`] (pinned to
//! an exact version) → [`Installation`] (pinned + a real cache path) →
//! [`ResolvedEnvironment`] (one or more installations plus their composed
//! env vars — what [`crate::resolve::resolve_multi`] ultimately returns).

use anyhow::Result;
use regex::Regex;
use semver::{Version, VersionReq};
use std::collections::HashMap;
use std::path::PathBuf;

/// A fully pinned package with an exact version.
#[derive(Debug, Clone)]
pub struct Package {
    pub project: String,
    pub version: Version,
}

/// A package request with a semver constraint (e.g. "node@^20").
///
/// Parsed from spec strings like:
/// - `node@20`          → "nodejs.org", constraint "^20"
/// - `python@3.11.0`   → "python.org", constraint "=3.11.0"
/// - `rust@latest`     → "rust-lang.org", constraint "*"
/// - `go@>=1.22`       → "golang.org", constraint ">=1.22"
/// - `node`            → "nodejs.org", constraint "*"
#[derive(Debug, Clone)]
pub struct PackageReq {
    pub project: String,
    pub constraint: VersionReq,
}

impl PackageReq {
    /// Parse a spec string using pkgx-compatible semantics.
    ///
    /// The regex captures: `(<project>)(<constraint>)?`
    /// where constraint can be `@<semver>`, `@latest`, `*`, or absent (→ latest).
    /// Bare numbers like `20` are treated as `^20`.
    pub fn parse(spec: &str) -> Result<Self> {
        let spec = spec.trim();
        let re = Regex::new(r"^(.+?)((?:@[\w~^=<>.*-]+)|\*)?$")
            .expect("invalid PackageReq regex");

        let caps = re.captures(spec)
            .unwrap_or_else(|| re.captures("").unwrap());

        let project_raw = caps.get(1).map(|m| m.as_str()).unwrap_or(spec);
        let constraint_str = caps.get(2).map(|m| m.as_str());

        let constraint = match constraint_str {
            None | Some("") | Some("@latest") | Some("*") => VersionReq::STAR,
            Some(cs) => {
                // Strip leading '@' or '*'
                let cs = cs.strip_prefix('@').unwrap_or(cs);
                let cs = cs.strip_prefix('*').unwrap_or(cs);

                // If it's a bare number (e.g. "20" or "3.11"), treat as caret "^20"
                if cs.chars().all(|c| c.is_ascii_digit() || c == '.')
                    && !cs.contains(|c: char| matches!(c, '^' | '~' | '>' | '<' | '='))
                {
                    VersionReq::parse(&format!("^{cs}"))?
                } else {
                    VersionReq::parse(cs)?
                }
            }
        };

        Ok(Self {
            project: project_raw.to_string(),
            constraint,
        })
    }

    /// Return true if this constraint matches the given version.
    #[allow(dead_code)]
    pub fn matches(&self, version: &Version) -> bool {
        self.constraint.matches(version)
    }
}

/// A cached installation on disk.
#[derive(Debug, Clone)]
pub struct Installation {
    pub pkg: Package,
    pub path: PathBuf,
}

/// The final resolved environment ready for execution.
#[derive(Debug, Clone)]
pub struct ResolvedEnvironment {
    pub installations: Vec<Installation>,
    pub env: HashMap<String, String>,
    #[allow(dead_code)]
    pub entry: Package,
    #[allow(dead_code)]
    pub all_packages: Vec<Package>,
}

/// Host platform detection.
#[derive(Debug, Clone, PartialEq)]
pub enum HostOs {
    Linux,
    #[allow(dead_code)]
    MacOs,
    #[allow(dead_code)]
    Windows,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostArch {
    Amd64,
    #[allow(dead_code)]
    Aarch64,
}

/// Detect the current host OS.
pub fn detect_host_os() -> HostOs {
    #[cfg(target_os = "linux")]
    { HostOs::Linux }
    #[cfg(target_os = "macos")]
    { HostOs::MacOs }
    #[cfg(target_os = "windows")]
    { HostOs::Windows }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    { HostOs::Linux }
}

/// Detect the current host architecture.
pub fn detect_host_arch() -> HostArch {
    #[cfg(target_arch = "x86_64")]
    { HostArch::Amd64 }
    #[cfg(target_arch = "aarch64")]
    { HostArch::Aarch64 }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    { HostArch::Amd64 }
}

/// Bottle URL suffix (arch/os prefix used by pkgx dist).
pub fn platform_prefix() -> String {
    let os = match detect_host_os() {
        HostOs::Linux => "linux",
        HostOs::MacOs => "darwin",
        HostOs::Windows => "windows",
    };
    let arch = match detect_host_arch() {
        HostArch::Amd64 => "x86_64",
        HostArch::Aarch64 => "aarch64",
    };
    format!("{os}/{arch}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_node_at_20() {
        let req = PackageReq::parse("node@20").unwrap();
        assert_eq!(req.project, "node");
        assert!(req.matches(&Version::parse("20.11.0").unwrap()));
        assert!(!req.matches(&Version::parse("21.0.0").unwrap()));
    }

    #[test]
    fn test_parse_bare_name_is_star() {
        let req = PackageReq::parse("node").unwrap();
        assert_eq!(req.project, "node");
        assert_eq!(req.constraint, VersionReq::STAR);
    }

    #[test]
    fn test_parse_latest() {
        let req = PackageReq::parse("rust@latest").unwrap();
        assert_eq!(req.project, "rust");
        assert_eq!(req.constraint, VersionReq::STAR);
    }

    #[test]
    fn test_parse_exact() {
        let req = PackageReq::parse("python@=3.11.0").unwrap();
        assert_eq!(req.project, "python");
        assert!(req.matches(&Version::parse("3.11.0").unwrap()));
        assert!(!req.matches(&Version::parse("3.11.1").unwrap()));
    }

    #[test]
    fn test_parse_caret() {
        let req = PackageReq::parse("go@^1.22").unwrap();
        assert_eq!(req.project, "go");
        assert!(req.matches(&Version::parse("1.22.0").unwrap()));
        assert!(req.matches(&Version::parse("1.23.0").unwrap()));
        assert!(!req.matches(&Version::parse("2.0.0").unwrap()));
    }

    #[test]
    fn test_parse_greater_equal() {
        let req = PackageReq::parse("node@>=18").unwrap();
        assert!(req.matches(&Version::parse("18.0.0").unwrap()));
        assert!(req.matches(&Version::parse("20.0.0").unwrap()));
        assert!(!req.matches(&Version::parse("17.0.0").unwrap()));
    }

    #[test]
    fn test_parse_tilde() {
        let req = PackageReq::parse("rust@~1.70").unwrap();
        assert!(req.matches(&Version::parse("1.70.0").unwrap()));
        assert!(req.matches(&Version::parse("1.70.5").unwrap()));
        assert!(!req.matches(&Version::parse("1.71.0").unwrap()));
    }

    #[test]
    fn test_parse_tolerates_v_prefix() {
        let _result = PackageReq::parse("node@v20");
    }

    #[test]
    fn test_detect_platform() {
        let p = platform_prefix();
        assert!(p.contains('/'));
        let parts: Vec<&str> = p.split('/').collect();
        assert_eq!(parts.len(), 2);
        assert!(!parts[0].is_empty());
        assert!(!parts[1].is_empty());
    }
}
