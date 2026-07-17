# BUCKETS-11 — buck-herd: mandala-pattern orchestration for bucket fleets

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-11 |
| **Priority** | P2 |
| **Status** | Done |
| **Phase** | M4 |
| **Assignee** | antigravity (feature), kai (signal-handling fix) |
| **Estimated effort** | L |

## Problem / Goal

Orchestrate N replicas of a bucket as a named "herd" with health polling and
auto-restart, so agent fleets can run resilient long-lived worker pools instead
of one-off `buckets run` invocations.

## What shipped

`src/herd.rs` (520 lines) — `HerdController`/`HerdSpec`: desired-state
reconciliation loop (same pattern as mandala's `ReconciliationLoop`, native
std threads, no tokio/extra deps beyond `libc` for `kill()`), health polling
via `try_wait()`, exponential-backoff auto-restart (1s→60s cap), 3 restart
policies (`on_failure`/`always`/`never`), per-replica max-restart cap, state
persisted to `cache_dir/herds/<name>/state.json`, buck-net integration
(replicas share loopback via `--net`). New CLI: `buckets herd
deploy/ls/status/scale/stop`.

**Note on provenance:** this landed as a single commit (`ea2a8d5`) directly on
`master` — no `agent/antigravity/BUCKETS-11` branch, no ticket, no merge-wave
review, unlike every other `BUCKETS-*` change. This ticket is filed
retroactively to bring it into the normal tracking. The 95 unit tests it
shipped with are real and pass, but (see below) they only exercise
`HerdController`'s components in isolation, not the `herd deploy` CLI path
end-to-end — which is exactly why the bug below wasn't caught.

## Bug found + fixed (kai, 2026-07-17)

**`herd deploy`'s "wait for Ctrl-C" was `stdin().read_line()`, not a signal
handler** (`src/main.rs`, the `HerdSubcommand::Deploy` arm). Consequences,
both reproduced live:

1. Any non-interactive invocation (background job, systemd unit, CI, agent
   dispatch, or even just `&`) hits stdin EOF almost immediately, so the
   herd printed "running" and tore itself down within ~1 second — the
   fleet never actually stayed up outside a live, attached terminal.
2. Even in a genuine interactive terminal, no `SIGINT` handler was
   installed, so a real Ctrl-C would hit the process's default signal
   action (immediate termination) rather than the intended graceful
   `stop.store`→`handle.join`→cleanup path the code already had — the
   cleanup logic was correct, it just was never reachable via an actual
   signal.

**Fix:** installed real `SIGINT`+`SIGTERM` handlers via raw `libc::signal`
(no new dependency — `libc` was already pulled in for this feature's own
`kill()` calls), storing to a `static AtomicBool` (async-signal-safe) that
the wait loop now polls instead of blocking on stdin.

**Also renamed `swarm`→`herd` throughout** (captain call: "swarm" didn't fit
the project's existing `buck`/deer naming pun, "herd" does) — module
(`src/swarm.rs`→`src/herd.rs`), all types (`SwarmController`→`HerdController`,
`SwarmSpec`/`SwarmState`/`SwarmInfo`/`SwarmSubcommand`→`Herd*`), the CLI
(`buckets swarm ...`→`buckets herd ...`), the env vars
(`SWARM_NAME`/`SWARM_REPLICA_INDEX`→`HERD_NAME`/`HERD_REPLICA_INDEX`), and
this ticket. Verified no `swarm` references remain anywhere in the repo.

**Verified live** (not just re-running the unit tests):
- Deployed a 3-replica herd backgrounded with `&` — previously exited
  within ~1s; now stays up indefinitely, `herd status`/`herd ls` report
  real running PIDs while it's alive.
- Sent a real `kill -INT <deploy-pid>` — confirmed the graceful shutdown
  path now fires (`▶ shutting down herd '<name>'...` printed, reconciler
  thread joined, replica processes cleanly terminated, `state.json`
  removed) — this is the path that was previously unreachable outside of
  pressing Enter on stdin.
- Re-ran the full suite: 95/95 still passing.

## Follow-up worth a closer look (not chased further — out of this fix's scope)

While live-testing, one restart cycle hit `Failed to resolve version for
'cargo:ripgrep': ... dist.pkgx.dev/.../versions.txt: status code 404` on a
replica restart, immediately after an *identical* spec had resolved fine on
initial spawn moments earlier. Plausibly transient upstream flakiness
(`dist.pkgx.dev`) rather than a buck-herd bug — `spawn_replica` re-runs the
full resolve pipeline on every restart rather than reusing the already-warm
cellar cache from the initial spawn, so any external hiccup shows up as a
restart-storm rather than a quiet cache hit. Worth a `buckets` recon pass if
it recurs; not reproduced deterministically here.

## Acceptance

- [x] `cargo build --release` clean (only a pre-existing unrelated
      `HerdInfo` dead-code Debug-derive warning, not introduced by this fix).
- [x] `cargo test` 95/95 passing.
- [x] Live-verified: herd survives backgrounded/non-interactive invocation.
- [x] Live-verified: real SIGINT triggers the existing graceful-shutdown
      path (was previously dead code, unreachable via any actual signal).

## Notes

- No overlap with the same-session `SQ-139`/`SEC-007` identity/capability
  tickets — unrelated subsystem, noted here only because this ticket's own
  commit-author field surfaced the same "who actually did this" question
  those tickets are about.
