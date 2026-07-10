# buckets

**Throwaway runtime buckets for AI agents.**

Resolve, fetch, and run any CLI tool in an isolated ephemeral environment — without installing it globally.

Inspired by [pkgx](https://pkgx.dev) and [exosphere](https://github.com/nixpulvis/exosphere)'s provisioning pipeline.

## Why buckets?

AI agents need ad-hoc runtimes: "run this script with Node 20", "test this with Python 3.11", "check this with ripgrep". Installing these globally pollutes the host and creates version conflicts.

**buckets** gives each command its own throwaway runtime — fetched on demand, cached locally, cleaned up when done.

## Usage

```bash
# Run one-off commands
buckets run node@20 -- script.js --flag arg
buckets run python@3.11 -- -c "print('hello from a bucket')"
buckets run rust@latest -- rustc --version

# Open an interactive shell with the runtime in PATH
buckets shell node@20
buckets shell python@3.11 --shell /bin/zsh

# See what a spec resolves to (no installation)
buckets info node@20
buckets info python@latest

# List cached installations
buckets list
```

## Spec format

```
<tool>[@<version>]
```

- `node@20` — Node.js 20.x (latest in range)
- `python@3.11.0` — exact Python 3.11.0
- `rust@latest` �� latest stable Rust
- `node` — latest (same as `@latest`)
- `go@^1.22` — standard semver range

Aliases resolve to pkgx project names:
- `node` → `nodejs.org`
- `python` → `python.org`
- `rust` ��� `rust-lang.org`
- `go` → `golang.org`
- `ripgrep` → `BurntSushi/ripgrep`
- And 20+ more (see `src/index.rs`)

## How it works

1. **Parse** spec → project + version constraint
2. **Resolve** alias → full pkgx project name
3. **Check cache** for installed version matching constraint
4. **Fetch remote** `versions.txt` from `dist.pkgx.dev` if not cached
5. **Download** the `.tar.xz` bottle from `dist.pkgx.dev`
6. **Extract** to `~/.cache/buckets/<project>/v<version>/`
7. **Compose** env (PATH, LD_LIBRARY_PATH, etc.) from extracted directories
8. **Run** the command or open a shell with the composed environment

## Configuration

| Env var | Default | Description |
|---|---|---|
| `BUCKETS_DIST_URL` | `https://dist.pkgx.dev` | Distribution server URL |
| `BUCKETS_CACHE_DIR` | `~/.cache/buckets/` or `~/.buckets/` | Local cache directory |

## Installation

```bash
cargo install --path .
# or
cargo build --release && cp target/release/buckets ~/.local/bin/
```

## Design

Borrowed from exosphere's `exo-hydra` provisioning crate but simplified:
- **No SQLite/daemon** — baked-in alias index, filesystem-based cache
- **No transitive deps** — resolves single packages (most runtimes are self-contained)
- **No capability system** — just PATH setup + exec
- **Synchronous** — uses `ureq` instead of tokio
- **Standalone** — no exosphere or project workspace dependency

## License

MIT
