//! Bottle download + extraction: fetch a `.tar.xz` bottle from the dist
//! server, stream-decompress it into a temp directory, then atomically
//! rename it into the cache (crash-safe — a killed download never leaves a
//! half-extracted directory `is_installed` would mistake for complete).

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::fs;

use crate::cellar;
use crate::config::Config;
use crate::types::{dist_version_string, Installation, Package};

fn read_package_json_bin(source_dir: &Path) -> Option<Vec<(String, String)>> {
    let pkg_json = source_dir.join("package.json");
    let text = std::fs::read_to_string(pkg_json).ok()?;
    let val: serde_json::Value = serde_json::from_str(&text).ok()?;
    let bin = val.get("bin")?;
    if let Some(s) = bin.as_str() {
        let name = val.get("name")?.as_str()?.to_string();
        return Some(vec![(name, s.to_string())]);
    }
    if let Some(obj) = bin.as_object() {
        let mut res = Vec::new();
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                res.push((k.clone(), s.to_string()));
            }
        }
        return Some(res);
    }
    None
}

fn install_path(config: &Config, pkg: &Package) -> Result<Installation> {
    let source_path_str = pkg.project.strip_prefix("path:")
        .context("Missing path: prefix")?;
    let source_dir = PathBuf::from(source_path_str).canonicalize()
        .with_context(|| format!("Local path does not exist: {source_path_str}"))?;

    let version_str = dist_version_string(&pkg.version);
    let target_dir = config.version_dir(&pkg.project, &version_str);

    eprintln!("▶ building local source path: {}", source_dir.display());

    let plan = crate::project::detect(&source_dir)
        .with_context(|| format!("Failed to detect build system at {}", source_dir.display()))?;

    // Resolve build toolchain (sandboxed build needs its toolchain resolved first)
    let resolved = crate::resolve::resolve_multi(&plan.toolchain_specs, config, &crate::index::Index::builtin())
        .with_context(|| format!("Failed to resolve toolchain: {:?}", plan.toolchain_specs))?;

    // Create the target cellar directory structures
    let bin_dir = target_dir.join("bin");
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir)?;
    }
    fs::create_dir_all(&bin_dir)?;

    if source_dir.join("Cargo.toml").exists() {
        eprintln!("↓ compiling cargo project via path...");
        let mut cmd = std::process::Command::new("cargo");
        cmd.arg("install")
            .arg("--path")
            .arg(&source_dir)
            .arg("--root")
            .arg(&target_dir)
            .arg("--force");
        for (key, value) in &resolved.env {
            cmd.env(key, value);
        }
        let status = cmd.status()?;
        if !status.success() {
            anyhow::bail!("cargo install failed for {}", source_dir.display());
        }
    } else if source_dir.join("go.mod").exists() {
        eprintln!("↓ compiling go project via path...");
        let bin_name = source_dir.file_name().unwrap().to_string_lossy().to_string();
        let mut cmd = std::process::Command::new("go");
        cmd.arg("build")
            .arg("-o")
            .arg(bin_dir.join(&bin_name))
            .arg(".")
            .current_dir(&source_dir);
        for (key, value) in &resolved.env {
            cmd.env(key, value);
        }
        let status = cmd.status()?;
        if !status.success() {
            anyhow::bail!("go build failed for {}", source_dir.display());
        }
    } else {
        // Run standard build command if present
        let (program, args) = plan.build_cmd.split_first().context("empty build command")?;
        eprintln!("↓ running build command: {}", plan.build_cmd.join(" "));
        let mut cmd = std::process::Command::new(program);
        cmd.args(args).current_dir(&source_dir);
        for (key, value) in &resolved.env {
            cmd.env(key, value);
        }
        let status = cmd.status()?;
        if !status.success() {
            anyhow::bail!("Build command failed for {}", source_dir.display());
        }

        // Symlink or wrap binaries/scripts
        if source_dir.join("bin").exists() {
            for entry in fs::read_dir(source_dir.join("bin"))? {
                let entry = entry?;
                let name = entry.file_name();
                let dest = bin_dir.join(name);
                #[cfg(unix)]
                std::os::unix::fs::symlink(entry.path(), dest)?;
            }
        } else if source_dir.join("package.json").exists() {
            if let Some(bins) = read_package_json_bin(&source_dir) {
                for (name, rel_path) in bins {
                    let src_file = source_dir.join(rel_path);
                    let dest = bin_dir.join(name);
                    let wrapper_content = format!(
                        "#!/usr/bin/env node\nrequire('{}');\n",
                        src_file.to_string_lossy()
                    );
                    fs::write(&dest, wrapper_content)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))?;
                    }
                }
            }
        }
    }

    cellar::update_version_symlinks(config, &pkg.project, &pkg.version)?;
    eprintln!("✓ successfully built local path {} → v{}", pkg.project, version_str);

    Ok(Installation {
        pkg: pkg.clone(),
        path: target_dir,
    })
}

fn install_cargo(config: &Config, pkg: &Package) -> Result<Installation> {
    let version_str = dist_version_string(&pkg.version);
    let target_dir = config.version_dir(&pkg.project, &version_str);
    let crate_name = pkg.project.strip_prefix("cargo:")
        .context("Missing cargo: prefix")?;

    eprintln!("↓ compiling cargo package {crate_name} v{version_str}...");

    let status = std::process::Command::new("cargo")
        .arg("install")
        .arg(crate_name)
        .arg("--version")
        .arg(&version_str)
        .arg("--root")
        .arg(&target_dir)
        .status()
        .context("Failed to run 'cargo install'. Is cargo installed on the host?")?;

    if !status.success() {
        bail!("'cargo install' failed for {crate_name} v{version_str}");
    }

    // Create version symlinks (v*, v<major>, v<major.minor>)
    cellar::update_version_symlinks(config, &pkg.project, &pkg.version)?;

    eprintln!("✓ cached cargo package {} v{}", pkg.project, version_str);

    Ok(Installation {
        pkg: pkg.clone(),
        path: target_dir,
    })
}

/// Install a package: download, extract, and cache it.
///
/// Returns the `Installation` pointing to the cached directory.
/// Uses a temp dir + atomic rename pattern for crash safety.
pub fn install(config: &Config, pkg: &Package) -> Result<Installation> {
    if pkg.project.starts_with("path:") {
        return install_path(config, pkg);
    }
    if pkg.project.starts_with("cargo:") {
        return install_cargo(config, pkg);
    }
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
    fn test_install_path_cargo() {
        let tempdir = tempfile::tempdir().unwrap();
        let src_dir = tempdir.path().join("my-test-bin");
        fs::create_dir_all(src_dir.join("src")).unwrap();
        
        // Write Cargo.toml
        fs::write(
            src_dir.join("Cargo.toml"),
            r#"[package]
name = "my-test-bin"
version = "0.1.0"
edition = "2021"
"#
        ).unwrap();
        
        // Write main.rs
        fs::write(
            src_dir.join("src").join("main.rs"),
            r#"fn main() {
    println!("hello from local path!");
}
"#
        ).unwrap();
        
        let config = Config::default();
        let pkg = Package {
            project: format!("path:{}", src_dir.to_string_lossy()),
            version: semver::Version::new(0, 0, 0),
        };
        
        let inst = install(&config, &pkg).unwrap();
        let bin_path = inst.path.join("bin").join("my-test-bin");
        assert!(bin_path.exists());
        
        let output = std::process::Command::new(bin_path)
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(stdout.trim(), "hello from local path!");
    }

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
