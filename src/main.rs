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
mod session;
mod site;
mod types;
mod worktree;
mod bucketfile;

use config::Config;
use index::Index;

#[derive(Parser)]
#[command(name = "buckets", version, about = "Throwaway runtime buckets for AI agents")]
struct Cli {
    /// Use a zram-backed cache dir instead of host disk — requires an
    /// already-running `flare-up` session (see `squadron/bin/flare-up`).
    /// This flag does NOT provision one itself: flare sessions are
    /// deliberately session-scoped (a zram device's contents don't survive
    /// reboot on their own, so per-command provisioning would just add
    /// mount/unmount latency for no real ephemerality gain — see
    /// /workspace/projects/FLARE_FIREFLY_DESIGN.md). Run `sudo flare-up`
    /// once per session/agent, then pass `--flare` to any buckets
    /// invocation to use it. Errors clearly if no session is live rather
    /// than silently falling back to host disk.
    #[arg(long, global = true)]
    flare: bool,

    #[command(subcommand)]
    command: Command,
}

/// Resolve `--flare` into a `BUCKETS_CACHE_DIR` override by asking
/// `flare-status` (no root needed — it only reads state, doesn't
/// provision) for the mount path of an already-live session. Errors with
/// a pointer to `flare-up` rather than silently falling back to the
/// host-disk default, since a caller that explicitly asked for `--flare`
/// almost certainly doesn't want a silent downgrade.
fn resolve_flare_cache_dir() -> Result<String> {
    let output = std::process::Command::new("flare-status")
        .arg("--quiet")
        .output()
        .context("--flare: failed to run flare-status (is squadron/bin on PATH?)")?;

    if !output.status.success() {
        anyhow::bail!(
            "--flare: no live flare session — run 'sudo flare-up' first \
             (see squadron/bin/flare-up --help)"
        );
    }

    let mount = String::from_utf8(output.stdout)
        .context("--flare: flare-status printed non-UTF8 output")?
        .trim()
        .to_string();
    if mount.is_empty() {
        anyhow::bail!("--flare: flare-status succeeded but printed no mount path");
    }
    Ok(mount)
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
        #[arg(last = true)]
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

        /// Path to Bucketfile. If provided, builds a Bucketfile spec instead of a standard source directory.
        #[arg(short = 'f', long)]
        bucketfile: Option<String>,

        /// Name/tag for the built local bucket (required when building a Bucketfile).
        #[arg(short = 't', long)]
        tag: Option<String>,

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

        /// Start x11vnc on this port for remote viewing of the virtual display.
        #[arg(long)]
        vnc_port: Option<u16>,

        /// VNC password (default: no password).
        #[arg(long)]
        vnc_password: Option<String>,

        /// Serve a noVNC web UI alongside the VNC server.
        #[arg(long)]
        web: bool,
    },

    /// Run a browser against a URL with a real, OS-enforced per-origin
    /// storage sandbox — reviving the intent behind exosphere-apps'
    /// site-capsulizer (storage/net/worker isolation), which was found
    /// unenforced scaffolding, using bwrap for actual enforcement instead.
    /// Defaults to a headless browser binary (e.g. `surfer`); pass --gui
    /// for a windowed one (e.g. `super-surfer`) inside a fresh Xvfb.
    ///
    /// Usage:
    ///   buckets site https://example.com
    ///   buckets site https://example.com --gui --screenshot /tmp/out.png
    Site {
        /// URL to open.
        url: String,

        /// Path to the browser binary. Defaults to `surfer` on PATH
        /// (headless), or `super-surfer` on PATH with --gui.
        #[arg(long)]
        browser_bin: Option<String>,

        /// Extra arguments passed through to the browser binary.
        #[arg(last = true)]
        extra_args: Vec<String>,

        /// Launch a windowed browser in a fresh Xvfb instead of headless.
        #[arg(long)]
        gui: bool,

        /// Ephemeral storage: a fresh directory removed on exit, instead
        /// of the default persistent per-host directory.
        #[arg(long)]
        incognito: bool,

        /// Virtual display width (--gui only).
        #[arg(long, default_value_t = 1024)]
        width: u32,

        /// Virtual display height (--gui only).
        #[arg(long, default_value_t = 768)]
        height: u32,

        /// Save a screenshot of the virtual display after exit (--gui only).
        #[arg(long)]
        screenshot: Option<String>,

        /// Kill the browser after this many seconds if it hasn't exited.
        #[arg(long)]
        timeout: Option<u64>,

        /// Disable network access inside the sandbox (on by default —
        /// fetching a page needs it).
        #[arg(long)]
        no_network: bool,

        /// Run unsandboxed (plain subprocess, no bwrap containment).
        #[arg(long)]
        no_sandbox: bool,
    },

    /// Manage persistent sessions with OverlayFS isolation.
    ///
    /// Sessions share a writable overlay filesystem across multiple
    /// `session exec` calls — files written by one command survive
    /// into the next. Backed by disk (default), tmpfs (--tmpfs), or
    /// zram (--zram).
    #[command(subcommand)]
    Session(SessionCommand),
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

