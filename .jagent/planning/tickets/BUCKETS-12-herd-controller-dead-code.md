# BUCKETS-12 — HerdController in-process API is dead code (snapshot/scale/stop unwired)

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-12 |
| **Priority** | P3 |
| **Status** | Backlog |
| **Phase** | M4 |
| **Assignee** | unassigned |
| **Dependencies** | BUCKETS-11 |
| **Estimated effort** | M |

## Problem

`HerdController::{snapshot, scale, stop}` in `src/herd.rs` are flagged as dead code by `cargo build`. They were designed as the in-process API for a supervisor/library consumer holding a live `HerdController`, but BUCKETS-11 shipped the CLI without wiring them — `cmd_herd`'s `status`/`scale`/`stop` subcommands are separate process invocations that read `state.json` directly and kill PIDs via `libc::kill`, bypassing the controller entirely. The `deploy` shutdown path (main.rs ~1648) also duplicates `HerdController::stop`'s kill loop inline rather than calling it.

This is not a behavioral bug — herds deploy, reconcile, list, status, and stop correctly. It is an architectural seam: the in-process API surface is reserved but unused, so the compiler warns.

## Reproduction

```bash
cd projects/buckets && cargo build 2>&1 | grep herd.rs
```

Expected warnings:
```
warning: methods `snapshot`, `scale`, and `stop` are never used
   --> src/herd.rs:203:12
warning: field `instances` is never read
   --> src/herd.rs:472:9
```

## Root cause

- `HerdController` methods operate on live in-memory `Child` handles. They only make sense in the process that ran `deploy` (holds the controller).
- `buckets herd status|scale|stop` are separate process invocations that cannot reach the deploy process's memory — they correctly fall back to `state.json`.
- `deploy` moves `ctrl` into the reconciler thread (`move || ctrl.run_reconciler(stop2)`), so `ctrl.stop()` cannot be called from the main thread after `handle.join()`. Wiring `stop()` requires restructuring: share the controller via `Arc` (so `stop()` can run from the main thread after signaling), or have the reconciler thread call `stop()` on shutdown.
- `Scale` subcommand is a stub ("Live hot-scale requires the herd's controlling process"). Live hot-scale needs IPC (Unix socket / signal / shared state file) so a separate `buckets herd scale` process can instruct the deploy process's reconciler to resize. `HerdController::scale` is the in-process half of that.
- `HerdInfo.instances` is populated by `list_all` (from `state.json`) but `Ls` only prints name/bucket/replicas/net — never reads `instances`. (Fixed in the doc-refresh branch: `Ls` now shows running/failed counts.)

## Success criteria

- [ ] `cargo build` produces zero warnings in `herd.rs`.
- [ ] `HerdController`'s in-process API is either (a) wired to a real caller, or (b) explicitly marked `#[allow(dead_code)]` with a doc comment naming this ticket and explaining the cross-process architecture.
- [ ] `deploy` shutdown either calls `ctrl.stop()` (after Arc refactor) or documents why the inline kill loop is kept.
- [ ] `Scale` subcommand either implements live hot-scale via IPC, or the stub is honest about the missing piece and `HerdController::scale` is marked accordingly.

## Technical approach

Two reasonable directions — pick one in planning:

1. **Arc-share the controller** (preferred for real wiring):
   - Wrap `HerdController` internals in `Arc` so the reconciler thread and the main `deploy` thread both hold a handle.
   - `deploy` shutdown calls `ctrl.stop()` instead of the inline kill loop → dedupes + removes `stop` warning.
   - Add an IPC channel (Unix socket at `herds_dir/{name}/ctrl.sock`) so a separate `buckets herd scale` process can send a `Scale { replicas }` command to the deploy process's reconciler → wires `scale()`.
   - `snapshot()` stays as the in-process status API; either wire it to a `buckets herd status --live` flag (hits the socket) or mark it as the library API.

2. **Accept the cross-process design** (minimal):
   - Keep `status`/`stop` reading `state.json` (correct for cross-process).
   - Mark `snapshot`/`scale`/`stop` `#[allow(dead_code)]` with a doc comment naming BUCKETS-12 and explaining they're the in-process API reserved for a future supervisor/library consumer.
   - Implement live hot-scale as a separate follow-up (IPC) — keep the `Scale` stub honest.

## Files to modify

- `src/herd.rs` — `HerdController::{snapshot, scale, stop}` wiring or `#[allow(dead_code)]` + docs; `HerdInfo.instances` already wired to `Ls` in the doc-refresh branch.
- `src/main.rs` — `cmd_herd` `Scale`/`Stop`/`deploy` shutdown paths (if direction 1 chosen).

## Non-goals

- Changing herd's reconciliation/backoff logic (works correctly, BUCKETS-11 verified).
- Replacing `state.json` persistence (correct for crash recovery + cross-process visibility).
- Adding a full supervisor daemon (herds are documented as session-scoped; supervisor is out of scope).
