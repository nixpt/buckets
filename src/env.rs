use std::collections::HashMap;
use std::path::PathBuf;

use crate::types::{Installation, Package, ResolvedEnvironment};

/// Compose environment variables for a set of installations.
///
/// For each installation, scans standard subdirectories and maps them to
/// the corresponding environment variables:
/// - `bin/`, `sbin/` → `PATH`
/// - `lib/`, `lib64/` → `LD_LIBRARY_PATH` (Linux) / `DYLD_FALLBACK_LIBRARY_PATH` (macOS)
/// - `include/` → `CPATH`
/// - `man/`, `share/man/` → `MANPATH`
/// - `share/` → `XDG_DATA_DIRS`
/// - `share/pkgconfig/`, `lib/pkgconfig/` → `PKG_CONFIG_PATH`
///
/// Values are prepended to the current environment (if any).
pub fn compose_env(installations: &[Installation]) -> HashMap<String, String> {
    let mut env = HashMap::new();

    let mut path_entries: Vec<PathBuf> = Vec::new();
    let mut ld_library_path: Vec<PathBuf> = Vec::new();
    let mut cpath: Vec<PathBuf> = Vec::new();
    let mut manpath: Vec<PathBuf> = Vec::new();
    let mut xdg_data_dirs: Vec<PathBuf> = Vec::new();
    let mut pkg_config_path: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    let mut dyld_fallback_path: Vec<PathBuf> = Vec::new();

    for inst in installations {
        let base = &inst.path;

        // bin/ — PATH
        let bin_dir = base.join("bin");
        if bin_dir.exists() {
            path_entries.push(bin_dir);
        }
        let sbin_dir = base.join("sbin");
        if sbin_dir.exists() {
            path_entries.push(sbin_dir);
        }

        // lib/ + lib64/ — LD_LIBRARY_PATH
        let lib_dir = base.join("lib");
        if lib_dir.exists() {
            ld_library_path.push(lib_dir);
        }
        let lib64_dir = base.join("lib64");
        if lib64_dir.exists() {
            ld_library_path.push(lib64_dir);
        }

        // include/ — CPATH
        let include_dir = base.join("include");
        if include_dir.exists() {
            cpath.push(include_dir);
        }

        // man/ — MANPATH
        let man_dir = base.join("man");
        if man_dir.exists() {
            manpath.push(man_dir);
        }
        let share_man_dir = base.join("share").join("man");
        if share_man_dir.exists() {
            manpath.push(share_man_dir);
        }

        // share/ — XDG_DATA_DIRS
        let share_dir = base.join("share");
        if share_dir.exists() {
            xdg_data_dirs.push(share_dir);
        }

        // share/pkgconfig/ + lib/pkgconfig/ — PKG_CONFIG_PATH
        let share_pc = base.join("share").join("pkgconfig");
        if share_pc.exists() {
            pkg_config_path.push(share_pc);
        }
        let lib_pc = base.join("lib").join("pkgconfig");
        if lib_pc.exists() {
            pkg_config_path.push(lib_pc);
        }

        #[cfg(target_os = "macos")]
        {
            let lib_dir = base.join("lib");
            if lib_dir.exists() {
                dyld_fallback_path.push(lib_dir);
            }
        }
    }

    prepend_path(&mut env, "PATH", &dedup_ordered(&path_entries));
    prepend_path(&mut env, "LD_LIBRARY_PATH", &dedup_ordered(&ld_library_path));
    prepend_path(&mut env, "CPATH", &dedup_ordered(&cpath));
    prepend_path(&mut env, "MANPATH", &dedup_ordered(&manpath));
    prepend_path(&mut env, "XDG_DATA_DIRS", &dedup_ordered(&xdg_data_dirs));
    prepend_path(&mut env, "PKG_CONFIG_PATH", &dedup_ordered(&pkg_config_path));

    #[cfg(target_os = "macos")]
    prepend_path(&mut env, "DYLD_FALLBACK_LIBRARY_PATH", &dedup_ordered(&dyld_fallback_path));

    env
}

