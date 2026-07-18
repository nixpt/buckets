# Handoff

Updated: 2026-07-18T12:00:00-05:00

## Summary
Docs-herd-hygiene branch: filed BUCKETS-12 (HerdController in-process API dead code), wired `Ls` to show instance counts, added `#[allow(dead_code)]` + doc comments on `snapshot`/`scale`/`stop` pointing to BUCKETS-12. Build zero warnings, 188 tests pass.

## Next Steps
- BUCKETS-3 (Android/Termux PRoot verification) — next open ticket in TASKS.md
- BUCKETS-12 (herd controller wiring refactor) — filed, needs planning/execution
- buck-net's expose_port (socat/nsenter) has zero live-test coverage — worth a live pass

## Boot Instructions
Read `.dejavue/handoff.md`, `.dejavue/state.md`, `.dejavue/decisions.md`, and `.dejavue/timeline.jsonl` before making changes.
