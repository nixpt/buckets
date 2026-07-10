//! Top-level resolution pipeline: parse a spec → resolve its alias → collect
//! companion packages → pick a version (cached, else best remote match) →
//! install → compose a runnable environment. [`resolve`] handles one spec;
//! [`resolve_multi`] does the same for several at once, deduplicating shared
//! dependencies and composing one unified environment (what `buckets run
//! node@20 python@3.11 -- ...` uses).

use anyhow::{bail, Context, Result};

use crate::cellar;
use crate::config::Config;
use crate::index::Index;
use crate::install;
use crate::inventory;
use crate::types::{Installation, Package, PackageReq, ResolvedEnvironment};

/// Resolve a single spec into a runnable environment.
///
/// Pipeline: parse → alias → resolve version → install → compose env
#[allow(dead_code)]
pub fn resolve(spec: &str, config: &Config, index: &Index) -> Result<ResolvedEnvironment> {
    resolve_multi(&[spec.to_string()], config, index)
}

/// Resolve multiple specs into a unified runnable environment.
///
/// Unlike single-package resolution, this:
/// 1. Resolves all specs + their companions
/// 2. Deduplicates across specs
/// 3. Installs all packages
/// 4. Composes a unified environment with all installations
pub fn resolve_multi(specs: &[String], config: &Config, index: &Index) -> Result<ResolvedEnvironment> {
    if specs.is_empty() {
        bail!("At least one spec is required");
    }

    // Phase 1: Parse + alias + collect companions
    let mut all_reqs: Vec<PackageReq> = Vec::new();

    for spec in specs {
        let req = PackageReq::parse(spec)
            .with_context(|| format!("Failed to parse spec: {spec}"))?;
        let project = index.resolve_alias(&req.project).to_string();

        let resolved_req = PackageReq {
            project: project.clone(),
            constraint: req.constraint.clone(),
        };
        all_reqs.push(resolved_req);

        // Add companions for this project. Companion entries are full specs
        // ("openssl@^1.1", not bare "openssl") so a companion can pin an
        // exact version range — some packages dynamically link a SPECIFIC
        // major of a shared companion (e.g. node needs openssl 1.1's
        // libcrypto.so.1.1, not the latest 3.x's libcrypto.so.3), so
        // defaulting to STAR/latest here would silently install a version
        // that doesn't satisfy the actual runtime dependency. Parsed and
        // alias-resolved the same way the top-level spec is, just above.
        let companions = index.companions(&project);
        for companion in companions {
            let companion_spec = PackageReq::parse(companion)
                .with_context(|| format!("Failed to parse companion spec: {companion}"))?;
            let companion_project = index.resolve_alias(&companion_spec.project).to_string();
            all_reqs.push(PackageReq {
                project: companion_project,
                constraint: companion_spec.constraint,
            });
        }
    }

    // Phase 2: Deduplicate by project, keeping the first constraint
    let mut seen_projects = std::collections::HashSet::new();
    let mut deduped_reqs: Vec<PackageReq> = Vec::new();
    for req in all_reqs {
        if seen_projects.insert(req.project.clone()) {
            deduped_reqs.push(req);
        }
    }

    // Phase 3: Resolve all packages to concrete versions
    let mut packages: Vec<Package> = Vec::new();
    for req in &deduped_reqs {
        let version = resolve_version(config, &req.project, &req.constraint)
            .with_context(|| format!("Failed to resolve version for '{}'", req.project))?;

        // Print resolution info
        if req.constraint == semver::VersionReq::STAR {
            eprintln!("✓ resolved {}@latest → v{version}", req.project);
        } else {
            eprintln!("�� resolved {}@{} → v{version}", req.project, req.constraint);
        }

        packages.push(Package {
            project: req.project.clone(),
            version,
        });
    }

    // Phase 4: Install all packages (parallel if multiple)
    let installations = install_all(config, &packages)?;

    // Phase 5: Compose unified environment
    let env = crate::env::compose_env(&installations);

    // The entry package is the first spec's resolved package
    let entry_project = index.resolve_alias(
        &PackageReq::parse(&specs[0])
            .context("Failed to re-parse first spec")?
            .project
    ).to_string();

    let entry_pkg = packages.iter()
        .find(|p| p.project == entry_project)
        .cloned()
        .unwrap_or_else(|| packages[0].clone());

    Ok(ResolvedEnvironment {
        installations,
        env,
        entry: entry_pkg,
        all_packages: packages,
    })
}

/// Install a list of packages, caching already-installed ones.
fn install_all(config: &Config, packages: &[Package]) -> Result<Vec<Installation>> {
    let mut installations = Vec::with_capacity(packages.len());
    let mut to_install: Vec<Package> = Vec::new();

    for pkg in packages {
        if cellar::is_installed(config, &pkg.project, &pkg.version) {
            installations.push(cellar::get_installation(config, pkg));
        } else {
            to_install.push(pkg.clone());
        }
    }

    // Install remaining packages in parallel
    if !to_install.is_empty() {
        eprintln!("↓ installing {} package(s)...", to_install.len());
        let new_installs = install::install_multi(config, &to_install)?;
        installations.extend(new_installs);
    }

    // Preserve original order
    installations.sort_by(|a, b| {
        let a_idx = packages.iter().position(|p| p.project == a.pkg.project).unwrap_or(0);
        let b_idx = packages.iter().position(|p| p.project == b.pkg.project).unwrap_or(0);
        a_idx.cmp(&b_idx)
    });

    Ok(installations)
}

/// Resolve the best matching version for a project: cache first, then remote.
fn resolve_version(config: &Config, project: &str, constraint: &semver::VersionReq) -> Result<semver::Version> {
    // 1. Check local cache for a matching version
    if let Some(cached) = cellar::best_installed(config, project, constraint) {
        return Ok(cached);
    }

    // 2. If constraint is STAR (`*` or `latest`), get the latest remote version
    if constraint == &semver::VersionReq::STAR || constraint.to_string() == "*" {
        match inventory::latest_version(config, project)? {
            Some(v) => return Ok(v),
            None => bail!(
                "No versions found for '{}' on dist server ({})",
                project, config.dist_url
            ),
        }
    }

    // 3. Find best remote match
    match inventory::best_match(config, project, constraint)? {
        Some(v) => Ok(v),
        None => bail!(
            "No version of '{}' matches constraint '{}' on dist server ({})",
            project, constraint, config.dist_url
        ),
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

    // Show companions
    let companions = index.companions(project);
    if !companions.is_empty() {
        println!("companions:  {}", companions.join(", "));
    }

    // Show binaries provided
    let index_bins = index.provides(project);
    if !index_bins.is_empty() {
        println!("provides:    {}", index_bins.join(", "));
    }

    Ok(())
}
