# BUCKETS-8 — Local Path Spec Support

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-8 |
| **Priority** | P1 |
| **Status** | Done |
| **Phase** | M3 |
| **Assignee** | antigravity |
| **Dependencies** | none |
| **Estimated effort** | M |

## Problem

Currently, `buckets` only runs packages resolved from `dist.pkgx.dev` or `crates.io`. If a developer makes local changes to a workspace project (like `crush-ast` or `crush-vm`), they cannot dynamically execute the local code inside buckets alongside other toolchains. We need a `path:` spec prefix (e.g. `path:/workspace/projects/crush-ast` or `path:.`) to automatically compile the local source code and run the compiled binaries inside the throwaway bucket environment.

## Success criteria

- [x] Support the `path:` spec prefix (e.g., `buckets run path:. -- my-local-binary`).
- [x] For `path:` specs, resolve version to `0.0.0` and always trigger a rebuild (`install_path` has no "already installed" fast-path, unlike the main `install()` dispatcher — confirmed by reading `install.rs`).
- [x] Detect the build system of the target path, resolve its toolchain specs, and execute the build sandboxed.
- [x] For Rust Cargo projects, run `cargo install --path <source_path> --root <target_cellar_dir>` to compile and install binaries. **Live-verified** (foreman, s388): a real scratch Cargo project resolved, compiled via a genuine `cargo install --path`, cached under `~/.cache/buckets/path/<abs-path>/v0.0.0/bin/`, and the freshly-built binary actually executed and printed its real output.
- [ ] For Go projects, run `go build -o <target_cellar_dir>/bin/<name>` to compile binaries. Implemented in code, **not independently live-tested** by this review (no Go toolchain exercised).
- [ ] For Node/npm projects, parse the `bin` field of `package.json` and generate appropriate wrapper scripts in `<target_cellar_dir>/bin/`. Implemented in code, **not independently live-tested** by this review.
- [ ] For others, copy or symlink the `bin/` contents of the source directory if it exists. Implemented in code, **not independently live-tested** by this review.
- [x] Verify that `buckets run path:<local-path>` correctly mounts the built cellar directory and executes the local binary successfully. Confirmed for the Cargo case (see above).

## Resolution

Implemented in `install.rs` (`install_path`, `read_package_json_bin`) + `inventory.rs`/`config.rs` (spec parsing, cache-path sanitization). Shares the same colon-in-cache-path fix as BUCKETS-4 (`sanitize_project_name` maps `path:/abs/path` → `path/abs/path`, avoiding the PATH-env-var colon-splitting bug BUCKETS-4 hit first). Cargo path independently live-verified end-to-end (real `cargo install --path`, real cached binary, real execution) — Go/npm/generic fallback paths are implemented and covered by the ticket's own design but weren't independently exercised in this review; flagging honestly rather than claiming full verification.

## Technical approach

- Parse `path:` specs in `types.rs` as valid package request projects.
- Map `path:` in `inventory.rs` version listing to return `0.0.0`.
- Implement build-and-install logic in `install.rs` delegating to `project::detect`.
