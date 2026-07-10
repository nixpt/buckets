# buckets

Throwaway runtime environments for AI agents — resolve, fetch, and run any
CLI tool in an isolated ephemeral environment without installing it
globally. Standalone binary + lib crate (not a workspace), sync throughout.

## Lineage

Third in a pkgx-derived provisioning lineage built in this order:
exosphere's `exo-hydra` (original, async, daemon-manifest output) →
exo-light's `exo-hydra` (ported, same async/manifest shape) → **buckets**
(here — deliberately separate, sync, no manifest/daemon handoff, doesn't
touch exosphere). Also borrows directly from `pkgx` itself
(`/workspace/external/pkgx`) for the bottle format and distribution
protocol — see the README's "Features borrowed from pkgx" section.

## Modules

`types` (spec/package/installation types) → `index` (alias resolution) →
`resolve` (top-level pipeline) → `cellar` (cache inspection) → `inventory`
(remote version lookup) → `install` (download/extract) → `env` (compose
PATH/etc.) → `main` (the `buckets` CLI, the only consumer of all of the
above). Full pipeline description in `src/lib.rs`'s crate doc.

## Build

```bash
cargo build
cargo test    # 23 tests, all unit-level (no network)
cargo doc --no-deps    # should produce zero warnings
```

No `CARGO_TARGET_DIR` redirection needed — standalone crate, no path-deps
on any peer project.

## Task IDs

`BUCKETS-XX`, branch `agent/<name>/BUCKETS-XX`, per `projects/CLAUDE.md`.

## Project memory

This repo uses [dejavue](https://github.com/nixpt/dejavue) for persistent architectural context.
Run `dejavue context` before making changes.
Fallback if not on PATH: `python3 .dejavue/dejavue context`

<!-- dejavue:discovery -->
