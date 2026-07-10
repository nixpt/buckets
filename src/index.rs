//! The built-in alias index: shorthand names (`node`) to their pkgx pantry
//! project names (`nodejs.org`), the binaries each project provides, and
//! companion packages to auto-include (e.g. `openssl` alongside `curl`).
//! [`crate::resolve` module](mod@crate::resolve) consults this before doing any network/cache work.

use std::collections::HashMap;

/// A curated index of commonly-used tools, mapping shorthand names
/// to their pkgx pantry project names and the binaries they provide.
///
/// This allows writing `node@20` instead of `nodejs.org@20`.
pub struct Index {
    /// Shorthand alias ��� project name (e.g. "node" → "nodejs.org")
    aliases: HashMap<&'static str, &'static str>,
    // Project name → list of provided binaries
    provides: HashMap<&'static str, Vec<&'static str>>,
    /// Project name �� companion packages (auto-included deps)
    companions: HashMap<&'static str, Vec<&'static str>>,
}

impl Index {
    /// Create the built-in index with curated entries
    /// borrowing heavily from pkgx's pantry knowledge.
    pub fn builtin() -> Self {
        let mut aliases = HashMap::new();
        let mut provides = HashMap::new();
        let mut companions = HashMap::new();

        // ═══════════════════════════════════════════════════════════
        // Languages & Runtimes
        // ═══════════════════════════════════════════════════════════

        aliases.insert("node",    "nodejs.org");
        provides.insert("nodejs.org", vec!["node", "npm", "npx", "corepack"]);
        // node itself dynamically links openssl 1.1 + icu4c 73 (verified via
        // `ldd` against a real v20.20.2 bottle — libcrypto.so.1.1/libssl.so.1.1/
        // libicu{i18n,uc,data}.so.73 all "not found" without these). Pinned,
        // not bare "openssl"/"icu4c" latest — 3.x openssl ships .so.3, not
        // .so.1.1, so an unpinned companion would still leave node broken.
        companions.insert("nodejs.org", vec!["openssl@^1.1", "icu4c@^73"]);

        aliases.insert("python",  "python.org");
        provides.insert("python.org", vec!["python3", "python", "pip3", "pip", "venv"]);
        companions.insert("python.org", vec![]);

        // rust-lang.org's bottle ships rustc/rustfmt/clippy/rust-analyzer/
        // rustdoc — but NOT cargo (or rustup). Verified live: `bin/` has no
        // `cargo` at all. pkgx models cargo as a separate nested project
        // (`rust-lang.org/cargo`), so it's a companion, not bundled.
        aliases.insert("rust",    "rust-lang.org");
        provides.insert("rust-lang.org", vec!["rustc", "rustfmt", "rustdoc", "rust-analyzer"]);
        companions.insert("rust-lang.org", vec!["rust-lang.org/cargo"]);
        // cargo itself dynamically links openssl 1.1 (verified via ldd on a
        // real downloaded bottle, the same libcrypto/libssl.so.1.1 gap node
        // hit — see the nodejs.org companions comment above).
        companions.insert("rust-lang.org/cargo", vec!["openssl@^1.1"]);
        provides.insert("rust-lang.org/cargo", vec!["cargo"]);

        aliases.insert("go",      "golang.org");
        provides.insert("golang.org", vec!["go", "gofmt"]);
        companions.insert("golang.org", vec![]);

        aliases.insert("deno",    "deno.land");
        provides.insert("deno.land", vec!["deno"]);
        companions.insert("deno.land", vec![]);

        aliases.insert("bun",     "bun.sh");
        provides.insert("bun.sh", vec!["bun"]);
        companions.insert("bun.sh", vec![]);

        aliases.insert("php",     "php.net");
        provides.insert("php.net", vec!["php", "phar", "phpize"]);
        companions.insert("php.net", vec![]);

        aliases.insert("ruby",    "ruby-lang.org");
        provides.insert("ruby-lang.org", vec!["ruby", "gem", "bundler", "irb"]);
        companions.insert("ruby-lang.org", vec![]);

        aliases.insert("perl",    "perl.org");
        provides.insert("perl.org", vec!["perl", "cpan"]);
        companions.insert("perl.org", vec![]);

        // ═══════════════════════════════════════════════════════════
        // Shells
        // ��══════════════════════════════════════════════════════════

        aliases.insert("bash",    "gnu.org/bash");
        provides.insert("gnu.org/bash", vec!["bash"]);

        aliases.insert("zsh",     "zsh.org");
        provides.insert("zsh.org", vec!["zsh"]);

        aliases.insert("fish",    "fishshell.com");
        provides.insert("fishshell.com", vec!["fish"]);

        aliases.insert("dash",    "dash.interface");
        provides.insert("dash.interface", vec!["dash"]);

        aliases.insert("sh",      "gnu.org/bash"); // symlink alias

        // ═══════════════════════════════════════════════════════════
        // VCS / Source Control
        // ═══════════════════════════════════════════════════════════

        aliases.insert("git",     "git-scm.com");
        provides.insert("git-scm.com", vec!["git"]);
        companions.insert("git-scm.com", vec!["openssl", "curl", "expat", "zlib"]);

        aliases.insert("hg",      "mercurial-scm.org");
        provides.insert("mercurial-scm.org", vec!["hg"]);

        aliases.insert("svn",     "subversion.apache.org");
        provides.insert("subversion.apache.org", vec!["svn"]);

        // ═══════════════════════════════════════════════════════════
        // Networking
        // ═══════════════════════════════════════════════════════════

        aliases.insert("curl",    "curl.se");
        provides.insert("curl.se", vec!["curl"]);
        companions.insert("curl.se", vec!["openssl", "zlib", "nghttp2"]);

        aliases.insert("wget",    "gnu.org/wget");
        provides.insert("gnu.org/wget", vec!["wget"]);

        aliases.insert("httpie",  "httpie.io");
        provides.insert("httpie.io", vec!["http"]);

        aliases.insert("websocat","websocat");
        provides.insert("websocat", vec!["websocat"]);

        // ═══════════════════════════════════════════════════════════
        // Text Processing
        // ═══════════════════════════════════════════════════════════

        // pkgx's real dist-server project name is "stedolan.github.io/jq"
        // (jq's original author's domain) — verified live, "jqlang.org" 404s.
        aliases.insert("jq",      "stedolan.github.io/jq");
        provides.insert("stedolan.github.io/jq", vec!["jq"]);

        aliases.insert("yq",      "mikefarah.git.io/yq");
        provides.insert("mikefarah.git.io/yq", vec!["yq"]);

        aliases.insert("ripgrep", "BurntSushi/ripgrep");
        provides.insert("BurntSushi/ripgrep", vec!["rg"]);

        aliases.insert("rg",      "BurntSushi/ripgrep");

        aliases.insert("fd",      "sharkdp/fd");
        provides.insert("sharkdp/fd", vec!["fd"]);

        aliases.insert("bat",     "sharkdp/bat");
        provides.insert("sharkdp/bat", vec!["bat"]);

        aliases.insert("delta",   "dandavison/delta");
        provides.insert("dandavison/delta", vec!["delta"]);

        aliases.insert("sd",      "chmln/sd");
        provides.insert("chmln/sd", vec!["sd"]);

        aliases.insert("fzf",     "junegunn/fzf");
        provides.insert("junegunn/fzf", vec!["fzf"]);

        aliases.insert("zoxide",  "zoxide");
        provides.insert("zoxide", vec!["zoxide"]);

        // ═══════════════════════════════════════════════════════════
        // File System
        // ═══════════════════════════════════════════════════════════

        aliases.insert("eza",     "eza.rocks");
        provides.insert("eza.rocks", vec!["eza"]);

        aliases.insert("lsd",     "lsd");
        provides.insert("lsd", vec!["lsd"]);

        aliases.insert("tree",    "oldman.science/tree");
        provides.insert("oldman.science/tree", vec!["tree"]);

        aliases.insert("duf",     "muesli/duf");
        provides.insert("muesli/duf", vec!["duf"]);

        aliases.insert("dust",    "bootandy/dust");
        provides.insert("bootandy/dust", vec!["dust"]);

        aliases.insert("procs",   "dalance/procs");
        provides.insert("dalance/procs", vec!["procs"]);

        aliases.insert("bottom",  "bottom");
        provides.insert("bottom", vec!["btm"]);

        aliases.insert("htop",    "htop.dev");
        provides.insert("htop.dev", vec!["htop"]);

        // ══════════════════════════════════════════════���════════════
        // Build Tools
        // ════���══════════════════════════════════════════════════════

        aliases.insert("make",    "gnu.org/make");
        provides.insert("gnu.org/make", vec!["make"]);

        aliases.insert("cmake",   "cmake.org");
        provides.insert("cmake.org", vec!["cmake", "ctest", "cpack"]);
        companions.insert("cmake.org", vec!["openssl", "zlib", "curl"]);

        // cmake companions already handle deps via PATH

        aliases.insert("ninja",   "ninja-build.org");
        provides.insert("ninja-build.org", vec!["ninja"]);

        aliases.insert("maven",   "apache.org/maven");
        provides.insert("apache.org/maven", vec!["mvn"]);

        aliases.insert("gradle",  "gradle.org");
        provides.insert("gradle.org", vec!["gradle"]);

        aliases.insert("bazel",   "bazel.build");
        provides.insert("bazel.build", vec!["bazel"]);

        aliases.insert("just",    "just.systems");
        provides.insert("just.systems", vec!["just"]);

        aliases.insert("task",    "go-task.io/task");
        provides.insert("go-task.io/task", vec!["task"]);

        aliases.insert("scons",   "scons.org");
        provides.insert("scons.org", vec!["scons"]);

        aliases.insert("meson",   "mesonbuild.com");
        provides.insert("mesonbuild.com", vec!["meson"]);

        // ═══════════════════════════════════════════════════════════
        // Package Managers
        // ═══════════════════════════════════════════════════════════

        aliases.insert("pnpm",    "pnpm.io");
        provides.insert("pnpm.io", vec!["pnpm"]);

        aliases.insert("yarn",    "yarnpkg.com");
        provides.insert("yarnpkg.com", vec!["yarn"]);

        aliases.insert("pipx",    "pypa/pipx");
        provides.insert("pypa/pipx", vec!["pipx"]);
        companions.insert("pypa/pipx", vec!["python.org"]);

        aliases.insert("uv",      "astral.sh/uv");
        provides.insert("astral.sh/uv", vec!["uv", "uvx"]);

        aliases.insert("cargo-binstall", "cargo-binstall");
        provides.insert("cargo-binstall", vec!["cargo-binstall"]);

        // ═══════════════════════════════════════════════════════════
        // Databases
        // ═══════════════════════════════════════════════════════════

        aliases.insert("sqlite",  "sqlite.org");
        provides.insert("sqlite.org", vec!["sqlite3"]);

        aliases.insert("postgres","postgresql.org");
        provides.insert("postgresql.org", vec!["psql", "pg_dump", "pg_restore", "createdb"]);

        aliases.insert("mysql",   "mysql.com");
        provides.insert("mysql.com", vec!["mysql", "mysqldump"]);

        aliases.insert("redis",   "redis.io");
        provides.insert("redis.io", vec!["redis-cli", "redis-server"]);

        aliases.insert("sqlite3", "sqlite.org");

        // ═══════════════════════════════════════════════════════════
        // Cloud / Infrastructure
        // ═══════════���═══════════════════════════════════════════════

        aliases.insert("docker",  "docker.com");
        provides.insert("docker.com", vec!["docker"]);

        aliases.insert("kubectl", "kubernetes.io/kubectl");
        provides.insert("kubernetes.io/kubectl", vec!["kubectl"]);

        aliases.insert("helm",    "helm.sh");
        provides.insert("helm.sh", vec!["helm"]);

        aliases.insert("terraform", "terraform.io");
        provides.insert("terraform.io", vec!["terraform"]);

        aliases.insert("aws",     "amazon.com/aws-cli");
        provides.insert("amazon.com/aws-cli", vec!["aws"]);

        aliases.insert("gcloud",  "google.com/cloud-sdk");
        provides.insert("google.com/cloud-sdk", vec!["gcloud", "gsutil"]);

        aliases.insert("gh",      "github.com/cli");
        provides.insert("github.com/cli", vec!["gh"]);

        aliases.insert("doctl",   "digitalocean.com/doctl");
        provides.insert("digitalocean.com/doctl", vec!["doctl"]);

        // ═══════════════════════════════════════════════════════════
        // Compression / Archival
        // ═══════════════════════════════════════════════════════════

        aliases.insert("tar",     "gnu.org/tar");
        provides.insert("gnu.org/tar", vec!["tar"]);

        aliases.insert("gzip",    "gnu.org/gzip");
        provides.insert("gnu.org/gzip", vec!["gzip"]);

        aliases.insert("zstd",    "facebook.com/zstd");
        provides.insert("facebook.com/zstd", vec!["zstd"]);

        aliases.insert("unzip",   "info-zip.org/unzip");
        provides.insert("info-zip.org/unzip", vec!["unzip"]);

        aliases.insert("xz",      "tukaani.org/xz");
        provides.insert("tukaani.org/xz", vec!["xz"]);

        aliases.insert("brotli",  "google.com/brotli");
        provides.insert("google.com/brotli", vec!["brotli"]);

        // ═══════════════════════════════════════════════════════════
        // System / Utils
        // ═══════════════════════════════════════════════════════════

        aliases.insert("tmux",    "tmux.github.io");
        provides.insert("tmux.github.io", vec!["tmux"]);

        aliases.insert("neovim",  "neovim.io");
        provides.insert("neovim.io", vec!["nvim"]);

        aliases.insert("vim",     "vim.org");
        provides.insert("vim.org", vec!["vim"]);

        aliases.insert("emacs",   "gnu.org/emacs");
        provides.insert("gnu.org/emacs", vec!["emacs"]);

        aliases.insert("less",    "gnu.org/less");
        provides.insert("gnu.org/less", vec!["less"]);

        aliases.insert("ssh",     "openssh.org");
        provides.insert("openssh.org", vec!["ssh", "scp", "sshd"]);

        aliases.insert("rsync",   "rsync.samba.org");
        provides.insert("rsync.samba.org", vec!["rsync"]);

        aliases.insert("screen",  "gnu.org/screen");
        provides.insert("gnu.org/screen", vec!["screen"]);

        aliases.insert("strace",  "strace.io");
        provides.insert("strace.io", vec!["strace"]);

        // ═══════════════════════════════════════════════════════════
        // Companion-only packages (no alias entry needed)
        // ═══════════════════════════════════════════════════════════

        companions.insert("openssl",  vec![]);
        companions.insert("zlib",     vec![]);
        companions.insert("expat",    vec![]);
        companions.insert("nghttp2",  vec![]);
        companions.insert("icu4c",    vec![]);
        // Provide aliases for companion packages too
        aliases.insert("openssl", "openssl.org");
        provides.insert("openssl.org", vec!["openssl"]);
        aliases.insert("icu4c", "unicode.org");
        provides.insert("unicode.org", vec!["icu4c"]);

        Self { aliases, provides, companions }
    }

