//! CLI entry point: `run`/`shell`/`env`/`info`/`list` subcommands (clap),
//! each driving [`resolve::resolve_multi`] then either exec'ing a command,
//! opening a shell, or printing the composed environment. See the
//! top-level README for the full command reference and spec format.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::Path;

mod cellar;
mod config;
mod env;
mod gui;
mod index;
mod install;
mod inventory;
mod project;
mod resolve;
mod sandbox;
mod types;
mod worktree;

use config::Config;
use index::Index;

#[derive(Parser)]
#[command(name = "buckets", version, about = "Throwaway runtime buckets for AI agents")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a command in an ephemeral runtime environment.
    ///
    /// Usage: buckets run node@20 -- script.js [args...]
    Run {
        /// Runtime spec(s) — can specify multiple like "node@20" "python@3.11"
        specs: Vec<String>,

        /// Command and arguments to run
        #[arg(last = true, required = true)]
        command: Vec<String>,

        /// Run unsandboxed (plain subprocess, no bwrap containment).
        #[arg(long)]
        no_sandbox: bool,
    },

    /// Open an interactive shell with the runtime in PATH.
    Shell {
        /// Runtime spec(s)
        specs: Vec<String>,

        /// Shell to use (default: current SHELL)
        #[arg(short, long)]
        shell: Option<String>,

        /// Run unsandboxed (plain subprocess, no bwrap containment).
        #[arg(long)]
        no_sandbox: bool,
    },

    /// Print the composed environment as shell exports or JSON.
    ///
    /// Usage:
    ///   buckets env node@20
    ///   buckets env node@20 python@3.11 --json
    ///   eval "$(buckets env node@20)"
    Env {
        /// Runtime spec(s)
        specs: Vec<String>,

        /// Output as JSON instead of shell exports
        #[arg(short, long)]
        json: bool,
    },

    /// Show resolution info for a spec (no installation).
    Info {
        /// Runtime spec(s)
        specs: Vec<String>,
    },

    /// List cached installations.
    List,

    /// Clone (if a git URL) or use (if a local path) a source repo, detect
    /// its build system, resolve the toolchain it needs, and build it in a
    /// sandboxed bucket — without touching the host.
    ///
    /// Usage:
    ///   buckets build /path/to/repo
    ///   buckets build https://github.com/owner/repo
    ///   buckets build . --test --run
    Build {
        /// Git URL or local path.
        path_or_url: String,

        /// Also run the detected test command after a successful build.
        #[arg(long)]
        test: bool,

        /// Also run the detected run command after a successful build
        /// (and after tests, if --test was also given).
        #[arg(long)]
        run: bool,

        /// Build/test/run unsandboxed (plain subprocess, no bwrap containment).
        #[arg(long)]
        no_sandbox: bool,
    },

    /// Ephemeral git worktrees — a task gets its own working copy at a
    /// fresh branch (cheap: shares the repo's object store, not a full
    /// clone), buildable via `buckets build <worktree path>` like any
    /// other local path. "Destroyed once you merge": `remove` uses `git
    /// branch -d`, which git itself refuses on an unmerged branch — pass
    /// --force to discard anyway.
    #[command(subcommand)]
    Worktree(WorktreeCommand),

    /// Run a GUI command against a fresh, isolated Xvfb X server — the
    /// sandboxed process gets its own X display, not the host's real one.
    ///
    /// Usage:
    ///   buckets gui -- glxgears --screenshot /tmp/out.png --timeout 5
    ///   buckets gui node@20 -- node gui-script.js
    Gui {
        /// Runtime spec(s) — may be empty (a GUI test often just needs a
        /// system binary already on PATH, no resolved toolchain).
        specs: Vec<String>,

        /// Command and arguments to run
        #[arg(last = true, required = true)]
        command: Vec<String>,

        /// Save a screenshot of the virtual display (root window) to this
        /// path after the command exits or is killed by --timeout.
        #[arg(long)]
        screenshot: Option<String>,

        /// Kill the command after this many seconds if it hasn't exited
        /// (GUI apps often have no natural exit).
        #[arg(long)]
        timeout: Option<u64>,

        /// Virtual display width.
        #[arg(long, default_value_t = 1024)]
        width: u32,

        /// Virtual display height.
        #[arg(long, default_value_t = 768)]
        height: u32,

        /// Run unsandboxed (plain subprocess, no bwrap containment).
        #[arg(long)]
        no_sandbox: bool,
    },
}

