# BUCKETS-15 — PyPI + npm Spec Resolvers

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-15 |
| **Priority** | P2 |
| **Status** | Done |
| **Phase** | M5 — Lang registries |
| **Assignee** | nixp |
| **Dependencies** | BUCKETS-4 (cargo: pattern) |
| **Estimated effort** | L |

## Problem

Flame's `pypi:` / `npm:` kindle adapters reimplemented install+cellar logic
outside buckets. The cargo: path (BUCKETS-4) proved the right shape —
`resolve_multi` → shared cellar → compose_env — but buckets had no
`pypi:` / `npm:` resolvers for flame to thin-delegate to.

## Success criteria

- [x] `pypi:<pkg>[@ver]` resolves versions via PyPI JSON API
- [x] `npm:<pkg>[@ver]` resolves versions via registry.npmjs.org
- [x] Install via `uv`/`pip --target` (pypi) and `npm install --prefix` (npm)
- [x] Cache under `pypi/<pkg>/v…` and `npm/<pkg>/v…` (colon sanitized)
- [x] `compose_env` sets `PYTHONPATH` / `NODE_PATH` (+ `node_modules/.bin` on PATH)
- [x] `.buckets-installed` marker so pure-library installs count as installed
- [x] Unscoped packages only for v1 (scoped `@scope/name` rejected — `@` collision)
- [x] Tests: inventory, compose_env, install, resolve_multi e2e

## Technical approach

Mirror `cargo:`: branch in `inventory::list_remote_versions` +
`install::install` + `config::sanitize_project_name` + `env::compose_env`.
No new Source trait yet — prefix dispatch matches the cargo pattern.

## Files

- `src/inventory.rs` — PyPI/npm version listing + validation
- `src/install.rs` — `install_pypi` / `install_npm`
- `src/env.rs` — PYTHONPATH / NODE_PATH
- `src/config.rs` — sanitize `pypi:` / `npm:` → nested dirs
- `src/cellar.rs` — `.buckets-installed` marker for `is_installed`
- `README.md` — usage examples
