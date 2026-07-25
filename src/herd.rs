//! Buck-herd: replica management and health reconciliation for bucket fleets.
//!
//! Implements the same desired-state reconciliation pattern as mandala's
//! [`ReconciliationLoop`](https://github.com/nixpt/mandala) but entirely
//! within the buckets crate — no async runtime, no heavy dependencies. A
//! background thread polls each replica's PID every `check_interval` seconds
//! and restarts dead instances with exponential backoff.
//!
//! ## Usage
//!
//! ```bash
//! # Deploy 5 node@20 workers sharing a virtual network
//! buckets net create herd-net
//! buckets herd deploy worker --spec node@20 --replicas 5 --net herd-net -- node worker.js
//!
//! # Inspect the fleet
//! buckets herd ls
//! buckets herd status worker
//!
//! # Scale up/down live (talks to deploy via Unix control socket)
//! buckets herd scale worker --replicas 8
//!
//! # Tear down (IPC stop preferred; kills + cleans state)
//! buckets herd stop worker
//! ```
//!
//! ## State
//!
//! Herd state is persisted to `cache_dir/herds/{name}/state.json`. The
//! reconciliation thread reads this file so it survives config changes. The
//! deploy process also binds `cache_dir/herds/{name}/control.sock` so
//! cross-process `scale` / `status` / `stop` can reach the live controller
//! (BUCKETS-14). When the process that ran `buckets herd deploy` exits, the
//! background reconciler stops — herds are session-scoped unless wrapped in
//! a supervisor (systemd, tmux, etc.).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ── Public types ─────────────────────────────────────────────────────────────

/// Desired configuration for a named herd.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HerdSpec {
    pub name: String,
    /// Bucket spec string: "node@20", "python@3.11", "local/my-app"
    pub bucket: String,
    /// Command to run inside each bucket
    pub command: Vec<String>,
    /// Environment overrides for each instance
    pub env: HashMap<String, String>,
    /// Number of desired replicas
    pub replicas: u32,
    /// Optional named buck-net (all replicas share the same net)
    pub net: Option<String>,
    /// How often the reconciler checks instance health (seconds)
    #[serde(default = "default_check_interval")]
    pub check_interval_secs: u64,
    /// Restart policy
    #[serde(default)]
    pub restart: RestartPolicy,
    /// Maximum restarts before marking an instance permanently failed
    #[serde(default = "default_max_restarts")]
    pub max_restarts: u32,
}

fn default_check_interval() -> u64 { 5 }
fn default_max_restarts() -> u32 { 10 }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    #[default]
    OnFailure,
    Always,
    Never,
}

/// Snapshot of a single replica's runtime state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstanceState {
    pub index: u32,
    pub pid: Option<u32>,
    pub status: InstanceStatus,
    pub restart_count: u32,
    pub last_exit_code: Option<i32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStatus {
    Running,
    Restarting,
    Failed,
    Stopped,
}

/// Persisted herd state.
#[derive(Debug, Serialize, Deserialize)]
pub struct HerdState {
    pub spec: HerdSpec,
    pub instances: Vec<InstanceState>,
}

// ── HerdController ───────────────────────────────────────────────────────────

/// Runtime controller for a named herd. Holds the live child processes and
/// runs a background reconciliation thread.
#[derive(Clone, Debug)]
pub struct HerdController {
    pub name: String,
    state: Arc<Mutex<InternalState>>,
    state_dir: PathBuf,
}

struct InternalState {
    spec: HerdSpec,
    /// Live child handles indexed by replica index
    children: HashMap<u32, ChildEntry>,
}

impl std::fmt::Debug for InternalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InternalState")
            .field("spec", &self.spec)
            .field("children_count", &self.children.len())
            .finish()
    }
}

struct ChildEntry {
    child: Child,
    restart_count: u32,
    last_backoff: Duration,
    status: InstanceStatus,
    failed_at: Option<Instant>,
}

impl ChildEntry {
    fn pid(&self) -> Option<u32> {
        Some(self.child.id())
    }
}