#[derive(Subcommand)]
enum SessionCommand {
    /// Start a new persistent session.
    ///
    /// Resolves toolchains, creates an OverlayFS mount, and optionally
    /// runs a command under bwrap with the overlay bound at /session/.
    ///
    /// Usage:
    ///   buckets session start node@20
    ///   buckets session start --tmpfs python@3.11
    ///   buckets session start node@20 python@3.11 -- node script.js
    Start {
        /// Runtime spec(s) — can specify multiple like "node@20" "python@3.11"
        specs: Vec<String>,

        /// Command and arguments to run inside the session (optional)
        #[arg(last = true)]
        command: Vec<String>,

        /// Use tmpfs for the overlay upper dir (faster, ephemeral, default 4G)
        #[arg(long)]
        tmpfs: bool,

        /// Use zram for the overlay upper dir (compressed RAM)
        #[arg(long)]
        zram: bool,

        /// Size for tmpfs/zram backing (e.g. "2G", "512M"). Default: "4G"
        #[arg(long)]
        size: Option<String>,
    },

    /// Execute a command in an existing session.
    ///
    /// The command runs under bwrap with the session's overlay mount
    /// bound at /session/. Files written by previous exec calls are
    /// visible.
    ///
    /// Usage:
    ///   buckets session exec <session-id> -- node another-script.js
    Exec {
        /// Session ID
        session_id: String,

        /// Command and arguments to run in the session
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },

    /// Stop and optionally destroy a session.
    ///
    /// Unmounts the overlay. Without --purge, the session state (upper
    /// dir) is preserved but unmounted.
    ///
    /// Usage:
    ///   buckets session stop <session-id>
    ///   buckets session stop --purge <session-id>
    Stop {
        /// Session ID to stop
        session_id: String,

        /// Also remove the session upper/work dirs
        #[arg(long)]
        purge: bool,
    },

    /// List active sessions.
    ///
    /// Shows session ID, specs, backing type, age, and PID.
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.flare {
        let mount = resolve_flare_cache_dir()?;
        eprintln!("buckets: --flare active, cache dir = {mount}");
        // SAFETY: single-threaded at this point (no other threads spawned
        // yet — Config::new() below is the first consumer of this env var).
        unsafe { std::env::set_var("BUCKETS_CACHE_DIR", &mount) };
    }

    let config = Config::new();
    let index = Index::builtin();

