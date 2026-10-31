//! Real process isolation for `run`/`shell`/`build`, via `bwrap`
//! (bubblewrap). Before this module, every executed command was a plain
//! `std::process::Command::new(program).status()` with a composed env —
//! "bucket" meant "isolated toolchain version," not "isolated execution."
//! `buckets run node@20 -- rm -rf /` really did that.
//!
//! Deliberately standalone (raw `bwrap` invocation, not exosphere's
//! `exo-light`/`exo/container` — see `.dejavue/decisions.md` for why:
//! consistent with buckets' founding "no peer dependency" decision). `bwrap`
//! is a well-audited, widely-deployed (Flatpak's own sandboxing) tool —
//! simpler and safer than hand-rolling raw `unshare()` + mounts.
//!
//! ## What a sandbox gets (BUCKETS-17)
//!
//! * **Fail closed.** No `bwrap` → [`SandboxError::BwrapNotFound`]. proot
//!   (mount view only: no network/PID/IPC isolation, no seccomp) is used
//!   only when the caller opts in with [`Fallback::Proot`]; running with no
//!   sandbox at all is only ever the caller's explicit choice
//!   ([`unsandboxed`]). Nothing downgrades silently.
//! * **Clean environment.** `bwrap --clearenv` then one `--setenv` per
//!   variable: the composed toolchain env, a small passthrough allowlist
//!   ([`ENV_PASSTHROUGH`] + `LC_*`), `HOME=/tmp` (sandbox-private tmpfs) and a
//!   default `PATH`. The `bwrap` process itself also starts from an empty
//!   environment, so nothing of the host env reaches either process.
//!   [`EnvPolicy::Inherit`] restores the old pass-everything behaviour.
//! * **Namespaces / process hygiene.** Fresh mount, PID, IPC, UTS
//!   (hostname `bucket`), cgroup (`-try`) namespaces; network unshared unless
//!   requested; `--die-with-parent`, `--new-session` (no TIOCSTI into the
//!   caller's terminal), `--cap-drop ALL`, and `--disable-userns` (no nested
//!   user namespaces) where the installed bwrap supports it.
//! * **seccomp inside bwrap.** A BPF program built with
//!   [zandbox](https://crates.io/crates/zandbox) and handed to bwrap through
//!   an inherited memfd (`--seccomp <fd>`); bwrap installs it right before
//!   exec'ing the command, after its own mount/pivot_root/unshare work.
//!   See [`SeccompMode`].
//! * **Limits outside bwrap.** cgroup v2 `memory.max` / `pids.max` /
//!   `cpu.max` via zandbox, joined in the `pre_exec` of the `bwrap` process
//!   itself (so bwrap and everything under it is inside the cgroup), plus a
//!   wall-clock timeout enforced by [`Sandbox::spawn`]/[`SandboxChild::wait`]
//!   (SIGKILL of bwrap — `--die-with-parent` + the PID namespace take the
//!   whole tree down — plus `cgroup.kill` when a cgroup exists). zandbox's
//!   own namespaces/seccomp/landlock are deliberately NOT applied before
//!   `exec bwrap`: bwrap needs mount, pivot_root and unshare.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

// ── Policy types ──────────────────────────────────────────────────────────────

/// What to do when `bwrap` is not available.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Fallback {
    /// Refuse with [`SandboxError::BwrapNotFound`]. The default.
    #[default]
    Refuse,
    /// Accept proot instead (`buckets run --allow-weaker-sandbox`): a mount
    /// view only — the host network, PID and IPC namespaces are shared and no
    /// seccomp filter is installed. cgroup limits and the timeout still apply.
    Proot,
}

/// Which environment the sandboxed program sees.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EnvPolicy {
    /// Only the composed env + [`ENV_PASSTHROUGH`]/`LC_*` + `HOME` + a default
    /// `PATH`. The default.
    #[default]
    Clean,
    /// The host environment, with the composed env layered on top
    /// (`buckets run --inherit-env`; the pre-BUCKETS-17 behaviour).
    Inherit,
}

/// seccomp filter installed inside the sandbox (bwrap backend only).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SeccompMode {
    /// No filter.
    Off,
    /// zandbox `Application` (files, processes, threads, signals, timers,
    /// sockets) + [`SECCOMP_COMPAT_EXTRA`] − [`SECCOMP_BLOCKED`]; anything
    /// else fails with `EPERM`. Measured (strace -f) to cover python3
    /// (threads, subprocess, multiprocessing, venv), node (child_process,
    /// worker_threads), bash, pip, uv, npm and git. The default.
    #[default]
    Default,
    /// zandbox `Default` (no sockets) + [`SECCOMP_COMPAT_EXTRA`] −
    /// [`SECCOMP_BLOCKED`]; anything else kills the process (`SIGSYS`).
    /// Programs that open any socket (NSS lookups in `ls -l`/`id`, every
    /// network client) die under it.
    Strict,
}

impl std::str::FromStr for SeccompMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "off" => Ok(SeccompMode::Off),
            "default" => Ok(SeccompMode::Default),
            "strict" => Ok(SeccompMode::Strict),
            other => Err(format!("unknown seccomp mode '{other}' (expected off, default or strict)")),
        }
    }
}

/// Resource limits. cgroup limits need a delegated cgroup v2 directory
/// ([`Limits::cgroup_parent`] or `$BUCKETS_CGROUP_PARENT`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Limits {
    /// `memory.max` in bytes (the whole sandbox, bwrap included).
    pub memory_bytes: Option<u64>,
    /// `pids.max`. bwrap itself accounts for 2 of these (its monitor and the
    /// PID-namespace init), on top of the workload's own processes/threads.
    pub pids: Option<u64>,
    /// `cpu.max` as a percentage of one CPU (`150` = one and a half CPUs).
    pub cpu_percent: Option<u32>,
    /// Delegated cgroup v2 directory to create the per-run cgroup under.
    /// `None` → `$BUCKETS_CGROUP_PARENT`.
    pub cgroup_parent: Option<PathBuf>,
    /// Wall-clock limit; enforced by [`Sandbox::spawn`]/[`Sandbox::status`]
    /// only (a bare [`Command`] from [`sandboxed_command`] cannot carry it).
    pub timeout: Option<Duration>,
}

impl Limits {
    /// Whether any cgroup limit is requested.
    pub fn needs_cgroup(&self) -> bool {
        self.memory_bytes.is_some() || self.pids.is_some() || self.cpu_percent.is_some()
    }
}

/// A bind mount with an explicit destination inside the sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindMount {
    pub source: PathBuf,
    pub dest: PathBuf,
    pub writable: bool,
}

impl BindMount {
    pub fn rw(source: impl Into<PathBuf>, dest: impl Into<PathBuf>) -> Self {
        Self { source: source.into(), dest: dest.into(), writable: true }
    }
    pub fn ro(source: impl Into<PathBuf>, dest: impl Into<PathBuf>) -> Self {
        Self { source: source.into(), dest: dest.into(), writable: false }
    }
}

/// What a sandboxed command gets to see and touch. Every field starts from
/// "nothing"/"strict" — `bwrap`'s default is a fresh, empty mount
/// namespace, so anything not explicitly granted here simply isn't there.
/// Construct with `SandboxProfile { field: .., ..Default::default() }`.
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
    /// Additional read-write binds for building/caching.
    pub extra_rw_binds: Vec<PathBuf>,
    /// Build commands generally need their package registry (crates.io,
    /// npm, ...); plain `run`/`shell` default to no network.
    pub allow_network: bool,
    /// If set, join this named buck-net namespace instead of unsharing or
    /// sharing the host network. Value is a path like `/proc/{pid}/ns/net`.
    /// Takes precedence over `allow_network` for namespace routing — the
    /// bucket gets whatever the namespace has (loopback only by default;
    /// NAT if the namespace was configured for it).
    pub net_ns: Option<String>,
    /// Binds whose destination differs from the source (e.g. a session
    /// overlay at `/session`). Emitted before the `--` separator.
    pub binds: Vec<BindMount>,
    /// What to do when bwrap is missing. Default: refuse.
    pub fallback: Fallback,
    /// Environment policy. Default: clean.
    pub env_policy: EnvPolicy,
    /// seccomp filter. Default: [`SeccompMode::Default`].
    pub seccomp: SeccompMode,
    /// cgroup limits + timeout. Default: none.
    pub limits: Limits,
    /// Keep the caller's terminal session (omit `--new-session`). Needed for
    /// an interactive shell's job control; costs TIOCSTI protection on
    /// kernels that still allow it (`dev.tty.legacy_tiocsti=1`).
    pub keep_terminal_session: bool,
    /// Share the host IPC namespace (omit `--unshare-ipc`). X11 clients using
    /// MIT-SHM against an Xvfb outside the sandbox need it.
    pub share_ipc: bool,
    /// Allow the sandboxed program to create user namespaces (omit
    /// `--disable-userns`). Browsers' own sandboxes need it.
    pub allow_nested_userns: bool,
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