impl HerdController {
    /// Create a new herd: spawn all replicas and start the reconciler.
    pub fn create(spec: HerdSpec, herds_dir: &Path) -> Result<Self> {
        let state_dir = herds_dir.join(&spec.name);
        if state_dir.exists() {
            bail!(
                "Herd '{}' already exists — stop it first with 'buckets herd stop {}'",
                spec.name, spec.name
            );
        }

        // Verify buck-net exists before spawning
        if let Some(ref net) = spec.net {
            let net_ns = format!("/proc");
            let _ = net_ns; // checked at spawn time via load
            eprintln!("▶ herd '{}' will use buck-net '{}'", spec.name, net);
        }

        fs::create_dir_all(&state_dir)?;

        let state = Arc::new(Mutex::new(InternalState {
            spec: spec.clone(),
            children: HashMap::new(),
        }));

        let ctrl = HerdController {
            name: spec.name.clone(),
            state: state.clone(),
            state_dir: state_dir.clone(),
        };

        // Spawn initial replicas
        {
            let mut s = state.lock().unwrap();
            for idx in 0..spec.replicas {
                match spawn_replica(&s.spec, idx) {
                    Ok(child) => {
                        s.children.insert(idx, ChildEntry {
                            child,
                            restart_count: 0,
                            last_backoff: Duration::from_secs(1),
                            status: InstanceStatus::Running,
                            failed_at: None,
                        });
                        eprintln!(
                            "  ✓ replica {}/{} started",
                            idx + 1,
                            spec.replicas
                        );
                    }
                    Err(e) => {
                        eprintln!("  ✗ replica {}/{} failed to start: {e}", idx + 1, spec.replicas);
                    }
                }
            }
        }

        ctrl.persist_state()?;

        eprintln!("✓ herd '{}' running ({} replicas)", spec.name, spec.replicas);
        Ok(ctrl)
    }

    /// Get a snapshot of current herd state for display.
    ///
    /// In-process API: operates on the live `Child` handles held by *this*
    /// controller, so it only works in the process that ran `buckets herd
    /// deploy`. The `buckets herd status` CLI subcommand is a *separate*
    /// process and cannot reach this controller's memory — it correctly
    /// reads `state.json` instead.
    ///
    /// Used by `deploy`'s shutdown path to log live instance status before
    /// calling `stop()`.
    pub fn snapshot(&self) -> Vec<InstanceState> {
        let mut s = self.state.lock().unwrap();
        let spec = s.spec.clone();
        (0..spec.replicas)
            .map(|idx| {
                if let Some(entry) = s.children.get_mut(&idx) {
                    // Poll for exit without blocking
                    let exited = entry.child.try_wait().ok().flatten().is_some();
                    let status = if exited {
                        InstanceStatus::Failed
                    } else {
                        entry.status
                    };
                    InstanceState {
                        index: idx,
                        pid: entry.pid(),
                        status,
                        restart_count: entry.restart_count,
                        last_exit_code: None,
                    }
                } else {
                    InstanceState {
                        index: idx,
                        pid: None,
                        status: InstanceStatus::Stopped,
                        restart_count: 0,
                        last_exit_code: None,
                    }
                }
            })
            .collect()
    }

    /// Run the reconciliation loop on the calling thread (blocks until stopped).
    /// Call this in a background thread.
    pub fn run_reconciler(self: &Arc<Self>, stop_signal: Arc<std::sync::atomic::AtomicBool>) {
        eprintln!("▶ reconciler for herd '{}' started", self.name);

        loop {
            if stop_signal.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }

            let check_interval = {
                let s = self.state.lock().unwrap();
                Duration::from_secs(s.spec.check_interval_secs)
            };
            std::thread::sleep(check_interval);

            if stop_signal.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }

