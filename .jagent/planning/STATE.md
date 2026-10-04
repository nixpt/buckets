# Planning state — buckets

**Updated:** 2026-07-25T02:43:40-05:00
**Milestone focus:** Close M2 (BUCKETS-3 Termux); M3/M4 largely shipped (see ROADMAP.md, TASKS.md)
**Branch:** `agent/nixp/BUCKETS-14` — PR https://github.com/nixpt/buckets/pull/3

## Delivery snapshot

| Track | Status | Notes |
|-------|--------|--------|
| CLI core | **shipped** | CLI argument dispatcher for `run`, `shell`, `env`, `info`, `list`, `build`, `worktree`, `gui`, `site`. |
| pkgx Resolver | **shipped** | Resolves spec names, versions, and transitive companion dependencies from `dist.pkgx.dev` aliases. |
| Cellar & Cache | **shipped** | Downloads, decompresses, caches, and unlinks package runtimes locally. |
| Bubblewrap Sandbox | **shipped** | Process containment with read-only hosts binds, custom rw binds, and network unsharing. |
| Ephemeral Worktrees | **shipped** | Cheap checkout management using git worktree. |
| GUI X11 Sandbox | **shipped** | Per-session isolated Xvfb server and Xauthority cookie management. |
| Site sandboxing | **shipped** | Persistent origin-keyed storage or incognito directories wrapper around surfer browser. |
| PRoot Portability Fallback | **shipped** | Fallback ptrace-based syscall path remapper when user namespaces/bwrap are unavailable. |
| Cargo Spec Resolver | **shipped** | `cargo:` scheme resolver to build and cache cargo binaries locally via crates.io API. |
| PyPI + npm Spec Resolvers | **shipped** | `pypi:` / `npm:` via registry APIs → shared cellar + PYTHONPATH/NODE_PATH (BUCKETS-15). |
| Cellar Cache Locking | **shipped** | Exclusive advisory file locks (`fd-lock`) around cellar installs — safe for concurrent fleet agents installing the same package. |
| Local Path Spec Support | **shipped** | `path:<local-path>` specs — detects the build system (Cargo/Go/npm/generic) and compiles+caches a local project's binaries for sandboxed execution. |
| **buck-herd** | **shipped** | Mandala-pattern fleet orchestration (`buckets herd deploy/ls/status/scale/stop`), health polling + exponential-backoff auto-restart. |
| **buckets clean** | **shipped** | Cache eviction command (`buckets clean --older-than <duration>`). |
| **BUCKETS-12** | **Done** | HerdController `snapshot`/`stop` wired into deploy shutdown via Arc-share. |
| **BUCKETS-14** | **Done** | Unix-socket IPC: live hot-scale + IPC stop/status. `control.sock` under herd state dir. |

---

## Active work

M4 herd IPC complete. Next open: BUCKETS-3 (Android/Termux), buck-net expose_port live-test.

---

## Blockers

_None known._

---

## Metrics

| Metric | Value |
|--------|--------|
| Total crates | 1 (Standalone binary + library) |
| Tests passed | **196** (96 lib + 100 binary unit tests; modules counted in both targets) |
| Tests failed | 0 |
| Tests ignored | 0 |
| Warnings | 0 |
| Composed features | CLI running, bwrap sandboxing, Xvfb GUI, surfer Site browser, Git worktree, herd+IPC, clean |
| Cache location | `~/.cache/squadron-buckets` |
| Build time (from clean) | ~15s (debug) |
| Release binary size | ~1.5MB (stripped + LTO) |
| Decisions captured | 6 |

---

## Next 3 (from TASKS.md, priority order)

1. **Merge PR #3** (BUCKETS-14 herd IPC) — then continue from master.
2. **BUCKETS-3 (Android/Termux Verification)**: Verify PRoot behavior and Yama ptrace policy under Termux.
3. **buck-net expose_port live-test** / CLAUDE.md test-count hygiene.

---

## Memory split

| Concern | Path |
|---------|------|
| *Why* | `.dejavue/` (`dejavue context`) |
| *What / when* | `.jagent/planning/` (this file, ROADMAP, TASKS, tickets) |
| *How to work this backlog* | `.jagent/planning/RULES.md` |
| Identity | `.jagent/PROJECT.md` |
