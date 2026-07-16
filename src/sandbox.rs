//! Real process isolation for `run`/`shell`/`build`, via `bwrap`
//! (bubblewrap). Before this module, every executed command was a plain
//! `std::process::Command::new(program).status()` with a composed env —
//! "bucket" meant "isolated toolchain version," not "isolated execution."
//! `buckets run node@20 -- rm -rf /` really did that.
//!
//! Deliberately standalone (raw `bwrap` invocation, not exosphere's
//! `exo-light`/`exo/container` — see `.dejavue/decisions.md` for why:
//! consistent with buckets' founding "no peer dependency" decision, and
//! exo-light's own subprocess backend documents itself as having "no
//! Exosphere or Linux namespace or cgroup dependency" anyway — the real
//! namespace/cgroup code lives in exosphere's `crates/exo/container`, a
//! heavier crate this project isn't pulling in). `bwrap` is a well-audited,
//! widely-deployed (Flatpak's own sandboxing) tool — simpler and safer
//! than hand-rolling raw `unshare()` + cgroupfs writes from scratch.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What a sandboxed command gets to see and touch. Every field starts from
/// "nothing" — `bwrap`'s default is a fresh, empty mount namespace, so
/// anything not explicitly granted here simply isn't there.
#[derive(Debug, Clone, Default)]
pub struct SandboxProfile {
    /// Read-write bind for the project/build directory, if the command
    /// needs to write here (e.g. `cargo build`'s `target/`). `None` for
    /// plain `run`/`shell` — they don't target a specific project dir.
    pub project_dir: Option<PathBuf>,
    /// Read-only binds beyond the base OS dirs — each resolved
    /// [`crate::types::Installation`]'s path, so the toolchain itself is
    /// visible without being writable from inside the sandbox.
    pub extra_ro_binds: Vec<PathBuf>,
    /// Build commands generally need their package registry (crates.io,
    /// npm, ...); plain `run`/`shell` default to no network.
    pub allow_network: bool,
}

/// Base host directories bound read-only into every sandbox — enough for
/// a dynamically-linked binary and a shell to function (loader, libc,
/// coreutils). Checked with `exists()` rather than assumed, since not
/// every host has an `/lib64` (usr-merge symlinks vs. real dirs vary).
const BASE_RO_DIRS: &[&str] = &["/usr", "/bin", "/sbin", "/lib", "/lib64"];

/// DNS/TLS config, bound read-only ONLY when `allow_network` is set (they
/// reveal host info — resolver, hosts file — not worth exposing to a
/// sandbox that can't use the network anyway). Sharing the network
/// *namespace* (by omitting `--unshare-net`) is NOT enough on its own for
/// working name resolution — found live: `cargo build`'s crates.io fetch
/// failed with "Could not resolve host: index.crates.io" because
/// `/etc/resolv.conf` didn't exist inside the fresh mount namespace, even
/// though the network itself was reachable. `/etc/ssl` and
/// `/etc/ca-certificates` are both needed together because
/// `/etc/ssl/certs/ca-certificates.crt` is a symlink INTO
/// `/etc/ca-certificates/` on this host (same usr-merge-style pattern as
/// `/lib` -> `/usr/lib`) — binding just one leaves a dangling symlink.
const NETWORK_RO_FILES: &[&str] = &[
    "/etc/resolv.conf",
    "/etc/hosts",
    "/etc/nsswitch.conf",
    "/etc/ssl",
    "/etc/ca-certificates",
];

/// Wrap `program`/`args` to run under `bwrap` with `profile`'s grants. If
/// `bwrap` isn't on `PATH`, falls back to a plain unsandboxed `Command`
/// with a stderr warning — buckets must keep working somewhere without
/// it installed, just without the containment.
pub fn sandboxed_command(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &HashMap<String, String>,
    profile: &SandboxProfile,
) -> Command {
    if let Some(bwrap_path) = which_bwrap() {
        let mut cmd = Command::new(bwrap_path);
        for arg in build_bwrap_args(program, args, cwd, profile) {
            cmd.arg(arg);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd
    } else if let Some(proot_path) = which_proot() {
        eprintln!(
            "⚠ bucket: bwrap not found — falling back to proot compatibility isolation \
             (network and PID namespaces are NOT isolated under this backend)"
        );
        let mut cmd = Command::new(proot_path);
        for arg in build_proot_args(program, args, cwd, profile) {
            cmd.arg(arg);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd
    } else {
        eprintln!(
            "⚠ bucket: bwrap and proot not found on PATH — running WITHOUT sandbox isolation \
             (install bubblewrap for real containment)"
        );
        let mut cmd = Command::new(program);
        cmd.args(args).current_dir(cwd);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd
    }
}

fn which_bwrap() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("bwrap"))
            .find(|p| p.is_file())
    })
}

