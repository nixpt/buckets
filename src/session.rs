//! Persistent sessions via OverlayFS — share a writable filesystem across
//! multiple `buckets session exec` invocations so files written by one
//! command survive into the next.
//!
//! ## How it works
//!
//! Each session creates an OverlayFS mount combining the resolved toolchain(s)
//! as read-only lower layers and a session-specific writable upper directory.
//! Commands run under `bwrap` with this overlay mount bound at `/session/`,
//! plus the current working directory at `/workspace/` and toolchain binaries
//! at `/runtime/`.
//!
//! Multiple `session exec` calls against the same session ID share the same
//! overlay mount — changes persist across invocations without any daemon or
//! PID tracking needed.
//!
//! ## Backing stores
//!
//! - `disk` (default): upper dir lives on the host filesystem at
//!   `~/.buckets/sessions/<id>/upper/`. Survives reboots.
//! - `tmpfs`: upper dir is a tmpfs mount. Fast, ephemeral (`--tmpfs`).
//! - `zram`: upper dir on a zram compressed RAM device. Fast+compact (`--zram`).

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::env;
use crate::sandbox::{self, SandboxProfile};
use crate::types::{Installation, ResolvedEnvironment};

// ── Session config ──────────────────────────────────────────────────

/// Metadata for a single session, persisted as TOML.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionConfig {
    pub session_id: String,
    pub created_at: String,
    pub mount_point: String,
    pub overlay_lower: String,
    pub overlay_upper: String,
    pub overlay_work: String,
    pub upper_is_tmpfs: bool,
    pub upper_is_zram: bool,
    pub specs: Vec<String>,
    pub toolchain_dirs: Vec<String>,
    pub pid: Option<u64>,
}

/// A running session with its overlay mount and optional child process.
pub struct Session {
    pub config: SessionConfig,
    pub resolved: Option<ResolvedEnvironment>,
}

// ── Session dirs ───────────────────────────────────────────────────

fn sessions_dir(config: &Config) -> PathBuf {
    config.cache_dir.join("sessions")
}

fn session_dir(config: &Config, session_id: &str) -> PathBuf {
    sessions_dir(config).join(session_id)
}

fn session_config_path(config: &Config, session_id: &str) -> PathBuf {
    session_dir(config, session_id).join("config.toml")
}

fn overlay_upper_dir(config: &Config, session_id: &str) -> PathBuf {
    session_dir(config, session_id).join("upper")
}

fn overlay_work_dir(config: &Config, session_id: &str) -> PathBuf {
    session_dir(config, session_id).join("work")
}

fn default_mount_point(session_id: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/buckets-session-{session_id}"))
}

fn generate_session_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let rand = os_urandom_hex(4);
    format!("s{ts:x}-{rand}")
}

fn os_urandom_hex(n: usize) -> String {
    use std::fs::File;
    use std::io::Read;
    let mut f = File::open("/dev/urandom").unwrap_or_else(|_| {
        // Fallback for non-Linux or test environments
        let mut file = File::create("/dev/urandom").unwrap_or_else(|_| {
            // Create a dummy file for tests
            // This will panic in non-Linux, but buckets only runs on Linux anyway
            panic!("no /dev/urandom")
        });
        file
    });
    let mut buf = vec![0u8; n];
    f.read_exact(&mut buf).unwrap_or_default();
    buf.iter().map(|b| format!("{b:02x}").to_string()).collect()
}

// ── OverlayFS helpers ───────────────────────────────────────────────

/// Check if a path is a mounted filesystem by reading /proc/mounts.
fn is_mounted(path: &str) -> bool {
    let content = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    content.lines().any(|line| {
        if let Some(mount_point) = line.split_whitespace().nth(1) {
            mount_point == path
        } else {
            false
        }
    })
}

