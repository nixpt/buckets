//! GUI buckets: a fresh, isolated `Xvfb` X server per session, not the
//! host's real display. Borrows the concept from `x11docker`
//! (`workspace/external/x11docker`) — its actual security mechanism
//! (confirmed by reading its ~7000-line script, not just its docs): a
//! session-scoped MIT-MAGIC-COOKIE Xauthority cookie, and binding only the
//! ONE X11 socket file the session needs, never the whole
//! `/tmp/.X11-unix/` directory. x11docker itself is docker/podman-only
//! (zero bwrap usage) — this is a from-scratch bwrap-shaped reimplementation
//! of the same idea, matching this project's existing standalone/
//! no-peer-dependency stance.
//!
//! Deliberately a NESTED server (`Xvfb`), not `--hostdisplay`-style reuse
//! of the real `:0.0` session — X11 has no native per-client window
//! isolation, so reusing the real display would let a sandboxed app see/
//! interact with every other window on it. A fresh `Xvfb` instance is a
//! completely separate X server; nothing on the real display is ever
//! exposed.
//!
//! No new Cargo.toml dependencies — shells out to `Xvfb`/`xauth`/
//! `mcookie` (cookie generation)/`import` (screenshot, ImageMagick),
//! same pattern as `worktree.rs` shelling out to `git`.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// A running, isolated Xvfb X server + its session-scoped auth cookie.
/// Killed (best-effort) and its cookie file removed on drop.
pub struct XvfbSession {
    /// e.g. ":99"
    pub display: String,
    /// Session-scoped Xauthority cookie file — valid ONLY for this
    /// display, not the host's real `~/.Xauthority`.
    pub xauthority: PathBuf,
    display_number: u32,
    child: Child,
}

impl XvfbSession {
    /// Start a fresh Xvfb on the first free display number, with a
    /// session-scoped Xauthority cookie generated and installed BEFORE
    /// the server starts (order matters: `xauth generate` needs a live
    /// X connection and doesn't work here; the working sequence, verified
    /// live, is create-cookie-file -> `xauth add` a fresh `mcookie` ->
    /// THEN start Xvfb pointed at that pre-populated auth file — Xvfb
    /// reads it at startup as its own security database).
    pub fn start(width: u32, height: u32, depth: u32) -> Result<Self> {
        for bin in ["Xvfb", "xauth", "mcookie"] {
            if which(bin).is_none() {
                bail!("'{bin}' not found on PATH — GUI buckets need Xvfb, xauth, and mcookie (util-linux) installed");
            }
        }

        let display_number = find_free_display()?;
        let display = format!(":{display_number}");

        let xauthority = std::env::temp_dir().join(format!("buckets-gui-{display_number}-{}.xauth", std::process::id()));
        std::fs::File::create(&xauthority)
            .with_context(|| format!("Failed to create {}", xauthority.display()))?;

        let cookie = run_capture("mcookie", &[])?.trim().to_string();
        if cookie.is_empty() {
            bail!("mcookie produced an empty cookie");
        }
        let status = Command::new("xauth")
            .arg("-f").arg(&xauthority)
            .arg("add").arg(&display).arg(".").arg(&cookie)
            .status()
            .context("Failed to run xauth add")?;
        if !status.success() {
            bail!("xauth add failed");
        }

        eprintln!("▶ starting Xvfb {display} ({width}x{height}x{depth})");
        let child = Command::new("Xvfb")
            .arg(&display)
            .arg("-screen").arg("0").arg(format!("{width}x{height}x{depth}"))
            .arg("-auth").arg(&xauthority)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("Failed to spawn Xvfb")?;

        wait_for_socket(display_number)?;

        Ok(Self { display, xauthority, display_number, child })
    }

    /// The single X11 socket file to bind into the sandbox — deliberately
    /// just this one file, not the whole `/tmp/.X11-unix/` directory
    /// (matches x11docker's own minimal-bind behavior, confirmed live).
    pub fn socket_path(&self) -> PathBuf {
        socket_path_for(self.display_number)
    }

