# BUCKETS-4 — Cargo Spec Type Engine

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-4 |
| **Priority** | P3 |
| **Status** | Done |
| **Phase** | M3 |
| **Assignee** | antigravity |
| **Dependencies** | none |
| **Estimated effort** | L |

## Problem

`buckets` currently resolves packages against `dist.pkgx.dev`'s bottle index. When a package isn't formatted as a pkgx bottle, it cannot be run. Adding a `cargo:` spec type would allow fetching, building, and executing Rust-based binaries from crates.io natively.

## Success criteria

- [x] Support the `cargo:` prefix in package specs (e.g. `cargo:crush-ast@0.2.0`).
- [x] Parse crate names and versions/semver constraints from cargo specs.
- [x] Query crates.io API to resolve package version constraints.
- [x] Build the crate using `cargo install --root <cached-cellar-path>` or a manual `cargo build` pipeline.
- [x] Cache resulting binary in the cellar under a `cargo/` prefix directory.
- [x] Composing path environment includes the cached binary directory.

## Technical approach

- Update `types.rs` to support a new spec backend type.
- Add registry resolver code in `inventory.rs` or a new module to query crates.io registry.
- Compile packages using target tooling and verify sandbox bindings for cargo directories.