            self.reconcile_tick();
            let _ = self.persist_state();
        }

        eprintln!("▶ reconciler for herd '{}' stopped", self.name);
    }

    fn reconcile_tick(&self) {
        let mut s = self.state.lock().unwrap();
        let desired = s.spec.replicas;
        let policy = s.spec.restart;
        let max_restarts = s.spec.max_restarts;
        let herd_name = s.spec.name.clone(); // Clone up-front to avoid borrow conflicts

        let indices: Vec<u32> = (0..desired).collect();
        for idx in indices {
            let entry = s.children.get_mut(&idx);
            match entry {
                None => {
                    // Replica slot has no child — spawn if desired
                    if policy != RestartPolicy::Never {
                        match spawn_replica(&s.spec, idx) {
                            Ok(child) => {
                                s.children.insert(idx, ChildEntry {
                                    child,
                                    restart_count: 0,
                                    last_backoff: Duration::from_secs(1),
                                    status: InstanceStatus::Running,
                                    failed_at: None,
                                });
                                eprintln!("  ↺ herd '{herd_name}' replica {idx}: spawned");
                            }
                            Err(e) => eprintln!("  ✗ herd '{herd_name}' replica {idx}: spawn failed: {e}"),
                        }
                    }
                }
                Some(entry) => {
                    // Poll for exit
                    let exit_status = entry.child.try_wait().ok().flatten();
                    if exit_status.is_none() {
                        // Still running
                        entry.status = InstanceStatus::Running;
                        continue;
                    }

                    // Instance exited
                    let code = exit_status.and_then(|s| s.code());
                    let should_restart = match policy {
                        RestartPolicy::Never => false,
                        RestartPolicy::Always => true,
                        RestartPolicy::OnFailure => code != Some(0),
                    };

                    if !should_restart || entry.restart_count >= max_restarts {
                        let restart_count = entry.restart_count;
                        eprintln!(
                            "  ✗ herd '{herd_name}' replica {idx}: permanently failed (restarts: {restart_count})"
                        );
                        entry.status = InstanceStatus::Failed;
                        entry.failed_at = Some(Instant::now());
                        continue;
                    }

                    let backoff = entry.last_backoff;
                    let backoff_secs = backoff.as_secs();
                    let restart_count = entry.restart_count;
                    entry.last_backoff = (backoff * 2).min(Duration::from_secs(60));
                    entry.restart_count += 1;
                    entry.status = InstanceStatus::Restarting;

                    eprintln!(
                        "  ↺ herd '{herd_name}' replica {idx}: restarting (attempt {restart_count}, backoff {backoff_secs}s)"
                    );

                    // Drop the mutex lock to sleep (can't hold lock across sleep)
                    drop(s);
                    std::thread::sleep(backoff);
                    s = self.state.lock().unwrap();

                    match spawn_replica(&s.spec, idx) {
                        Ok(child) => {
                            if let Some(e) = s.children.get_mut(&idx) {
                                e.child = child;
                                e.status = InstanceStatus::Running;
                            }
                        }
                        Err(err) => {
                            eprintln!("  ✗ herd '{herd_name}' replica {idx}: restart failed: {err}");
                        }
                    }
                }
            }
        }
    }

    /// Scale the herd to a new replica count.
    ///
    /// In-process API: only the process holding this controller can apply a
    /// live hot-scale (it owns the `Child` handles). Cross-process
    /// `buckets herd scale` reaches this method via the Unix-socket IPC
    /// served by [`serve_control`] (BUCKETS-14).
    pub fn scale(&self, new_replicas: u32) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        let current = s.spec.replicas;
        s.spec.replicas = new_replicas;

        if new_replicas > current {
            // Spawn additional replicas
            for idx in current..new_replicas {
                match spawn_replica(&s.spec, idx) {
                    Ok(child) => {
                        s.children.insert(idx, ChildEntry {
                            child,
                            restart_count: 0,
                            last_backoff: Duration::from_secs(1),
                            status: InstanceStatus::Running,
                            failed_at: None,
                        });
                        eprintln!("  ✓ scale up: replica {idx} started");
                    }
                    Err(e) => eprintln!("  ✗ scale up: replica {idx} failed: {e}"),
                }
            }
        } else {
            // Kill extra replicas
            for idx in new_replicas..current {
                if let Some(mut entry) = s.children.remove(&idx) {
                    let _ = entry.child.kill();
                    let _ = entry.child.wait();
                    eprintln!("  ✓ scale down: replica {idx} stopped");
                }
            }
        }
        drop(s);
        self.persist_state()
    }

    /// Stop all replicas and clean up.
    ///
    /// In-process API: consumes `self` (owns the `Child` handles). Called by
    /// `buckets herd deploy`'s shutdown path after the reconciler thread exits
    /// to kill any surviving replicas and remove the state directory.
    pub fn stop(self) -> Result<()> {
        eprintln!("▶ stopping herd '{}'", self.name);
        let mut s = self.state.lock().unwrap();
        for (idx, mut entry) in s.children.drain() {
            let _ = entry.child.kill();
            let _ = entry.child.wait();
            eprintln!("  ✓ replica {idx} stopped");
        }
        drop(s);
        let _ = fs::remove_dir_all(&self.state_dir);
        eprintln!("✓ herd '{}' stopped", self.name);
        Ok(())
    }

    fn persist_state(&self) -> Result<()> {
        let s = self.state.lock().unwrap();
        let instances: Vec<InstanceState> = (0..s.spec.replicas)
            .map(|idx| {
                s.children.get(&idx).map(|e| InstanceState {
                    index: idx,
                    pid: e.pid(),
                    status: e.status,
                    restart_count: e.restart_count,
                    last_exit_code: None,
                }).unwrap_or(InstanceState {
                    index: idx, pid: None,
                    status: InstanceStatus::Stopped,
                    restart_count: 0, last_exit_code: None,
                })
            })
            .collect();
        let state = HerdState { spec: s.spec.clone(), instances };
        drop(s);
        fs::write(
            self.state_dir.join("state.json"),
            serde_json::to_string_pretty(&state)?,
        ).context("Failed to persist herd state")
    }
}

