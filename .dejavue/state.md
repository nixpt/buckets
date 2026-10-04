# State

Updated: 2026-07-25T02:43:40-05:00

BUCKETS-14 shipped on agent/nixp/BUCKETS-14 (PR https://github.com/nixpt/buckets/pull/3, bae594e). Herd Unix-socket IPC: deploy binds control.sock; scale/status/stop are cross-process clients. Live smoke verified scale 1→2 + IPC stop. 96 lib + 100 bin tests, 0 warnings. Next: BUCKETS-3 (Termux), expose_port live-test, CLAUDE.md test-count hygiene.
