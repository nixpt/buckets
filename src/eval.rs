//! Deterministic, network-disabled evaluation runner for practice environments.
//!
//! Unlike `build`, this command is intentionally strict: it refuses to run
//! when the real sandbox backend is unavailable and emits one JSON result on
//! stdout so an environment such as event-horizon can record it as evidence.

use anyhow::{Context, Result};
use serde::Serialize;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::index::Index;
use crate::resolve;
use crate::sandbox::{self, SandboxProfile};

#[derive(Debug, Serialize)]
pub struct EvaluationResult {
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
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
    if !project.is_dir() {
        anyhow::bail!(
            "evaluation project is not a directory: {}",
            project.display()
        );
    }
    if command.is_empty() {
        anyhow::bail!("evaluation command is required after '--'");
    }
    if !sandbox::sandbox_available() {
        anyhow::bail!("refusing evaluation: bwrap is required for sandboxed practice");
    }
    let resolved = if specs.is_empty() {
        None
    } else {
        Some(
            resolve::resolve_multi(specs, config, index).with_context(|| {
                format!("failed to resolve evaluation specs: {}", specs.join(", "))
            })?,
        )
    };
    let (program, args) = command.split_first().expect("command checked above");
    let mut cmd = sandbox::sandboxed_command(
        program,
        args,
        project,
        &resolved
            .as_ref()
            .map(|env| env.env.clone())
            .unwrap_or_default(),
        &SandboxProfile {
            project_dir: Some(project.to_path_buf()),
            extra_ro_binds: resolved
                .as_ref()
                .map(|env| env.installations.iter().map(|i| i.path.clone()).collect())
                .unwrap_or_default(),
            extra_rw_binds: Vec::new(),
            binds_at_path: Vec::new(),
            allow_network: false,
            net_ns: None,
        },
    );
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn evaluation command {program}"))?;
    let deadline = started + Duration::from_secs(timeout_secs);
    let mut timed_out = false;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            child.kill().ok();
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output()?;
    Ok(EvaluationResult {
        passed: !timed_out && output.status.success(),
        exit_code: output.status.code(),
        timed_out,
        duration_ms: started.elapsed().as_millis(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}
