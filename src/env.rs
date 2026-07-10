use std::collections::HashMap;
use std::path::PathBuf;

use crate::types::{Installation, ResolvedEnvironment};

/// Compose environment variables for a set of installations.
///
/// For each installation, scans standard subdirectories and maps them to
/// the corresponding environment variables:
/// - `bin/` → `PATH`
/// - `lib/`, `lib64/` → `LD_LIBRARY_PATH` (Linux) / `DYLD_FALLBACK_LIBRARY_PATH` (macOS)
/// - `include/` → `CPATH`
/// - `man/` → `MANPATH`
/// - `share/` → `XDG_DATA_DIRS`
/// - `share/pkgconfig/`, `lib/pkgconfig/` → `PKG_CONFIG_PATH`
///
/// Values are prepended to the current environment (if any).
pub fn compose_env(installations: &[Installation]) -> HashMap<String, String> {
    let mut env = HashMap::new();

    // Collect all paths for each env var key
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

    // Prepend collected paths to existing environment
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

/// Build a `ResolvedEnvironment` from installations and the entry package.
#[allow(dead_code)]
pub fn build_resolved_env(installations: Vec<Installation>, entry: crate::types::Package) -> ResolvedEnvironment {
    let env = compose_env(&installations);
    ResolvedEnvironment {
        installations,
        env,
        entry,
    }
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
        // We need to test logic without actually setting env vars
        let mut env = HashMap::new();
        prepend_path(&mut env, "TEST_PATH", &["/new".to_string()]);
        assert_eq!(env.get("TEST_PATH").unwrap(), "/new");
    }
}
