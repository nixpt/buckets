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
`resolve` (top-level pipeline, transitive companion expansion) → `cellar`
(cache inspection) → `inventory` (remote version lookup) → `install`
(download/extract) → `env` (compose PATH/etc.) → `sandbox` (bwrap process
isolation — real containment, not just an isolated toolchain version) →
`project` (git-clone/local-path source resolution + build-system
detection for `buckets build`) → `worktree` (ephemeral git worktrees for
`buckets worktree` — a thin wrapper over `git worktree`/`git branch -d`;
produces a path `buckets build`/`run`/`shell` can target directly, no
separate build machinery) → `main` (the `buckets` CLI, the only consumer
of all of the above). Full pipeline description in `src/lib.rs`'s crate
doc.

`worktree`'s default worktree location is a SIBLING of the source repo,
not a fixed cache dir — found live that a fixed location breaks any
relative sibling path-dependency (`../other-repo`, this workspace's own
convention) since the worktree is no longer sitting next to its
siblings. `BUCKETS_WORKTREE_DIR` overrides this default.

Live-testing this project against real dist-server requests and real
builds has repeatedly found bugs `cargo test`'s pure-unit-level suite
can't see (wrong URL formats, missing/unpinned companions, non-semver
versions, DNS/sandbox binding gaps) — see `.dejavue/decisions.md` for the
full trail. Trust a green `cargo test` for logic, not for "does this
actually work against the real network/filesystem."

## Build

```bash
cargo build
cargo test    # 61 tests, all unit-level (no network) — see the live-testing note above
cargo doc --no-deps    # should produce zero warnings
```

`bwrap` (bubblewrap) must be installed for real sandboxing; `buckets`
still works without it (falls back to unsandboxed exec with a warning).

No `CARGO_TARGET_DIR` redirection needed — standalone crate, no path-deps
on any peer project.

## Task IDs

`BUCKETS-XX`, branch `agent/<name>/BUCKETS-XX`, per `projects/CLAUDE.md`.

## Project memory

This repo uses [dejavue](https://github.com/nixpt/dejavue) for persistent architectural context.
Run `dejavue context` before making changes.
Fallback if not on PATH: `python3 .dejavue/dejavue context`

<!-- dejavue:discovery -->