// ── Unix-socket IPC (BUCKETS-14) ─────────────────────────────────────────────

/// Path to the control socket for a named herd.
pub fn control_sock_path(herds_dir: &Path, name: &str) -> PathBuf {
    herds_dir.join(name).join("control.sock")
}

/// One-shot request from a CLI process to the deploy process.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum HerdRequest {
    Scale { replicas: u32 },
    Status,
    Stop,
}

/// Response from the deploy process.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HerdResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replicas: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instances: Option<Vec<InstanceState>>,
}

impl HerdResponse {
    fn ok_empty() -> Self {
        Self {
            ok: true,
            error: None,
            name: None,
            bucket: None,
            replicas: None,
            net: None,
            instances: None,
        }
    }

    fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(msg.into()),
            name: None,
            bucket: None,
            replicas: None,
            net: None,
            instances: None,
        }
    }
}

/// Serve control requests until `stop_signal` is set. Binds
/// `{state_dir}/control.sock` and unlinks it on exit.
///
/// Intended to run on a background thread owned by `buckets herd deploy`.
pub fn serve_control(ctrl: Arc<HerdController>, stop_signal: Arc<AtomicBool>) {
    let sock = ctrl.state_dir.join("control.sock");
    let _ = fs::remove_file(&sock);

    let listener = match UnixListener::bind(&sock) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("  ✗ herd '{}': failed to bind control socket {}: {e}", ctrl.name, sock.display());
            return;
        }
    };
    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!("  ✗ herd '{}': control socket nonblocking failed: {e}", ctrl.name);
        let _ = fs::remove_file(&sock);
        return;
    }
    eprintln!("▶ herd '{}' control socket: {}", ctrl.name, sock.display());

    while !stop_signal.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(e) = handle_control_client(&ctrl, stream, &stop_signal) {
                    eprintln!("  ✗ herd '{}': control client error: {e}", ctrl.name);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                eprintln!("  ✗ herd '{}': control accept error: {e}", ctrl.name);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    let _ = fs::remove_file(&sock);
    eprintln!("▶ herd '{}' control socket closed", ctrl.name);
}

