# BUCKETS-14 — Herd Unix-socket IPC for live hot-scale

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-14 |
| **Priority** | P2 |
| **Status** | Done |
| **Phase** | M4 |
| **Assignee** | nixp |
| **Dependencies** | BUCKETS-12 |
| **Estimated effort** | M |

## Problem

`HerdController::scale` is still dead code: `buckets herd scale` is a separate process from `buckets herd deploy`, so it cannot reach the live `Child` handles. BUCKETS-12 wired `stop`/`snapshot` in-process and deferred live hot-scale to Unix-socket IPC. Cross-process `stop` also only SIGTERMs replica PIDs from `state.json` — it does not tell the deploy process to shut down, so the reconciler can respawn them.

## Success criteria

- [x] `buckets herd deploy` binds `control.sock` under the herd state dir and serves Scale/Status/Stop.
- [x] `buckets herd scale <name> --replicas N` connects to the socket and applies a live scale (up or down).
- [x] `buckets herd stop <name>` prefers IPC Stop (clean deploy shutdown) with kill-from-state.json fallback if the socket is absent.
- [x] `HerdController::scale` no longer needs `#[allow(dead_code)]`.
- [x] Unit tests cover the request/response protocol; `cargo test` / `cargo build` clean (0 warnings). Live smoke: scale 1→2 + IPC stop verified.

## Technical approach

- Line-delimited JSON over a Unix domain socket at `{herds_dir}/{name}/control.sock`.
- Deploy process: spawn an IPC accept loop alongside the reconciler; Scale → `ctrl.scale()`, Status → live snapshot, Stop → set the shared stop signal so deploy tears down via the existing Arc/`stop()` path.
- Scale/Stop/Status CLI clients: connect, send one request, print the response; clear errors when the herd or socket is missing.
- On deploy shutdown, unlink `control.sock`.

## Files modified

- `src/herd.rs` — protocol types, `serve_control` / `send_control`, removed scale dead-code allow.
- `src/main.rs` — deploy starts IPC server; Scale/Stop/Status use the client.
- `README.md` — document live hot-scale via control socket.

## Non-goals

- Full supervisor daemon / systemd unit generation.
- Authenticated multi-user socket ACLs beyond filesystem permissions on the sock.
- Changing reconciliation/backoff logic.
