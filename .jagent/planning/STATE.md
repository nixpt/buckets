# Planning state — buckets

**Updated:** 2026-07-16T14:39:00-05:00
**Milestone focus:** M4 — Fleet Concurrency & Optimization (see ROADMAP.md, TASKS.md)
**Branch:** `agent/antigravity/BUCKETS-6`

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
| Cargo Spec Resolver | **backlog** | `cargo:` scheme resolver to build and cache cargo binaries locally. |

---

## Active work

Current focus is on M2 (Sandbox Portability via PRoot Fallback) to allow execution on Android/Termux and hardened kernels where bwrap fails. The next milestones target Cargo spec resolution (M3) and parallel cellar cache lock optimizations (M4).

---

## Blockers

_None known._

---

## Metrics

| Metric | Value |
|--------|--------|
| Total crates | 1 (Standalone binary + library) |
| Tests passed | **155** (76 lib tests + 79 binary tests) |
| Tests failed | 0 |
| Tests ignored | 0 |
| Warnings | 0 |
| Composed features | CLI running, bwrap sandboxing, Xvfb GUI, surfer Site browser, Git worktree |
| Cache location | `~/.cache/squadron-buckets` |
| Build time (from clean) | ~15s (debug) |
| Release binary size | ~1.5MB (stripped + LTO) |
| Decisions captured | 6 |

---

## Next 3 (from TASKS.md, priority order)

1. **Android/Termux Verification**: Verify PRoot behavior and Yama ptrace policy under Termux (BUCKETS-3).
2. **Cargo spec resolution**: Design `cargo:` scheme spec type resolver to fetch and cache crates.io binaries (BUCKETS-4).
3. **Local Pantry Overrides**: Create configuration to override package distributions with local directories or custom manifests (BUCKETS-5).

---

## Memory split

| Concern | Path |
|---------|------|
| *Why* | `.dejavue/` (`dejavue context`) |
| *What / when* | `.jagent/planning/` (this file, ROADMAP, TASKS, tickets) |
| *How to work this backlog* | `.jagent/planning/RULES.md` |
| Identity | `.jagent/PROJECT.md` |