fn handle_control_client(
    ctrl: &HerdController,
    stream: UnixStream,
    stop_signal: &Arc<AtomicBool>,
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let line = line.trim();
    if line.is_empty() {
        return Ok(());
    }

    let req: HerdRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            write_response(&stream, &HerdResponse::err(format!("invalid request: {e}")))?;
            return Ok(());
        }
    };

    let resp = match req {
        HerdRequest::Scale { replicas } => match ctrl.scale(replicas) {
            Ok(()) => {
                eprintln!("  ✓ herd '{}': scaled to {replicas} via IPC", ctrl.name);
                let mut r = HerdResponse::ok_empty();
                r.name = Some(ctrl.name.clone());
                r.replicas = Some(replicas);
                r.instances = Some(ctrl.snapshot());
                r
            }
            Err(e) => HerdResponse::err(e.to_string()),
        },
        HerdRequest::Status => {
            let s = ctrl.state.lock().unwrap();
            let mut r = HerdResponse::ok_empty();
            r.name = Some(s.spec.name.clone());
            r.bucket = Some(s.spec.bucket.clone());
            r.replicas = Some(s.spec.replicas);
            r.net = s.spec.net.clone();
            drop(s);
            r.instances = Some(ctrl.snapshot());
            r
        }
        HerdRequest::Stop => {
            eprintln!("  ✓ herd '{}': stop requested via IPC", ctrl.name);
            stop_signal.store(true, Ordering::SeqCst);
            let mut r = HerdResponse::ok_empty();
            r.name = Some(ctrl.name.clone());
            r
        }
    };

    write_response(&stream, &resp)
}

fn write_response(mut stream: &UnixStream, resp: &HerdResponse) -> Result<()> {
    let mut payload = serde_json::to_string(resp)?;
    payload.push('\n');
    stream.write_all(payload.as_bytes())?;
    stream.flush()?;
    Ok(())
}

