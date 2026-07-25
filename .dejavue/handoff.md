# Handoff

Updated: 2026-07-25T02:41:00-05:00

## Summary
BUCKETS-14: herd live hot-scale via Unix-socket IPC. `serve_control` /
`send_control` in `herd.rs`; deploy spawns IPC thread; Scale/Status/Stop
CLI use the socket. Live smoke verified scale 1→2 + IPC stop. Branch
`agent/nixp/BUCKETS-14`.

## Next Steps
- BUCKETS-3 (Android/Termux PRoot verification)
- buck-net expose_port (socat/nsenter) live-test coverage
- CLAUDE.md test-count hygiene

## Boot Instructions
Read `.dejavue/handoff.md`, `.dejavue/state.md`, `.dejavue/decisions.md`, and `.dejavue/timeline.jsonl` before making changes.
