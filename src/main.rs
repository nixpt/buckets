//! CLI entry point: `run`/`shell`/`env`/`info`/`list` subcommands (clap),
//! each driving [`resolve::resolve_multi`] then either exec'ing a command,
//! opening a shell, or printing the composed environment. See the
//! top-level README for the full command reference and spec format.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod cellar;
mod config;
mod env;
mod index;
mod install;
mod inventory;
mod resolve;
mod types;

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
        /// Runtime spec(s) ��� can specify multiple like "node@20" "python@3.11"
        specs: Vec<String>,

        /// Command and arguments to run
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },

    /// Open an interactive shell with the runtime in PATH.
    Shell {
        /// Runtime spec(s)
        specs: Vec<String>,

        /// Shell to use (default: current SHELL)
        #[arg(short, long)]
        shell: Option<String>,
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::new();
    let index = Index::builtin();

    match cli.command {
        Command::Run { specs, command } => cmd_run(&specs, &command, &config, &index),
        Command::Shell { specs, shell } => cmd_shell(&specs, shell.as_deref(), &config, &index),
        Command::Env { specs, json } => cmd_env(&specs, json, &config, &index),
        Command::Info { specs } => cmd_info(&specs, &config, &index),
        Command::List => cmd_list(&config),
    }
}

/// Run a command with the resolved runtime environment.
fn cmd_run(specs: &[String], command: &[String], config: &Config, index: &Index) -> Result<()> {
    if specs.is_empty() {
        anyhow::bail!("At least one spec is required (e.g. 'node@20')");
    }

    let resolved = resolve::resolve_multi(specs, config, index)
        .with_context(|| format!("Failed to resolve specs: {}", specs.join(", ")))?;

    let (program, args) = command.split_first().context("No command specified")?;

    let mut cmd = std::process::Command::new(program);
    cmd.args(args);

    for (key, value) in &resolved.env {
        cmd.env(key, value);
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

/// Open an interactive shell with the runtime in PATH.
fn cmd_shell(specs: &[String], shell: Option<&str>, config: &Config, index: &Index) -> Result<()> {
    if specs.is_empty() {
        anyhow::bail!("At least one spec is required (e.g. 'node@20')");
    }

    let resolved = resolve::resolve_multi(specs, config, index)?;

    let shell_program = shell
        .map(|s| s.to_string())
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "/bin/sh".to_string());

    let mut cmd = std::process::Command::new(&shell_program);

    for (key, value) in &resolved.env {
        cmd.env(key, value);
    }

    let bucket_name = specs.join("+");
    let bucket_prompt = format!("[{}] ", bucket_name);
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
