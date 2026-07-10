use std::collections::HashMap;

/// A curated index of commonly-used tools, mapping shorthand names
/// to their pkgx pantry project names and the binaries they provide.
///
/// This allows writing `node@20` instead of `nodejs.org@20`.
pub struct Index {
    /// Shorthand alias ��� project name (e.g. "node" → "nodejs.org")
    aliases: HashMap<&'static str, &'static str>,
    /// Project name → list of provided binaries
    provides: HashMap<&'static str, Vec<&'static str>>,
}

impl Index {
    /// Create the built-in index with curated entries.
    pub fn builtin() -> Self {
        let mut aliases = HashMap::new();
        let mut provides = HashMap::new();

        // ── Languages & Runtimes ──────────────────────────────────
        add(&mut aliases, &mut provides, "node",     "nodejs.org",     &["node", "npm", "npx", "corepack"]);
        add(&mut aliases, &mut provides, "python",   "python.org",     &["python3", "python", "pip3", "pip", "venv"]);
        add(&mut aliases, &mut provides, "rust",     "rust-lang.org",  &["rustc", "cargo", "rustup", "rustdoc"]);
        add(&mut aliases, &mut provides, "go",       "golang.org",     &["go", "gofmt"]);
        add(&mut aliases, &mut provides, "deno",     "deno.land",      &["deno"]);
        add(&mut aliases, &mut provides, "bun",      "bun.sh",         &["bun"]);

        // ── Shells & Tools ────────────────────────────────────────
        add(&mut aliases, &mut provides, "bash",     "gnu.org/bash",           &["bash"]);
        add(&mut aliases, &mut provides, "zsh",      "zsh.org",               &["zsh"]);
        add(&mut aliases, &mut provides, "fish",     "fishshell.com",         &["fish"]);
        add(&mut aliases, &mut provides, "git",      "git-scm.com",           &["git"]);
        add(&mut aliases, &mut provides, "curl",     "curl.se",               &["curl"]);
        add(&mut aliases, &mut provides, "wget",     "gnu.org/wget",          &["wget"]);
        add(&mut aliases, &mut provides, "jq",       "jqlang.org",            &["jq"]);
        add(&mut aliases, &mut provides, "yq",       "mikefarah.git.io/yq",   &["yq"]);
        add(&mut aliases, &mut provides, "ripgrep",  "BurntSushi/ripgrep",    &["rg"]);
        add(&mut aliases, &mut provides, "fd",       "sharkdp/fd",            &["fd"]);
        add(&mut aliases, &mut provides, "bat",      "sharkdp/bat",           &["bat"]);
        add(&mut aliases, &mut provides, "eza",      "eza.rocks",             &["eza"]);

        // ── Build Tools ───────────────────────────────────────────
        add(&mut aliases, &mut provides, "make",     "gnu.org/make",    &["make"]);
        add(&mut aliases, &mut provides, "cmake",    "cmake.org",       &["cmake", "ctest", "cpack"]);
        add(&mut aliases, &mut provides, "ninja",    "ninja-build.org", &["ninja"]);
        add(&mut aliases, &mut provides, "maven",    "apache.org/maven",&["mvn"]);
        add(&mut aliases, &mut provides, "gradle",   "gradle.org",      &["gradle"]);

        // ── Package Managers ──────────────────────────────────────
        add(&mut aliases, &mut provides, "pnpm",     "pnpm.io",        &["pnpm"]);
        add(&mut aliases, &mut provides, "yarn",     "yarnpkg.com",    &["yarn"]);
        add(&mut aliases, &mut provides, "pipx",     "pypa/pipx",      &["pipx"]);
        add(&mut aliases, &mut provides, "uv",       "astral.sh/uv",   &["uv", "uvx"]);

        // ── Databases ─────────────────────────────────────────────
        add(&mut aliases, &mut provides, "sqlite",   "sqlite.org",     &["sqlite3"]);
        add(&mut aliases, &mut provides, "postgres", "postgresql.org", &["psql", "pg_dump", "pg_restore"]);

        Self { aliases, provides }
    }

    /// Resolve a shorthand alias to the full project name.
    /// If no alias is found, returns the input as-is (pass-through for
    /// direct project names like "nodejs.org").
    pub fn resolve_alias<'a>(&'a self, name: &'a str) -> &'a str {
        self.aliases.get(name).copied().unwrap_or(name)
    }

    /// List the binaries provided by a project.
    pub fn provides(&self, project: &str) -> &[&str] {
        self.provides.get(project).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Iterate over all alias entries.
    #[allow(dead_code)]
    pub fn iter_aliases(&self) -> impl Iterator<Item = (&str, &str)> {
        self.aliases.iter().map(|(k, v)| (*k, *v))
    }
}

fn add<'a>(
    aliases: &mut HashMap<&'static str, &'static str>,
    provides: &mut HashMap<&'static str, Vec<&'static str>>,
    alias: &'static str,
    project: &'static str,
    bins: &[&'static str],
) {
    aliases.insert(alias, project);
    provides.insert(project, bins.to_vec());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_known_alias() {
        let idx = Index::builtin();
        assert_eq!(idx.resolve_alias("node"), "nodejs.org");
        assert_eq!(idx.resolve_alias("python"), "python.org");
    }

    #[test]
    fn test_resolve_unknown_alias() {
        let idx = Index::builtin();
        // Unknown names pass through
        assert_eq!(idx.resolve_alias("nodejs.org"), "nodejs.org");
        assert_eq!(idx.resolve_alias("some-random-tool"), "some-random-tool");
    }

    #[test]
    fn test_provides_known() {
        let idx = Index::builtin();
        let bins = idx.provides("nodejs.org");
        assert!(bins.contains(&"node"));
        assert!(bins.contains(&"npm"));
    }

    #[test]
    fn test_provides_unknown() {
        let idx = Index::builtin();
        assert!(idx.provides("nonexistent").is_empty());
    }
}
