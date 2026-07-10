use anyhow::{Result};
use semver::{Version, VersionReq};
use std::path::PathBuf;
use std::collections::HashMap;

/// A fully pinned package with an exact version.
#[derive(Debug, Clone)]
pub struct Package {
    pub project: String,
    pub version: Version,
}

/// A package request with a semver constraint (e.g. "node@^20").
#[derive(Debug, Clone)]
pub struct PackageReq {
    pub project: String,
    pub constraint: VersionReq,
}

impl PackageReq {
    /// Parse a spec string in the form `<project>@<constraint>`.
    ///
    /// Supports:
    /// - `node@20`          → project="nodejs.org", constraint="^20"
    /// - `python@3.11.0`   → project="python.org", constraint="=3.11.0"
    /// - `rust@latest`     → project="rust-lang.org", constraint="*"
    /// - `nodejs.org@^20`  → project="nodejs.org", constraint="^20"
    /// - `node`            → project="nodejs.org", constraint="*"
    pub fn parse(spec: &str) -> Result<Self> {
        let spec = spec.trim();

        // Split on the last '@' to handle project names containing '@'
        // (though uncommon, we're defensive)
        let (project_raw, version_part) = match spec.rsplit_once('@') {
            Some((proj, ver)) if !proj.is_empty() => (proj, Some(ver)),
            _ => (spec, None),
        };

        let constraint = match version_part {
            Some("latest") | None => VersionReq::STAR,
            Some(v) => {
                // If it looks like a bare number (e.g. "20"), treat as caret "^20"
                if v.chars().all(|c| c.is_ascii_digit() || c == '.') && !v.contains(['^', '~', '>', '<', '=', '*']) {
                    VersionReq::parse(&format!("^{v}"))?
                } else {
                    VersionReq::parse(v)?
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
    #[allow(dead_code)]
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
    { HostOs::Linux } // fallback
}

/// Detect the current host architecture.
pub fn detect_host_arch() -> HostArch {
    #[cfg(target_arch = "x86_64")]
    { HostArch::Amd64 }
    #[cfg(target_arch = "aarch64")]
    { HostArch::Aarch64 }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    { HostArch::Amd64 } // fallback
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
