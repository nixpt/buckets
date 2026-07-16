# BUCKETS-8 — Local Path Spec Support

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-8 |
| **Priority** | P1 |
| **Status** | In Progress |
| **Phase** | M3 |
| **Assignee** | antigravity |
| **Dependencies** | none |
| **Estimated effort** | M |

## Problem

Currently, `buckets` only runs packages resolved from `dist.pkgx.dev` or `crates.io`. If a developer makes local changes to a workspace project (like `crush-ast` or `crush-vm`), they cannot dynamically execute the local code inside buckets alongside other toolchains. We need a `path:` spec prefix (e.g. `path:/workspace/projects/crush-ast` or `path:.`) to automatically compile the local source code and run the compiled binaries inside the throwaway bucket environment.

## Success criteria

- [ ] Support the `path:` spec prefix (e.g., `buckets run path:. -- my-local-binary`).
- [ ] For `path:` specs, resolve version to `0.0.0` and always trigger a rebuild (allowing developer edits to be recompiled).
- [ ] Detect the build system of the target path, resolve its toolchain specs, and execute the build sandboxed.
- [ ] For Rust Cargo projects, run `cargo install --path <source_path> --root <target_cellar_dir>` to compile and install binaries.
- [ ] For Go projects, run `go build -o <target_cellar_dir>/bin/<name>` to compile binaries.
- [ ] For Node/npm projects, parse the `bin` field of `package.json` and generate appropriate wrapper scripts in `<target_cellar_dir>/bin/`.
- [ ] For others, copy or symlink the `bin/` contents of the source directory if it exists.
- [ ] Verify that `buckets run path:<local-path>` correctly mounts the built cellar directory and executes the local binary successfully.

## Technical approach

- Parse `path:` specs in `types.rs` as valid package request projects.
- Map `path:` in `inventory.rs` version listing to return `0.0.0`.
- Implement build-and-install logic in `install.rs` delegating to `project::detect`.