fn which_proot() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("proot"))
            .find(|p| p.is_file())
    })
}

/// Build the `bwrap` argument list. Split out from [`sandboxed_command`]
/// so the arg-building logic is unit-testable without actually spawning
/// `bwrap` (CI/sandboxed test environments may not have it either).
fn build_bwrap_args(program: &str, args: &[String], cwd: &Path, profile: &SandboxProfile) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();

    for dir in BASE_RO_DIRS {
        if Path::new(dir).exists() {
            a.push("--ro-bind".into());
            a.push((*dir).into());
            a.push((*dir).into());
        }
    }
    if profile.allow_network {
        for path in NETWORK_RO_FILES {
            if Path::new(path).exists() {
                a.push("--ro-bind".into());
                a.push((*path).into());
                a.push((*path).into());
            }
        }
    }
    a.push("--proc".into());
    a.push("/proc".into());
    a.push("--dev".into());
    a.push("/dev".into());
    a.push("--tmpfs".into());
    a.push("/tmp".into());
    a.push("--unshare-pid".into());
    if !profile.allow_network {
        a.push("--unshare-net".into());
    }

    for bind in &profile.extra_ro_binds {
        let s = bind.to_string_lossy().to_string();
        a.push("--ro-bind".into());
        a.push(s.clone());
        a.push(s);
    }

    if let Some(dir) = &profile.project_dir {
        let s = dir.to_string_lossy().to_string();
        a.push("--bind".into());
        a.push(s.clone());
        a.push(s);
    }

    a.push("--chdir".into());
    a.push(cwd.to_string_lossy().to_string());

    a.push("--".into());
    a.push(program.to_string());
    a.extend(args.iter().cloned());

    a
}