    /// Resolve a shorthand alias to the full project name.
    /// If no alias is found, returns the input as-is.
    pub fn resolve_alias<'a>(&'a self, name: &'a str) -> &'a str {
        self.aliases.get(name).copied().unwrap_or(name)
    }

    /// List the binaries provided by a project.
    pub fn provides(&self, project: &str) -> &[&str] {
        self.provides.get(project).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// List companion packages for a project (auto-included deps).
    pub fn companions(&self, project: &str) -> &[&str] {
        // Check both the raw project name and the alias-resolved name
        // (companions are stored by project name, not alias)
        self.companions.get(project).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Check if a project is known to the index (has an alias or provides entry).
    #[allow(dead_code)]
    pub fn is_known(&self, project: &str) -> bool {
        // Resolve alias first, then check if we have provides or companions info
        let resolved = self.resolve_alias(project);
        self.provides.contains_key(resolved) || self.companions.contains_key(resolved)
    }

    /// Iterate over all alias entries.
    #[allow(dead_code)]
    pub fn iter_aliases(&self) -> impl Iterator<Item = (&str, &str)> {
        self.aliases.iter().map(|(k, v)| (*k, *v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_known_alias() {
        let idx = Index::builtin();
        assert_eq!(idx.resolve_alias("node"), "nodejs.org");
        assert_eq!(idx.resolve_alias("python"), "python.org");
        assert_eq!(idx.resolve_alias("rg"), "BurntSushi/ripgrep");
    }

    #[test]
    fn test_resolve_unknown_alias() {
        let idx = Index::builtin();
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

    #[test]
    fn test_companions_git() {
        let idx = Index::builtin();
        let comps = idx.companions("git-scm.com");
        assert!(comps.contains(&"openssl"));
        assert!(comps.contains(&"curl"));
    }

    #[test]
    fn test_companions_unknown() {
        let idx = Index::builtin();
        assert!(idx.companions("nonexistent").is_empty());
    }

    #[test]
    fn test_is_known() {
        let idx = Index::builtin();
        assert!(idx.is_known("node"));
        assert!(idx.is_known("nodejs.org"));
        assert!(!idx.is_known("nonexistent"));
    }
}
