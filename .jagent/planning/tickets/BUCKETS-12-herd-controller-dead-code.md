# BUCKETS-12 — HerdController in-process API wiring (snapshot/stop wired, scale deferred)

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-12 |
| **Priority** | P3 |
| **Status** | In Progress |
| **Phase** | M4 |
| **Assignee** | nara |
| **Dependencies** | BUCKETS-11 |
| **Estimated effort** | M |

## Problem

`HerdController::{snapshot, scale, stop}` in `src/herd.rs` are flagged as dead code by `cargo build`. They were designed as the in-process API for a supervisor/library consumer holding a live `HerdController`, but BUCKETS-11 shipped the CLI without wiring them — `cmd_herd`'s `status`/`scale`/`stop` subcommands are separate process invocations that read `state.json` directly and kill PIDs via `libc::kill`, bypassing the controller entirely. The `deploy` shutdown path (main.rs ~1648) also duplicated `HerdController::stop`'s kill loop inline rather than calling it.

This is not a behavioral bug — herds deploy, reconcile, list, status, and stop correctly. It is an architectural seam: the in-process API surface was reserved but unused, so the compiler warned.

## Resolution (partial — in progress)

**Completed:**
- `deploy` shutdown now uses `Arc<HerdController>` + `Arc::try_unwrap` to call `ctrl.stop()` instead of the inline kill loop (deduplicates cleanup, removes `stop` dead-code warning)
- `snapshot()` wired into deploy shutdown (logs live instance status before cleanup)
- `Ls` displays `RUNNING/FAIL` counts from `HerdInfo.instances` (removes `instances` dead-code warning)
- `#[allow(dead_code)]` + doc comments removed for `snapshot` and `stop` (now wired), kept for `scale` (still needs IPC)

**Deferred to follow-up ticket:**
- `HerdController::scale()` — still dead code. Requires IPC (Unix socket) to allow a separate `buckets herd scale` process to instruct the deploy process's reconciler. Out of scope for this pass.

## Success criteria

- [x] `cargo build` produces zero warnings in `herd.rs`.
- [x] `HerdController::stop` wired into `deploy` shutdown path via `Arc::try_unwrap` (deduplicates kill loop).
- [x] `HerdController::snapshot` wired into deploy shutdown (logs live status before cleanup).
- [x] `HerdController::scale` marked `#[allow(dead_code)]` with doc explaining IPC requirement.
- [x] `HerdInfo.instances` displayed by `Ls` subcommand.
- [ ] Live hot-scale via IPC — deferred to follow-up ticket.

## Technical approach

### Direction chosen: Arc-share controller + wire ctrl.stop()

1. **Wrap controller in Arc**: `let ctrl = Arc::new(ctrl)` before spawning reconciler.
2. **Clone Arc for reconciler thread**: `let ctrl2 = ctrl.clone(); move || ctrl2.run_reconciler(stop2)` — thread borrows via Arc reference, not ownership.
3. **Add Clone + Debug derives** to `HerdController` (needed for Arc::try_unwrap error path).
4. **Manual Debug impl for InternalState** (Child doesn't implement Debug).
5. **After reconciler join**: `Arc::try_unwrap(ctrl)` to get owned controller back, then `ctrl.stop()`.
6. **Log snapshot before stop**: iterate `ctrl.snapshot()` to log live instance status.
7. **Remove inline kill loop** (was duplicating what `stop()` already does).
8. **Wire Ls to use HerdInfo.instances**: display RUNNING/FAIL counts.
9. **Keep scale() as dead code** with `#[allow(dead_code)]` + doc — live hot-scale needs IPC, deferred.

### Why not cross-process IPC for status?

`buckets herd status` is a separate process invocation — it can't reach the deploy process's memory. Reading `state.json` is the correct approach for cross-process visibility. The `snapshot()` method is now wired only in the deploy process (same-process logging before cleanup).

## Files modified

- `src/herd.rs` — `HerdController` derives `Clone, Debug`; `InternalState` gets manual `Debug`; `run_reconciler` takes `&Arc<Self>`; `snapshot`/`stop` wired (removed `#[allow(dead_code)]`); `scale` kept as `#[allow(dead_code)]` with doc.
- `src/main.rs` — deploy path wraps controller in Arc, clones for reconciler thread, calls `ctrl.stop()` after join, logs snapshot; Ls displays instance counts.

## Non-goals

- Changing herd's reconciliation/backoff logic (works correctly, BUCKETS-11 verified).
- Adding IPC for live hot-scale (deferred to follow-up ticket).
- Replacing `state.json` persistence (correct for crash recovery + cross-process visibility).
- Adding a full supervisor daemon (herds are documented as session-scoped; supervisor is out of scope).