    match cli.command {
        Command::Run { specs, command, no_sandbox } => cmd_run(&specs, &command, no_sandbox, &config, &index),
        Command::Shell { specs, shell, no_sandbox } => cmd_shell(&specs, shell.as_deref(), no_sandbox, &config, &index),
        Command::Env { specs, json } => cmd_env(&specs, json, &config, &index),
        Command::Info { specs } => cmd_info(&specs, &config, &index),
        Command::List => cmd_list(&config),
        Command::Build { path_or_url, bucketfile, tag, test, run, no_sandbox } => {
            cmd_build(&path_or_url, bucketfile.as_deref(), tag.as_deref(), test, run, no_sandbox, &config, &index)
        }
        Command::Worktree(cmd) => cmd_worktree(cmd, &config),
        Command::Gui { specs, command, screenshot, timeout, width, height, no_sandbox, vnc_port, vnc_password, web } => {
            cmd_gui(&specs, &command, screenshot.as_deref(), timeout, width, height, no_sandbox, vnc_port, vnc_password.as_deref(), web, &config, &index)
        }
        Command::Site { url, browser_bin, extra_args, gui, incognito, width, height, screenshot, timeout, no_network, no_sandbox } => {
            cmd_site(&url, browser_bin.as_deref(), &extra_args, gui, incognito, width, height, screenshot.as_deref(), timeout, no_network, no_sandbox, &config)
        }
        Command::Session(cmd) => cmd_session(cmd, &config, &index),
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

    let mut metadata: Option<crate::bucketfile::BucketMetadata> = None;
    let mut bucket_workspace_path = None;

    for inst in &resolved.installations {
        if inst.pkg.project.starts_with("local/") {
            let meta_path = inst.path.join("metadata.json");
            if meta_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&meta_path) {
                    if let Ok(meta) = serde_json::from_str::<crate::bucketfile::BucketMetadata>(&content) {
                        bucket_workspace_path = Some(inst.path.join("workspace"));
                        metadata = Some(meta);
                        break;
                    }
                }
            }
        }
    }

    let (program_str, args_vec) = if !command.is_empty() {
        let (p, a) = command.split_first().context("No command specified")?;
        (p.clone(), a.to_vec())
    } else if let Some(ref meta) = metadata {
        let entrypoint_parts: Vec<String> = meta.entrypoint.split_whitespace().map(|s| s.to_string()).collect();
        let (ep_program, ep_args) = entrypoint_parts.split_first()
            .context("Empty entrypoint in local bucket metadata")?;
        (ep_program.clone(), ep_args.to_vec())
    } else {
        anyhow::bail!("No command specified. Use 'buckets run <spec> -- <command>' or 'buckets shell <spec>'");
    };

    let program = &program_str;
    let args = &args_vec;

    let mut final_env = resolved.env.clone();
    if let Some(ref meta) = metadata {
        for (k, v) in &meta.env {
            final_env.insert(k.clone(), v.clone());
        }
    }

    let mut cmd = if no_sandbox {
        let mut c = std::process::Command::new(program);
        c.args(args);
        c
    } else {
        let cwd = if let Some(ref meta) = metadata {
            let base_ws = bucket_workspace_path.clone().unwrap();
            if let Some(ref wd) = meta.workdir {
                base_ws.join(wd)
            } else {
                base_ws
            }
        } else {
            std::env::current_dir()?
        };
        let profile = sandbox::SandboxProfile {
            project_dir: Some(cwd.clone()),
            extra_ro_binds: resolved.installations.iter().map(|i| i.path.clone()).collect(),
            extra_rw_binds: Vec::new(),
            allow_network: false,
        };
        sandbox::sandboxed_command(program, args, &cwd, &final_env, &profile)
    };

    if no_sandbox {
        for (key, value) in &final_env {
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
            extra_rw_binds: Vec::new(),
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
fn cmd_build(
    path_or_url: &str,
    bucketfile: Option<&str>,
    tag: Option<&str>,
    test: bool,
    run: bool,
    no_sandbox: bool,
    config: &Config,
    index: &Index,
) -> Result<()> {
    let (source_dir, is_temp) = project::resolve_source(path_or_url)?;

    // Check if we should build a Bucketfile
    let bucketfile_path = if let Some(bf) = bucketfile {
        Some(std::path::PathBuf::from(bf))
    } else {
        let default_bf = source_dir.join("Bucketfile");
        if default_bf.exists() {
            Some(default_bf)
        } else {
            None
        }
    };

    if let Some(bf_path) = bucketfile_path {
        let tag_name = tag.ok_or_else(|| {
            anyhow::anyhow!("Please specify a name/tag for the built bucket using --tag or -t (e.g. -t my-bucket)")
        })?;

        crate::bucketfile::build_bucketfile(config, &bf_path, tag_name)?;
        return Ok(());
    }

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
            extra_rw_binds: Vec::new(),
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
    vnc_port: Option<u16>,
    vnc_password: Option<&str>,
    _web: bool,
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
            extra_rw_binds: Vec::new(),
            allow_network: false,
        };
        sandbox::sandboxed_command(program, args, &cwd, &env, &profile)
    };

    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    eprintln!("▶ running: {}", command.join(" "));
    let mut child = cmd.spawn().with_context(|| format!("Failed to execute {program}"))?;

    // Start VNC server if requested
    if let Some(port) = vnc_port {
        let _ = session.start_vnc(port, vnc_password);
    }

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

/// Run a browser against a URL with a real, OS-enforced per-origin storage
/// sandbox (see `site.rs`), sandboxed via `bwrap` unless `no_sandbox`.
#[allow(clippy::too_many_arguments)]
fn cmd_site(
    url: &str,
    browser_bin: Option<&str>,
    extra_args: &[String],
    gui: bool,
    incognito: bool,
    width: u32,
    height: u32,
    screenshot: Option<&str>,
    timeout: Option<u64>,
    no_network: bool,
    no_sandbox: bool,
    config: &Config,
) -> Result<()> {
    let target = site::SiteTarget::resolve(url, config, incognito)?;

    let default_bin_name = if gui { "super-surfer" } else { "surfer" };
    let browser_path = match browser_bin {
        Some(p) => std::path::PathBuf::from(p),
        None => which(default_bin_name).with_context(|| {
            format!(
                "'{default_bin_name}' not found on PATH. Build it first: {}",
                if gui {
                    "cargo build --release -p super-surfer --features bliss (in surfer-browser)"
                } else {
                    "cargo build --release -p surfer --features cli (in surfer-browser)"
                }
            )
        })?,
    };
    let browser_dir = browser_path
        .parent()
        .map(|p| p.to_path_buf())
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let session = if gui { Some(gui::XvfbSession::start(width, height, 24)?) } else { None };
    if let Some(s) = &session {
        eprintln!("▶ site bucket for {} on {} ({width}x{height})", target.host, s.display);
    } else {
        eprintln!("▶ site bucket for {}", target.host);
    }

    let mut args = vec![url.to_string()];
    args.extend(extra_args.iter().cloned());

    let mut cmd = if no_sandbox {
        let mut c = std::process::Command::new(&browser_path);
        c.args(&args);
        if let Some(s) = &session {
            c.env("DISPLAY", &s.display);
            c.env("XAUTHORITY", &s.xauthority);
        }
        c
    } else {
        let mut env = std::collections::HashMap::new();
        let mut extra_ro_binds = vec![browser_dir];
        if let Some(s) = &session {
            env.insert("DISPLAY".to_string(), s.display.clone());
            env.insert("XAUTHORITY".to_string(), s.xauthority.display().to_string());
            extra_ro_binds.push(s.socket_path());
            extra_ro_binds.push(s.xauthority.clone());
        }
        let profile = sandbox::SandboxProfile {
            project_dir: Some(target.storage_dir.clone()),
            extra_ro_binds,
            extra_rw_binds: Vec::new(),
            allow_network: !no_network,
        };
        let args_rest = &args[..];
        sandbox::sandboxed_command(browser_path.to_string_lossy().as_ref(), args_rest, &target.storage_dir, &env, &profile)
    };

    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    eprintln!("▶ running: {} {}", browser_path.display(), args.join(" "));
    let mut child = cmd.spawn().with_context(|| format!("Failed to execute {}", browser_path.display()))?;

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
        match &session {
            Some(s) => {
                eprintln!("▶ screenshot -> {path}");
                s.screenshot(Path::new(path))?;
            }
            None => eprintln!("⚠ --screenshot requires --gui — skipped"),
        }
    }

    if !status.success() && timeout.is_none() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

fn which(bin: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).map(|dir| dir.join(bin)).find(|p| p.is_file())
    })
}