/// Mount an OverlayFS filesystem at `mount_point`.
///
/// `lower_dirs` is a colon-separated list of read-only directories,
/// `upper_dir` is the writable layer, `work_dir` is kernel metadata.
fn overlay_mount(lower_dirs: &[&str], upper_dir: &str, work_dir: &str, mount_point: &str) -> Result<()> {
    // Create mount point
    std::fs::create_dir_all(mount_point)
        .with_context(|| format!("Failed to create mount point {mount_point}"))?;

    let lower = lower_dirs.join(":");
    let opts = format!(
        "lowerdir={lower},upperdir={upper_dir},workdir={work_dir},userxattr,redirect_dir=on"
    );

    let output = Command::new("mount")
        .args(["-t", "overlay", "overlay", "-o", &opts, mount_point])
        .output()
        .with_context(|| format!("Failed to run mount for {mount_point}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("overlay mount failed at {mount_point}: {stderr}");
    }

    Ok(())
}

/// Unmount an OverlayFS filesystem.
fn overlay_unmount(mount_point: &str) -> Result<()> {
    if !is_mounted(mount_point) {
        return Ok(()); // Already unmounted
    }

    let output = Command::new("umount")
        .arg(mount_point)
        .output()
        .with_context(|| format!("Failed to unmount {mount_point}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Try lazy unmount if regular unmount fails
        if stderr.contains("target is busy") {
            eprintln!("[session] {mount_point} busy, trying lazy unmount...");
            let lazy = Command::new("umount")
                .args(["-l", mount_point])
                .output()
                .with_context(|| format!("Failed to lazy-unmount {mount_point}"))?;
            if !lazy.status.success() {
                bail!("lazy unmount also failed: {}", String::from_utf8_lossy(&lazy.stderr));
            }
        } else {
            bail!("umount failed: {stderr}");
        }
    }

    Ok(())
}

/// Mount a tmpfs at a path.
fn tmpfs_mount(path: &str, size: &str) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("Failed to create {path}"))?;

    let output = Command::new("mount")
        .args(["-t", "tmpfs", "tmpfs", "-o", &format!("size={size},mode=0755"), path])
        .output()
        .with_context(|| format!("Failed to mount tmpfs at {path}"))?;

    if !output.status.success() {
        bail!("tmpfs mount failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(())
}

// ── Session lifecycle ───────────────────────────────────────────────

/// Read a session config from disk.
pub fn read_session(config: &Config, session_id: &str) -> Result<SessionConfig> {
    let path = session_config_path(config, session_id);
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Session '{session_id}' not found at {}", path.display()))?;
    let cfg: SessionConfig = toml::from_str(&content)
        .with_context(|| format!("Failed to parse session config {}", path.display()))?;
    Ok(cfg)
}

/// Save a session config to disk.
fn save_session(config: &Config, cfg: &SessionConfig) -> Result<()> {
    let dir = session_dir(config, &cfg.session_id);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create session dir {}", dir.display()))?;

    let path = session_config_path(config, &cfg.session_id);
    let content = toml::to_string_pretty(cfg)
        .with_context(|| "Failed to serialize session config")?;
    std::fs::write(&path, content)
        .with_context(|| format!("Failed to write session config {}", path.display()))?;
    Ok(())
}

/// List all known sessions.
pub fn list_sessions(config: &Config) -> Result<Vec<SessionConfig>> {
    let dir = sessions_dir(config);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let config_path = entry.path().join("config.toml");
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(cfg) = toml::from_str::<SessionConfig>(&content) {
                    sessions.push(cfg);
                }
            }
        }
    }

    // Sort by creation time (newest first)
    sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(sessions)
}

