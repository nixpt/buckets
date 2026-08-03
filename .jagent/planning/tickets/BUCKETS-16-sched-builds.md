# BUCKETS-16 — `buckets build --sched`: admission-controlled cargo builds via buildsched

**Status:** done _(antigravity, 2026-08-03)_ · **= buildsched's BSC-5** · branch `agent/antigravity/BUCKETS-16`

## What

A `--sched` flag on `buckets build` that replaces the plain `cargo build` step
with `buildsched::pipeline::run` — per-crate jobs admitted against memory
watermarks, PSI pressure, and disk headroom (`[bob]` progress lines).

## Design points (ratified)

- **Opt-in cargo feature, default OFF.** `[features] buildsched = ["dep:buildsched"]`
  with `buildsched = { path = "../buildsched", optional = true }`. Preserves the
  standing "self-contained, no peer path-deps" posture for the default build
  (okd-core[compute] precedent). Extra reason: buildsched carries an
  absolute-path dep on zpu that only resolves on fleet boxes.
  Verified: default `cargo tree | grep -c buildsched` = 0.
- **Without the feature**, `--sched` fails fast (before any clone/resolve) with
  a rebuild-with-`--features buildsched` error.
- **Package selection:** `[package] name` parsed from the project's Cargo.toml
  (`toml` crate, already a dep). Virtual workspace (no `[package]`) → one honest
  line, fall back to plain `cargo build`. Non-cargo project → one-line warning,
  normal path.
- **Env:** the same composed toolchain env the normal unsandboxed path sets on
  its child (`resolved.env`) is injected into every pipeline subprocess via
  `PipelineConfig::with_child_env` (pipeline's own CARGO_TARGET_DIR /
  CARGO_BUILD_JOBS win on conflict — buildsched BSC-12 precedence).
- **Sandbox:** sched builds always run unsandboxed — the pipeline spawns its own
  cargo subprocess tree, which can't live inside the per-command bwrap wrap.
  `--sched` without `--no-sandbox` prints one notice line, then proceeds.
- **Target dir:** cargo's default (`<source_dir>/target`) via
  `PipelineConfig::simple`. No tiered/zram wiring here — embedders wanting that
  use bob directly.

## Verification

- Default build/test green (104 lib + 100 bin tests, unchanged), zero warnings,
  zero buildsched in `cargo tree`.
- Feature build/test green (107 lib-target + 100), zero warnings (+3 tests for
  the `cargo_package_name` helper).
- E2E: default binary errors on `--sched`; feature binary runs the pipeline on a
  single-package crate (`[bob] all 1 jobs completed`), prints the unsandboxed
  notice when `--no-sandbox` is omitted, falls back on a two-member virtual
  workspace, and the no-`--sched` path is untouched.
