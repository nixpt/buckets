# State

Updated: 2026-07-18T12:00:00-05:00

BUCKETS-9 and BUCKETS-10 marked Done. 188 passing tests. BUCKETS-11 (buck-herd) shipped on master. BUCKETS-12 filed: HerdController in-process API (snapshot/scale/stop) is dead code due to cross-process CLI design — wired `#[allow(dead_code)]` + doc comments, Ls now displays instance counts. `buckets clean` shipped. All herd-related docs documented in README.

Next open: BUCKETS-3 (Android/Termux PRoot verification), BUCKETS-12 (herd controller wiring refactor).

Known gap: buck-net expose_port (socat/nsenter) has zero live-test coverage.
