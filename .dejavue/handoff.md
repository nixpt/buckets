# Handoff

Updated: 2026-07-18T12:00:00-05:00

## Summary
Docs-herd-hygiene branch: BUCKETS-12 in progress — `snapshot`/`stop` wired into deploy shutdown via Arc-share + Arc::try_unwrap, deduplicated inline kill loop. `scale` kept as `#[allow(dead_code)]` (needs IPC). Ls displays instance counts. Build zero warnings, 188 tests pass.

## Next Steps
- BUCKETS-3 (Android/Termux PRoot verification) — next open ticket in TASKS.md
- BUCKETS-12 follow-up: live hot-scale via Unix socket IPC
- buck-net's expose_port (socat/nsenter) has zero live-test coverage

## Boot Instructions
Read `.dejavue/handoff.md`, `.dejavue/state.md`, `.dejavue/decisions.md`, and `.dejavue/timeline.jsonl` before making changes.
