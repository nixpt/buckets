# BUCKETS-6 — Cellar Cache Locking

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-6 |
| **Priority** | P2 |
| **Status** | Done |
| **Phase** | M4 |
| **Assignee** | antigravity |
| **Dependencies** | none |
| **Estimated effort** | M |

## Problem

Multiple concurrent fleet agents executing `buckets run` or `buckets build` can cause write collisions and directory corruption when downloading, compiling, or extracting the same package concurrently in the shared cellar cache. We need process-level mutual exclusion during package installation.

## Success criteria

- [x] Create and acquire an exclusive advisory file lock (e.g., using `fd-lock`) on a package-specific lock file (e.g., `<project_dir>/.install.lock`) during the installation phase.
- [x] Safely block and wait for the lock to be released if another process is currently installing the package.
- [x] Ensure that once the lock is acquired, we perform a final "already installed" check to avoid re-installing if the other process just finished the installation.
- [x] Ensure that locks are safely released via RAII guard even in the event of panic or crash.

## Technical approach

- Use the `fd-lock` crate for cross-platform advisory file locking.
- Lock `<project_dir>/.install.lock` inside `install.rs` before checking existence/downloading.
- Verify concurrent installations using a multi-threaded integration test case.