// ── Environment ──────────────────────────────────────────────────────────────

/// Host variables copied into a clean sandbox environment (plus every
/// `LC_*`). Terminal capabilities, locale and time zone only — nothing that
/// identifies the user or carries credentials.
pub const ENV_PASSTHROUGH: &[&str] = &["TERM", "COLORTERM", "NO_COLOR", "LANG", "LANGUAGE", "TZ"];

/// `HOME` inside a bwrap sandbox: the sandbox-private `/tmp` tmpfs.
pub const SANDBOX_HOME: &str = "/tmp";

/// `PATH` when the composed env has none.
pub const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

fn is_passthrough(key: &str) -> bool {
    ENV_PASSTHROUGH.contains(&key) || key.starts_with("LC_")
}

/// The full environment of a clean-env child, sorted by name: passthrough
/// vars from `host`, `HOME=home`, a default `PATH`, then `composed` (which
/// wins on conflict, so a caller can set `HOME`/`PATH` explicitly).
pub fn clean_env(
    composed: &HashMap<String, String>,
    home: &str,
    host: impl IntoIterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in host {
        if is_passthrough(&k) {
            out.insert(k, v);
        }
    }
    out.insert("HOME".into(), home.into());
    out.insert("PATH".into(), DEFAULT_PATH.into());
    for (k, v) in composed {
        out.insert(k.clone(), v.clone());
    }
    out.into_iter().collect()
}

fn host_env() -> Vec<(String, String)> {
    std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect()
}

// ── seccomp profile ──────────────────────────────────────────────────────────

/// Allowed on top of the zandbox base profile. `copy_file_range` is the one
/// call the strace survey (python3/node/bash/pip/uv/npm/git) found outside
/// zandbox `Application`; the rest are calls ordinary runtimes make on paths
/// the survey didn't exercise (abort/raise → `tgkill`, signal-interrupted
/// sleeps → `restart_syscall`, POSIX timers, SysV semaphores, libaio, JIT
/// `membarrier`, programs installing their own seccomp/landlock filters —
/// which can only narrow what they may do).
pub const SECCOMP_COMPAT_EXTRA: &[&str] = &[
    "copy_file_range",
    "restart_syscall",
    "tgkill",
    "tkill",
    "rt_tgsigqueueinfo",
    "clock_gettime",
    "clock_getres",
    "getcpu",
    "membarrier",
    "sched_setaffinity",
    "setsid",
    "setpgid",
    "memfd_create",
    "mbind",
    "get_mempolicy",
    "set_mempolicy",
    "timer_create",
    "timer_settime",
    "timer_gettime",
    "timer_getoverrun",
    "timer_delete",
    "fchmodat2",
    "utimes",
    "futimesat",
    "mknod",
    "readahead",
    "sync",
    "syncfs",
    "setrlimit",
    "mlock2",
    "pkey_alloc",
    "pkey_free",
    "pkey_mprotect",
    "removexattr",
    "lremovexattr",
    "fremovexattr",
    "shmdt",
    "semget",
    "semop",
    "semtimedop",
    "semctl",
    "io_setup",
    "io_destroy",
    "io_submit",
    "io_getevents",
    "io_cancel",
    "pidfd_send_signal",
    "seccomp",
    "landlock_create_ruleset",
    "landlock_add_rule",
    "landlock_restrict_self",
];

/// Removed from the allowlist even though the zandbox base profile has
/// them. `unshare`: new namespaces from inside the sandbox are not the
/// workload's business (bwrap's `--disable-userns` covers user namespaces
/// too, where available). Not on the list because never allowed in the
/// first place: `mount`, `pivot_root`, `chroot`, `setns`, `ptrace`,
/// `process_vm_*`, `bpf`, `perf_event_open`, `keyctl`/`add_key`, module
/// loading, `kexec_*`, `userfaultfd`, `io_uring_*` (libuv falls back to its
/// thread pool on `EPERM`).
pub const SECCOMP_BLOCKED: &[&str] = &["unshare"];

/// Build the raw `struct sock_filter[]` bytes bwrap's `--seccomp FD` reads.
fn seccomp_program(mode: SeccompMode) -> Result<Option<Vec<u8>>, SandboxError> {
    use zandbox::seccomp::Architecture;
    use zandbox::{Seccomp, SeccompAction, SeccompProfile};

    let (base, action) = match mode {
        SeccompMode::Off => return Ok(None),
        SeccompMode::Default => (SeccompProfile::Application, SeccompAction::Errno(libc::EPERM as u16)),
        SeccompMode::Strict => (SeccompProfile::Default, SeccompAction::KillProcess),
    };
    let arch = Architecture::current()
        .ok_or_else(|| SandboxError::Seccomp("no syscall table for this architecture".into()))?;
    let mut spec = Seccomp::new(base).action(action);
    for name in SECCOMP_COMPAT_EXTRA {
        spec = spec.allow(*name);
    }
    for name in SECCOMP_BLOCKED {
        spec = spec.block(*name);
    }
    // Names in our own lists that this architecture lacks (legacy x86_64
    // calls on aarch64) land in `unknown`; they are simply not allowed there.
    let compiled = spec.compile(arch).map_err(SandboxError::Seccomp)?;
    let mut bytes = Vec::with_capacity(compiled.program.len() * 8);
    for ins in &compiled.program {
        bytes.extend_from_slice(&ins.code.to_ne_bytes());
        bytes.push(ins.jt);
        bytes.push(ins.jf);
        bytes.extend_from_slice(&ins.k.to_ne_bytes());
    }
    Ok(Some(bytes))
}

/// An anonymous file holding `data`, opened `O_CLOEXEC` (the `pre_exec` hook
/// clears the flag in the bwrap child only).
fn memfd_with(data: &[u8]) -> io::Result<std::os::fd::OwnedFd> {
    use std::io::Write;
    use std::os::fd::{FromRawFd, OwnedFd};
    // SAFETY: NUL-terminated name, valid flags.
    let fd = unsafe { libc::memfd_create(c"buckets-seccomp".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd was just returned by memfd_create and is owned by nobody else.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut file = std::fs::File::from(owned);
    file.write_all(data)?;
    Ok(OwnedFd::from(file))
}

// ── Errors ───────────────────────────────────────────────────────────────────

/// Why a sandbox could not be built. Nothing here is ever downgraded to an
/// unsandboxed run behind the caller's back.
#[derive(Debug)]
pub enum SandboxError {
    /// No `bwrap` on `PATH` and the profile doesn't accept proot.
    BwrapNotFound { proot_on_path: bool },
    /// [`Fallback::Proot`] was accepted but neither bwrap nor proot exist.
    NoBackend,
    /// The installed bwrap lacks a flag this profile requires.
    BwrapTooOld { flag: &'static str },
    /// cgroup limits requested without a delegated cgroup v2 parent.
    NoCgroupParent,
    /// zandbox refused the cgroup setup (strict mode).
    Limits(String),
    /// The seccomp program could not be built or handed over.
    Seccomp(String),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SandboxError::BwrapNotFound { proot_on_path } => {
                write!(
                    f,
                    "bwrap (bubblewrap) not found on PATH — refusing to run without a sandbox. \
                     Install bubblewrap (e.g. `pacman -S bubblewrap`, `apt install bubblewrap`), \
                     or opt out explicitly: --allow-weaker-sandbox (proot: no network/PID/IPC \
                     isolation, no seccomp{}) or --no-sandbox (no isolation at all)",
                    if *proot_on_path { "" } else { "; proot is not installed either" }
                )
            }
            SandboxError::NoBackend => write!(
                f,
                "neither bwrap nor proot found on PATH — install bubblewrap, or pass --no-sandbox \
                 to run without isolation"
            ),
            SandboxError::BwrapTooOld { flag } => write!(
                f,
                "the installed bwrap does not support {flag} (needs bubblewrap >= 0.8) — \
                 upgrade bubblewrap, or pass --no-sandbox explicitly"
            ),
            SandboxError::NoCgroupParent => write!(
                f,
                "--memory/--pids/--cpu need a delegated cgroup v2 directory: pass \
                 --cgroup-parent DIR or set BUCKETS_CGROUP_PARENT. With systemd: \
                 `systemd-run --user --scope -p Delegate=yes bash -c 'CG=/sys/fs/cgroup$(sed -n \
                 \"s/^0:://p\" /proc/self/cgroup); mkdir $CG/leaf $CG/sandboxes; echo 0 > \
                 $CG/leaf/cgroup.procs; echo \"+pids +memory +cpu\" > $CG/cgroup.subtree_control; \
                 BUCKETS_CGROUP_PARENT=$CG/sandboxes buckets run ...'`"
            ),
            SandboxError::Limits(e) => write!(f, "cannot apply resource limits: {e}"),
            SandboxError::Seccomp(e) => write!(f, "cannot build the seccomp filter: {e}"),
        }
    }
}

impl std::error::Error for SandboxError {}

// ── bwrap feature probe ──────────────────────────────────────────────────────

/// Optional bwrap flags, probed from `bwrap --help` once per binary path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BwrapFeatures {
    clearenv: bool,
    die_with_parent: bool,
    new_session: bool,
    cap_drop: bool,
    unshare_ipc: bool,
    unshare_uts: bool,
    unshare_cgroup_try: bool,
    disable_userns: bool,
    seccomp: bool,
    hostname: bool,
    /// bwrap installed setuid-root: no implicit user namespace, so
    /// `--unshare-user`/`--disable-userns` are not added.
    setuid: bool,
}

