//! Runtime configuration: distribution server URL, cache directory, and
//! platform string, resolved from `BUCKETS_DIST_URL`/`BUCKETS_CACHE_DIR`
//! env vars or sane defaults. Threaded through every stage of the pipeline
//! ([`crate::inventory`], [`crate::install`], [`crate::cellar`]) so none of
//! them hardcode paths or URLs directly.

use std::path::PathBuf;

/// Configuration for buckets.
#[derive(Debug, Clone)]
pub struct Config {
    /// Base distribution URL (e.g. <https://dist.pkgx.dev>).
    pub dist_url: String,
    /// Cache directory for downloaded bottles.
    pub cache_dir: PathBuf,
    /// Platform string (e.g. "linux/x86_64").
    pub platform: String,
}

impl Config {
    /// Create a new config from environment variables or defaults.
    ///
    /// Env vars:
    /// - `BUCKETS_DIST_URL` ��� override the dist server URL
    /// - `BUCKETS_CACHE_DIR` — override the cache directory
    pub fn new() -> Self {
        let dist_url = std::env::var("BUCKETS_DIST_URL")
            .unwrap_or_else(|_| "https://dist.pkgx.dev".to_string());

        let cache_dir = match std::env::var("BUCKETS_CACHE_DIR") {
            Ok(dir) => PathBuf::from(dir),
            Err(_) => {
                // Default to ~/.buckets/cache/
                dirs_or_default()
            }
        };

        let platform = crate::types::platform_prefix();

        Self {
            dist_url,
            cache_dir,
            platform,
        }
    }

    /// Path to the cached installation directory for a specific project+version.
    pub fn version_dir(&self, project: &str, version: &str) -> PathBuf {
        self.cache_dir.join(project).join(format!("v{version}"))
    }

    /// Path to the project's cache directory (contains v* subdirs).
    pub fn project_dir(&self, project: &str) -> PathBuf {
        self.cache_dir.join(project)
    }

    /// URL for a bottle tarball.
    pub fn bottle_url(&self, project: &str, version: &str) -> String {
        format!(
            "{}/{}/{}/v{version}.tar.xz",
            self.dist_url, self.platform, project
        )
    }

    /// URL for the versions list of a project.
    pub fn versions_url(&self, project: &str) -> String {
        format!(
            "{}/{}/{}/versions.txt",
            self.dist_url, self.platform, project
        )
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

fn dirs_or_default() -> PathBuf {
    // Try XDG_CACHE_HOME or ~/.cache/buckets/
    if let Ok(val) = std::env::var("XDG_CACHE_HOME") {
        let p = PathBuf::from(val).join("buckets");
        if p.exists() || std::fs::create_dir_all(&p).is_ok() {
            return p;
        }
    }
    // Fallback: ~/.buckets/
    if let Some(home) = home_dir() {
        let p = home.join(".buckets");
        if p.exists() || std::fs::create_dir_all(&p).is_ok() {
            return p;
        }
    }
    // Last resort: local ./buckets-cache/
    let p = PathBuf::from("buckets-cache");
    let _ = std::fs::create_dir_all(&p);
    p
}

#[cfg(unix)]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(windows)]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}
