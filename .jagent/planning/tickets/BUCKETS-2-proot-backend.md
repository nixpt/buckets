# BUCKETS-2 — Implement ProotBackend in sandbox.rs

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-2 |
| **Priority** | P2 |
| **Status** | Done |
| **Phase** | M2 |
| **Assignee** | antigravity |
| **Dependencies** | BUCKETS-1 |
| **Estimated effort** | M |

## Problem

When `bwrap` is missing on a host (e.g. Android/Termux, or hardened Linux kernels where unprivileged user namespaces are disabled), the sandbox module falls back to completely unisolated bare execution. We want to add a third rung, a PRoot-based compatibility sandbox backend.

## Success criteria

- [x] Check if `proot` is installed on PATH when `bwrap` is missing.
- [x] Implement `build_proot_args` translating a `SandboxProfile` to proot `-b`, `-w`, and `-0` arguments.
- [x] Fall back gracefully: `bwrap` (if available) -> `proot` (if available) -> bare execution (with warning).
- [x] Log a clear, visible warning when using the `proot` backend (clarifying it provides path isolation but lacks namespace security guarantees).
- [x] Ensure all 146 unit and binary tests pass when simulated under both bwrap and proot paths.

## Technical approach

- Modify `src/sandbox.rs` to include a check for the `proot` binary.
- Translate existing profile directories and extra binds to `-b host:guest` arguments.
- Wire the fallback checks in `sandboxed_command()`.

## Resolution

Successfully implemented the PRoot compatibility sandbox fallback in `src/sandbox.rs`:
1. **Helper Functions**: Added `which_proot()` to detect binary presence on `PATH` and `build_proot_args()` to construct `-0`, `--kill-on-exit`, `-b <dir>:<dir>`, and `-w <cwd>` command line options.
2. **Fallback Chain**: Modified `sandboxed_command()` to attempt:
   - `bwrap` execution (default secure containment).
   - `proot` execution (path-isolation compatibility fallback), emitting a warning to stderr informing the user that network and PID namespaces are NOT isolated under this backend.
   - Bare `Command` execution (fallback for uncontained hosts), emitting a warning to stderr.
3. **Unit Tests**: Added four tests covering `proot` execution paths (`proot_chdir_set_to_cwd`, `proot_extra_ro_binds_included`, `proot_project_dir_gets_bind`, `proot_program_and_args_come_after_separator`). Verified `cargo test` is 100% green with 154 passing tests.

