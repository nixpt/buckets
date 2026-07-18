# Planning state — buckets

**Updated:** 2026-07-16T15:28:00-05:00
**Milestone focus:** M4 — Fleet Concurrency & Optimization (see ROADMAP.md, TASKS.md)
**Branch:** `agent/antigravity/BUCKETS-7`

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
| Cellar Cache Locking | **shipped** | Exclusive advisory file locks (`fd-lock`) around cellar installs — safe for concurrent fleet agents installing the same package. |
| Local Path Spec Support | **shipped** | `path:<local-path>` specs — detects the build system (Cargo/Go/npm/generic) and compiles+caches a local project's binaries for sandboxed execution. |

---

## Active work

Current focus is on M4 (Fleet Concurrency) — herd shipped, `buckets clean` shipped. BUCKETS-12 filed to address HerdController dead-code (in-process API unwired due to cross-process CLI design). Next open: BUCKETS-3 (Android/Termux PRoot verification).

---

## Blockers

_None known._

---

## Metrics

| Metric | Value |
|--------|--------|
| Total crates | 1 (Standalone binary + library) |
| Tests passed | **188** (85 lib tests + 89 binary tests + 14 herd tests) |
| Tests failed | 0 |
| Tests ignored | 0 |
| Warnings | 0 (BUCKETS-12: `#[allow(dead_code)]` on in-process API methods — see ticket) |
| Composed features | CLI running, bwrap sandboxing, Xvfb GUI, surfer Site browser, Git worktree, herd, clean |
| Cache location | `~/.cache/squadron-buckets` |
| Build time (from clean) | ~15s (debug) |
| Release binary size | ~1.5MB (stripped + LTO) |
| Decisions captured | 6 |

---

## Next 3 (from TASKS.md, priority order)

1. **BUCKETS-12 (HerdController dead-code)**: Wire or mark `snapshot`/`scale`/`stop` as in-process API. BUCKETS-12 ticket filed.
2. **Android/Termux Verification**: Verify PRoot behavior and Yama ptrace policy under Termux (BUCKETS-3).
3. **buck-net expose_port live-test**: socat/nsenter port forwarding has zero live-test coverage.

---

## Memory split

| Concern | Path |
|---------|------|
| *Why* | `.dejavue/` (`dejavue context`) |
| *What / when* | `.jagent/planning/` (this file, ROADMAP, TASKS, tickets) |
| *How to work this backlog* | `.jagent/planning/RULES.md` |
| Identity | `.jagent/PROJECT.md` |