#[derive(Subcommand)]
enum WorktreeCommand {
    /// Create a worktree at a fresh (or existing) branch. Prints the
    /// worktree's path on success (the argument to hand to `buckets
    /// build`/`run`/`shell` next).
    Create {
        /// Path to the git repo to branch off of.
        repo: String,
        /// Branch name for the new worktree.
        branch: String,
        /// Base branch/commit to branch from (default: repo's current HEAD).
        #[arg(long)]
        from: Option<String>,
    },

    /// Remove a worktree, and its branch if it's merged (git's own `git
    /// branch -d` refusal is the safety check — not reimplemented here).
    Remove {
        /// Path to the git repo the worktree belongs to.
        repo: String,
        /// Path to the worktree to remove.
        worktree_path: String,
        /// Branch name to also try deleting.
        branch: String,
        /// Discard even if unmerged/dirty (git worktree remove --force + git branch -D).
        #[arg(long)]
        force: bool,
    },

    /// List existing worktrees for a repo.
    List {
        /// Path to the git repo.
        repo: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::new();
    let index = Index::builtin();

    match cli.command {
        Command::Run { specs, command, no_sandbox } => cmd_run(&specs, &command, no_sandbox, &config, &index),
        Command::Shell { specs, shell, no_sandbox } => cmd_shell(&specs, shell.as_deref(), no_sandbox, &config, &index),
        Command::Env { specs, json } => cmd_env(&specs, json, &config, &index),
        Command::Info { specs } => cmd_info(&specs, &config, &index),
        Command::List => cmd_list(&config),
        Command::Build { path_or_url, test, run, no_sandbox } => {
            cmd_build(&path_or_url, test, run, no_sandbox, &config, &index)
        }
        Command::Worktree(cmd) => cmd_worktree(cmd, &config),
        Command::Gui { specs, command, screenshot, timeout, width, height, no_sandbox } => {
            cmd_gui(&specs, &command, screenshot.as_deref(), timeout, width, height, no_sandbox, &config, &index)
        }
    }
}

/// Run a command with the resolved runtime environment, sandboxed via
/// `bwrap` unless `no_sandbox` is set (see `sandbox.rs`).
fn cmd_run(specs: &[String], command: &[String], no_sandbox: bool, config: &Config, index: &Index) -> Result<()> {
    if specs.is_empty() {
        anyhow::bail!("At least one spec is required (e.g. 'node@20')");
    }

    let resolved = resolve::resolve_multi(specs, config, index)
        .with_context(|| format!("Failed to resolve specs: {}", specs.join(", ")))?;

    let (program, args) = command.split_first().context("No command specified")?;

    let mut cmd = if no_sandbox {
        let mut c = std::process::Command::new(program);
        c.args(args);
        c
    } else {
        let cwd = std::env::current_dir()?;
        let profile = sandbox::SandboxProfile {
            // The invocation cwd must be rw-bound: `--chdir` needs it to
            // exist inside bwrap's fresh mount namespace, and the common
            // case (`buckets run node@20 -- node script.js`) reads/writes
            // files right there.
            project_dir: Some(cwd.clone()),
            extra_ro_binds: resolved.installations.iter().map(|i| i.path.clone()).collect(),
            allow_network: false,
        };
        sandbox::sandboxed_command(program, args, &cwd, &resolved.env, &profile)
    };

    if no_sandbox {
        for (key, value) in &resolved.env {
            cmd.env(key, value);
        }
    }

    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    let bucket_name = specs.join("+");
    eprintln!("▶ running in {bucket_name} bucket");
    let status = cmd.status().with_context(|| format!("Failed to execute {program}"))?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

/// Open an interactive shell with the runtime in PATH, sandboxed via
/// `bwrap` unless `no_sandbox` is set (see `sandbox.rs`).
fn cmd_shell(specs: &[String], shell: Option<&str>, no_sandbox: bool, config: &Config, index: &Index) -> Result<()> {
    if specs.is_empty() {
        anyhow::bail!("At least one spec is required (e.g. 'node@20')");
    }

    let resolved = resolve::resolve_multi(specs, config, index)?;

    let shell_program = shell
        .map(|s| s.to_string())
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "/bin/sh".to_string());

    let bucket_name = specs.join("+");
    let bucket_prompt = format!("[{}] ", bucket_name);

    let mut cmd = if no_sandbox {
        let mut c = std::process::Command::new(&shell_program);
        for (key, value) in &resolved.env {
            c.env(key, value);
        }
        c
    } else {
        let cwd = std::env::current_dir()?;
        let profile = sandbox::SandboxProfile {
            project_dir: Some(cwd.clone()),
            extra_ro_binds: resolved.installations.iter().map(|i| i.path.clone()).collect(),
            allow_network: false,
        };
        sandbox::sandboxed_command(&shell_program, &[], &cwd, &resolved.env, &profile)
    };

    cmd.env("BUCKET_PROMPT", &bucket_prompt);
    if shell_program.contains("bash") || shell_program.contains("zsh") {
        cmd.env("PS1", format!("\\[\\e[1;34m\\]{bucket_prompt}\\[\\e[0m\\]\\w \\$ "));
    }

    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    eprintln!("▶ starting {bucket_name} bucket shell ({})", shell_program);
    let status = cmd.status()?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

/// Print the composed environment.
fn cmd_env(specs: &[String], json: bool, config: &Config, index: &Index) -> Result<()> {
    if specs.is_empty() {
        anyhow::bail!("At least one spec is required (e.g. 'node@20')");
    }

    let resolved = resolve::resolve_multi(specs, config, index)?;

    if json {
        let output = env::format_json(&resolved)?;
        println!("{output}");
    } else {
        let output = env::format_shell_exports(&resolved);
        print!("{output}");
    }

    Ok(())
}

/// Show resolution info.
fn cmd_info(specs: &[String], config: &Config, index: &Index) -> Result<()> {
    if specs.is_empty() {
        anyhow::bail!("At least one spec is required (e.g. 'node@20')");
    }

    for spec in specs {
        if specs.len() > 1 {
            println!("═══ {spec} ═══");
        }
        resolve::info(spec, config, index)?;
        println!();
    }

    Ok(())
}

/// List cached installations.
fn cmd_list(config: &Config) -> Result<()> {
    let cache_dir = &config.cache_dir;
    if !cache_dir.exists() {
        println!("No cached installations (cache dir: {})", cache_dir.display());
        return Ok(());
    }

    let mut any = false;
    for entry in std::fs::read_dir(cache_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let project = entry.file_name();
        let project_str = project.to_string_lossy();

        if project_str.starts_with('.') {
            continue;
        }

        let versions = cellar::list_installed(config, &project_str);
        if versions.is_empty() {
            continue;
        }

        any = true;
        let ver_strs: Vec<String> = versions.iter().map(types::dist_version_string).collect();
        println!("  {}  [{}]", project_str, ver_strs.join(", "));

        // Show symlink aliases
        for alias in &["v*", &format!("v{}", versions[0].major)] {
            if let Some(resolved) = cellar::resolve_symlink(config, &project_str, alias) {
                let resolved_str = types::dist_version_string(&resolved);
                if resolved != versions[0] {
                    println!("    {alias} → v{resolved_str}");
                } else {
                    println!("    {alias} → v{resolved_str} (latest)");
                }
            }
        }
    }

    if !any {
        println!("No cached installations.");
        println!("  Cache dir: {}", cache_dir.display());
    }

    Ok(())
}

/// Clone/use a source repo, detect its build system, resolve the toolchain
/// it needs, and run build (+ optionally test, run) sandboxed against it.
fn cmd_build(path_or_url: &str, test: bool, run: bool, no_sandbox: bool, config: &Config, index: &Index) -> Result<()> {
    let (source_dir, is_temp) = project::resolve_source(path_or_url)?;
    let mut plan = project::detect(&source_dir)
        .with_context(|| format!("Failed to detect a build system in {}", source_dir.display()))?;
    plan.is_temp = is_temp;

    eprintln!(
        "▶ detected {} project — toolchain: {}",
        source_dir.display(),
        plan.toolchain_specs.join(", ")
    );

    let resolved = resolve::resolve_multi(&plan.toolchain_specs, config, index)
        .with_context(|| format!("Failed to resolve toolchain: {}", plan.toolchain_specs.join(", ")))?;

    let outcome = (|| -> Result<()> {
        run_project_step("build", &plan.build_cmd, &plan.source_dir, no_sandbox, &resolved)?;

        if test {
            match &plan.test_cmd {
                Some(cmd) => run_project_step("test", cmd, &plan.source_dir, no_sandbox, &resolved)?,
                None => eprintln!("⚠ --test requested but no test command was detected for this project"),
            }
        }

        if run {
            match &plan.run_cmd {
                Some(cmd) => run_project_step("run", cmd, &plan.source_dir, no_sandbox, &resolved)?,
                None => eprintln!("⚠ --run requested but no run command was detected for this project"),
            }
        }

        Ok(())
    })();

    if plan.is_temp {
        match &outcome {
            Ok(()) => {
                let _ = std::fs::remove_dir_all(&plan.source_dir);
            }
            Err(_) => {
                eprintln!("⚠ build failed — leaving the clone at {} for inspection", plan.source_dir.display());
            }
        }
    }

    outcome
}

/// Run one step (build/test/run) of a `ProjectPlan`, sandboxed unless
/// `no_sandbox`. Network is allowed — build commands need their package
/// registries (crates.io, npm, ...), unlike plain `run`/`shell`.
fn run_project_step(
    label: &str,
    command: &[String],
    project_dir: &Path,
    no_sandbox: bool,
    resolved: &types::ResolvedEnvironment,
) -> Result<()> {
    let (program, args) = command.split_first().context("empty command")?;

    let mut cmd = if no_sandbox {
        let mut c = std::process::Command::new(program);
        c.args(args).current_dir(project_dir);
        for (key, value) in &resolved.env {
            c.env(key, value);
        }
        c
    } else {
        let profile = sandbox::SandboxProfile {
            project_dir: Some(project_dir.to_path_buf()),
            extra_ro_binds: resolved.installations.iter().map(|i| i.path.clone()).collect(),
            allow_network: true,
        };
        sandbox::sandboxed_command(program, args, project_dir, &resolved.env, &profile)
    };

    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    eprintln!("▶ {label}: {}", command.join(" "));
    let status = cmd.status().with_context(|| format!("Failed to execute {program}"))?;
    if !status.success() {
        anyhow::bail!("{label} failed (exit {})", status.code().unwrap_or(1));
    }
    Ok(())
}

/// Run a GUI command against a fresh Xvfb X server, sandboxed via `bwrap`
/// unless `no_sandbox` is set. See `gui.rs` for the Xvfb session lifecycle
/// and why the sandboxing core (`sandbox.rs`) needed zero changes for this.
#[allow(clippy::too_many_arguments)]
fn cmd_gui(
    specs: &[String],
    command: &[String],
    screenshot: Option<&str>,
    timeout: Option<u64>,
    width: u32,
    height: u32,
    no_sandbox: bool,
    config: &Config,
    index: &Index,
) -> Result<()> {
    let session = gui::XvfbSession::start(width, height, 24)?;
    eprintln!("▶ gui bucket on {} ({width}x{height})", session.display);

    let (resolved_env, resolved_installations) = if specs.is_empty() {
        (std::collections::HashMap::new(), Vec::new())
    } else {
        let resolved = resolve::resolve_multi(specs, config, index)
            .with_context(|| format!("Failed to resolve specs: {}", specs.join(", ")))?;
        (resolved.env, resolved.installations)
    };

    let (program, args) = command.split_first().context("No command specified")?;

    let mut cmd = if no_sandbox {
        let mut c = std::process::Command::new(program);
        c.args(args);
        for (key, value) in &resolved_env {
            c.env(key, value);
        }
        c.env("DISPLAY", &session.display);
        c.env("XAUTHORITY", &session.xauthority);
        c
    } else {
        let cwd = std::env::current_dir()?;
        let mut env = resolved_env.clone();
        env.insert("DISPLAY".to_string(), session.display.clone());
        env.insert("XAUTHORITY".to_string(), session.xauthority.display().to_string());

        let mut extra_ro_binds: Vec<std::path::PathBuf> =
            resolved_installations.iter().map(|i| i.path.clone()).collect();
        extra_ro_binds.push(session.socket_path());
        extra_ro_binds.push(session.xauthority.clone());

        let profile = sandbox::SandboxProfile {
            project_dir: Some(cwd.clone()),
            extra_ro_binds,
            allow_network: false,
        };
        sandbox::sandboxed_command(program, args, &cwd, &env, &profile)
    };

    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    eprintln!("▶ running: {}", command.join(" "));
    let mut child = cmd.spawn().with_context(|| format!("Failed to execute {program}"))?;

    let status = match timeout {
        None => child.wait()?,
        Some(secs) => {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
            loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if std::time::Instant::now() >= deadline {
                    eprintln!("▶ timeout reached — killing");
                    let _ = child.kill();
                    break child.wait()?;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    };

    if let Some(path) = screenshot {
        eprintln!("▶ screenshot -> {path}");
        session.screenshot(Path::new(path))?;
    }

    if !status.success() && timeout.is_none() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

fn cmd_worktree(cmd: WorktreeCommand, config: &Config) -> Result<()> {
    match cmd {
        WorktreeCommand::Create { repo, branch, from } => {
            let path = worktree::create(Path::new(&repo), &branch, from.as_deref(), config.worktree_dir.as_deref())?;
            println!("{}", path.display());
            Ok(())
        }
        WorktreeCommand::Remove { repo, worktree_path, branch, force } => {
            worktree::remove(Path::new(&repo), Path::new(&worktree_path), &branch, force)
        }
        WorktreeCommand::List { repo } => {
            print!("{}", worktree::list(Path::new(&repo))?);
            Ok(())
        }
    }
}
