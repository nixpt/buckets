# State

Updated: 2026-07-25T02:41:00-05:00

BUCKETS-14 shipped: herd Unix-socket IPC for live hot-scale. Deploy binds
`control.sock`; `scale`/`status`/`stop` are cross-process clients. `scale`
no longer dead code. 96 lib + 100 bin unit tests pass; 0 warnings.

Next open: BUCKETS-3 (Android/Termux PRoot verification), buck-net
expose_port live-test gap, CLAUDE.md stale test count.
