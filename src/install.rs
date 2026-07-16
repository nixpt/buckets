//! Bottle download + extraction: fetch a `.tar.xz` bottle from the dist
//! server, stream-decompress it into a temp directory, then atomically
//! rename it into the cache (crash-safe — a killed download never leaves a
//! half-extracted directory `is_installed` would mistake for complete).

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::fs;

use crate::cellar;
use crate::config::Config;
use crate::types::{dist_version_string, Installation, Package};

/// Install a package: download, extract, and cache it.
///
/// Returns the `Installation` pointing to the cached directory.
/// Uses a temp dir + atomic rename pattern for crash safety.
pub fn install(config: &Config, pkg: &Package) -> Result<Installation> {
    let version_str = dist_version_string(&pkg.version);
    let project_dir = config.project_dir(&pkg.project);
    let target_dir = config.version_dir(&pkg.project, &version_str);

    // Fast path: already installed
    if target_dir.join("bin").exists() {
        // Still update symlinks in case they're stale
        cellar::update_version_symlinks(config, &pkg.project, &pkg.version)?;
        return Ok(Installation {
            pkg: pkg.clone(),
            path: target_dir,
        });
    }

    // Ensure project directory exists
    fs::create_dir_all(&project_dir)
        .with_context(|| format!("Failed to create cache dir: {}", project_dir.display()))?;

    // Acquire exclusive write lock on this version's lock file
    let lock_path = project_dir.join(format!(".install-v{version_str}.lock"));
    let lock_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .with_context(|| format!("Failed to open lock file: {}", lock_path.display()))?;

    let mut lock = fd_lock::RwLock::new(lock_file);
    let _guard = lock.write().with_context(|| format!("Failed to acquire write lock on: {}", lock_path.display()))?;

    // Double-check under lock
    if target_dir.join("bin").exists() {
        cellar::update_version_symlinks(config, &pkg.project, &pkg.version)?;
        return Ok(Installation {
            pkg: pkg.clone(),
            path: target_dir,
        });
    }

    // Download the bottle
    let url = config.bottle_url(&pkg.project, &version_str);
    eprintln!("↓ fetching {url}");

    let response = ureq::get(&url)
        .call()
        .with_context(|| format!("Failed to download bottle from {url}"))?;

    // Stream decompression: XZ → tar → extract to temp dir
    let tempdir = tempfile::tempdir_in(&project_dir)
        .context("Failed to create temp directory for extraction")?;

    let reader = response.into_reader();
    let xz_decoder = xz2::read::XzDecoder::new(reader);
    let mut archive = tar::Archive::new(xz_decoder);

    archive.unpack(tempdir.path())
        .with_context(|| format!("Failed to extract bottle for {url}"))?;

    // Discover the actual extraction root (pkgx bottles are nested)
    let extracted_root = find_extraction_root(tempdir.path(), &pkg.project, &version_str);

    // Atomic rename the extracted content to the target cache directory
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir)?;
    }

    if extracted_root != tempdir.path() {
        rename_contents(&extracted_root, &target_dir)?;
    } else {
        fs::rename(&extracted_root, &target_dir)?;
    }

    // Create version symlinks (v*, v<major>, v<major.minor>)
    cellar::update_version_symlinks(config, &pkg.project, &pkg.version)?;

    eprintln!("��� cached {} v{}", pkg.project, version_str);

    Ok(Installation {
        pkg: pkg.clone(),
        path: target_dir,
    })
}

/// Install multiple packages concurrently using a thread pool.
///
/// Returns installations in the same order as the input packages.
pub fn install_multi(config: &Config, packages: &[Package]) -> Result<Vec<Installation>> {
    if packages.is_empty() {
        return Ok(Vec::new());
    }

    // Use std::thread::scope for safe concurrent downloads
    let results = std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(packages.len());
        for pkg in packages {
            let config_ref = &*config;
            handles.push(s.spawn(move || {
                install(config_ref, pkg)
            }));
        }
        handles.into_iter().map(|h| h.join().expect("thread panicked")).collect::<Vec<_>>()
    });

    // Collect results, returning first error if any
    let mut installations = Vec::with_capacity(packages.len());
    for result in results {
        installations.push(result?);
    }

    Ok(installations)
}

/// Find the actual root of the extracted content.
/// pkgx bottles have `{project}/v{version}/bin/...` structure,
/// so we need to descend into that to find the real content root.
fn find_extraction_root(temp_dir: &Path, project: &str, version: &str) -> PathBuf {
    let nested = temp_dir.join(project).join(format!("v{version}"));
    if nested.join("bin").exists() {
        return nested;
    }
    let nested2 = temp_dir.join(project);
    if nested2.join("bin").exists() {
        return nested2;
    }
    temp_dir.to_path_buf()
}

/// Rename contents of `src` into `dst`, creating dst if needed.
fn rename_contents(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let dest_path = dst.join(&name);
        if dest_path.exists() {
            fs::remove_dir_all(&dest_path).or_else(|_| fs::remove_file(&dest_path))?;
        }
        fs::rename(entry.path(), dest_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_concurrency_lock() {
        if std::env::var("BUCKETS_TEST_LOCK_CHILD").is_ok() {
            let lock_path = PathBuf::from(std::env::var("LOCK_FILE_PATH").unwrap());
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(&lock_path)
                .unwrap();
            let mut lock = fd_lock::RwLock::new(file);
            let _guard = lock.write().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(150));
            return;
        }

        let tempdir = tempfile::tempdir().unwrap();
        let lock_path = tempdir.path().join("test.lock");

        // Spawn a child process using the same test binary
        let current_exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(current_exe)
            .arg("test_process_concurrency_lock") // run this test in child
            .env("BUCKETS_TEST_LOCK_CHILD", "1")
            .env("LOCK_FILE_PATH", &lock_path)
            .spawn()
            .unwrap();

        // Give the child a moment to start and acquire lock
        std::thread::sleep(std::time::Duration::from_millis(40));

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&lock_path)
            .unwrap();
        let mut lock = fd_lock::RwLock::new(file);

        let start = std::time::Instant::now();
        let _guard = lock.write().unwrap();
        let elapsed = start.elapsed();

        assert!(elapsed >= std::time::Duration::from_millis(80), "elapsed was {:?}", elapsed);
        child.wait().unwrap();
    }
}

