//! Buck-net: isolated virtual networks for buckets.
//!
//! Analogous to how [`crate::gui`] gives each bucket its own isolated X
//! display via `XvfbSession`, buck-net gives buckets their own isolated
//! network namespace via `NetSession`. Multiple buckets can share the same
//! namespace and communicate over loopback — none can reach the real host
//! network unless explicitly connected.
//!
//! Implemented rootlessly via `unshare --user --net` (util-linux) — no root,
//! no new Cargo dependencies. The namespace is kept alive by a background
//! `sleep infinity` "keeper" process; bwrap joins it via `--netns
//! /proc/{pid}/ns/net`. When the keeper dies the kernel automatically reclaims
//! the namespace — no cleanup zombies.
//!
//! Why `--user --net` together? Creating a network namespace alone requires
//! `CAP_NET_ADMIN`, which most users don't have. Pairing it with a user
//! namespace (`--user`) avoids that requirement entirely — the process appears
//! as root inside its own user namespace and can freely create a net namespace
//! inside that. Confirmed rootless on a stock Linux desktop without any setuid
//! helper.
//!
//! Inter-bucket communication: since all buckets in the same `NetSession`
//! share the same network namespace, they share its loopback (`127.0.0.1`).
//! One bucket's server listening on `127.0.0.1:3000` is reachable by any
//! other bucket in the same net at the same address. No bridges or veth pairs
//! needed for that.
//!
//! Port forwarding to the host: `expose_port` uses `socat` to forward a
//! host-side TCP port into the namespace via `nsenter`. `socat` is widely
//! packaged (apt/dnf) and keeps the approach dependency-light.
//!
//! Internet access: not provided by default (the isolated net namespace has
//! no route to the host's NIC). Future work: `slirp4netns` integration for
//! rootless NAT.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Metadata persisted to `{nets_dir}/{name}/info.json` for a running session.
#[derive(Debug, Serialize, Deserialize)]
pub struct NetInfo {
    pub name: String,
    pub keeper_pid: u32,
}

/// A named, isolated virtual network namespace. Multiple buckets can join the
/// same session and talk over loopback without any host network access.
///
/// `NetSession` does NOT own the keeper in the Rust sense — dropping it does
/// not kill the keeper. Only [`NetSession::destroy`] ends the keeper.
pub struct NetSession {
    pub name: String,
    pub keeper_pid: u32,
    state_dir: PathBuf,
}