/// Prepend new entries to an existing env var (or set if not present).
fn prepend_path(env: &mut HashMap<String, String>, key: &str, new_entries: &[String]) {
    if new_entries.is_empty() {
        return;
    }

    let new_part = new_entries.join(":");
    let old_val = std::env::var(key).ok();

    let value = match old_val {
        Some(existing) if !existing.is_empty() => format!("{new_part}:{existing}"),
        _ => new_part,
    };

    env.insert(key.to_string(), value);
}

/// Deduplicate while preserving order.
fn dedup_ordered(items: &[PathBuf]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for item in items {
        let s = item.to_string_lossy().to_string();
        if seen.insert(s.clone()) {
            result.push(s);
        }
    }
    result
}

/// Build a `ResolvedEnvironment` from installations + entry + all packages.
#[allow(dead_code)]
pub fn build_resolved_env(
    installations: Vec<Installation>,
    entry: Package,
    all_packages: Vec<Package>,
) -> ResolvedEnvironment {
    let env = compose_env(&installations);
    ResolvedEnvironment {
        installations,
        env,
        entry,
        all_packages,
    }
}

/// Format a ResolvedEnvironment as shell export statements.
///
/// ```sh
/// export PATH="/home/user/.buckets/nodejs.org/v20.11.0/bin:$PATH"
/// export LD_LIBRARY_PATH="..."
/// ```
pub fn format_shell_exports(env: &ResolvedEnvironment) -> String {
    let mut out = String::new();

    // Comment header
    out.push_str("# buckets environment\n");

    for (key, value) in &env.env {
        out.push_str(&format!("export {}=\"{}\"\n", key, value));
    }

    // Also emit PATH additions as individual components for easy inspection
    if let Some(path) = env.env.get("PATH") {
        out.push_str("\n# PATH components:\n");
        for (i, component) in path.split(':').enumerate() {
            out.push_str(&format!("#   [{i}] {component}\n"));
        }
    }

    out
}

/// Format a ResolvedEnvironment as JSON.
pub fn format_json(env: &ResolvedEnvironment) -> Result<String, serde_json::Error> {
    #[derive(serde::Serialize)]
    struct JsonOutput<'a> {
        version: u32,
        environment: &'a HashMap<String, String>,
        packages: Vec<JsonPackage>,
    }

    #[derive(serde::Serialize)]
    struct JsonPackage {
        project: String,
        version: String,
        path: String,
    }

    let packages: Vec<JsonPackage> = env.installations.iter().map(|inst| JsonPackage {
        project: inst.pkg.project.clone(),
        version: inst.pkg.version.to_string(),
        path: inst.path.to_string_lossy().to_string(),
    }).collect();

    let output = JsonOutput {
        version: 2,
        environment: &env.env,
        packages,
    };

    serde_json::to_string_pretty(&output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compose_env_no_installations() {
        let env = compose_env(&[]);
        assert!(env.is_empty());
    }

    #[test]
    fn test_dedup_ordered() {
        let paths = vec![
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            PathBuf::from("/a"),
            PathBuf::from("/c"),
        ];
        let result = dedup_ordered(&paths);
        assert_eq!(result, vec!["/a", "/b", "/c"]);
    }

    #[test]
    fn test_prepend_path_preserves_existing() {
        let mut env = HashMap::new();
        prepend_path(&mut env, "TEST_PATH", &["/new".to_string()]);
        assert_eq!(env.get("TEST_PATH").unwrap(), "/new");
    }

    #[test]
    fn test_format_shell_exports() {
        let env = ResolvedEnvironment {
            installations: vec![],
            env: HashMap::from([("PATH".into(), "/a:/b".into())]),
            entry: Package { project: "test".into(), version: semver::Version::new(1, 0, 0) },
            all_packages: vec![],
        };
        let output = format_shell_exports(&env);
        assert!(output.contains("export PATH=\"/a:/b\""));
    }
}
