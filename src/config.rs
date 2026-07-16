//! Runtime configuration: distribution server URL, cache directory, and
//! platform string, resolved from `BUCKETS_DIST_URL`/`BUCKETS_CACHE_DIR`
//! env vars or sane defaults. Threaded through every stage of the pipeline
//! ([`crate::inventory`], [`crate::install`], [`crate::cellar`]) so none of
//! them hardcode paths or URLs directly.

use std::path::PathBuf;
use std::collections::HashMap;

/// Configuration for buckets.
#[derive(Debug, Clone)]
pub struct Config {
    /// Base distribution URL (e.g. <https://dist.pkgx.dev>).
    pub dist_url: String,
    /// Cache directory for downloaded bottles.
    pub cache_dir: PathBuf,
    /// Explicit override for the parent directory of `buckets
    /// worktree`-created worktrees. `None` (the default) means "create the
    /// worktree as a sibling of the source repo" — resolved per-call in
    /// `worktree::create`, not here, since it depends on which repo is
    /// being operated on. Found live: defaulting this to a fixed location
    /// like `~/.buckets/worktrees/` broke every relative sibling
    /// path-dependency a repo had (`../other-repo`, this workspace's own
    /// convention) — the worktree was no longer sitting next to its
    /// siblings. Sibling-of-the-repo is also the more common git-worktree
    /// convention generally, not just a fix for this workspace.
    pub worktree_dir: Option<PathBuf>,
    /// Platform string (e.g. "linux/x86_64").
    pub platform: String,
    /// Local pantry overrides mapping project names to custom directories/versions.
    pub pantry_overrides: HashMap<String, PantryOverride>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PantryOverride {
    pub path: String,
    pub version: Option<String>,
    pub provides: Option<Vec<String>>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct PantryToml {
    pub overrides: Option<HashMap<String, PantryOverride>>,
}

impl Config {
    /// Create a new config from environment variables or defaults.
    ///
    /// Env vars:
    /// - `BUCKETS_DIST_URL` — override the dist server URL
    /// - `BUCKETS_CACHE_DIR` — override the cache directory
    /// - `BUCKETS_WORKTREE_DIR` — override the worktree parent directory
    ///   (default: create as a sibling of the source repo — see
    ///   `worktree_dir`'s doc comment)
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

        let worktree_dir = std::env::var("BUCKETS_WORKTREE_DIR").ok().map(PathBuf::from);

        let platform = crate::types::platform_prefix();

        let pantry_overrides = load_pantry_overrides();

        Self {
            dist_url,
            cache_dir,
            worktree_dir,
            platform,
            pantry_overrides,
        }
    }

    /// Path to the cached installation directory for a specific project+version.
    pub fn version_dir(&self, project: &str, version: &str) -> PathBuf {
        self.cache_dir.join(sanitize_project_name(project)).join(format!("v{version}"))
    }

    /// Path to the project's cache directory (contains v* subdirs).
    pub fn project_dir(&self, project: &str) -> PathBuf {
        self.cache_dir.join(sanitize_project_name(project))
    }

    /// URL for a bottle tarball. Path order is `{project}/{platform}/v{version}.tar.xz`
    /// (`platform` = "os/arch") — matches pkgx's real dist-server layout
    /// (`{base}/{project}/{os}/{arch}/v{version}.tar.xz`); `self.platform`
    /// already carries the embedded `os/arch` slash. Verified against a
    /// live 200 from dist.pkgx.dev — the earlier `{platform}/{project}`
    /// order 404s.
    pub fn bottle_url(&self, project: &str, version: &str) -> String {
        format!(
            "{}/{}/{}/v{version}.tar.xz",
            self.dist_url, project, self.platform
        )
    }

    /// URL for the versions list of a project. Same path-order fix as
    /// [`bottle_url`](Self::bottle_url).
    pub fn versions_url(&self, project: &str) -> String {
        format!(
            "{}/{}/{}/versions.txt",
            self.dist_url, project, self.platform
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

fn sanitize_project_name(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("cargo:") {
        format!("cargo/{}", rest)
    } else if let Some(rest) = name.strip_prefix("path:") {
        let relative_rest = rest.trim_start_matches('/');
        format!("path/{}", relative_rest)
    } else {
        name.to_string()
    }
}

fn load_pantry_overrides() -> HashMap<String, PantryOverride> {
    let mut overrides = HashMap::new();

    // 1. Load user global config: ~/.config/buckets/pantry.toml
    if let Some(home) = home_dir() {
        let global_path = home.join(".config").join("buckets").join("pantry.toml");
        if global_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&global_path) {
                if let Ok(toml_data) = toml::from_str::<PantryToml>(&content) {
                    if let Some(ovr) = toml_data.overrides {
                        overrides.extend(ovr);
                    }
                }
            }
        }
    }

    // 2. Load local workspace: ./pantry.toml (precedence)
    let local_path = PathBuf::from("pantry.toml");
    if local_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&local_path) {
            if let Ok(toml_data) = toml::from_str::<PantryToml>(&content) {
                if let Some(ovr) = toml_data.overrides {
                    overrides.extend(ovr);
                }
            }
        }
    }

    overrides
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pantry_toml() {
        let toml_content = r#"
[overrides."nodejs.org"]
path = "/workspace/projects/custom-node"
version = "20.99.0"
provides = ["node", "npm"]

[overrides.crush]
path = "/workspace/projects/crush"
"#;
        let data: PantryToml = toml::from_str(toml_content).unwrap();
        let ovr = data.overrides.unwrap();
        assert_eq!(ovr.len(), 2);
        
        let node = ovr.get("nodejs.org").unwrap();
        assert_eq!(node.path, "/workspace/projects/custom-node");
        assert_eq!(node.version.as_deref(), Some("20.99.0"));
        assert_eq!(node.provides.as_ref().unwrap(), &vec!["node".to_string(), "npm".to_string()]);

        let crush = ovr.get("crush").unwrap();
        assert_eq!(crush.path, "/workspace/projects/crush");
        assert!(crush.version.is_none());
        assert!(crush.provides.is_none());
    }
}


