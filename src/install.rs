use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::fs::{self};

use crate::config::Config;
use crate::types::{Installation, Package};

/// Install a package: download, extract, and cache it.
///
/// Returns the `Installation` pointing to the cached directory.
/// Uses a temp file + atomic rename pattern for crash safety.
pub fn install(config: &Config, pkg: &Package) -> Result<Installation> {
    let version_str = pkg.version.to_string();
    let project_dir = config.project_dir(&pkg.project);
    let target_dir = config.version_dir(&pkg.project, &version_str);

    // Fast path: already installed
    if target_dir.join("bin").exists() {
        return Ok(Installation {
            pkg: pkg.clone(),
            path: target_dir,
        });
    }

    // Ensure project directory exists
    fs::create_dir_all(&project_dir)
        .with_context(|| format!("Failed to create cache dir: {}", project_dir.display()))?;

    // Download to a temp file in the same filesystem (for atomic rename)
    let url = config.bottle_url(&pkg.project, &version_str);
    eprintln!("↓ fetching {url}");

    let response = ureq::get(&url)
        .call()
        .with_context(|| format!("Failed to download bottle from {url}"))?;

    // Stream decompression: XZ → tar → extract to temp dir
    let tempdir = tempfile::tempdir_in(&project_dir)
        .context("Failed to create temp directory for extraction")?;

    // Read the response body as raw bytes, decompress xz, extract tar
    let reader = response.into_reader();
    let xz_decoder = xz2::read::XzDecoder::new(reader);
    let mut archive = tar::Archive::new(xz_decoder);

    archive.unpack(tempdir.path())
        .with_context(|| format!("Failed to extract bottle for {url}"))?;

    // The tarball contains {project}/v{version}/... (pkgx format)
    // or sometimes just flat content. Discover the actual extraction root.
    let extracted_root = find_extraction_root(tempdir.path(), &pkg.project, &version_str);

    // Atomic rename the extracted content to the target cache directory
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir)?;
    }

    // If extraction root is nested (pkgx format), move its contents
    if extracted_root != tempdir.path() {
        rename_contents(&extracted_root, &target_dir)?;
    } else {
        fs::rename(&extracted_root, &target_dir)?;
    }

    eprintln!("✓ cached {} v{}", pkg.project, version_str);

    Ok(Installation {
        pkg: pkg.clone(),
        path: target_dir,
    })
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
    // Flat extraction — just return the temp dir
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