/// Send a one-shot control request to a running herd's deploy process.
pub fn send_control(herds_dir: &Path, name: &str, req: HerdRequest) -> Result<HerdResponse> {
    let sock = control_sock_path(herds_dir, name);
    if !sock.exists() {
        bail!(
            "Herd '{name}' control socket not found at {} — is 'buckets herd deploy' still running?",
            sock.display()
        );
    }

    let mut stream = UnixStream::connect(&sock)
        .with_context(|| format!("Failed to connect to herd '{name}' control socket {}", sock.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;

    let mut payload = serde_json::to_string(&req)?;
    payload.push('\n');
    stream.write_all(payload.as_bytes())?;
    stream.flush()?;

    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("Failed to read control response")?;
    if line.trim().is_empty() {
        bail!("Empty response from herd '{name}' control socket");
    }
    let resp: HerdResponse = serde_json::from_str(line.trim())
        .context("Failed to parse control response")?;
    if !resp.ok {
        bail!(
            "Herd '{name}' control error: {}",
            resp.error.as_deref().unwrap_or("unknown")
        );
    }
    Ok(resp)
}

// ── Spawn helper ──────────────────────────────────────────────────────────────

fn spawn_replica(spec: &HerdSpec, idx: u32) -> Result<Child> {
    let mut cmd = Command::new("buckets");
    cmd.arg("run");

    if let Some(ref net) = spec.net {
        cmd.arg("--net").arg(net);
    }

    cmd.arg(&spec.bucket);

    if !spec.command.is_empty() {
        cmd.arg("--");
        cmd.args(&spec.command);
    }

    for (k, v) in &spec.env {
        cmd.env(k, v);
    }

    // Tag replica index in environment for apps that want it
    cmd.env("HERD_REPLICA_INDEX", idx.to_string());
    cmd.env("HERD_NAME", &spec.name);

    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    cmd.stdin(Stdio::null());

    cmd.spawn().with_context(|| {
        format!("Failed to spawn replica {idx} of herd '{}' (buckets run {} ...)", spec.name, spec.bucket)
    })
}

// ── Listing helpers ───────────────────────────────────────────────────────────

/// Info about a persisted herd (from state.json, no live process check).
#[derive(Debug)]
pub struct HerdInfo {
    pub name: String,
    pub bucket: String,
    pub replicas: u32,
    pub net: Option<String>,
    pub instances: Vec<InstanceState>,
}

/// List all persisted herds under `herds_dir`.
pub fn list_all(herds_dir: &Path) -> Vec<HerdInfo> {
    if !herds_dir.exists() { return vec![]; }
    let Ok(entries) = fs::read_dir(herds_dir) else { return vec![] };
    let mut result = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) { continue; }
        let state_path = entry.path().join("state.json");
        let Ok(content) = fs::read_to_string(&state_path) else { continue };
        let Ok(state) = serde_json::from_str::<HerdState>(&content) else { continue };
        result.push(HerdInfo {
            name: state.spec.name.clone(),
            bucket: state.spec.bucket.clone(),
            replicas: state.spec.replicas,
            net: state.spec.net.clone(),
            instances: state.instances,
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn herd_spec_defaults() {
        let spec = serde_json::from_str::<HerdSpec>(r#"{
            "name": "test",
            "bucket": "node@20",
            "command": [],
            "env": {},
            "replicas": 3,
            "net": null
        }"#).unwrap();
        assert_eq!(spec.check_interval_secs, 5);
        assert_eq!(spec.max_restarts, 10);
        assert_eq!(spec.restart, RestartPolicy::OnFailure);
    }

    #[test]
    fn list_all_empty_for_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list_all(&dir.path().join("herds")).is_empty());
    }

    #[test]
    fn control_sock_path_joins_herd_name() {
        let p = control_sock_path(Path::new("/cache/herds"), "worker");
        assert_eq!(p, PathBuf::from("/cache/herds/worker/control.sock"));
    }

    #[test]
    fn herd_request_roundtrip() {
        let cases = [
            HerdRequest::Scale { replicas: 8 },
            HerdRequest::Status,
            HerdRequest::Stop,
        ];
        for req in cases {
            let json = serde_json::to_string(&req).unwrap();
            let back: HerdRequest = serde_json::from_str(&json).unwrap();
            assert_eq!(back, req);
        }
        assert_eq!(
            serde_json::from_str::<HerdRequest>(r#"{"cmd":"scale","replicas":3}"#).unwrap(),
            HerdRequest::Scale { replicas: 3 }
        );
        assert_eq!(
            serde_json::from_str::<HerdRequest>(r#"{"cmd":"status"}"#).unwrap(),
            HerdRequest::Status
        );
    }

    #[test]
    fn herd_response_roundtrip() {
        let resp = HerdResponse {
            ok: true,
            error: None,
            name: Some("worker".into()),
            bucket: Some("node@20".into()),
            replicas: Some(2),
            net: None,
            instances: Some(vec![InstanceState {
                index: 0,
                pid: Some(42),
                status: InstanceStatus::Running,
                restart_count: 0,
                last_exit_code: None,
            }]),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: HerdResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, resp);
    }

    #[test]
    fn send_control_errors_when_socket_missing() {
        let dir = tempfile::tempdir().unwrap();
        let err = send_control(dir.path(), "ghost", HerdRequest::Status).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("control socket not found"), "got: {msg}");
    }

    #[test]
    fn serve_control_handles_status_and_stop() {
        let dir = tempfile::tempdir().unwrap();
        let herds_dir = dir.path().join("herds");
        fs::create_dir_all(herds_dir.join("test")).unwrap();

        // Minimal controller with empty children — Status/Stop don't need live spawns.
        let spec = HerdSpec {
            name: "test".into(),
            bucket: "node@20".into(),
            command: vec![],
            env: HashMap::new(),
            replicas: 0,
            net: None,
            check_interval_secs: 5,
            restart: RestartPolicy::Never,
            max_restarts: 10,
        };
        let ctrl = Arc::new(HerdController {
            name: "test".into(),
            state: Arc::new(Mutex::new(InternalState {
                spec,
                children: HashMap::new(),
            })),
            state_dir: herds_dir.join("test"),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let ctrl2 = ctrl.clone();
        let handle = std::thread::spawn(move || serve_control(ctrl2, stop2));

        // Wait for the socket to appear
        let sock = control_sock_path(&herds_dir, "test");
        for _ in 0..50 {
            if sock.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(sock.exists(), "control socket should be bound");

        let status = send_control(&herds_dir, "test", HerdRequest::Status).unwrap();
        assert!(status.ok);
        assert_eq!(status.name.as_deref(), Some("test"));
        assert_eq!(status.bucket.as_deref(), Some("node@20"));
        assert_eq!(status.replicas, Some(0));

        let stop_resp = send_control(&herds_dir, "test", HerdRequest::Stop).unwrap();
        assert!(stop_resp.ok);
        assert!(stop.load(Ordering::SeqCst));

        handle.join().unwrap();
        assert!(!sock.exists(), "control socket should be unlinked on exit");
    }
}