/// Manage persistent sessions: start, exec, stop, list.
fn cmd_session(cmd: SessionCommand, config: &Config, index: &Index) -> Result<()> {
    match cmd {
        SessionCommand::Start { specs, command, tmpfs, zram, size } => {
            if specs.is_empty() {
                anyhow::bail!("At least one spec is required (e.g. 'node@20')");
            }
            let session_id = session::session_start(
                &specs, &command, tmpfs, zram, size.as_deref(), config, index
            )?;
            println!("{session_id}");
            Ok(())
        }
        SessionCommand::Exec { session_id, command } => {
            let output = session::session_exec(&session_id, &command, config)?;
            print!("{output}");
            Ok(())
        }
        SessionCommand::Stop { session_id, purge } => {
            let msg = session::session_stop(&session_id, purge, config)?;
            eprintln!("{msg}");
            Ok(())
        }
        SessionCommand::List { json } => {
            let sessions = session::list_sessions(config)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&sessions)?);
            } else {
                if sessions.is_empty() {
                    println!("No active sessions.");
                    return Ok(());
                }
                for s in &sessions {
                    let backing = if s.upper_is_zram {
                        "zram"
                    } else if s.upper_is_tmpfs {
                        "tmpfs"
                    } else {
                        "disk"
                    };
                    let mounted = std::path::Path::new(&s.mount_point).exists();
                    let pid_str = s.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string());
                    println!("  {}  specs={}  backing={backing}  mounted={mounted}  pid={pid_str}",
                        s.session_id, s.specs.join("+"));
                }
            }
            Ok(())
        }
    }
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
