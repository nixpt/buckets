use anyhow::{bail, Context, Result};

use crate::cellar;
use crate::config::Config;
use crate::index::Index;
use crate::install;
use crate::inventory;
use crate::types::{Installation, Package, PackageReq, ResolvedEnvironment};

/// The main entry point: resolve a spec string into a runnable environment.
///
/// Pipeline:
/// 1. Parse spec → `PackageReq`
/// 2. Resolve alias → project name
/// 3. Resolve version: check cache first, then remote
/// 4. Install if not cached
/// 5. Compose environment variables
pub fn resolve(
    spec: &str,
    config: &Config,
    index: &Index,
) -> Result<ResolvedEnvironment> {
    // Step 1: Parse spec
    let req = PackageReq::parse(spec)
        .with_context(|| format!("Failed to parse spec: {spec}"))?;

    // Step 2: Resolve alias
    let project = index.resolve_alias(&req.project).to_string();

    // Step 3: Create a new request with resolved project name
    let resolved_req = PackageReq {
        project: project.clone(),
        constraint: req.constraint.clone(),
    };

    // Step 4: Resolve version
    let version = resolve_version(config, &resolved_req)
        .with_context(|| format!("Failed to resolve version for {spec}"))?;

    let pkg = Package {
        project: project.clone(),
        version: version.clone(),
    };

    // Step 5: Install if needed
    let installation: Installation;
    if cellar::is_installed(config, &project, &version) {
        installation = cellar::get_installation(config, &pkg);
    } else {
        installation = install::install(config, &pkg)?;
    }

    // Step 6: Compose environment
    let env = crate::env::compose_env(&[installation.clone()]);

    Ok(ResolvedEnvironment {
        installations: vec![installation],
        env,
        entry: pkg,
    })
}

/// Resolve the best matching version: cache first, then remote.
fn resolve_version(config: &Config, req: &PackageReq) -> Result<semver::Version> {
    // 1. Check local cache for a matching version
    if let Some(cached) = cellar::best_installed(config, &req.project, &req.constraint) {
        return Ok(cached);
    }

    // 2. If constraint is STAR (`*` or `latest`), get the latest remote version
    if req.constraint == semver::VersionReq::STAR || req.constraint.to_string() == "*" {
        match inventory::latest_version(config, &req.project)? {
            Some(v) => {
                eprintln!("✓ resolved {}@latest → v{v}", req.project);
                return Ok(v);
            }
            None => {
                bail!(
                    "No versions found for '{}' on dist server ({})",
                    req.project, config.dist_url
                );
            }
        }
    }

    // 3. Find best remote match
    match inventory::best_match(config, &req.project, &req.constraint)? {
        Some(v) => {
            eprintln!("✓ resolved {}@{} → v{v}", req.project, req.constraint);
            Ok(v)
        }
        None => {
            // No match found — try a relaxed lookup: maybe the exact spec
            // is the project name itself (e.g. "node" without version)
            // We already handled STAR above, so this is a genuine miss.
            bail!(
                "No version of '{}' matches constraint '{}' on dist server ({})",
                req.project,
                req.constraint,
                config.dist_url
            );
        }
    }
}

/// Show info about what a spec would resolve to (without installing).
pub fn info(spec: &str, config: &Config, index: &Index) -> Result<()> {
    let req = PackageReq::parse(spec)
        .with_context(|| format!("Failed to parse spec: {spec}"))?;
    let project = index.resolve_alias(&req.project);

    println!("spec:        {spec}");
    println!("project:     {project}");
    println!("constraint:  {}", req.constraint);

    // Check cache
    let cached = cellar::best_installed(config, project, &req.constraint);
    println!("cached:      {}", cached.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "none".into()));

    // Check remote
    match inventory::best_match(config, project, &req.constraint) {
        Ok(Some(v)) => println!("remote:      v{v}"),
        Ok(None) => println!("remote:      no match"),
        Err(e) => println!("remote:      error — {e}"),
    }

    // Show what binaries are provided
    let index_bins = index.provides(project);
    if !index_bins.is_empty() {
        println!("provides:    {}", index_bins.join(", "));
    }

    Ok(())
}
