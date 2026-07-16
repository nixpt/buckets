# BUCKETS-1 — Spike PRoot on Developer Box

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-1 |
| **Priority** | P2 |
| **Status** | Backlog |
| **Phase** | M2 |
| **Assignee** | unassigned |
| **Dependencies** | none |
| **Estimated effort** | S |

## Problem

Before implementing a PRoot sandbox backend fallback in `buckets`, we need to empirically verify `proot`'s execution and path rewriting behavior on a desktop Linux host. We need to confirm how standard streams and results survive PRoot wrapping.

## Success criteria

- [ ] Install `proot` on host (e.g. `paru -S proot` or `pacman`).
- [ ] Run a basic shell command and a `python3` command under `proot` wrapping.
- [ ] Verify standard streams (stdin, stdout, stderr) function correctly.
- [ ] Confirm path-remapping (`-b`) and fake-root (`-0`) options function as expected.
- [ ] Measure baseline latency/performance overhead of wrapping compared to bare execution and `bwrap`.

## Technical approach

- Run manual commands: `proot -b /host/path:/guest/path -w /guest/path python3 -c "import os; print(os.getcwd())"`
- Check if output redirects work and return statuses are propagated cleanly.