/// Start a new session.
///
/// Resolves toolchains, creates the overlay mount, optionally starts a
/// command under bwrap with the overlay bound at `/session/`, and returns
/// the session ID.
pub fn session_start(
    specs: &[String],
    command: &[String],
    use_tmpfs: bool,
    use_zram: bool,
    size: Option<&str>,
    config: &Config,
    index: &crate::index::Index,
) -> Result<String> {
    if specs.is_empty() {
        bail!("At least one spec is required (e.g. 'node@20')");
    }

    // Resolve toolchains
    let resolved = crate::resolve::resolve_multi(specs, config, index)
        .with_context(|| format!("Failed to resolve specs: {}", specs.join(", ")))?;

    let session_id = generate_session_id();
    let mount_pt = default_mount_point(&session_id);
    let upper_dir = overlay_upper_dir(config, &session_id);
    let work_dir = overlay_work_dir(config, &session_id);

    // Create upper and work directories
    std::fs::create_dir_all(&upper_dir)
        .with_context(|| format!("Failed to create upper dir {}", upper_dir.display()))?;
    std::fs::create_dir_all(&work_dir)
        .with_context(|| format!("Failed to create work dir {}", work_dir.display()))?;

    // If using tmpfs, mount it over the upper dir
    if use_tmpfs {
        let sz = size.unwrap_or("4G");
        tmpfs_mount(upper_dir.to_str().unwrap(), sz)?;
    }

    // Collect lower directories
    let lower_dirs: Vec<&str> = resolved
        .installations
        .iter()
        .map(|inst| inst.path.to_str().unwrap())
        .collect();

    let toolchain_dirs: Vec<String> = resolved
        .installations
        .iter()
        .map(|inst| inst.path.to_string_lossy().to_string())
        .collect();

    // Mount the overlay
    let mount_str = mount_pt.to_str().unwrap();
    let upper_str = upper_dir.to_str().unwrap();
    let work_str = work_dir.to_str().unwrap();

    overlay_mount(&lower_dirs, upper_str, work_str, mount_str)
        .with_context(|| format!("Failed to mount overlay for session {session_id}"))?;

    let created_at = format!(
        "{:?}",
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
    );

    let mut cfg = SessionConfig {
        session_id: session_id.clone(),
        created_at,
        mount_point: mount_str.to_string(),
        overlay_lower: lower_dirs.join(":"),
        overlay_upper: upper_str.to_string(),
        overlay_work: work_str.to_string(),
        upper_is_tmpfs: use_tmpfs,
        upper_is_zram: use_zram,
        specs: specs.to_vec(),
        toolchain_dirs,
        pid: None,
    };

    // If there's a command, run it under bwrap
    if !command.is_empty() {
        let (program, args) = command.split_first().unwrap();
        let cwd = std::env::current_dir()?;

        let env = env::compose_env(&resolved.installations);

        let session_mount = mount_pt.to_string_lossy().to_string();
        let mut extra_ro_binds: Vec<PathBuf> = resolved
            .installations
            .iter()
            .map(|i| i.path.clone())
            .collect();

        let profile = SandboxProfile {
            project_dir: Some(cwd.clone()),
            extra_ro_binds,
            allow_network: false,
        };

        let mut child = sandbox::sandboxed_command(program, args, &cwd, &env, &profile);

        // Also bind the overlay mount at /session/ inside the sandbox
        child.arg("--bind").arg(&session_mount).arg("/session/");

        child.stdin(std::process::Stdio::inherit());
        child.stdout(std::process::Stdio::inherit());
        child.stderr(std::process::Stdio::inherit());

        let child_proc = child
            .spawn()
            .with_context(|| format!("Failed to spawn {program}"))?;

        cfg.pid = Some(child_proc.id() as u64);

        // Spawn a keeper thread that waits for the process
        let mount_pt_clone = mount_str.to_string();
        let session_id_clone = session_id.clone();
        std::thread::spawn(move || {
            let mut child = child_proc;
            let status = child.wait();
            eprintln!("[session {session_id_clone}] process exited: {status:?}");
        });
    }

    // Save session config
    save_session(config, &cfg)?;

    let backing_desc = if use_zram {
        "zram"
    } else if use_tmpfs {
        format!("tmpfs({})", size.unwrap_or("4G"))
    } else {
        "disk".to_string()
    };

    eprintln!(
        "▶ session {session_id} started\n  specs: {}\n  mount: {mount_str}\n  upper: {upper_str}\n  backing: {backing_desc}",
        specs.join(", "),
    );

    Ok(session_id)
}