    /// Screenshot the whole virtual display (root window) via
    /// ImageMagick's `import` — proof-of-life for a caller that has no
    /// way to look at the virtual display directly.
    pub fn screenshot(&self, output: &Path) -> Result<()> {
        if which("import").is_none() {
            bail!("'import' (ImageMagick) not found on PATH — required for --screenshot");
        }
        let status = Command::new("import")
            .env("DISPLAY", &self.display)
            .env("XAUTHORITY", &self.xauthority)
            .arg("-display").arg(&self.display)
            .arg("-window").arg("root")
            .arg(output)
            .status()
            .context("Failed to run import (ImageMagick)")?;
        if !status.success() {
            bail!("import (screenshot) failed");
        }
        Ok(())
    }

    /// Start x11vnc bound to this session's display on a given port.
    /// Returns the VNC server child process (which the caller must keep
    /// alive for the duration of the session).
    pub fn start_vnc(&self, port: u16, password: Option<&str>) -> Result<Child> {
        if which("x11vnc").is_none() {
            bail!("'x11vnc' not found on PATH — install it for VNC support (e.g. apt install x11vnc)");
        }

        let mut cmd = Command::new("x11vnc");
        cmd.arg("-display").arg(&self.display)
            .arg("-rfbport").arg(port.to_string())
            .arg("-forever")
            .arg("-shared")
            .arg("-noipv6")
            .env("XAUTHORITY", &self.xauthority)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        if let Some(pwd) = password {
            if !pwd.is_empty() {
                cmd.arg("-passwd").arg(pwd);
            }
        } else {
            cmd.arg("-nopw");
        }

        let child = cmd.spawn()
            .with_context(|| format!("Failed to spawn x11vnc on port {port}"))?;

        eprintln!("▶ x11vnc running on port {port} for {}", self.display);
        Ok(child)
    }
}

impl Drop for XvfbSession {
    fn drop(&mut self) {
        // `Child::kill()` is SIGKILL, which gives Xvfb no chance to unlink
        // its own socket file — found live: a killed session left a dead
        // `/tmp/.X11-unix/X99` behind, permanently skipped by every future
        // `find_free_display()` call. SIGTERM first (Xvfb's own clean-exit
        // signal) so it unlinks the socket itself; SIGKILL as a fallback
        // if it doesn't exit in time, then remove the socket ourselves
        // regardless, as a backstop.
        let pid = self.child.id();
        let _ = Command::new("kill").arg("-TERM").arg(pid.to_string()).status();
        let exited = (0..20).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            matches!(self.child.try_wait(), Ok(Some(_)))
        });
        if !exited {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_file(socket_path_for(self.display_number));
        let _ = std::fs::remove_file(&self.xauthority);
    }
}

fn socket_path_for(display_number: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/.X11-unix/X{display_number}"))
}

/// First display number (starting at 99, well clear of any real
/// interactive session which is almost always :0-:9) with no existing
/// socket. `x11_unix_dir` is parameterized for unit testing without
/// touching the real `/tmp/.X11-unix/`.
fn find_free_display() -> Result<u32> {
    find_free_display_in(Path::new("/tmp/.X11-unix"))
}

fn find_free_display_in(x11_unix_dir: &Path) -> Result<u32> {
    for n in 99..10000 {
        if !x11_unix_dir.join(format!("X{n}")).exists() {
            return Ok(n);
        }
    }
    bail!("no free X11 display number found")
}

/// Poll for the socket to appear (Xvfb takes a moment to start).
fn wait_for_socket(display_number: u32) -> Result<()> {
    let path = socket_path_for(display_number);
    for _ in 0..50 {
        if path.exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    bail!("Xvfb did not create {} within 5s", path.display())
}

fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).map(|dir| dir.join(bin)).find(|p| p.is_file())
    })
}

fn run_capture(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program).args(args).output()
        .with_context(|| format!("Failed to run {program}"))?;
    if !output.status.success() {
        bail!("{program} exited {}", output.status.code().unwrap_or(1));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_free_display_skips_existing_sockets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("X99"), b"").unwrap();
        std::fs::write(dir.path().join("X100"), b"").unwrap();
        let n = find_free_display_in(dir.path()).unwrap();
        assert_eq!(n, 101);
    }

    #[test]
    fn find_free_display_starts_at_99_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let n = find_free_display_in(dir.path()).unwrap();
        assert_eq!(n, 99);
    }

    #[test]
    fn socket_path_matches_x11_unix_convention() {
        assert_eq!(socket_path_for(99), PathBuf::from("/tmp/.X11-unix/X99"));
        assert_eq!(socket_path_for(0), PathBuf::from("/tmp/.X11-unix/X0"));
    }
}
