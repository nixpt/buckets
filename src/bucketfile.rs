use std::path::Path;
use std::collections::HashMap;
use anyhow::{Result, Context};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketMetadata {
    pub entrypoint: String,
    pub dependencies: Vec<String>,
    pub env: HashMap<String, String>,
    pub workdir: Option<String>,
}

#[derive(Debug, Clone)]
pub enum BucketfileDirective {
    From(Vec<String>),
    Env(String, String),
    Copy(String, String),
    Run(String),
    Workdir(String),
    Entrypoint(String),
}

pub fn parse_bucketfile(content: &str) -> Result<Vec<BucketfileDirective>> {
    let mut directives = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let line_num = idx + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        
        let (cmd, args) = trimmed.split_once(char::is_whitespace)
            .unwrap_or((trimmed, ""));
        let args = args.trim();
        
        match cmd.to_uppercase().as_str() {
            "FROM" => {
                let specs = args.split_whitespace()
                    .map(|s| s.trim_matches(',').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                directives.push(BucketfileDirective::From(specs));
            }
            "ENV" => {
                let (k, v) = if let Some((k, v)) = args.split_once('=') {
                    (k.trim().to_string(), v.trim().to_string())
                } else if let Some((k, v)) = args.split_once(char::is_whitespace) {
                    (k.trim().to_string(), v.trim().to_string())
                } else {
                    anyhow::bail!("Invalid ENV directive on line {}: {}", line_num, line);
                };
                directives.push(BucketfileDirective::Env(k, v));
            }
            "COPY" => {
                let parts: Vec<&str> = args.split_whitespace().collect();
                if parts.len() != 2 {
                    anyhow::bail!("Invalid COPY directive on line {}: {}. Expected: COPY <src> <dest>", line_num, line);
                }
                directives.push(BucketfileDirective::Copy(parts[0].to_string(), parts[1].to_string()));
            }
            "RUN" => {
                if args.is_empty() {
                    anyhow::bail!("Empty RUN directive on line {}", line_num);
                }
                directives.push(BucketfileDirective::Run(args.to_string()));
            }
            "WORKDIR" => {
                if args.is_empty() {
                    anyhow::bail!("Empty WORKDIR directive on line {}", line_num);
                }
                directives.push(BucketfileDirective::Workdir(args.to_string()));
            }
            "ENTRYPOINT" | "CMD" => {
                if args.is_empty() {
                    anyhow::bail!("Empty ENTRYPOINT/CMD directive on line {}", line_num);
                }
                directives.push(BucketfileDirective::Entrypoint(args.to_string()));
            }
            _ => {
                anyhow::bail!("Unknown directive on line {}: {}", line_num, cmd);
            }
        }
    }
    Ok(directives)
}

fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(entry.path(), dest_path)?;
        } else {
            std::fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}

