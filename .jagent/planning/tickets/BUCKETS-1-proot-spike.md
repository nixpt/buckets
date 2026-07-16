# BUCKETS-1 — Spike PRoot on Developer Box

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-1 |
| **Priority** | P2 |
| **Status** | Done |
| **Phase** | M2 |
| **Assignee** | antigravity |
| **Dependencies** | none |
| **Estimated effort** | S |

## Problem

Before implementing a PRoot sandbox backend fallback in `buckets`, we need to empirically verify `proot`'s execution and path rewriting behavior on a desktop Linux host. We need to confirm how standard streams and results survive PRoot wrapping.

## Success criteria

- [x] Install `proot` on host (e.g. `paru -S proot` or `pacman`).
- [x] Run a basic shell command and a `python3` command under `proot` wrapping.
- [x] Verify standard streams (stdin, stdout, stderr) function correctly.
- [x] Confirm path-remapping (`-b`) and fake-root (`-0`) options function as expected.
- [x] Measure baseline latency/performance overhead of wrapping compared to bare execution and `bwrap`.

## Technical approach

- Run manual commands: `proot -b /host/path:/guest/path -w /guest/path python3 -c "import os; print(os.getcwd())"`
- Check if output redirects work and return statuses are propagated cleanly.

## Resolution

Successfully completed the PRoot spike on Arch Linux using the official latest static binary from GitLab CI/CD:
1. **Installation**: Downloaded and verified static `proot` at `~/.local/bin/proot` (v5.3.1-99a84175).
2. **Feature parity**:
   - Fake root `-0` correctly mapped the current user to `root`.
   - Path binding `-b` successfully mounted non-existent host directories/files dynamically into the guest container (verified via `/tmp/test_issue`).
   - Working directory `-w` configured the initial directory.
   - Standard streams (stdin, stdout, stderr) and return codes (e.g. 42) propagated cleanly without any interruption or modifications.
3. **Overhead Performance Metrics**:
   - *One-off startup latency (`true` run, 50 iterations)*:
     - Bare: 1.00ms
     - `bwrap`: 5.25ms
     - `proot`: 3.38ms (Faster startup than `bwrap`!)
   - *I/O heavy workload (10,000 file writes, 5 iterations)*:
     - Bare: 176.43ms
     - `bwrap`: 180.28ms (~2% overhead)
     - `proot`: 337.61ms (~91% overhead due to ptrace intercepting every system call)

Conclusion: PRoot is highly viable as a *portability* fallback. We will warn users about the I/O ptrace overhead and the lack of namespace security.
