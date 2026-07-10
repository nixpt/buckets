//! Per-origin storage isolation for `buckets site` — reviving the intent
//! behind exosphere-apps' `exo-site-capsulizer` (storage-capsule/
//! net-capsule/worker-capsule), which was found ~95% unenforced scaffolding
//! (`check_request()` never called, storage VFS dead, workers never
//! started — see session 298/EXO-DC8) and replaced in `surfer-browser`
//! with a shim explicitly documented as "not an enforcement layer."
//!
//! The real enforcement lives in [`crate::sandbox`]'s `bwrap` mount
//! namespace, not here — this module only decides WHERE a site's storage
//! lives: a persistent, host-keyed directory (real browsing-profile
//! semantics) or a removed-on-exit tempdir for `--incognito`. `buckets
//! site` binds that directory as the sandbox's one read-write path.

use crate::config::Config;
use anyhow::{Context, Result};
use std::path::PathBuf;

/// A resolved per-origin storage location for `buckets site`.
pub struct SiteTarget {
    pub host: String,
    pub storage_dir: PathBuf,
    is_temp: bool,
}

impl SiteTarget {
    /// Resolve `url`'s host and its storage directory. `incognito` = a
    /// fresh tempdir removed on drop; otherwise a persistent, host-keyed
    /// directory under `config.cache_dir/sites/<host>/` that survives
    /// across runs (cookies/localStorage/cache persisting is the point).
    pub fn resolve(url: &str, config: &Config, incognito: bool) -> Result<Self> {
        let parsed = url::Url::parse(url).with_context(|| format!("Invalid URL: {url}"))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("No host in URL: {url}"))?
            .to_string();

        let (storage_dir, is_temp) = if incognito {
            let dir = tempfile::Builder::new()
                .prefix(&format!("buckets-site-{host}-"))
                .tempdir()
                .context("Failed to create incognito storage dir")?
                .keep();
            (dir, true)
        } else {
            let dir = config.cache_dir.join("sites").join(&host);
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("Failed to create {}", dir.display()))?;
            (dir, false)
        };

        Ok(Self { host, storage_dir, is_temp })
    }
}

impl Drop for SiteTarget {
    fn drop(&mut self) {
        if self.is_temp {
            let _ = std::fs::remove_dir_all(&self.storage_dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_config(cache_dir: PathBuf) -> Config {
        Config {
            dist_url: "https://dist.pkgx.dev".to_string(),
            cache_dir,
            worktree_dir: None,
            platform: "linux/x86-64".to_string(),
        }
    }

    #[test]
    fn resolve_extracts_host() {
        let cache = tempfile::tempdir().unwrap();
        let target = SiteTarget::resolve("https://example.com/page", &fake_config(cache.path().to_path_buf()), false).unwrap();
        assert_eq!(target.host, "example.com");
    }

    #[test]
    fn resolve_extracts_host_with_port() {
        let cache = tempfile::tempdir().unwrap();
        let target = SiteTarget::resolve("http://localhost:8080/", &fake_config(cache.path().to_path_buf()), false).unwrap();
        assert_eq!(target.host, "localhost");
    }

    #[test]
    fn resolve_extracts_ipv6_host() {
        let cache = tempfile::tempdir().unwrap();
        let target = SiteTarget::resolve("http://[::1]:8080/", &fake_config(cache.path().to_path_buf()), false).unwrap();
        assert_eq!(target.host, "[::1]");
    }

    #[test]
    fn resolve_rejects_invalid_url() {
        let cache = tempfile::tempdir().unwrap();
        let result = SiteTarget::resolve("not a url", &fake_config(cache.path().to_path_buf()), false);
        assert!(result.is_err());
    }

    #[test]
    fn persistent_storage_dir_is_host_keyed_under_cache_dir() {
        let cache = tempfile::tempdir().unwrap();
        let target = SiteTarget::resolve("https://example.com", &fake_config(cache.path().to_path_buf()), false).unwrap();
        assert_eq!(target.storage_dir, cache.path().join("sites").join("example.com"));
        assert!(target.storage_dir.exists());
    }

    #[test]
    fn persistent_storage_dir_survives_drop() {
        let cache = tempfile::tempdir().unwrap();
        let dir = {
            let target = SiteTarget::resolve("https://example.com", &fake_config(cache.path().to_path_buf()), false).unwrap();
            target.storage_dir.clone()
        };
        assert!(dir.exists());
    }

    #[test]
    fn incognito_storage_dir_removed_on_drop() {
        let cache = tempfile::tempdir().unwrap();
        let dir = {
            let target = SiteTarget::resolve("https://example.com", &fake_config(cache.path().to_path_buf()), true).unwrap();
            assert!(target.storage_dir.exists());
            target.storage_dir.clone()
        };
        assert!(!dir.exists());
    }
}
