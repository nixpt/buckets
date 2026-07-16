# BUCKETS-7 — CLI Diagnostic Cleanups

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-7 |
| **Priority** | P2 |
| **Status** | Done |
| **Phase** | M4 |
| **Assignee** | antigravity |
| **Dependencies** | none |
| **Estimated effort** | S |

## Problem

When a package is not found (404), a network error occurs, or a version constraint cannot be met, `buckets` prints raw HTTP status exceptions or anyhow trace lists. We need to polish and format user-friendly, clean diagnostic errors.

## Success criteria

- [x] Map `ureq` HTTP errors (like 404) in `inventory.rs` to clean error messages indicating the package is not found in the index.
- [x] Map `ureq` Transport/Network errors in `inventory.rs` to clean error messages detailing DNS/connection failure.
- [x] Polish crates.io version listing errors in the Cargo resolver.
- [x] Map ureq errors in `install.rs` bottle downloading to helpful human-readable messages.
- [x] In `resolve.rs`, if a version constraint matches no versions, list the first few available versions of the package to help users troubleshoot.
- [x] Verify that running a request for a nonexistent package yields a clean error report.

## Technical approach

1. Implement `map_ureq_error` helper in `inventory.rs` and apply it to remote versions list.
2. Implement `map_cargo_ureq_error` in `inventory.rs` and apply it to Cargo versions list.
3. Implement `map_download_ureq_error` in `install.rs` and apply it to bottle downloader.
4. Update `resolve_version` in `resolve.rs` to query and append available versions on matching failures.
