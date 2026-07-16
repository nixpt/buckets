# Roadmap — buckets

Living plan. Dejavue holds *why*; this file holds *sequence*.

## North star

A complete ephemeral isolated execution engine for AI agents: multi-subcommand CLI (`buckets run`/`shell`/`build`/`worktree`/`gui`/`site`), sandboxed environment isolation (bubblewrap, PRoot portability fallback, X11 GUI sockets, per-origin site storage), and multi-source package resolution (pkgx bottles, crates.io cargo resolver, custom pantries) — all shipping from a single standalone crate.

## Current phase: Sandbox Portability & Cargo Spec Resolver (M2/M3)

The core pipeline (bwrap, pkgx resolver, GUI Xvfb, Site super-surfer) is shipped and verified. The next target is enabling portability fallbacks and expanding the distribution options:

1. **M2: Sandbox Portability (PRoot)** — Fallback to a ptrace-based syscall remapper (`proot`) for platforms that lack unprivileged user namespace support (Android/Termux, hardened Linux kernels).
2. **M3: Cargo Spec Resolver** — Build a `cargo:` spec engine to fetch, build, and cache Rust binaries from crates.io natively, resolving dependency gaps for tools not packaged as pkgx bottles.
3. **M4: Fleet Cache Lock Optimization** — Refine parallel cache locks (`cellar.rs`) and optimize concurrency for multiple fleet agents accessing the same cache directory simultaneously.

---

## Milestones

| Phase | Name | Goal | Exit criteria |
|-------|------|------|----------------|
| **M0** | Core CLI & pkgx | Resolve, install, compose env, and execute from `dist.pkgx.dev`. | CLI `run`/`shell`/`env`/`info` commands working. ✅ |
| **M1** | Sandbox & Exts | bubblewrap containment, worktrees, GUI (Xvfb), and Site sandboxing. | `bwrap` integration, `gui`, `site`, `worktree` verified. ✅ |
| **M2** | PRoot Portability | Fallback to PRoot when namespaces/bwrap are unavailable. | `ProotBackend` in `sandbox.rs` with Termux/Android tests. |
| **M3** | Cargo Spec Type | Fetch, build, and run cargo packages (e.g., `cargo:crush-ast`). | Resolving `cargo:<crate>` works without local cargo install. |
| **M4** | Fleet Concurrency | Make cache directories safe under heavy parallel fleet execution. | Multi-process lock validation and bucket-bridge integration. |

---

## Non-goals (standing)

- **General Container Management** — buckets does not replace Docker/Podman or exosphere's heavy container daemon. It is for quick, lightweight tool runtimes.
- **Complex Dependency Solvers** — buckets does not implement a full SAT solver for companion packages; it maps direct transitive dependencies sequentially.

## Version tags (when releasing)

| Tag | Maps to |
|-----|---------|
| v0.1.0 | M0 + M1 complete (current state) |
| v0.2.0 | M2 (PRoot portability) complete |
| v0.3.0 | M3 (Cargo spec resolution) complete |
| v0.4.0 | M4 (Fleet concurrency optimization) complete |
