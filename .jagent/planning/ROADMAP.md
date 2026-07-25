# Roadmap — buckets

Living plan. Dejavue holds *why*; this file holds *sequence*.

## North star

A complete ephemeral isolated execution engine for AI agents: multi-subcommand CLI (`buckets run`/`shell`/`build`/`worktree`/`gui`/`site`), sandboxed environment isolation (bubblewrap, PRoot portability fallback, X11 GUI sockets, per-origin site storage), and multi-source package resolution (pkgx bottles, crates.io cargo resolver, custom pantries) — all shipping from a single standalone crate.

## Current phase: Close M2 Termux gate; M3/M4 largely shipped

M0–M1 complete. M3 (cargo/path/pantry) and most of M4 (cellar locks, herd, herd IPC, clean) are shipped. Remaining:

1. **M2 exit**: BUCKETS-3 — verify PRoot on a real Termux/Android node (`phone-claude`).
2. **M4 gaps**: buck-net `expose_port` live-test coverage; CLAUDE.md test-count hygiene.

---

## Milestones

| Phase | Name | Goal | Exit criteria |
|-------|------|------|----------------|
| **M0** | Core CLI & pkgx | Resolve, install, compose env, and execute from `dist.pkgx.dev`. | CLI `run`/`shell`/`env`/`info` commands working. ✅ |
| **M1** | Sandbox & Exts | bubblewrap containment, worktrees, GUI (Xvfb), and Site sandboxing. | `bwrap` integration, `gui`, `site`, `worktree` verified. ✅ |
| **M2** | PRoot Portability | Fallback to PRoot when namespaces/bwrap are unavailable. | `ProotBackend` in `sandbox.rs` ✅; Termux/Android verification (**BUCKETS-3** open). |
| **M3** | Cargo Spec Type | Fetch, build, and run cargo packages (e.g., `cargo:crush-ast`). | Resolving `cargo:<crate>` works without local cargo install. ✅ |
| **M4** | Fleet Concurrency | Make cache directories safe under heavy parallel fleet execution. | Cellar locks ✅; herd + Unix IPC hot-scale ✅ (**BUCKETS-14** / PR #3); expose_port live-test open. |

---

## Non-goals (standing)

- **General Container Management** — buckets does not replace Docker/Podman or exosphere's heavy container daemon. It is for quick, lightweight tool runtimes.
- **Complex Dependency Solvers** — buckets does not implement a full SAT solver for companion packages; it maps direct transitive dependencies sequentially.

## Version tags (when releasing)

| Tag | Maps to |
|-----|---------|
| v0.1.0 | M0 + M1 complete |
| v0.2.0 | M2 (PRoot portability) complete — blocked on BUCKETS-3 |
| v0.3.0 | M3 (Cargo spec resolution) complete — ready |
| v0.4.0 | M4 (Fleet concurrency optimization) complete — nearly ready |