pub fn build_bucketfile(
    config: &crate::config::Config,
    bucketfile_path: &Path,
    name: &str,
) -> Result<()> {
    let content = std::fs::read_to_string(bucketfile_path)
        .with_context(|| format!("Failed to read Bucketfile at {}", bucketfile_path.display()))?;

    let directives = parse_bucketfile(&content)?;

    // COPY sources are relative paths in the Bucketfile itself, so they must
    // resolve against the Bucketfile's own directory — not the cwd of the
    // `buckets` process, which may be anywhere when `-f <path>` or a
    // non-cwd source directory is used.
    let bucketfile_dir = bucketfile_path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    
    // Extract base dependencies from FROM
    let mut from_specs = Vec::new();
    for dir in &directives {
        if let BucketfileDirective::From(specs) = dir {
            from_specs.extend(specs.clone());
        }
    }
    
    // Target directory under local cellar
    let project_name = format!("local/{}", name);
    let target_dir = config.version_dir(&project_name, "0.0.0");
    let bin_dir = target_dir.join("bin");
    
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir)?;
    }
    std::fs::create_dir_all(&bin_dir)?;
    
    // We will build inside a temporary workspace directory on the host
    let temp_workspace = tempfile::tempdir()?;
    let workspace_path = temp_workspace.path();
    
    // Resolve all FROM specs
    let index = crate::index::Index::builtin();
    let resolved = if !from_specs.is_empty() {
        crate::resolve::resolve_multi(&from_specs, config, &index)?
    } else {
        crate::types::ResolvedEnvironment {
            installations: Vec::new(),
            env: HashMap::new(),
            entry: crate::types::Package {
                project: "local".to_string(),
                version: semver::Version::new(0, 0, 0),
            },
            all_packages: Vec::new(),
        }
    };
    
    let mut current_workdir = workspace_path.to_path_buf();
    let mut runtime_env = HashMap::new();
    let mut build_env = resolved.env.clone();
    
    // Copy env vars from resolved environment
    for (k, v) in &resolved.env {
        runtime_env.insert(k.clone(), v.clone());
    }
    
    // Run-time entrypoint cmd
    let mut entrypoint_cmd = String::new();
    
    // Process directives
    for dir in &directives {
        match dir {
            BucketfileDirective::From(_) => {}
            BucketfileDirective::Env(k, v) => {
                build_env.insert(k.clone(), v.clone());
                runtime_env.insert(k.clone(), v.clone());
            }
            BucketfileDirective::Copy(src, dest) => {
                // Determine clean destination path relative to temp workspace
                let target_dest_path = if dest == "/" || dest == "." || dest == "./" {
                    workspace_path.to_path_buf()
                } else {
                    let cleaned_dest = dest.trim_start_matches('/');
                    workspace_path.join(cleaned_dest)
                };
                
                let src_path = Path::new(src);
                let host_src = if src_path.is_absolute() {
                    src_path.to_path_buf()
                } else {
                    bucketfile_dir.join(src_path)
                };
                if host_src.is_dir() {
                    copy_dir_all(host_src, &target_dest_path)?;
                } else {
                    if let Some(parent) = target_dest_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::copy(host_src, &target_dest_path)?;
                }
            }
            BucketfileDirective::Workdir(wd) => {
                let cleaned_wd = wd.trim_start_matches('/');
                current_workdir = workspace_path.join(cleaned_wd);
                std::fs::create_dir_all(&current_workdir)?;
            }
            BucketfileDirective::Run(cmd_str) => {
                eprintln!("▶ [RUN] {}", cmd_str);
                
                // Set up sandbox profile for building
                let profile = crate::sandbox::SandboxProfile {
                    project_dir: Some(workspace_path.to_path_buf()),
                    extra_ro_binds: resolved.installations.iter().map(|i| i.path.clone()).collect(),
                    extra_rw_binds: vec![target_dir.clone()],
                    allow_network: true,
                    net_ns: None,
                };
                
                // Execute command inside sandbox
                let args = vec!["-c".to_string(), cmd_str.clone()];
                let mut cmd = crate::sandbox::sandboxed_command("/bin/sh", &args, &current_workdir, &build_env, &profile);
                
                let status = cmd.status()
                    .with_context(|| format!("Failed to run build command: {}", cmd_str))?;
                if !status.success() {
                    anyhow::bail!("Build command failed with exit code: {:?}", status.code());
                }
            }
            BucketfileDirective::Entrypoint(cmd_str) => {
                entrypoint_cmd = cmd_str.clone();
            }
        }
    }
    
    // Validate entrypoint
    if entrypoint_cmd.is_empty() {
        anyhow::bail!("Bucketfile has no ENTRYPOINT or CMD defined");
    }
    
    // 5. Create launcher script
    let launcher_path = bin_dir.join(name);
    let launcher_content = format!(
        "#!/bin/sh\nexec buckets run {} -- \"$@\"\n",
        project_name
    );
    std::fs::write(&launcher_path, launcher_content)?;
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&launcher_path, std::fs::Permissions::from_mode(0o755))?;
    }
    
    // Save metadata.json
    let metadata_path = target_dir.join("metadata.json");
    let relative_workdir = if current_workdir == workspace_path {
        None
    } else {
        current_workdir.strip_prefix(workspace_path).ok()
            .map(|p| p.to_string_lossy().to_string())
    };
    
    let meta = BucketMetadata {
        entrypoint: entrypoint_cmd,
        dependencies: from_specs,
        env: runtime_env,
        workdir: relative_workdir,
    };
    
    let meta_json = serde_json::to_string_pretty(&meta)?;
    std::fs::write(&metadata_path, meta_json)?;
    
    // Copy the contents of the temp workspace into target_dir/workspace/
    let target_workspace = target_dir.join("workspace");
    copy_dir_all(workspace_path, &target_workspace)?;
    
    crate::cellar::update_version_symlinks(config, &project_name, &semver::Version::new(0, 0, 0))?;
    
    eprintln!("✓ successfully built local bucket: {} v0.0.0", name);
    Ok(())
}
