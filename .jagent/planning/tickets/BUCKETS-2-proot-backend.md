# BUCKETS-2 — Implement ProotBackend in sandbox.rs

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-2 |
| **Priority** | P2 |
| **Status** | Backlog |
| **Phase** | M2 |
| **Assignee** | unassigned |
| **Dependencies** | BUCKETS-1 |
| **Estimated effort** | M |

## Problem

When `bwrap` is missing on a host (e.g. Android/Termux, or hardened Linux kernels where unprivileged user namespaces are disabled), the sandbox module falls back to completely unisolated bare execution. We want to add a third rung, a PRoot-based compatibility sandbox backend.

## Success criteria

- [ ] Check if `proot` is installed on PATH when `bwrap` is missing.
- [ ] Implement `build_proot_args` translating a `SandboxProfile` to proot `-b`, `-w`, and `-0` arguments.
- [ ] Fall back gracefully: `bwrap` (if available) -> `proot` (if available) -> bare execution (with warning).
- [ ] Log a clear, visible warning when using the `proot` backend (clarifying it provides path isolation but lacks namespace security guarantees).
- [ ] Ensure all 146 unit and binary tests pass when simulated under both bwrap and proot paths.

## Technical approach

- Modify `src/sandbox.rs` to include a check for the `proot` binary.
- Translate existing profile directories and extra binds to `-b host:guest` arguments.
- Wire the fallback checks in `sandboxed_command()`.