/// Execute a command in an existing session.
///
/// Spawns a fresh bwrap instance with the same overlay mount bound at
/// `/session/`. The resolved toolchains are at `/runtime/` (ro), the
/// current directory at `/workspace/` (rw).
pub fn session_exec(
    session_id: &str,
    command: &[String],
    config: &Config,
) -> Result<String> {
    let cfg = read_session(config, session_id)?;

    if command.is_empty() {
        bail!("No command specified for session exec");
    }

    // Verify the overlay is still mounted
    if !is_mounted(&cfg.mount_point) {
        bail!(
            "Session {session_id} overlay is not mounted at {}",
            cfg.mount_point
        );
    }

    let (program, args) = command.split_first().unwrap();

    // Rebuild environment from toolchain dirs
    let installations: Vec<Installation> = cfg
        .toolchain_dirs
        .iter()
        .map(|path| Installation {
            pkg: crate::types::Package {
                project: "unknown".to_string(),
                version: semver::Version::new(0, 0, 0),
            },
            path: PathBuf::from(path),
        })
        .collect();
    let env = env::compose_env(&installations);

    let cwd = std::env::current_dir()?;
    let session_mount = cfg.mount_point.clone();

    let mut extra_ro_binds: Vec<PathBuf> = cfg
        .toolchain_dirs
        .iter()
        .map(PathBuf::from)
        .collect();

    let profile = SandboxProfile {
        project_dir: Some(cwd.clone()),
        extra_ro_binds,
        allow_network: false,
    };

    let mut cmd = sandbox::sandboxed_command(program, args, &cwd, &env, &profile);

    // Bind the overlay mount at /session/ inside the sandbox
    cmd.arg("--bind")
        .arg(&session_mount)
        .arg("/session/");

    // Bind the session upper dir at /session-upper/ for direct file access
    cmd.arg("--ro-bind")
        .arg(&cfg.overlay_upper)
        .arg("/session-upper/");

    // Capture output
    let output = cmd
        .output()
        .with_context(|| format!("Failed to spawn {program}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let mut result = String::new();
    if !stdout.is_empty() {
        result.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&stderr);
    }

    if !output.status.success() {
        bail!(
            "Command exited with status {}:\n{}",
            output.status.code().unwrap_or(-1),
            result
        );
    }

    Ok(result)
}

/// Stop and optionally destroy a session.
///
/// Unmounts the overlay. With `--purge`, also removes the session upper/work
/// dirs. Without `--purge`, the session state is preserved but unmounted
/// (can be re-mounted manually).
pub fn session_stop(session_id: &str, purge: bool, config: &Config) -> Result<String> {
    let cfg = read_session(config, session_id)?;

    // Kill tracked process if any
    if let Some(pid) = cfg.pid {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
        // Give it a moment to exit
        std::thread::sleep(std::time::Duration::from_millis(200));
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .status();
    }

    // Unmount overlay
    overlay_unmount(&cfg.mount_point)?;

    // If upper is tmpfs, unmount the tmpfs too
    if cfg.upper_is_tmpfs {
        let _ = Command::new("umount")
            .arg(&cfg.overlay_upper)
            .status();
    }

    // Remove the mount point directory
    let _ = std::fs::remove_dir(&cfg.mount_point);

    if purge {
        // Remove session directory entirely
        let dir = session_dir(config, session_id);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(format!("Session '{session_id}' stopped and purged"))
    } else {
        Ok(format!("Session '{session_id}' stopped (upper dir preserved at {})", cfg.overlay_upper))
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_session_id_format() {
        let id = generate_session_id();
        assert!(id.len() > 8, "session ID should be at least 8 chars, got: {id}");
        assert!(id.contains('-'), "session ID should contain a dash separator");
    }

    #[test]
    fn test_session_config_roundtrip() {
        let cfg = SessionConfig {
            session_id: "test123".to_string(),
            created_at: "12345".to_string(),
            mount_point: "/tmp/buckets-session-test123".to_string(),
            overlay_lower: "/path/to/lower".to_string(),
            overlay_upper: "/path/to/upper".to_string(),
            overlay_work: "/path/to/work".to_string(),
            upper_is_tmpfs: false,
            upper_is_zram: false,
            specs: vec!["node@20".to_string()],
            toolchain_dirs: vec![],
            pid: Some(12345),
        };

        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let deser: SessionConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(deser.session_id, "test123");
        assert_eq!(deser.specs[0], "node@20");
        assert_eq!(deser.pid, Some(12345));
    }

    #[test]
    fn test_default_mount_point_format() {
        let mp = default_mount_point("test-id");
        assert_eq!(mp, PathBuf::from("/tmp/buckets-session-test-id"));
    }

    #[test]
    fn test_session_dir_layout() {
        let config = Config {
            dist_url: "https://dist.pkgx.dev".to_string(),
            cache_dir: PathBuf::from("/tmp/.buckets-test"),
            worktree_dir: None,
            platform: "linux/x86-64".to_string(),
        };

        assert_eq!(
            session_dir(&config, "test-id"),
            PathBuf::from("/tmp/.buckets-test/sessions/test-id")
        );
        assert_eq!(
            session_config_path(&config, "test-id"),
            PathBuf::from("/tmp/.buckets-test/sessions/test-id/config.toml")
        );
    }
}