impl BwrapFeatures {
    #[cfg(test)]
    pub(crate) fn all() -> Self {
        Self::from_help(
            "--clearenv --die-with-parent --new-session --cap-drop --unshare-ipc --unshare-uts \
             --unshare-cgroup-try --disable-userns --seccomp --hostname",
            false,
        )
    }

    fn from_help(help: &str, setuid: bool) -> Self {
        let has = |flag: &str| {
            help.split(|c: char| c.is_whitespace() || c == ',' || c == '=')
                .any(|w| w == flag)
        };
        Self {
            clearenv: has("--clearenv"),
            die_with_parent: has("--die-with-parent"),
            new_session: has("--new-session"),
            cap_drop: has("--cap-drop"),
            unshare_ipc: has("--unshare-ipc"),
            unshare_uts: has("--unshare-uts"),
            unshare_cgroup_try: has("--unshare-cgroup-try"),
            disable_userns: has("--disable-userns"),
            seccomp: has("--seccomp"),
            hostname: has("--hostname"),
            setuid,
        }
    }

    fn probe(bwrap: &Path) -> Self {
        static CACHE: OnceLock<Mutex<HashMap<PathBuf, BwrapFeatures>>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(f) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(bwrap) {
            return *f;
        }
        let help = Command::new(bwrap)
            .arg("--help")
            .env_clear()
            .stdin(std::process::Stdio::null())
            .output()
            .map(|o| {
                let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                s.push_str(&String::from_utf8_lossy(&o.stderr));
                s
            })
            .unwrap_or_default();
        let setuid = {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(bwrap)
                .map(|m| m.permissions().mode() & 0o4000 != 0)
                .unwrap_or(false)
        };
        let f = Self::from_help(&help, setuid);
        cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(bwrap.to_path_buf(), f);
        f
    }
}

// ── Backends ─────────────────────────────────────────────────────────────────

/// Which isolation a [`Sandbox`] actually uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Bwrap,
    /// Only when the profile opted in with [`Fallback::Proot`].
    Proot,
    /// Only from [`unsandboxed`] (the caller's explicit `--no-sandbox`).
    None,
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|p| p.is_file())
    })
}

fn which_bwrap() -> Option<PathBuf> {
    which("bwrap")
}

fn which_proot() -> Option<PathBuf> {
    which("proot")
}

/// What [`SandboxChild::wait`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOutcome {
    pub status: ExitStatus,
    /// The wall-clock limit expired and the sandbox was killed.
    pub timed_out: bool,
}

/// Keeps the zandbox cgroup alive (it is removed when the last clone drops).
#[derive(Debug)]
struct CgroupHold(zandbox::Prepared);

impl CgroupHold {
    fn path(&self) -> Option<&Path> {
        self.0.cgroup_path()
    }
}

/// A ready-to-spawn sandboxed command plus what has to outlive it (the
/// cgroup, the seccomp memfd) and its timeout.
#[derive(Debug)]
pub struct Sandbox {
    cmd: Command,
    backend: Backend,
    cgroup: Option<Arc<CgroupHold>>,
    timeout: Option<Duration>,
}

impl Sandbox {
    /// The underlying command, for stdio redirection. Do not set env vars on
    /// it: under [`EnvPolicy::Clean`] they would not reach the sandboxed
    /// program — put them in the `env` map instead.
    pub fn command_mut(&mut self) -> &mut Command {
        &mut self.cmd
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// The per-run cgroup directory, if limits were requested.
    pub fn cgroup_path(&self) -> Option<&Path> {
        self.cgroup.as_ref().and_then(|c| c.path())
    }

    /// The bare command. The `pre_exec` hook keeps the cgroup and seccomp fd
    /// alive for as long as the `Command` lives; the timeout is lost.
    pub fn into_command(self) -> Command {
        self.cmd
    }

    pub fn spawn(mut self) -> io::Result<SandboxChild> {
        let child = self.cmd.spawn()?;
        Ok(SandboxChild {
            deadline: self.timeout.map(|t| Instant::now() + t),
            child,
            cgroup: self.cgroup,
            _cmd: self.cmd,
        })
    }

    /// Spawn and wait, honouring the timeout.
    pub fn status(self) -> io::Result<RunOutcome> {
        self.spawn()?.wait()
    }
}

/// A running sandbox.
#[derive(Debug)]
pub struct SandboxChild {
    child: Child,
    cgroup: Option<Arc<CgroupHold>>,
    deadline: Option<Instant>,
    /// Holds the `pre_exec` closure's captures until the child is reaped.
    _cmd: Command,
}

impl SandboxChild {
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Kill the whole sandbox: `cgroup.kill` when there is a cgroup, and
    /// SIGKILL to the outer process (bwrap's `--die-with-parent` and its PID
    /// namespace take every descendant down with it).
    pub fn kill(&mut self) -> io::Result<()> {
        if let Some(p) = self.cgroup.as_ref().and_then(|c| c.path()) {
            let _ = std::fs::write(p.join("cgroup.kill"), "1");
        }
        match self.child.kill() {
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => Ok(()), // already reaped
            r => r,
        }
    }

    pub fn wait(mut self) -> io::Result<RunOutcome> {
        let outcome = match self.deadline {
            None => RunOutcome { status: self.child.wait()?, timed_out: false },
            Some(deadline) => loop {
                if let Some(status) = self.child.try_wait()? {
                    break RunOutcome { status, timed_out: false };
                }
                let now = Instant::now();
                if now >= deadline {
                    self.kill()?;
                    break RunOutcome { status: self.child.wait()?, timed_out: true };
                }
                std::thread::sleep((deadline - now).min(Duration::from_millis(20)));
            },
        };
        self.sweep_cgroup();
        Ok(outcome)
    }