fn build_proot_args(program: &str, args: &[String], cwd: &Path, profile: &SandboxProfile) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();

    a.push("-0".into());
    a.push("--kill-on-exit".into());

    if let Some(dir) = &profile.project_dir {
        let s = dir.to_string_lossy().to_string();
        a.push("-b".into());
        a.push(format!("{}:{}", s, s));
    }

    for bind in &profile.extra_ro_binds {
        let s = bind.to_string_lossy().to_string();
        a.push("-b".into());
        a.push(format!("{}:{}", s, s));
    }

    a.push("-w".into());
    a.push(cwd.to_string_lossy().to_string());

    a.push("--".into());
    a.push(program.to_string());
    a.extend(args.iter().cloned());

    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(args: &[String], flag: &str) -> Vec<usize> {
        args.iter().enumerate().filter(|(_, a)| *a == flag).map(|(i, _)| i).collect()
    }

    #[test]
    fn base_dirs_are_ro_bound() {
        let profile = SandboxProfile::default();
        let args = build_bwrap_args("echo", &["hi".to_string()], Path::new("/tmp"), &profile);
        // At least /usr must be present on any Linux host this runs on.
        let ro_bind_positions = find(&args, "--ro-bind");
        assert!(!ro_bind_positions.is_empty());
        assert!(args.iter().any(|a| a == "/usr"));
    }

    #[test]
    fn network_unshared_by_default() {
        let profile = SandboxProfile::default();
        let args = build_bwrap_args("echo", &[], Path::new("/tmp"), &profile);
        assert!(args.iter().any(|a| a == "--unshare-net"));
    }

    #[test]
    fn network_allowed_when_requested() {
        let profile = SandboxProfile { allow_network: true, ..Default::default() };
        let args = build_bwrap_args("echo", &[], Path::new("/tmp"), &profile);
        assert!(!args.iter().any(|a| a == "--unshare-net"));
    }

    /// Regression test for a real bug: sharing the network namespace
    /// (omitting --unshare-net) alone doesn't give working DNS — found
    /// live when `cargo build`'s crates.io fetch failed with "Could not
    /// resolve host" because /etc/resolv.conf wasn't bound.
    #[test]
    fn network_allowed_binds_resolv_conf() {
        let profile = SandboxProfile { allow_network: true, ..Default::default() };
        let args = build_bwrap_args("echo", &[], Path::new("/tmp"), &profile);
        assert!(args.iter().any(|a| a == "/etc/resolv.conf"), "{args:?}");
    }

    #[test]
    fn network_disallowed_does_not_bind_etc_files() {
        let profile = SandboxProfile::default(); // allow_network: false
        let args = build_bwrap_args("echo", &[], Path::new("/tmp"), &profile);
        assert!(!args.iter().any(|a| a == "/etc/resolv.conf"), "{args:?}");
    }

    #[test]
    fn pid_always_unshared() {
        let profile = SandboxProfile::default();
        let args = build_bwrap_args("echo", &[], Path::new("/tmp"), &profile);
        assert!(args.iter().any(|a| a == "--unshare-pid"));
    }

    #[test]
    fn project_dir_gets_rw_bind() {
        let profile = SandboxProfile {
            project_dir: Some(PathBuf::from("/home/user/proj")),
            ..Default::default()
        };
        let args = build_bwrap_args("cargo", &["build".to_string()], Path::new("/home/user/proj"), &profile);
        let bind_positions = find(&args, "--bind");
        assert_eq!(bind_positions.len(), 1);
        assert_eq!(args[bind_positions[0] + 1], "/home/user/proj");
        assert_eq!(args[bind_positions[0] + 2], "/home/user/proj");
    }

    #[test]
    fn no_project_dir_means_no_rw_bind() {
        let profile = SandboxProfile::default();
        let args = build_bwrap_args("node", &[], Path::new("/tmp"), &profile);
        assert!(find(&args, "--bind").is_empty());
    }

    #[test]
    fn extra_ro_binds_included_for_each_installation() {
        let profile = SandboxProfile {
            extra_ro_binds: vec![
                PathBuf::from("/home/user/.buckets/nodejs.org/v20.20.2"),
                PathBuf::from("/home/user/.buckets/openssl.org/v1.1.1w"),
            ],
            ..Default::default()
        };
        let args = build_bwrap_args("node", &[], Path::new("/tmp"), &profile);
        assert!(args.iter().any(|a| a == "/home/user/.buckets/nodejs.org/v20.20.2"));
        assert!(args.iter().any(|a| a == "/home/user/.buckets/openssl.org/v1.1.1w"));
    }

    #[test]
    fn program_and_args_come_after_separator() {
        let profile = SandboxProfile::default();
        let args = build_bwrap_args("node", &["-e".to_string(), "1+1".to_string()], Path::new("/tmp"), &profile);
        let sep = args.iter().position(|a| a == "--").expect("-- separator present");
        assert_eq!(args[sep + 1], "node");
        assert_eq!(args[sep + 2], "-e");
        assert_eq!(args[sep + 3], "1+1");
    }

    #[test]
    fn chdir_set_to_cwd() {
        let profile = SandboxProfile::default();
        let args = build_bwrap_args("echo", &[], Path::new("/some/cwd"), &profile);
        let pos = args.iter().position(|a| a == "--chdir").expect("--chdir present");
        assert_eq!(args[pos + 1], "/some/cwd");
    }

    #[test]
    fn proot_chdir_set_to_cwd() {
        let profile = SandboxProfile::default();
        let args = build_proot_args("echo", &[], Path::new("/some/cwd"), &profile);
        let pos = args.iter().position(|a| a == "-w").expect("-w present");
        assert_eq!(args[pos + 1], "/some/cwd");
    }

    #[test]
    fn proot_extra_ro_binds_included() {
        let profile = SandboxProfile {
            extra_ro_binds: vec![
                PathBuf::from("/home/user/.buckets/nodejs.org/v20.20.2"),
            ],
            ..Default::default()
        };
        let args = build_proot_args("node", &[], Path::new("/tmp"), &profile);
        let pos = args.iter().position(|a| a == "-b").expect("-b present");
        assert_eq!(args[pos + 1], "/home/user/.buckets/nodejs.org/v20.20.2:/home/user/.buckets/nodejs.org/v20.20.2");
    }

    #[test]
    fn proot_project_dir_gets_bind() {
        let profile = SandboxProfile {
            project_dir: Some(PathBuf::from("/home/user/proj")),
            ..Default::default()
        };
        let args = build_proot_args("cargo", &["build".to_string()], Path::new("/home/user/proj"), &profile);
        let pos = args.iter().position(|a| a == "-b").expect("-b present");
        assert_eq!(args[pos + 1], "/home/user/proj:/home/user/proj");
    }

    #[test]
    fn proot_program_and_args_come_after_separator() {
        let profile = SandboxProfile::default();
        let args = build_proot_args("node", &["-e".to_string(), "1+1".to_string()], Path::new("/tmp"), &profile);
        let sep = args.iter().position(|a| a == "--").expect("-- separator present");
        assert_eq!(args[sep + 1], "node");
        assert_eq!(args[sep + 2], "-e");
        assert_eq!(args[sep + 3], "1+1");
    }
}
