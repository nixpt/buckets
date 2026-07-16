# BUCKETS-5 — Local Pantry Overrides

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-5 |
| **Priority** | P1 |
| **Status** | In Progress |
| **Phase** | M3 |
| **Assignee** | antigravity |
| **Dependencies** | none |
| **Estimated effort** | M |

## Problem

Currently, `buckets` only resolves packages using the built-in index aliases or crates.io (in BUCKETS-4) and fetches binaries from remote registry servers. There is no way to override standard package definitions (e.g. `nodejs.org` or a custom tool name like `crush`) locally. We need a configuration-based override mechanism to redirect any package request to a local source directory.

## Success criteria

- [ ] Support loading overrides from a `pantry.toml` configuration file.
- [ ] Load `pantry.toml` from the current directory (local workspace) or `~/.config/buckets/pantry.toml` (global user configuration).
- [ ] Let `pantry.toml` define overrides:
  ```toml
  [overrides."nodejs.org"]
  path = "/workspace/projects/node"
  version = "20.99.0"  # Optional, default: 0.0.0
  provides = ["node", "npm"]  # Optional
  ```
- [ ] Intercept version resolution in `inventory.rs`: if the package name has a pantry override, immediately return its declared version (bypassing the network).
- [ ] Implement `install_pantry_override` in `install.rs` (sharing core logic with local path building) to compile the overridden local package and cache it.
- [ ] Verify that running `buckets run node` uses the overridden local node installation path when the override is active.

## Technical approach

1. Define `PantryOverride` struct in `config.rs` and load `pantry.toml` from `./pantry.toml` and `~/.config/buckets/pantry.toml` during `Config::new()`.
2. Update `inventory::list_remote_versions` to return the override version if active.
3. Refactor `install::install_path` into a shared `install_local_dir` helper.
4. Call `install_local_dir` inside `install::install` when a pantry override is matched.
5. Add unit and integration tests.
