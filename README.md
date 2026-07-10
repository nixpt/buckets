# buckets

**Throwaway runtime buckets for AI agents.**

Resolve, fetch, and run any CLI tool in an isolated ephemeral environment —
without installing it globally.

Borrowing concepts from [pkgx](https://pkgx.dev) (bottle format, dist server,
env composition, companion deps) and [exosphere](https://github.com/nixpulvis/exosphere)'s
exo-hydra provisioning pipeline (resolve → install → compose → exec).

## Why buckets?

AI agents need ad-hoc runtimes: "run this with Node 20", "test this with
Python 3.11", "build with Go 1.22 + CMake". Installing these globally pollutes
the host and creates version conflicts.

**buckets** gives each command its own throwaway runtime ��� fetched on demand
from `dist.pkgx.dev`, cached locally, cleaned up when done.

## Usage

```bash
# Run one-off commands
buckets run node@20 -- script.js --flag arg
buckets run python@3.11 -- -c "print('hello from a bucket')"
buckets run rust@latest -- rustc --version

# Multi-package environments
buckets run node@20 python@3.11 -- node -e "console.log('and python is at', process.env.PATH)"
buckets run go@1.22 cmake@latest -- go version

# Open an interactive shell with the runtime in PATH
buckets shell node@20
buckets shell python@3.11 --shell /bin/zsh

# Print the composed environment as shell exports (composable)
eval "$(buckets env node@20)"
buckets env node@20 python@3.11 --json

# See what a spec resolves to (no installation)
buckets info node@20
buckets info git@latest

# List cached installations
buckets list
```

## Spec format

```
<tool>[@<version>]
```

| Spec | Meaning |
|---|---|
| `node@20` | Node.js 20.x (latest in `^20` range) |
| `python@=3.11.0` | Exact Python 3.11.0 |
| `rust@latest` | Latest stable Rust |
| `node` | Latest (same as `@latest`) |
| `go@^1.22` | Standard caret semver |
| `go@>=1.22` | Greater-or-equal |
| `rust@~1.70` | Tilde (patch-level) semver |

## Featured aliases (60+ tools)

| Alias | Resolves to |
|---|---|
| `node` | `nodejs.org` |
| `python` | `python.org` |
| `rust` | `rust-lang.org` |
| `go` | `golang.org` |
| `git` | `git-scm.com` |
| `cmake` | `cmake.org` |
| `ripgrep` / `rg` | `BurntSushi/ripgrep` |
| `curl` | `curl.se` |
| `gh` | `github.com/cli` |
| `docker` | `docker.com` |
| `kubectl` | `kubernetes.io/kubectl` |
| `terraform` | `terraform.io` |
| `aws` | `amazon.com/aws-cli` |
| `neovim` | `neovim.io` |
| `tmux` | `tmux.github.io` |
| ... and 45+ more | See `src/index.rs` |

## How it works

1. **Parse** spec → project + semver constraint
2. **Resolve** alias → full pkgx project name
3. **Collect companions** ��� auto-include deps (e.g. openssl for curl, cmake)
4. **Resolve versions** → check cache first, then remote `versions.txt`
5. **Install** → download `.tar.xz` bottle, XZ-decompress, extract, atomic rename
6. **Symlink** → create `v*`, `v<major>`, `v<major.minor>` → latest version
7. **Compose** env → PATH, LD_LIBRARY_PATH, CPATH from installation dirs
8. **Run** or **export** the environment

## Commands

| Command | Description |
|---|---|
| `run <specs> -- <cmd>` | Resolve, install, and exec a command |
| `shell <specs>` | Open an interactive shell with the runtime |
| `env <specs>` | Print shell exports (`--json` for structured output) |
| `info <specs>` | Show resolution without installing |
| `list` | Show cached installations |

## Configuration

| Env var | Default | Description |
|---|---|---|
| `BUCKETS_DIST_URL` | `https://dist.pkgx.dev` | Distribution server URL |
| `BUCKETS_CACHE_DIR` | `~/.cache/buckets/` or `~/.buckets/` | Local cache directory |

## Features borrowed from pkgx

- **Bottle format**: `.tar.xz` from `dist.pkgx.dev/<platform>/<arch>/<project>/v<version>.tar.xz`
- **Cache layout**: `~/.buckets/<project>/v<version>/bin/...`
- **Symlink version scheme**: `v*` → latest, `v20` → latest v20.x.x
- **Companion packages**: auto-included deps (openssl for curl, etc.)
- **Env composition**: scan `bin/`, `lib/`, `include/`, `share/`, `man/` per installation
- **Shell exports format**: `export PATH="/path/to/bucket/bin:$PATH"`
- **JSON output**: `--json` for structured consumption
- **Multi-package**: `buckets run node@20 python@3.11 -- node -e "..."`

## Installation

```bash
cargo install --path .
```

## License

MIT