    /// Kill stragglers and give the kernel a moment to empty the cgroup so
    /// zandbox's drop can `rmdir` it.
    fn sweep_cgroup(&self) {
        let Some(p) = self.cgroup.as_ref().and_then(|c| c.path()) else { return };
        let _ = std::fs::write(p.join("cgroup.kill"), "1");
        let events = p.join("cgroup.events");
        let until = Instant::now() + Duration::from_secs(1);
        while Instant::now() < until {
            match std::fs::read_to_string(&events) {
                Ok(s) if s.lines().any(|l| l == "populated 1") => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                _ => break,
            }
        }
    }
}

/// Resolve the cgroup parent and prepare the zandbox cgroup (strict).
fn prepare_cgroup(limits: &Limits) -> Result<Option<Arc<CgroupHold>>, SandboxError> {
    if !limits.needs_cgroup() {
        return Ok(None);
    }
    let parent = limits
        .cgroup_parent
        .clone()
        .or_else(|| std::env::var_os("BUCKETS_CGROUP_PARENT").map(PathBuf::from))
        .ok_or(SandboxError::NoCgroupParent)?;
    // Only cgroup limits: no namespaces / caps / landlock / seccomp, so
    // `apply_in_child` does exactly one thing — write 0 to cgroup.procs.
    let mut p = zandbox::Profile::new().cgroup_parent(parent).strict(true);
    if let Some(m) = limits.memory_bytes {
        p = p.memory_max(m);
    }
    if let Some(n) = limits.pids {
        p = p.pids_max(n);
    }
    if let Some(c) = limits.cpu_percent {
        p = p.cpu_percent(c);
    }
    let prepared = p.prepare().map_err(|e| SandboxError::Limits(e.to_string()))?;
    if limits.memory_bytes.is_some() {
        // memory.max alone lets the workload spill into swap (zandbox leaves
        // memory.swap.max at "max"), so --memory would not really cap it.
        if let Some(cg) = prepared.cgroup_path() {
            let swap = cg.join("memory.swap.max");
            if swap.exists() {
                std::fs::write(&swap, "0")
                    .map_err(|e| SandboxError::Limits(format!("{}: {e}", swap.display())))?;
            }
        }
    }
    Ok(Some(Arc::new(CgroupHold(prepared))))
}

/// Install the `pre_exec` hook: join the cgroup, and make the seccomp memfd
/// inheritable (and rewound) in this child only.
fn install_pre_exec(
    cmd: &mut Command,
    cgroup: Option<Arc<CgroupHold>>,
    seccomp_fd: Option<Arc<std::os::fd::OwnedFd>>,
) {
    use std::os::fd::AsRawFd;
    if cgroup.is_none() && seccomp_fd.is_none() {
        return;
    }
    // SAFETY: the closure only issues async-signal-safe syscalls on data
    // prepared before fork (zandbox's apply_in_child guarantees the same).
    unsafe {
        cmd.pre_exec(move || {
            if let Some(cg) = &cgroup {
                cg.0.apply_in_child().map_err(io::Error::from)?;
            }
            if let Some(fd) = &seccomp_fd {
                let raw = fd.as_raw_fd();
                if libc::fcntl(raw, libc::F_SETFD, 0) != 0
                    || libc::lseek(raw, 0, libc::SEEK_SET) != 0
                {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

/// Build a sandboxed command for `program`/`args`, failing closed.
///
/// * bwrap present → bwrap with every flag described in the module docs;
/// * bwrap missing → [`SandboxError::BwrapNotFound`], unless the profile
///   says [`Fallback::Proot`] (then proot, with a stderr warning naming what
///   is not isolated);
/// * limits requested without a usable delegated cgroup → error.
pub fn try_sandboxed_command(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &HashMap<String, String>,
    profile: &SandboxProfile,
) -> Result<Sandbox, SandboxError> {
    try_sandboxed_command_with(which_bwrap(), which_proot(), program, args, cwd, env, profile)
}

fn try_sandboxed_command_with(
    bwrap: Option<PathBuf>,
    proot: Option<PathBuf>,
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &HashMap<String, String>,
    profile: &SandboxProfile,
) -> Result<Sandbox, SandboxError> {
    if let Some(bwrap_path) = bwrap {
        let features = BwrapFeatures::probe(&bwrap_path);
        let seccomp_fd = match seccomp_program(profile.seccomp)? {
            Some(bytes) => Some(Arc::new(
                memfd_with(&bytes).map_err(|e| SandboxError::Seccomp(format!("memfd: {e}")))?,
            )),
            None => None,
        };
        let env_pairs = match profile.env_policy {
            EnvPolicy::Clean => clean_env(env, SANDBOX_HOME, host_env()),
            EnvPolicy::Inherit => {
                let mut v: Vec<_> = env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                v.sort();
                v
            }
        };
        let fd_num = seccomp_fd.as_ref().map(|fd| {
            use std::os::fd::AsRawFd;
            fd.as_raw_fd()
        });
        let argv = build_bwrap_args(program, args, cwd, profile, &env_pairs, &features, fd_num)?;
        let cgroup = prepare_cgroup(&profile.limits)?;

        let mut cmd = Command::new(bwrap_path);
        cmd.args(argv);
        if profile.env_policy == EnvPolicy::Clean {
            // bwrap itself needs nothing from the environment; don't let a
            // host secret sit in /proc/<bwrap>/environ either.
            cmd.env_clear();
        }
        install_pre_exec(&mut cmd, cgroup.clone(), seccomp_fd);
        return Ok(Sandbox { cmd, backend: Backend::Bwrap, cgroup, timeout: profile.limits.timeout });
    }

    if profile.fallback != Fallback::Proot {
        return Err(SandboxError::BwrapNotFound { proot_on_path: proot.is_some() });
    }
    let proot_path = proot.ok_or(SandboxError::NoBackend)?;
    eprintln!(
        "⚠ bucket: bwrap not found — using proot as requested (--allow-weaker-sandbox): \
         network, PID and IPC namespaces are NOT isolated and no seccomp filter is installed"
    );
    let cgroup = prepare_cgroup(&profile.limits)?;
    let mut cmd = Command::new(proot_path);
    cmd.args(build_proot_args(program, args, cwd, profile));
    apply_env(&mut cmd, env, profile.env_policy, SANDBOX_HOME);
    install_pre_exec(&mut cmd, cgroup.clone(), None);
    Ok(Sandbox { cmd, backend: Backend::Proot, cgroup, timeout: profile.limits.timeout })
}

/// Set the child environment on a non-bwrap command (proot / unsandboxed),
/// where the environment of the spawned process IS the program's.
fn apply_env(cmd: &mut Command, env: &HashMap<String, String>, policy: EnvPolicy, home: &str) {
    match policy {
        EnvPolicy::Clean => {
            cmd.env_clear();
            cmd.envs(clean_env(env, home, host_env()));
        }
        EnvPolicy::Inherit => {
            cmd.envs(env);
        }
    }
}

/// The caller's explicit "no sandbox" (`--no-sandbox`): a plain command,
/// but still with the profile's env policy (clean by default — `HOME` is the
/// host's, since nothing is isolated anyway), cgroup limits and timeout.
/// seccomp is not applied.
pub fn unsandboxed(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    env: &HashMap<String, String>,
    profile: &SandboxProfile,
) -> Result<Sandbox, SandboxError> {
    let cgroup = prepare_cgroup(&profile.limits)?;
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| SANDBOX_HOME.to_string());
    apply_env(&mut cmd, env, profile.env_policy, &home);
    if profile.limits.timeout.is_some() {
        // Its own process group, so the timeout can kill the whole tree.
        cmd.process_group(0);
    }
    install_pre_exec(&mut cmd, cgroup.clone(), None);
    Ok(Sandbox { cmd, backend: Backend::None, cgroup, timeout: profile.limits.timeout })
}

/// Source-compatible wrapper over [`try_sandboxed_command`] for callers that
/// need an infallible `Command` (crush-vm's `bucket_exec`).
///
/// It no longer falls back to running unsandboxed: when the sandbox cannot
/// be built, the returned command runs **nothing** — it prints the
/// [`SandboxError`] on stderr and exits with status 126. Prefer
/// [`try_sandboxed_command`], which reports the error as a value and also
/// supports the wall-clock timeout ([`Sandbox::status`]).
pub fn sandboxed_command(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &HashMap<String, String>,
    profile: &SandboxProfile,
) -> Command {
    match try_sandboxed_command(program, args, cwd, env, profile) {
        Ok(sb) => sb.into_command(),
        Err(e) => refusal_command(&e),
    }
}

fn refusal_command(e: &SandboxError) -> Command {
    let mut cmd = Command::new("/bin/sh");
    cmd.env_clear()
        .args(["-c", "printf 'buckets: %s\\n' \"$1\" >&2; exit 126", "buckets-sandbox"])
        .arg(e.to_string());
    cmd
}

/// Build the `bwrap` argument list. Split out from [`try_sandboxed_command`]
/// so the arg-building logic is unit-testable without spawning `bwrap`.
/// Every option — including [`SandboxProfile::binds`] — precedes the `--`
/// separator; only the program and its arguments follow it.
pub(crate) fn build_bwrap_args(
    program: &str,
    args: &[String],
    cwd: &Path,
    profile: &SandboxProfile,
    env: &[(String, String)],
    features: &BwrapFeatures,
    seccomp_fd: Option<i32>,
) -> Result<Vec<String>, SandboxError> {
    let mut a: Vec<String> = Vec::new();
    let push = |a: &mut Vec<String>, items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));
    let require = |ok: bool, flag: &'static str| if ok { Ok(()) } else { Err(SandboxError::BwrapTooOld { flag }) };

    for dir in BASE_RO_DIRS {
        if Path::new(dir).exists() {
            push(&mut a, &["--ro-bind", dir, dir]);
        }
    }
    if profile.allow_network {
        for path in NETWORK_RO_FILES {
            if Path::new(path).exists() {
                push(&mut a, &["--ro-bind", path, path]);
            }
        }
    }
    push(&mut a, &["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]);

    // Namespaces.
    if features.disable_userns && !features.setuid && !profile.allow_nested_userns {
        push(&mut a, &["--unshare-user", "--disable-userns"]);
    }
    push(&mut a, &["--unshare-pid"]);
    if !profile.share_ipc {
        require(features.unshare_ipc, "--unshare-ipc")?;
        push(&mut a, &["--unshare-ipc"]);
    }
    require(features.unshare_uts, "--unshare-uts")?;
    push(&mut a, &["--unshare-uts"]);
    if features.hostname {
        push(&mut a, &["--hostname", "bucket"]);
    }
    if features.unshare_cgroup_try {
        push(&mut a, &["--unshare-cgroup-try"]);
    }
    // Network: three modes in priority order —
    //   1. buck-net namespace (--netns): join an isolated virtual net
    //   2. unshare (--unshare-net): completely cut off
    //   3. host network (neither flag): full access for build toolchains
    if let Some(ref ns_path) = profile.net_ns {
        push(&mut a, &["--netns", ns_path]);
    } else if !profile.allow_network {
        push(&mut a, &["--unshare-net"]);
    }

    // Process hygiene.
    require(features.die_with_parent, "--die-with-parent")?;
    push(&mut a, &["--die-with-parent"]);
    if !profile.keep_terminal_session {
        require(features.new_session, "--new-session")?;
        push(&mut a, &["--new-session"]);
    }
    if features.cap_drop {
        push(&mut a, &["--cap-drop", "ALL"]);
    }

    for bind in &profile.extra_ro_binds {
        let s = bind.to_string_lossy();
        push(&mut a, &["--ro-bind", &s, &s]);
    }
    if let Some(dir) = &profile.project_dir {
        let s = dir.to_string_lossy();
        push(&mut a, &["--bind", &s, &s]);
    }
    for bind in &profile.extra_rw_binds {
        let s = bind.to_string_lossy();
        push(&mut a, &["--bind", &s, &s]);
    }
    for b in &profile.binds {
        let flag = if b.writable { "--bind" } else { "--ro-bind" };
        push(&mut a, &[flag, &b.source.to_string_lossy(), &b.dest.to_string_lossy()]);
    }

    // Environment: set by bwrap inside the sandbox, never inherited.
    if profile.env_policy == EnvPolicy::Clean {
        require(features.clearenv, "--clearenv")?;
        push(&mut a, &["--clearenv"]);
    }
    for (k, v) in env {
        push(&mut a, &["--setenv", k, v]);
    }

    if let Some(fd) = seccomp_fd {
        require(features.seccomp, "--seccomp")?;
        push(&mut a, &["--seccomp", &fd.to_string()]);
    }

    push(&mut a, &["--chdir", &cwd.to_string_lossy()]);

    a.push("--".into());
    a.push(program.to_string());
    a.extend(args.iter().cloned());

    Ok(a)
}

fn build_proot_args(program: &str, args: &[String], cwd: &Path, profile: &SandboxProfile) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();

    a.push("-0".into());
    a.push("--kill-on-exit".into());
    a.push("--link2symlink".into());

    if let Some(dir) = &profile.project_dir {
        let s = dir.to_string_lossy().to_string();
        a.push("-b".into());
        a.push(format!("{}:{}", s, s));
    }

    for bind in &profile.extra_rw_binds {
        let s = bind.to_string_lossy().to_string();
        a.push("-b".into());
        a.push(format!("{}:{}", s, s));
    }

    for bind in &profile.extra_ro_binds {
        let s = bind.to_string_lossy().to_string();
        a.push("-b".into());
        a.push(format!("{}:{}", s, s));
    }

    for b in &profile.binds {
        a.push("-b".into());
        a.push(format!("{}:{}", b.source.to_string_lossy(), b.dest.to_string_lossy()));
    }

    a.push("-w".into());
    a.push(cwd.to_string_lossy().to_string());

    a.push(program.to_string());
    a.extend(args.iter().cloned());

    a
}

// ── CLI value parsers (shared by `main.rs`) ──────────────────────────────────

/// `512M`, `1G`, `1.5G`, `64MiB`, `4096` (bytes), `100K`.
pub fn parse_size(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num.parse().map_err(|_| format!("invalid size '{s}'"))?;
    let mult: u64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1 << 10,
        "m" | "mb" | "mib" => 1 << 20,
        "g" | "gb" | "gib" => 1 << 30,
        "t" | "tb" | "tib" => 1 << 40,
        _ => return Err(format!("invalid size unit in '{s}' (use K, M, G or T)")),
    };
    let bytes = n * mult as f64;
    if bytes < 1.0 {
        return Err(format!("size '{s}' must be positive"));
    }
    Ok(bytes as u64)
}

/// `50%`, `150%`, `50` (percent of one CPU).
pub fn parse_cpu_percent(s: &str) -> Result<u32, String> {
    let n: u32 = s
        .trim()
        .trim_end_matches('%')
        .parse()
        .map_err(|_| format!("invalid cpu limit '{s}' (expected a percentage of one CPU, e.g. 50%)"))?;
    if n == 0 {
        return Err("cpu limit must be > 0%".into());
    }
    Ok(n)
}

/// `30s`, `2m`, `1h`, `500ms`, `30` (seconds).
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let t = s.trim();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num.parse().map_err(|_| format!("invalid duration '{s}'"))?;
    let secs = match unit {
        "" | "s" => n,
        "ms" => n / 1000.0,
        "m" => n * 60.0,
        "h" => n * 3600.0,
        _ => return Err(format!("invalid duration unit in '{s}' (use ms, s, m or h)")),
    };
    if secs <= 0.0 {
        return Err(format!("duration '{s}' must be positive"));
    }
    Ok(Duration::from_secs_f64(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(args: &[String], flag: &str) -> Vec<usize> {
        args.iter().enumerate().filter(|(_, a)| *a == flag).map(|(i, _)| i).collect()
    }

    fn bw(program: &str, args: &[&str], cwd: &str, profile: &SandboxProfile) -> Vec<String> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        build_bwrap_args(program, &args, Path::new(cwd), profile, &[], &BwrapFeatures::all(), None).unwrap()
    }

    fn sep(args: &[String]) -> usize {
        args.iter().position(|a| a == "--").expect("-- separator present")
    }

    #[test]
    fn base_dirs_are_ro_bound() {
        let args = bw("echo", &["hi"], "/tmp", &SandboxProfile::default());
        assert!(!find(&args, "--ro-bind").is_empty());
        assert!(args.iter().any(|a| a == "/usr"));
    }

    #[test]
    fn network_unshared_by_default() {
        let args = bw("echo", &[], "/tmp", &SandboxProfile::default());
        assert!(args.iter().any(|a| a == "--unshare-net"));
    }

    #[test]
    fn network_allowed_when_requested() {
        let profile = SandboxProfile { allow_network: true, ..Default::default() };
        let args = bw("echo", &[], "/tmp", &profile);
        assert!(!args.iter().any(|a| a == "--unshare-net"));
    }

    /// Regression test for a real bug: sharing the network namespace
    /// (omitting --unshare-net) alone doesn't give working DNS — found
    /// live when `cargo build`'s crates.io fetch failed with "Could not
    /// resolve host" because /etc/resolv.conf wasn't bound.
    #[test]
    fn network_allowed_binds_resolv_conf() {
        let profile = SandboxProfile { allow_network: true, ..Default::default() };
        let args = bw("echo", &[], "/tmp", &profile);
        if Path::new("/etc/resolv.conf").exists() {
            assert!(args.iter().any(|a| a == "/etc/resolv.conf"), "{args:?}");
        }
    }

    #[test]
    fn network_disallowed_does_not_bind_etc_files() {
        let args = bw("echo", &[], "/tmp", &SandboxProfile::default());
        assert!(!args.iter().any(|a| a == "/etc/resolv.conf"), "{args:?}");
    }

    #[test]
    fn pid_always_unshared() {
        let args = bw("echo", &[], "/tmp", &SandboxProfile::default());
        assert!(args.iter().any(|a| a == "--unshare-pid"));
    }

    #[test]
    fn hardening_flags_present_by_default() {
        let args = bw("echo", &[], "/tmp", &SandboxProfile::default());
        let s = sep(&args);
        for flag in [
            "--die-with-parent",
            "--new-session",
            "--unshare-ipc",
            "--unshare-uts",
            "--unshare-cgroup-try",
            "--clearenv",
            "--disable-userns",
        ] {
            let p = args.iter().position(|a| a == flag).unwrap_or_else(|| panic!("{flag} missing: {args:?}"));
            assert!(p < s, "{flag} after --");
        }
        let cap = args.iter().position(|a| a == "--cap-drop").expect("--cap-drop");
        assert_eq!(args[cap + 1], "ALL");
    }

    #[test]
    fn interactive_and_x11_knobs_drop_their_flags() {
        let profile = SandboxProfile {
            keep_terminal_session: true,
            share_ipc: true,
            allow_nested_userns: true,
            env_policy: EnvPolicy::Inherit,
            ..Default::default()
        };
        let args = bw("sh", &[], "/tmp", &profile);
        for flag in ["--new-session", "--unshare-ipc", "--disable-userns", "--clearenv"] {
            assert!(!args.iter().any(|a| a == flag), "{flag} present: {args:?}");
        }
        assert!(args.iter().any(|a| a == "--die-with-parent"));
    }

    #[test]
    fn missing_required_flag_is_an_error_not_a_downgrade() {
        let mut f = BwrapFeatures::all();
        f.clearenv = false;
        let e = build_bwrap_args("echo", &[], Path::new("/tmp"), &SandboxProfile::default(), &[], &f, None)
            .unwrap_err();
        assert!(matches!(e, SandboxError::BwrapTooOld { flag: "--clearenv" }), "{e}");
        let mut f = BwrapFeatures::all();
        f.seccomp = false;
        let e = build_bwrap_args("echo", &[], Path::new("/tmp"), &SandboxProfile::default(), &[], &f, Some(3))
            .unwrap_err();
        assert!(matches!(e, SandboxError::BwrapTooOld { flag: "--seccomp" }), "{e}");
    }

    #[test]
    fn optional_flags_skipped_when_unsupported() {
        let f = BwrapFeatures::from_help(
            "--clearenv --die-with-parent --new-session --unshare-ipc --unshare-uts --seccomp",
            false,
        );
        let args = build_bwrap_args("echo", &[], Path::new("/tmp"), &SandboxProfile::default(), &[], &f, None).unwrap();
        for flag in ["--cap-drop", "--disable-userns", "--unshare-cgroup-try", "--hostname"] {
            assert!(!args.iter().any(|a| a == flag), "{flag}");
        }
    }

    #[test]
    fn setuid_bwrap_gets_no_user_namespace_flags() {
        let mut f = BwrapFeatures::all();
        f.setuid = true;
        let args = build_bwrap_args("echo", &[], Path::new("/tmp"), &SandboxProfile::default(), &[], &f, None).unwrap();
        assert!(!args.iter().any(|a| a == "--unshare-user" || a == "--disable-userns"));
    }

    #[test]
    fn env_goes_through_setenv_before_separator() {
        let env = vec![("PATH".to_string(), "/x/bin:/usr/bin".to_string()), ("HOME".to_string(), "/tmp".to_string())];
        let args = build_bwrap_args("env", &[], Path::new("/tmp"), &SandboxProfile::default(), &env, &BwrapFeatures::all(), None).unwrap();
        let s = sep(&args);
        let clear = args.iter().position(|a| a == "--clearenv").unwrap();
        let sets = find(&args, "--setenv");
        assert_eq!(sets.len(), 2);
        for p in sets {
            assert!(p > clear && p < s);
        }
        let p = args.iter().position(|a| a == "/x/bin:/usr/bin").unwrap();
        assert_eq!(args[p - 1], "PATH");
    }

    #[test]
    fn seccomp_fd_passed_before_separator() {
        let args = build_bwrap_args("echo", &[], Path::new("/tmp"), &SandboxProfile::default(), &[], &BwrapFeatures::all(), Some(7)).unwrap();
        let p = args.iter().position(|a| a == "--seccomp").unwrap();
        assert_eq!(args[p + 1], "7");
        assert!(p < sep(&args));
    }

    /// BUCKETS-17 regression: session.rs used to append `--bind <overlay>
    /// /session/` AFTER the `--` separator, so it became program argv and the
    /// overlay was never mounted.
    #[test]
    fn custom_binds_come_before_separator() {
        let profile = SandboxProfile {
            binds: vec![
                BindMount::rw("/var/lib/buckets/sess/merged", "/session"),
                BindMount::ro("/var/lib/buckets/sess/upper", "/session-upper"),
            ],
            ..Default::default()
        };
        let args = bw("ls", &["/session"], "/tmp", &profile);
        let s = sep(&args);
        let p = args.iter().position(|a| a == "/session").unwrap();
        assert_eq!(args[p - 2], "--bind");
        assert_eq!(args[p - 1], "/var/lib/buckets/sess/merged");
        assert!(p < s);
        let q = args.iter().position(|a| a == "/session-upper").unwrap();
        assert_eq!(args[q - 2], "--ro-bind");
        assert!(q < s);
        assert_eq!(&args[s + 1..], &["ls".to_string(), "/session".to_string()]);
    }

    #[test]
    fn project_dir_gets_rw_bind() {
        let profile = SandboxProfile {
            project_dir: Some(PathBuf::from("/home/user/proj")),
            ..Default::default()
        };
        let args = bw("cargo", &["build"], "/home/user/proj", &profile);
        let bind_positions = find(&args, "--bind");
        assert_eq!(bind_positions.len(), 1);
        assert_eq!(args[bind_positions[0] + 1], "/home/user/proj");
        assert_eq!(args[bind_positions[0] + 2], "/home/user/proj");
    }

    #[test]
    fn no_project_dir_means_no_rw_bind() {
        let args = bw("node", &[], "/tmp", &SandboxProfile::default());
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
        let args = bw("node", &[], "/tmp", &profile);
        assert!(args.iter().any(|a| a == "/home/user/.buckets/nodejs.org/v20.20.2"));
        assert!(args.iter().any(|a| a == "/home/user/.buckets/openssl.org/v1.1.1w"));
    }

    #[test]
    fn program_and_args_come_after_separator() {
        let args = bw("node", &["-e", "1+1"], "/tmp", &SandboxProfile::default());
        let s = sep(&args);
        assert_eq!(args[s + 1], "node");
        assert_eq!(args[s + 2], "-e");
        assert_eq!(args[s + 3], "1+1");
        assert_eq!(args.len(), s + 4);
    }

    #[test]
    fn chdir_set_to_cwd() {
        let args = bw("echo", &[], "/some/cwd", &SandboxProfile::default());
        let pos = args.iter().position(|a| a == "--chdir").expect("--chdir present");
        assert_eq!(args[pos + 1], "/some/cwd");
    }

    #[test]
    fn clean_env_keeps_only_allowlist_home_path_and_composed() {
        let host = vec![
            ("BUCKETS_TEST_SECRET".to_string(), "1".to_string()),
            ("AWS_SECRET_ACCESS_KEY".to_string(), "x".to_string()),
            ("TERM".to_string(), "xterm".to_string()),
            ("LC_ALL".to_string(), "C.UTF-8".to_string()),
            ("HOME".to_string(), "/home/someone".to_string()),
        ];
        let mut composed = HashMap::new();
        composed.insert("PATH".to_string(), "/b/bin:/usr/bin".to_string());
        composed.insert("PYTHONPATH".to_string(), "/b/lib".to_string());
        let env: HashMap<_, _> = clean_env(&composed, SANDBOX_HOME, host).into_iter().collect();
        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm"));
        assert_eq!(env.get("LC_ALL").map(String::as_str), Some("C.UTF-8"));
        assert_eq!(env.get("HOME").map(String::as_str), Some("/tmp"));
        assert_eq!(env.get("PATH").map(String::as_str), Some("/b/bin:/usr/bin"));
        assert_eq!(env.get("PYTHONPATH").map(String::as_str), Some("/b/lib"));
        assert!(!env.contains_key("BUCKETS_TEST_SECRET"));
        assert!(!env.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert_eq!(env.len(), 5);
    }

    #[test]
    fn clean_env_defaults_path() {
        let env: HashMap<_, _> = clean_env(&HashMap::new(), "/tmp", Vec::new()).into_iter().collect();
        assert_eq!(env.get("PATH").map(String::as_str), Some(DEFAULT_PATH));
    }

    #[test]
    fn refuses_without_bwrap_unless_proot_opted_in() {
        let e = try_sandboxed_command_with(None, None, "true", &[], Path::new("/tmp"), &HashMap::new(), &SandboxProfile::default())
            .unwrap_err();
        assert!(matches!(e, SandboxError::BwrapNotFound { proot_on_path: false }));
        assert!(e.to_string().contains("--allow-weaker-sandbox"));
        let e = try_sandboxed_command_with(
            None,
            Some(PathBuf::from("/usr/bin/proot")),
            "true",
            &[],
            Path::new("/tmp"),
            &HashMap::new(),
            &SandboxProfile::default(),
        )
        .unwrap_err();
        assert!(matches!(e, SandboxError::BwrapNotFound { proot_on_path: true }));
        let profile = SandboxProfile { fallback: Fallback::Proot, ..Default::default() };
        let e = try_sandboxed_command_with(None, None, "true", &[], Path::new("/tmp"), &HashMap::new(), &profile).unwrap_err();
        assert!(matches!(e, SandboxError::NoBackend));
        let sb = try_sandboxed_command_with(
            None,
            Some(PathBuf::from("/usr/bin/proot")),
            "true",
            &[],
            Path::new("/tmp"),
            &HashMap::new(),
            &profile,
        )
        .unwrap();
        assert_eq!(sb.backend(), Backend::Proot);
    }

    #[test]
    fn limits_without_cgroup_parent_refuse() {
        if std::env::var_os("BUCKETS_CGROUP_PARENT").is_some() {
            return;
        }
        let profile = SandboxProfile {
            limits: Limits { pids: Some(10), ..Default::default() },
            fallback: Fallback::Proot,
            ..Default::default()
        };
        let e = try_sandboxed_command_with(
            None,
            Some(PathBuf::from("/usr/bin/proot")),
            "true",
            &[],
            Path::new("/tmp"),
            &HashMap::new(),
            &profile,
        )
        .unwrap_err();
        assert!(matches!(e, SandboxError::NoCgroupParent), "{e}");
    }

    #[test]
    fn seccomp_programs_compile_and_differ() {
        assert!(seccomp_program(SeccompMode::Off).unwrap().is_none());
        let d = seccomp_program(SeccompMode::Default).unwrap().unwrap();
        let s = seccomp_program(SeccompMode::Strict).unwrap().unwrap();
        assert_eq!(d.len() % 8, 0);
        assert_eq!(s.len() % 8, 0);
        assert!(d.len() > s.len(), "default (with sockets) should allow more than strict");
    }

    #[test]
    fn seccomp_mode_parses() {
        assert_eq!("off".parse::<SeccompMode>().unwrap(), SeccompMode::Off);
        assert_eq!("default".parse::<SeccompMode>().unwrap(), SeccompMode::Default);
        assert_eq!("strict".parse::<SeccompMode>().unwrap(), SeccompMode::Strict);
        assert!("lax".parse::<SeccompMode>().is_err());
    }

    #[test]
    fn size_cpu_duration_parsers() {
        assert_eq!(parse_size("512M").unwrap(), 512 << 20);
        assert_eq!(parse_size("1G").unwrap(), 1 << 30);
        assert_eq!(parse_size("64MiB").unwrap(), 64 << 20);
        assert_eq!(parse_size("4096").unwrap(), 4096);
        assert_eq!(parse_size("1.5G").unwrap(), 3 << 29);
        assert!(parse_size("12Q").is_err());
        assert!(parse_size("0").is_err());
        assert_eq!(parse_cpu_percent("50%").unwrap(), 50);
        assert_eq!(parse_cpu_percent("150").unwrap(), 150);
        assert!(parse_cpu_percent("0%").is_err());
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("3").unwrap(), Duration::from_secs(3));
        assert!(parse_duration("3d").is_err());
    }

    #[test]
    fn refusal_command_runs_nothing_and_exits_126() {
        let out = refusal_command(&SandboxError::BwrapNotFound { proot_on_path: false })
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(126));
        assert!(String::from_utf8_lossy(&out.stderr).contains("bwrap (bubblewrap) not found"));
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
            extra_ro_binds: vec![PathBuf::from("/home/user/.buckets/nodejs.org/v20.20.2")],
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
    fn proot_custom_binds_use_dest() {
        let profile = SandboxProfile { binds: vec![BindMount::rw("/a/merged", "/session")], ..Default::default() };
        let args = build_proot_args("ls", &[], Path::new("/tmp"), &profile);
        assert!(args.iter().any(|a| a == "/a/merged:/session"));
    }

    #[test]
    fn proot_program_and_args_come_at_the_end() {
        let profile = SandboxProfile::default();
        let args = build_proot_args("node", &["-e".to_string(), "1+1".to_string()], Path::new("/tmp"), &profile);
        let len = args.len();
        assert_eq!(args[len - 3], "node");
        assert_eq!(args[len - 2], "-e");
        assert_eq!(args[len - 1], "1+1");
    }

    #[test]
    fn proot_execution_smoke_test() {
        if which_proot().is_none() {
            return;
        }
        let profile = SandboxProfile::default();
        let args = build_proot_args("true", &[], Path::new("/tmp"), &profile);
        let status = Command::new("proot")
            .args(&args)
            .status()
            .expect("Failed to execute proot command");
        assert!(status.success());
    }
}

/// Live tests: really spawn bwrap. Skipped (early return, with a note on
/// stderr) when bwrap isn't installed. cgroup tests need
/// `BUCKETS_CGROUP_PARENT` (a delegated cgroup v2 dir) and are `#[ignore]`d.
#[cfg(test)]
mod live {
    use super::*;
    use std::process::Stdio;

    fn have_bwrap() -> bool {
        let ok = which_bwrap().is_some();
        if !ok {
            eprintln!("bwrap not installed — skipping live sandbox test");
        }
        ok
    }

    fn sh(script: &str, profile: &SandboxProfile, env: &HashMap<String, String>) -> (RunOutcome, String, String) {
        let args = vec!["-c".to_string(), script.to_string()];
        let mut sb = try_sandboxed_command("/bin/sh", &args, Path::new("/tmp"), env, profile).expect("sandbox");
        sb.command_mut().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = sb.spawn().expect("spawn");
        // Drain both pipes on threads so `wait` (and its deadline) runs now.
        fn drain(r: Option<impl std::io::Read + Send + 'static>) -> std::thread::JoinHandle<String> {
            std::thread::spawn(move || {
                let mut s = String::new();
                if let Some(mut r) = r {
                    let _ = r.read_to_string(&mut s);
                }
                s
            })
        }
        let o = drain(child.child.stdout.take());
        let e = drain(child.child.stderr.take());
        let outcome = child.wait().expect("wait");
        (outcome, o.join().unwrap(), e.join().unwrap())
    }

    fn env_with_path(path: &str) -> HashMap<String, String> {
        HashMap::from([("PATH".to_string(), path.to_string())])
    }

    #[test]
    fn host_env_does_not_leak_composed_env_does() {
        if !have_bwrap() {
            return;
        }
        // The variable lives in this test process's environment, so bwrap
        // would inherit it without --clearenv / env_clear.
        // SAFETY: test-only; the name is unique to this test.
        unsafe { std::env::set_var("BUCKETS_TEST_SECRET", "1") };
        let (o, out, err) = sh(
            "echo SECRET=${BUCKETS_TEST_SECRET:-unset}; echo PATH=$PATH; echo HOME=$HOME",
            &SandboxProfile::default(),
            &env_with_path("/opt/bucket/bin:/usr/bin:/bin"),
        );
        assert!(o.status.success(), "{err}");
        assert!(out.contains("SECRET=unset"), "{out}");
        assert!(out.contains("PATH=/opt/bucket/bin:/usr/bin:/bin"), "{out}");
        assert!(out.contains("HOME=/tmp"), "{out}");

        let inherit = SandboxProfile { env_policy: EnvPolicy::Inherit, ..Default::default() };
        let (_, out, _) = sh("echo SECRET=${BUCKETS_TEST_SECRET:-unset}", &inherit, &HashMap::new());
        assert!(out.contains("SECRET=1"), "inherit should pass host env: {out}");
    }

    #[test]
    fn capabilities_empty_inside() {
        if !have_bwrap() {
            return;
        }
        let (o, out, err) = sh("grep Cap /proc/self/status", &SandboxProfile::default(), &HashMap::new());
        assert!(o.status.success(), "{err}");
        for line in out.lines().filter(|l| l.starts_with("CapInh") || l.starts_with("CapPrm") || l.starts_with("CapEff") || l.starts_with("CapAmb")) {
            assert!(line.ends_with("0000000000000000"), "{line}");
        }
        assert!(out.contains("CapEff"), "{out}");
    }

    #[test]
    fn uts_and_ipc_are_unshared() {
        if !have_bwrap() {
            return;
        }
        let host_ipc = std::fs::read_link("/proc/self/ns/ipc").unwrap();
        let host_uts = std::fs::read_link("/proc/self/ns/uts").unwrap();
        let host_name = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap();
        let (o, out, err) = sh(
            "readlink /proc/self/ns/ipc; readlink /proc/self/ns/uts; cat /proc/sys/kernel/hostname",
            &SandboxProfile::default(),
            &HashMap::new(),
        );
        assert!(o.status.success(), "{err}");
        let lines: Vec<&str> = out.lines().collect();
        assert_ne!(lines[0], host_ipc.to_string_lossy(), "ipc ns shared");
        assert_ne!(lines[1], host_uts.to_string_lossy(), "uts ns shared");
        assert_eq!(lines[2], "bucket");
        // The sandbox's hostname never reached the host.
        assert_eq!(std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap(), host_name);
    }

    #[test]
    fn new_session_makes_command_session_leader() {
        if !have_bwrap() {
            return;
        }
        // With --new-session bwrap calls setsid() inside the sandbox, so
        // the session leader is a process of the sandbox's PID namespace
        // (non-zero sid). Without it the session leader is the caller's,
        // outside the namespace, and the sid reads as 0.
        let script = "cut -d' ' -f6 /proc/self/stat";
        let (o, out, err) = sh(script, &SandboxProfile::default(), &HashMap::new());
        assert!(o.status.success(), "{err}");
        assert_ne!(out.trim(), "0", "still in the caller's session: {out}");
        let keep = SandboxProfile { keep_terminal_session: true, ..Default::default() };
        let (_, out, _) = sh(script, &keep, &HashMap::new());
        assert_eq!(out.trim(), "0", "keep_terminal_session should stay in the caller's session: {out}");
    }

    /// `ptrace(PTRACE_TRACEME)` succeeds for any unprivileged process, so
    /// EPERM can only come from the filter.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn seccomp_default_denies_ptrace_allows_threads() {
        if !have_bwrap() || which("python3").is_none() {
            return;
        }
        let script = "import ctypes, os, threading, subprocess\n\
                      t = threading.Thread(target=lambda: None); t.start(); t.join()\n\
                      subprocess.run(['true'], check=True)\n\
                      libc = ctypes.CDLL(None, use_errno=True)\n\
                      r = libc.syscall(101, 0, 0, 0, 0)\n\
                      print('ptrace', r, os.strerror(ctypes.get_errno()) if r else 'ok')\n";
        let args = vec!["-c".to_string(), script.to_string()];
        let run = |profile: &SandboxProfile| {
            let mut sb = try_sandboxed_command("python3", &args, Path::new("/tmp"), &env_with_path("/usr/bin:/bin"), profile).unwrap();
            sb.command_mut().stdin(Stdio::null());
            let out = sb.into_command().output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        let filtered = run(&SandboxProfile::default());
        assert!(filtered.contains("ptrace -1 Operation not permitted"), "{filtered}");
        let unfiltered = run(&SandboxProfile { seccomp: SeccompMode::Off, ..Default::default() });
        assert!(unfiltered.contains("ptrace 0 ok"), "{unfiltered}");
    }

    #[test]
    fn seccomp_strict_kills_on_socket() {
        if !have_bwrap() || which("python3").is_none() {
            return;
        }
        let strict = SandboxProfile { seccomp: SeccompMode::Strict, ..Default::default() };
        let args = vec!["-c".to_string(), "import socket; socket.socket()".to_string()];
        let mut sb = try_sandboxed_command("python3", &args, Path::new("/tmp"), &env_with_path("/usr/bin:/bin"), &strict).unwrap();
        sb.command_mut().stdin(Stdio::null()).stderr(Stdio::null());
        let o = sb.status().unwrap();
        assert!(!o.status.success(), "socket() should be fatal under strict");
    }

    #[test]
    fn custom_bind_visible_inside() {
        if !have_bwrap() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("marker"), "overlay-ok").unwrap();
        let profile = SandboxProfile { binds: vec![BindMount::rw(dir.path(), "/session")], ..Default::default() };
        let (o, out, err) = sh("cat /session/marker && echo w > /session/written", &profile, &HashMap::new());
        assert!(o.status.success(), "{err}");
        assert_eq!(out.trim(), "overlay-ok");
        assert!(dir.path().join("written").exists());
    }

    #[test]
    fn timeout_kills_whole_tree() {
        if !have_bwrap() {
            return;
        }
        let profile = SandboxProfile {
            limits: Limits { timeout: Some(Duration::from_secs(2)), ..Default::default() },
            ..Default::default()
        };
        let t0 = Instant::now();
        let (o, _, _) = sh("sleep 60 & sleep 61; wait", &profile, &HashMap::new());
        let took = t0.elapsed();
        assert!(o.timed_out);
        assert!(took < Duration::from_secs(10), "{took:?}");
        // No `sleep 60`/`sleep 61` left behind on the host.
        let left = Command::new("pgrep").args(["-f", "^sleep 6[01]$"]).output().unwrap();
        assert!(left.stdout.is_empty(), "stragglers: {}", String::from_utf8_lossy(&left.stdout));
    }

    fn cgroup_parent() -> PathBuf {
        PathBuf::from(std::env::var_os("BUCKETS_CGROUP_PARENT").expect("BUCKETS_CGROUP_PARENT"))
    }

    #[test]
    #[ignore = "needs BUCKETS_CGROUP_PARENT (delegated cgroup v2 dir)"]
    fn cgroup_pids_caps_fork_bomb() {
        assert!(have_bwrap());
        let profile = SandboxProfile {
            limits: Limits { pids: Some(20), cgroup_parent: Some(cgroup_parent()), timeout: Some(Duration::from_secs(20)), ..Default::default() },
            ..Default::default()
        };
        if which("python3").is_none() {
            return;
        }
        // Fork bomb that holds its children: python raises on EAGAIN
        // instead of retrying the way bash does.
        let script = "import os, time\n\
                      kids = []\n\
                      try:\n    for _ in range(200):\n        pid = os.fork()\n        if pid == 0:\n            time.sleep(30); os._exit(0)\n        kids.append(pid)\n\
                      except OSError as e:\n    print('blocked after', len(kids), e.errno)\n\
                      else:\n    print('unbounded', len(kids))\n\
                      for k in kids: os.kill(k, 9)\n";
        let args = vec!["-c".to_string(), script.to_string()];
        let mut sb = try_sandboxed_command("python3", &args, Path::new("/tmp"), &env_with_path("/usr/bin:/bin"), &profile).unwrap();
        sb.command_mut().stdin(Stdio::null()).stdout(Stdio::piped());
        let out = sb.into_command().output().unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(s.starts_with("blocked after"), "pids.max=20 not enforced: {s}");
        let n: u32 = s.split_whitespace().nth(2).unwrap().parse().unwrap();
        assert!(n < 20, "{s}");
    }

    #[test]
    #[ignore = "needs BUCKETS_CGROUP_PARENT (delegated cgroup v2 dir)"]
    fn cgroup_memory_caps_allocation() {
        assert!(have_bwrap());
        if which("python3").is_none() {
            return;
        }
        let profile = SandboxProfile {
            limits: Limits { memory_bytes: Some(64 << 20), cgroup_parent: Some(cgroup_parent()), timeout: Some(Duration::from_secs(30)), ..Default::default() },
            ..Default::default()
        };
        let args = vec!["-c".to_string(), "b = b'x' * (512 << 20); print('allocated', len(b))".to_string()];
        let mut sb = try_sandboxed_command("python3", &args, Path::new("/tmp"), &env_with_path("/usr/bin:/bin"), &profile).unwrap();
        sb.command_mut().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        let cg = sb.cgroup_path().map(Path::to_path_buf).expect("cgroup created");
        assert_eq!(std::fs::read_to_string(cg.join("memory.max")).unwrap().trim(), (64u64 << 20).to_string());
        if cg.join("memory.swap.max").exists() {
            assert_eq!(std::fs::read_to_string(cg.join("memory.swap.max")).unwrap().trim(), "0");
        }
        let o = sb.status().unwrap();
        assert!(!o.status.success(), "512 MiB allocation should fail under memory.max=64M");
    }

    #[test]
    #[ignore = "needs BUCKETS_CGROUP_PARENT (delegated cgroup v2 dir)"]
    fn cgroup_cpu_max_written_and_removed() {
        assert!(have_bwrap());
        let profile = SandboxProfile {
            limits: Limits { cpu_percent: Some(50), cgroup_parent: Some(cgroup_parent()), ..Default::default() },
            ..Default::default()
        };
        let mut sb = try_sandboxed_command("/bin/sh", &["-c".into(), "cat /proc/self/cgroup".into()], Path::new("/tmp"), &HashMap::new(), &profile).unwrap();
        sb.command_mut().stdin(Stdio::null()).stdout(Stdio::null());
        let cg = sb.cgroup_path().map(Path::to_path_buf).unwrap();
        assert_eq!(std::fs::read_to_string(cg.join("cpu.max")).unwrap().trim(), "50000 100000");
        let o = sb.status().unwrap();
        assert!(o.status.success());
        assert!(!cg.exists(), "per-run cgroup left behind: {}", cg.display());
    }
}
