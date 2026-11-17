//! Deterministic, network-disabled evaluation runner for practice
//! environments (BUCKETS-12: rewritten on master's sandbox API —
//! [`sandbox::try_sandboxed_command`] and the BUCKETS-17 fail-closed
//! posture, instead of the salvage branch's pre-17 checks).
//!
//! Unlike `build`/`run`, this command is intentionally strict: it refuses to
//! run when the real sandbox backend is unavailable (no unsandboxed
//! fallback, ever) and emits exactly one JSON result on stdout so an
//! environment such as event-horizon can record it as evidence.
//!
//! JSON contract — field set taken from the salvage design, kept identical:
//! `passed`, `exit_code`, `timed_out`, `duration_ms`, `stdout`, `stderr`.
//! Refusals and internal errors print nothing on stdout and exit non-zero.

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::index::Index;
use crate::resolve;
use crate::sandbox::{self, SandboxProfile};

/// One JSON result: exactly this field set, one line on stdout.
#[derive(Debug, Serialize)]
pub struct EvaluationResult {
    /// `!timed_out && exit status was 0`.
    pub passed: bool,
    /// The child's exit code, or `null` when it died to a signal (incl.
    /// the timeout kill).
    pub exit_code: Option<i32>,
    /// The wall-clock limit expired and the sandbox was killed.
    pub timed_out: bool,
    /// Wall-clock duration of the child, milliseconds.
    pub duration_ms: u128,
    pub stdout: String,
    pub stderr: String,
}

pub fn run(
    project: &Path,
    specs: &[String],
    command: &[String],
    timeout_secs: u64,
    config: &Config,
    index: &Index,
) -> Result<EvaluationResult> {
    run_with(
        sandbox::try_sandboxed_command,
        project,
        specs,
        command,
        timeout_secs,
        config,
        index,
    )
}

/// The sandbox-construction step, injected so tests can simulate a missing
/// backend without touching `PATH` or widening sandbox.rs. Signature
/// matches [`sandbox::try_sandboxed_command`].
type SandboxProbe = fn(
    &str,
    &[String],
    &Path,
    &HashMap<String, String>,
    &SandboxProfile,
) -> std::result::Result<sandbox::Sandbox, sandbox::SandboxError>;

