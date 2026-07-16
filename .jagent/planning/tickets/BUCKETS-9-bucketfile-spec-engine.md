# BUCKETS-9 — Bucketfile Specification & Build Engine

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-9 |
| **Priority** | P1 |
| **Status** | In Progress |
| **Phase** | M5 |
| **Assignee** | antigravity |
| **Dependencies** | none |
| **Estimated effort** | L |

## Problem

Currently, `buckets` resolves packages from remote registries or compiles single local directories directly. There is no declarative way to specify a recipe (similar to a Dockerfile) that defines:
1. Base dependencies (`FROM`).
2. Build-time commands (`RUN`).
3. Files/folders to copy into the sandbox (`COPY`).
4. Custom environment variables (`ENV`).
5. A default command (`ENTRYPOINT` or `CMD`).

We need a "Bucketfile" specification and a `buckets build` CLI command that parses this file, builds the target bucket hermetically in the sandbox, and registers it as a reusable package in the local cellar.

## Success criteria

- [ ] Define the `Bucketfile` syntax supporting:
  * `FROM <spec1> <spec2> ...` (base dependencies to pull and mount).
  * `ENV <key>=<value>` (environment variables for build/runtime).
  * `COPY <host-src> <sandbox-dest>` (copy local host files to build workspace).
  * `RUN <cmd>` (execute compilation/installation commands inside the sandbox).
  * `ENTRYPOINT <cmd>` or `CMD <cmd>` (define default entrypoint wrapper command).
- [ ] Implement `buckets build` CLI command:
  * Looks for `Bucketfile` in the current directory (or `-f <path>`).
  * Resolves and installs the `FROM` dependencies.
  * Creates a target local package directory: `~/.cache/buckets/local/<bucket-name>/v0.0.0/`.
  * Runs the build plan sequentially inside the `bwrap`/`proot` sandbox.
  * Generates a launcher script in the local package's `bin/` directory that wraps the `ENTRYPOINT` command with the dependencies mounted.
- [ ] Verify execution of a `Bucketfile` by running a sample project (e.g. compiling a small script or running a custom CLI utility) using the generated local bucket.

## Technical approach

1. Create `src/bucketfile.rs` defining `BucketfilePlan` parser and compiler logic.
2. Wire `build` sub-command in `src/main.rs`.
3. Generate a wrapper script under `local/<name>/v0.0.0/bin/` so that `buckets run local/<name>` resolves and runs the entrypoint script inside the sandbox.
4. Add comprehensive tests validating parsing, sandboxed build, and execution of Bucketfiles.