impl NetSession {
    /// Create a new named buck-net session.
    ///
    /// Spawns a rootless `unshare --user --net -- sleep infinity` process to
    /// hold the network namespace alive, brings loopback up inside it, and
    /// persists the keeper PID to `nets_dir/{name}/info.json`.
    pub fn create(name: &str, nets_dir: &Path) -> Result<Self> {
        let state_dir = nets_dir.join(name);
        if state_dir.exists() {
            bail!(
                "Buck-net '{}' already exists — use 'buckets net rm {}' to remove it first.",
                name, name
            );
        }

        for bin in ["unshare", "nsenter", "ip"] {
            if which(bin).is_none() {
                bail!(
                    "'{bin}' not found on PATH — buck-net requires unshare, nsenter, \
                     and ip (util-linux + iproute2)"
                );
            }
        }

        eprintln!("▶ creating buck-net '{name}'");

        // --user: rootless user namespace (maps host uid -> uid 0 inside)
        // --net:  fresh network namespace inside the user namespace
        // sleep infinity: keeps the namespace alive; we store its PID
        let keeper = Command::new("unshare")
            .args(["--user", "--net", "--", "sleep", "infinity"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("Failed to spawn namespace keeper (unshare --user --net)")?;

        let keeper_pid = keeper.id();

        // Poll until the kernel publishes the namespace fd
        wait_for_ns(keeper_pid)?;

        // Bring loopback up so buckets can bind to 127.0.0.1 and reach peers
        let ns_net = format!("/proc/{keeper_pid}/ns/net");
        let ns_user = format!("/proc/{keeper_pid}/ns/user");

        let lo_status = Command::new("nsenter")
            .arg(format!("--user={ns_user}"))
            .arg(format!("--net={ns_net}"))
            .arg("--preserve-credentials")
            .arg("--")
            .args(["ip", "link", "set", "lo", "up"])
            .status()
            .context("Failed to run nsenter to bring up loopback")?;

        if !lo_status.success() {
            let _ = Command::new("kill").arg(keeper_pid.to_string()).status();
            bail!("Failed to configure loopback inside buck-net namespace (nsenter ip link set lo up failed)");
        }

        fs::create_dir_all(&state_dir)
            .with_context(|| format!("Failed to create net state dir: {}", state_dir.display()))?;

        let info = NetInfo { name: name.to_string(), keeper_pid };
        fs::write(
            state_dir.join("info.json"),
            serde_json::to_string_pretty(&info)?,
        )
        .context("Failed to write buck-net info.json")?;

        // Forget the Rust child handle — we do NOT want to kill the keeper on drop
        std::mem::forget(keeper);

        eprintln!("✓ buck-net '{name}' ready (keeper PID: {keeper_pid})");
        Ok(Self { name: name.to_string(), keeper_pid, state_dir })
    }

    /// Reattach to a named buck-net session from a previous `buckets net create`.
    /// Returns an error if the session doesn't exist or its keeper has died
    /// (stale state is cleaned up automatically in that case).
    pub fn load(name: &str, nets_dir: &Path) -> Result<Self> {
        let state_dir = nets_dir.join(name);
        if !state_dir.exists() {
            bail!("Buck-net '{name}' does not exist. Create it with 'buckets net create {name}'.");
        }

        let info: NetInfo = serde_json::from_str(
            &fs::read_to_string(state_dir.join("info.json"))
                .with_context(|| format!("Failed to read buck-net info for '{name}'"))?,
        )
        .with_context(|| format!("Failed to parse buck-net info for '{name}'"))?;

        if !pid_alive(info.keeper_pid) {
            let _ = fs::remove_dir_all(&state_dir);
            bail!(
                "Buck-net '{name}' is stale — keeper process ({}) is gone. \
                 Stale state cleaned up.",
                info.keeper_pid
            );
        }

        Ok(Self { name: info.name, keeper_pid: info.keeper_pid, state_dir })
    }

    /// Path to the network namespace fd — pass this to bwrap's `--netns` flag.
    pub fn ns_path(&self) -> String {
        format!("/proc/{}/ns/net", self.keeper_pid)
    }

    /// Path to the user namespace fd — used in nsenter calls alongside ns_path.
    pub fn user_ns_path(&self) -> String {
        format!("/proc/{}/ns/user", self.keeper_pid)
    }

    /// Kill the keeper process and remove persisted state.
    pub fn destroy(self) -> Result<()> {
        eprintln!("▶ stopping buck-net '{}'", self.name);
        let _ = Command::new("kill").arg(self.keeper_pid.to_string()).status();
        fs::remove_dir_all(&self.state_dir).with_context(|| {
            format!("Failed to remove net state dir: {}", self.state_dir.display())
        })?;
        eprintln!("✓ buck-net '{}' removed", self.name);
        Ok(())
    }

    /// Forward `host_port` on the host's loopback to `bucket_port` on the
    /// namespace's loopback, via `socat` + `nsenter`. Returns the socat child
    /// process; the caller must keep it alive (wait or hold a handle).
    pub fn expose_port(&self, host_port: u16, bucket_port: u16) -> Result<std::process::Child> {
        if which("socat").is_none() {
            bail!("'socat' not found on PATH — install socat for port forwarding");
        }

        let ns_net = self.ns_path();
        let ns_user = self.user_ns_path();

        // socat listens on the host; for each connection execs nsenter to
        // reach the bucket's loopback
        let exec_cmd = format!(
            "nsenter --user={ns_user} --net={ns_net} --preserve-credentials -- \
             socat - TCP:127.0.0.1:{bucket_port}"
        );

        let child = Command::new("socat")
            .arg(format!("TCP-LISTEN:{host_port},fork,reuseaddr,bind=127.0.0.1"))
            .arg(format!("EXEC:\"{exec_cmd}\""))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("Failed to start socat forward {host_port}→{bucket_port}"))?;

        eprintln!(
            "▶ buck-net '{}': 127.0.0.1:{host_port} → namespace:127.0.0.1:{bucket_port}",
            self.name
        );
        Ok(child)
    }

    /// List all active sessions under `nets_dir`. Silently removes entries
    /// whose keeper process is no longer alive.
    pub fn list_all(nets_dir: &Path) -> Vec<NetInfo> {
        if !nets_dir.exists() {
            return vec![];
        }
        let Ok(entries) = fs::read_dir(nets_dir) else { return vec![] };

        let mut result = Vec::new();
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let info_path = entry.path().join("info.json");
            let Ok(content) = fs::read_to_string(&info_path) else { continue };
            let Ok(info) = serde_json::from_str::<NetInfo>(&content) else { continue };

            if pid_alive(info.keeper_pid) {
                result.push(info);
            } else {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
        result
    }
}

/// Poll until `/proc/{pid}/ns/net` appears (the kernel takes a moment to
/// publish namespace fds after fork).
fn wait_for_ns(keeper_pid: u32) -> Result<()> {
    let ns_path = format!("/proc/{keeper_pid}/ns/net");
    for _ in 0..50 {
        if Path::new(&ns_path).exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    bail!(
        "Namespace keeper (PID {keeper_pid}) did not expose /proc/{keeper_pid}/ns/net \
         within 2.5s"
    )
}

/// Send signal 0 to `pid` to check if the process is alive.
fn pid_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(bin))
            .find(|p| p.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_alive_self() {
        assert!(pid_alive(std::process::id()));
    }

    #[test]
    fn pid_alive_bogus_pid() {
        assert!(!pid_alive(999_999));
    }

    #[test]
    fn list_all_empty_for_nonexistent_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(NetSession::list_all(&dir.path().join("nets")).is_empty());
    }

    #[test]
    fn list_all_sweeps_stale_entries() {
        let dir = tempfile::tempdir().unwrap();
        let net_dir = dir.path().join("my-net");
        fs::create_dir_all(&net_dir).unwrap();
        let info = NetInfo { name: "my-net".to_string(), keeper_pid: 999_999 };
        fs::write(
            net_dir.join("info.json"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        let sessions = NetSession::list_all(dir.path());
        // Stale entry filtered out
        assert!(sessions.is_empty());
        // Stale dir cleaned up
        assert!(!net_dir.exists());
    }
}
