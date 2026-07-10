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

**buckets** gives each command its own throwaway runtime — fetched on demand
from `dist.pkgx.dev`, cached locally, cleaned up when done.

## Using it from the squad fleet

Any agent with shell access (bro-cli's `bash` tool, a dispatched Claude Code
horse, any runner) can call `buckets` directly once it's on `PATH` — it's a
plain CLI, no MCP/tool registration needed. For concurrent fleet agents,
prefer `bucket-bridge` (`jokersquad/bin/bucket-bridge`, symlinked to
`~/.local/bin/`) over calling `buckets` directly: it's a transparent wrapper
that points every agent at one shared `BUCKETS_CACHE_DIR`
(`~/.cache/squadron-buckets`) instead of each agent maintaining its own
cache, so N agents resolving the same spec (`node@20`) don't each
redundantly download the same bottle. Safe under concurrency — see
`install.rs`'s atomic-rename install.

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

# Build/test/run a real project — clone (git URL) or use (local path),
# detect the build system, resolve the toolchain it needs, build inside
# a sandboxed bucket. Doesn't touch the host filesystem outside the
# project dir + the resolved toolchain's own cache.
buckets build /path/to/repo
buckets build https://github.com/owner/repo --test
buckets build . --test --run

# Ephemeral worktrees — a task gets its own working copy at a fresh
# branch (git worktree add, not a full clone — cheap, shares the repo's
# object store). Build/test it like any other local path. "Destroyed
# once you merge": removing an unmerged worktree's branch is refused by
# git itself (git branch -d's own safety check) unless --force.
buckets worktree create /path/to/repo my-task-branch
buckets build "$(buckets worktree create /path/to/repo my-task-branch)" --test
buckets worktree remove /path/to/repo /path/to/repo-my-task-branch my-task-branch
buckets worktree list /path/to/repo

# GUI buckets — run a GUI app against a fresh, isolated Xvfb X server
# (not the host's real display). Only the throwaway server's own socket
# + a session-scoped Xauthority cookie are visible inside the sandbox.
buckets gui --screenshot /tmp/out.png --timeout 5 -- glxgears
buckets gui node@20 --width 1280 --height 800 -- node gui-script.js
```

## Real process isolation

`run`/`shell`/`build` all execute under [bubblewrap](https://github.com/containers/bubblewrap)
(`bwrap`) — a fresh mount + PID namespace, not just an isolated toolchain
version. Only the resolved toolchain's own install dirs (read-only) and
the invocation/project directory (read-write) are visible inside; nothing
else on the host is. Network is off by default for `run`/`shell` (most
tool invocations don't need it) and on for `build` (package registries
do). Falls back to a plain unsandboxed subprocess with a warning if
`bwrap` isn't installed — use `--no-sandbox` to opt out explicitly.

## Real GUI isolation

`buckets gui` runs a GUI command against a brand-new [`Xvfb`](https://www.x.org/releases/X11R7.6/doc/man/man1/Xvfb.1.xhtml)
X server, not the host's real `:0`. Borrows the concept from
[x11docker](https://github.com/mviereck/x11docker) — a session-scoped
MIT-MAGIC-COOKIE Xauthority cookie, generated fresh per session and bound
into the sandbox alongside exactly ONE file (the specific `/tmp/.X11-unix/X<N>`
socket, never the whole directory) plus `DISPLAY`/`XAUTHORITY` env vars.
Deliberately a nested server, not a `--hostdisplay`-style reuse of the real
session — X11 has no native per-client window isolation, so a fresh `Xvfb`
instance means nothing on the real display is ever exposed. Verified live:
a client with the wrong/missing `XAUTHORITY` is refused by the X server
("Authorization required, but no authorization protocol specified");
the right cookie succeeds. Session cleanup (Xvfb process, socket, cookie
file) happens on drop regardless of how the command exited.

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
| `build <path-or-url> [--test] [--run]` | Detect + build (+ test/run) a real project, sandboxed |
| `worktree create <repo> <branch> [--from <base>]` | Create an ephemeral worktree (prints its path) |
| `worktree remove <repo> <path> <branch> [--force]` | Remove a worktree + its branch (git refuses if unmerged, unless --force) |
| `worktree list <repo>` | List existing worktrees |
| `gui [specs] -- <cmd> [--screenshot <path>] [--timeout <secs>] [--width <N>] [--height <N>]` | Run a GUI command in a sandboxed bucket against a fresh Xvfb X server |

## Configuration

| Env var | Default | Description |
|---|---|---|
| `BUCKETS_DIST_URL` | `https://dist.pkgx.dev` | Distribution server URL |
| `BUCKETS_CACHE_DIR` | `~/.cache/buckets/` or `~/.buckets/` | Local cache directory |
| `BUCKETS_WORKTREE_DIR` | sibling of the source repo | Parent directory for `worktree create` (see below) |

`BUCKETS_WORKTREE_DIR` defaults to creating each worktree as a sibling of
its source repo (`/path/to/repo-my-branch` next to `/path/to/repo`), not
a fixed cache location — required for relative sibling path-dependencies
(`../other-repo`) to keep resolving correctly from inside the worktree.

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

## Acknowledgments

buckets is a clean-room reimplementation, not a fork — no source from the
projects below is copied here, only the formats/behaviors listed under
"Features borrowed from pkgx" above, plus the general
resolve→install→compose→exec pipeline shape.

- **[pkgx](https://github.com/pkgxdev/pkgx)** (Max Howell, Jacob Heider;
  Copyright 2022–23 pkgx inc.; Apache-2.0) — the bottle format, `dist.pkgx.dev`
  distribution protocol, cache/symlink layout, companion-package resolution,
  and env-composition approach are all pkgx's design, reimplemented here.
- **exosphere's `exo-hydra`** and **exo-light's `exo-hydra`** — the same
  pkgx-derived provisioning pipeline shape (resolve → install → compose →
  exec), adapted for this project's standalone/sync use case. See
  `/workspace/projects/CLAUDE.md`'s provisioning-lineage note for how the
  three relate.

## License

MIT — see `LICENSE`. (Acknowledgments above are for design/format credit,
not a licensing obligation — no pkgx source is redistributed.)
