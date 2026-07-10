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
        /// Runtime spec (e.g. "node@20", "python@3.11", "rust@latest")
        spec: String,

        /// Command and arguments to run
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },

    /// Open an interactive shell with the runtime in PATH.
    Shell {
        /// Runtime spec (e.g. "node@20", "python@3.11")
        spec: String,

        /// Shell to use (default: current SHELL)
        #[arg(short, long)]
        shell: Option<String>,
    },

    /// Show resolution info for a spec (no installation).
    Info {
        /// Runtime spec (e.g. "node@20")
        spec: String,
    },

    /// List cached installations.
    List,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::new();
    let index = Index::builtin();

    match cli.command {
        Command::Run { spec, command } => cmd_run(&spec, &command, &config, &index),
        Command::Shell { spec, shell } => cmd_shell(&spec, shell.as_deref(), &config, &index),
        Command::Info { spec } => cmd_info(&spec, &config, &index),
        Command::List => cmd_list(&config),
    }
}

/// Run a command with the resolved runtime environment.
fn cmd_run(spec: &str, command: &[String], config: &Config, index: &Index) -> Result<()> {
    let resolved = resolve::resolve(spec, config, index)
        .with_context(|| format!("Failed to resolve {spec}"))?;

    let entry_bin = resolved
        .installations
        .first()
        .map(|i| i.path.join("bin"))
        .filter(|p| p.exists());

    if let Some(bin_dir) = entry_bin {
        eprintln!("  bin: {}", bin_dir.display());
    }

    // Extract the program and args from command
    let (program, args) = command.split_first().context("No command specified")?;

    // Merge resolved env into current process's env for the child
    let resolved_env = &resolved.env;

    // Set up the child process
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);

    // Pass through and extend the current environment
    for (key, value) in resolved_env {
        cmd.env(key, value);
    }

    // Inherit stdio so the user sees output directly
    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    eprintln!("▶ running in {} bucket", spec);
    let status = cmd.status().with_context(|| format!("Failed to execute {program}"))?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

/// Open an interactive shell with the runtime in PATH.
fn cmd_shell(spec: &str, shell: Option<&str>, config: &Config, index: &Index) -> Result<()> {
    let resolved = resolve::resolve(spec, config, index)?;

    let shell_program = shell
        .map(|s| s.to_string())
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "/bin/sh".to_string());

    let mut cmd = std::process::Command::new(&shell_program);

    // Merge resolved env
    for (key, value) in &resolved.env {
        cmd.env(key, value);
    }

    // Set a friendly prompt to indicate we're in a bucket
    let bucket_prompt = format!("[{}] ", spec);
    cmd.env("BUCKET_PROMPT", &bucket_prompt);
    // Set PS1 if using bash/zsh
    if shell_program.contains("bash") || shell_program.contains("zsh") {
        cmd.env("PS1", format!("\\[\\e[1;34m\\]{bucket_prompt}\\[\\e[0m\\]\\w \\$ "));
    }

    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    eprintln!("▶ starting {spec} bucket shell ({})", shell_program);
    let status = cmd.status()?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

/// Show resolution info.
fn cmd_info(spec: &str, config: &Config, index: &Index) -> Result<()> {
    resolve::info(spec, config, index)
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

        // Skip metadata files
        if project_str.starts_with('.') {
            continue;
        }

        let versions = cellar::list_installed(config, &project_str);
        if versions.is_empty() {
            continue;
        }

        any = true;
        let ver_strs: Vec<String> = versions.iter().map(|v| v.to_string()).collect();
        println!("  {}  [{}]", project_str, ver_strs.join(", "));
    }

    if !any {
        println!("No cached installations.");
        println!("  Cache dir: {}", cache_dir.display());
    }

    Ok(())
}