fn run_with(
    probe: SandboxProbe,
    project: &Path,
    specs: &[String],
    command: &[String],
    timeout_secs: u64,
    config: &Config,
    index: &Index,
) -> Result<EvaluationResult> {
    if !project.is_dir() {
        anyhow::bail!("evaluation project is not a directory: {}", project.display());
    }
    // Canonicalize before handing the directory to bwrap: a relative path
    // (`--project .`) must reach the sandbox as an absolute one, resolved
    // against the caller's cwd — bwrap execs with the cwd as given, and a
    // bare "." does not survive the mount-namespace pivot (found live).
    let project = &project
        .canonicalize()
        .with_context(|| format!("evaluation project is not accessible: {}", project.display()))?;
    if command.is_empty() {
        anyhow::bail!("evaluation command is required after '--'");
    }
    let resolved = if specs.is_empty() {
        None
    } else {
        Some(resolve::resolve_multi(specs, config, index).with_context(|| {
            format!("failed to resolve evaluation specs: {}", specs.join(", "))
        })?)
    };
    let (program, args) = command.split_first().expect("command checked above");
    let env = resolved
        .as_ref()
        .map(|e| e.env.clone())
        .unwrap_or_default();
    // Network off and fail closed are the contract: the default profile
    // already has both (allow_network: false; Fallback::Refuse → a missing
    // bwrap is an Err here, nothing is spawned, no JSON, non-zero exit).
    let profile = SandboxProfile {
        project_dir: Some(project.to_path_buf()),
        extra_ro_binds: resolved
            .as_ref()
            .map(|e| e.installations.iter().map(|i| i.path.clone()).collect())
            .unwrap_or_default(),
        allow_network: false,
        ..Default::default()
    };
    let sb = probe(program, args, project, &env, &profile).context(
        "refusing evaluation: the real sandbox backend is required (no unsandboxed fallback)",
    )?;
    // `into_command` keeps the pre_exec hooks (the seccomp memfd stays alive
    // for this child); the timeout is enforced by the loop below instead of
    // `Sandbox::status`, because eval must report `timed_out` in its JSON
    // result rather than exit(124).
    let mut cmd = sb.into_command();
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn evaluation command {program}"))?;
    let deadline = started + Duration::from_secs(timeout_secs);
    // Drain both pipes on threads so a chatty child can't fill its pipe
    // buffer and deadlock the deadline loop (same pattern as sandbox.rs's
    // own live-test helper).
    fn drain(r: Option<impl std::io::Read + Send + 'static>) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut r) = r {
                let _ = r.read_to_string(&mut s);
            }
            s
        })
    }
    let out_t = drain(child.stdout.take());
    let err_t = drain(child.stderr.take());
    let mut timed_out = false;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill(); // SIGKILL: bwrap's PID namespace takes the tree
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output()?;
    Ok(EvaluationResult {
        passed: !timed_out && output.status.success(),
        exit_code: output.status.code(),
        timed_out,
        duration_ms: started.elapsed().as_millis(),
        stdout: out_t.join().unwrap_or_default(),
        stderr: err_t.join().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Master's own sandbox tests self-skip the same way (`have_bwrap`):
    /// these live tests only run where a real bwrap exists.
    fn bwrap_on_path() -> bool {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join("bwrap"))
                .find(|p| p.is_file())
        }).is_some()
    }

    fn project_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn sh_args(script: &str) -> Vec<String> {
        vec!["/bin/sh".to_string(), "-c".to_string(), script.to_string()]
    }

    /// A probe that reports the backend as missing, like a PATH without
    /// bwrap would — but deterministically, without mutating the process
    /// environment (cargo runs tests in parallel threads).
    fn refuse_probe(
        _program: &str,
        _args: &[String],
        _cwd: &Path,
        _env: &HashMap<String, String>,
        _profile: &SandboxProfile,
    ) -> std::result::Result<sandbox::Sandbox, sandbox::SandboxError> {
        Err(sandbox::SandboxError::BwrapNotFound { proot_on_path: false })
    }

    /// A passing command → one JSON result with exit code 0.
    #[test]
    fn passing_command_json_exit_zero() {
        if !bwrap_on_path() {
            return;
        }
        let dir = project_dir();
        let r = run_with(
            sandbox::try_sandboxed_command,
            dir.path(),
            &[],
            &sh_args("echo eval-ok"),
            30,
            &Config::default(),
            &Index::builtin(),
        )
        .unwrap();
        assert!(r.passed, "{r:?}");
        assert_eq!(r.exit_code, Some(0));
        assert!(!r.timed_out);
        assert!(r.stdout.contains("eval-ok"), "{r:?}");
        // Exactly ONE JSON object with EXACTLY the contract's field set,
        // on one line (no pretty-printing, no extra keys).
        let v = serde_json::to_value(&r).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["duration_ms", "exit_code", "passed", "stderr", "stdout", "timed_out"]
        );
        assert!(!serde_json::to_string(&r).unwrap().contains('\n'));
    }

    /// A failing command → JSON carrying its non-zero exit. Eval's own
    /// process exit is 0-iff-passed (main.rs bails after printing the JSON;
    /// documented in the README) — the child's status lives in the result.
    #[test]
    fn failing_command_json_nonzero_exit() {
        if !bwrap_on_path() {
            return;
        }
        let dir = project_dir();
        let r = run_with(
            sandbox::try_sandboxed_command,
            dir.path(),
            &[],
            &sh_args("echo to-stderr >&2; exit 3"),
            30,
            &Config::default(),
            &Index::builtin(),
        )
        .unwrap();
        assert!(!r.passed, "{r:?}");
        assert_eq!(r.exit_code, Some(3));
        assert!(!r.timed_out);
        assert!(r.stderr.contains("to-stderr"), "{r:?}");
    }

    /// A timeout → `timed_out: true`, the child (and its whole tree) dead.
    /// Straggler check scoped to this test's own descendants, BUCKETS-11
    /// style: the sandboxed sleeps carry a unique fractional-second tag and
    /// the host-wide pgrep matches only that tag.
    #[test]
    fn timeout_kills_tree_and_reports_timed_out() {
        if !bwrap_on_path() {
            return;
        }
        let tag = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let script = format!("sleep 61.{tag} & sleep 60.{tag}");
        let dir = project_dir();
        let r = run_with(
            sandbox::try_sandboxed_command,
            dir.path(),
            &[],
            &sh_args(&script),
            2,
            &Config::default(),
            &Index::builtin(),
        )
        .unwrap();
        assert!(r.timed_out, "{r:?}");
        assert!(!r.passed);
        // bwrap was SIGKILLed: no exit code, and the kill took well under
        // the old 30s default (it fired at the 2s deadline).
        assert!(r.exit_code.is_none(), "{r:?}");
        assert!(r.duration_ms < 10_000, "{r:?}");
        // No stragglers — scoped to this test's own tag.
        let pat = format!(r"^sleep 6[01]\.{tag}$");
        let left = Command::new("pgrep").args(["-f", &pat]).output().unwrap();
        assert!(
            left.stdout.is_empty(),
            "stragglers: {}",
            String::from_utf8_lossy(&left.stdout)
        );
    }

    /// Invalid UTF-8 on the child's stdout and stderr must NOT silently
    /// drop the surrounding text: the capture is bytes → from_utf8_lossy,
    /// so the valid parts survive (bad bytes become U+FFFD) and the JSON
    /// evidence record stays complete (usagi's blocking finding on
    /// ebed5ae8: read_to_string errored on the bad byte and the whole
    /// capture came back empty with passed:true).
    #[test]
    fn stdout_stderr_capture_survives_invalid_utf8() {
        if !bwrap_on_path() {
            return;
        }
        // \377 is one raw 0xFF byte in both streams, wrapped in valid text.
        let dir = project_dir();
        let r = run_with(
            sandbox::try_sandboxed_command,
            dir.path(),
            &[],
            &sh_args("printf 'out:abc\\377def\\n'; echo out-good; printf 'err:abc\\377def\\n' >&2; echo err-good >&2"),
            30,
            &Config::default(),
            &Index::builtin(),
        )
        .unwrap();
        assert!(r.passed, "{r:?}");
        // Valid text present in both captures…
        assert!(r.stdout.contains("out-good"), "stdout lost text: {r:?}");
        assert!(r.stderr.contains("err-good"), "stderr lost text: {r:?}");
        // …including around the bad byte, which shows up as U+FFFD.
        assert!(r.stdout.contains("out:abc\u{FFFD}def"), "stdout: {r:?}");
        assert!(r.stderr.contains("err:abc\u{FFFD}def"), "stderr: {r:?}");
    }

    /// A missing sandbox backend → refusal: non-zero exit (an Err), and
    /// the command is NOT run.
    #[test]
    fn missing_backend_refuses_without_running() {
        let dir = project_dir();
        let marker = dir.path().join("marker-should-not-exist");
        let err = run_with(
            refuse_probe,
            dir.path(),
            &[],
            &sh_args("touch marker-should-not-exist"),
            30,
            &Config::default(),
            &Index::builtin(),
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("sandbox"), "refusal should name the sandbox: {msg}");
        assert!(!marker.exists(), "the command ran despite a missing backend");
    }
}
