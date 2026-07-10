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
/// 1. Resolves all specs + their companions, transitively (a companion
///    can itself have companions) and deduplicated across specs
/// 2. Installs all packages
/// 3. Composes a unified environment with all installations
pub fn resolve_multi(specs: &[String], config: &Config, index: &Index) -> Result<ResolvedEnvironment> {
    if specs.is_empty() {
        bail!("At least one spec is required");
    }

    // Phase 1: Parse + alias + collect companions, transitively.
    let all_reqs = collect_transitive_reqs(specs, index)?;

    // Phase 2: Resolve all packages to concrete versions. `all_reqs` is
    // already deduplicated (see `collect_transitive_reqs`), first
    // constraint per project wins.
    let mut packages: Vec<Package> = Vec::new();
    for req in &all_reqs {
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

    // Phase 3: Install all packages (parallel if multiple)
    let installations = install_all(config, &packages)?;

    // Phase 4: Compose unified environment
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

/// Parse `specs`, alias-resolve each, and expand companions TRANSITIVELY —
/// a companion can itself have companions (e.g. rust's `cargo` companion
/// itself needs `openssl@^1.1`), so this is a worklist/BFS over the
/// companion graph, not a single fixed-depth pass. Deduplicated: the first
/// constraint *processed* (BFS order — top-level specs left-to-right,
/// then each one's companions) for a project wins, and a project already
/// queued is never re-expanded (also guards against a cycle in
/// hand-authored index data spinning forever).
///
/// **Known limitation**: when two different paths need the SAME companion
/// under DIFFERENT constraints (e.g. one spec's companion wants bare
/// `openssl` [any version] and another's wants `openssl@^1.1`
/// specifically), whichever is processed first wins — there's no
/// constraint-intersection logic, so the loser's real requirement can be
/// silently unsatisfied if the winner resolves to an incompatible version.
/// Not hit by any single-toolchain `buckets build`/`run` today (the
/// conflict needs two top-level specs whose companion graphs collide);
/// worth fixing with real intersection logic if that changes.
fn collect_transitive_reqs(specs: &[String], index: &Index) -> Result<Vec<PackageReq>> {
    let mut all_reqs: Vec<PackageReq> = Vec::new();
    let mut queued = std::collections::HashSet::new();
    let mut worklist: std::collections::VecDeque<PackageReq> = specs
        .iter()
        .map(|spec| PackageReq::parse(spec).with_context(|| format!("Failed to parse spec: {spec}")))
        .collect::<Result<_>>()?;

    while let Some(req) = worklist.pop_front() {
        let project = index.resolve_alias(&req.project).to_string();
        if !queued.insert(project.clone()) {
            continue;
        }
        all_reqs.push(PackageReq { project: project.clone(), constraint: req.constraint });

        // Companion entries are full specs ("openssl@^1.1", not bare
        // "openssl") so a companion can pin an exact version range — some
        // packages dynamically link a SPECIFIC major of a shared companion
        // (e.g. node needs openssl 1.1's libcrypto.so.1.1, not the latest
        // 3.x's libcrypto.so.3), so defaulting to STAR/latest here would
        // silently install a version that doesn't satisfy the actual
        // runtime dependency.
        for companion in index.companions(&project) {
            let companion_spec = PackageReq::parse(companion)
                .with_context(|| format!("Failed to parse companion spec: {companion}"))?;
            worklist.push_back(companion_spec);
        }
    }

    Ok(all_reqs)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for a real bug: "rust" -> companion "rust-lang.org/
    /// cargo" -> companion "openssl@^1.1" is a TWO-level chain. The
    /// original single-pass companion loop only expanded one level, so
    /// `buckets build` on a real Rust project resolved rustc+cargo but
    /// silently dropped openssl — cargo then failed at runtime with
    /// "libssl.so.1.1: cannot open shared object file".
    #[test]
    fn transitive_companion_of_companion_is_included() {
        let index = Index::builtin();
        let reqs = collect_transitive_reqs(&["rust".to_string()], &index).unwrap();
        let projects: Vec<&str> = reqs.iter().map(|r| r.project.as_str()).collect();
        assert!(projects.contains(&"rust-lang.org"), "{projects:?}");
        assert!(projects.contains(&"rust-lang.org/cargo"), "{projects:?}");
        assert!(projects.contains(&"openssl.org"), "{projects:?}");
    }

    /// node -> openssl/icu4c (one level) still works after the rewrite.
    #[test]
    fn single_level_companions_still_included() {
        let index = Index::builtin();
        let reqs = collect_transitive_reqs(&["node".to_string()], &index).unwrap();
        let projects: Vec<&str> = reqs.iter().map(|r| r.project.as_str()).collect();
        assert!(projects.contains(&"nodejs.org"), "{projects:?}");
        assert!(projects.contains(&"openssl.org"), "{projects:?}");
        assert!(projects.contains(&"unicode.org"), "{projects:?}");
    }

    /// A project pulled in as a companion from two different top-level
    /// specs (or two branches of the companion graph) is only queued once.
    #[test]
    fn shared_companion_deduplicated() {
        let index = Index::builtin();
        // Both "rust" (via cargo) and "curl" declare openssl as a companion,
        // under DIFFERENT constraints (^1.1 vs bare/STAR) — only asserting
        // dedup happens (exactly one entry), not which constraint wins.
        // See collect_transitive_reqs's doc comment: that's a known,
        // unresolved limitation, not something this test claims is correct.
        let reqs = collect_transitive_reqs(&["rust".to_string(), "curl".to_string()], &index).unwrap();
        let openssl_count = reqs.iter().filter(|r| r.project == "openssl.org").count();
        assert_eq!(openssl_count, 1, "{reqs:?}");
    }

    #[test]
    fn no_companions_is_just_the_spec_itself() {
        let index = Index::builtin();
        let reqs = collect_transitive_reqs(&["python".to_string()], &index).unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].project, "python.org");
    }
}
