---
name: buckets-usage
description: Use when a task needs a CLI tool or language runtime (node, python, rust, go, cmake, etc.) that either isn't installed on this machine, or is installed at the wrong version, and installing/upgrading it globally would be intrusive or risky. Reach for `buckets` instead of a global install/version-manager switch — it fetches the exact version on demand into an isolated, sandboxed, throwaway environment and leaves the host untouched. Also covers building/testing an unfamiliar repo without polluting the host with its toolchain.
---

# buckets

`buckets` resolves, fetches, and runs any CLI tool in an isolated ephemeral
environment — without installing it globally. Bottles are pulled on demand
from `dist.pkgx.dev`, cached locally, and executed under `bwrap` sandboxing
(fresh mount/PID namespace; falls back to a plain unsandboxed exec with a
warning if `bwrap` isn't installed).

## When to reach for it

- A one-off script needs a specific runtime/version ("run this with
  node@20", "test with python@3.11") and the host either doesn't have it,
  has a different version, or you don't want to touch the system install.
- Building/testing a cloned repo whose toolchain you don't want polluting
  the host filesystem or its package caches.
- You want the command to run in a real sandbox (no access to the rest of
  the host filesystem) rather than trusting an arbitrary script.

Don't reach for it when the tool is already correctly installed and
globally available on `PATH` for every task in this session — `buckets`
adds sandboxing/isolation overhead that's wasted if isolation isn't needed.

## Prerequisite

`buckets` must be installed first — this plugin does not vendor a binary.
Install it with `cargo install crush-buckets` (binary is named `buckets`),
or `cargo install --path .` from a local clone of
https://github.com/nixpt/buckets. Check with `command -v buckets` before
relying on it; if missing, tell the user to install it rather than
attempting a global install of the underlying tool as a workaround.

## Core commands

```bash
# Resolve + install + run a one-off command in a fresh sandboxed environment
buckets run node@20 -- node script.js
buckets run python@3.11 node@20 -- <cmd>    # multiple runtimes in one env

# Open an interactive shell with the runtime on PATH
buckets shell node@20

# See what a spec resolves to, without installing anything
buckets info node@20

# List what's already cached locally
buckets list

# Reclaim disk space (bare `clean` with no target is refused on purpose)
buckets clean node@18
buckets clean --all --older-than 30d

# Build/test/run a real project (git URL or local path), sandboxed,
# with its toolchain auto-resolved from the build-system it detects
buckets build /path/to/repo --test
buckets build https://github.com/owner/repo --test --run
```

Spec format is `<tool>[@<version>]` — e.g. `node@20`, `python@=3.11.0`,
`rust@latest`, `go@^1.22`. See the repo README for the full command
reference (`session`, `net`, `herd`, `gui`, `site`, `worktree`) and the
list of 60+ featured tool aliases.
