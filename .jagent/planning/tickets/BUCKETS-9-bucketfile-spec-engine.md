# BUCKETS-9 — Bucketfile Specification & Build Engine

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-9 |
| **Priority** | P1 |
| **Status** | Done |
| **Phase** | M5 |
| **Assignee** | antigravity, cece-buckets |
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

- [x] Define the `Bucketfile` syntax supporting:
  * `FROM <spec1> <spec2> ...` (base dependencies to pull and mount).
  * `ENV <key>=<value>` (environment variables for build/runtime).
  * `COPY <host-src> <sandbox-dest>` (copy local host files to build workspace).
  * `RUN <cmd>` (execute compilation/installation commands inside the sandbox).
  * `ENTRYPOINT <cmd>` or `CMD <cmd>` (define default entrypoint wrapper command).
- [x] Implement `buckets build` CLI command:
  * Looks for `Bucketfile` in the current directory (or `-f <path>`).
  * Resolves and installs the `FROM` dependencies.
  * Creates a target local package directory: `~/.cache/buckets/local/<bucket-name>/v0.0.0/`.
  * Runs the build plan sequentially inside the `bwrap`/`proot` sandbox.
  * Generates a launcher script in the local package's `bin/` directory that wraps the `ENTRYPOINT` command with the dependencies mounted.
- [x] Verify execution of a `Bucketfile` by running a sample project (e.g. compiling a small script or running a custom CLI utility) using the generated local bucket.

## Technical approach

1. Create `src/bucketfile.rs` defining `BucketfilePlan` parser and compiler logic.
2. Wire `build` sub-command in `src/main.rs`.
3. Generate a wrapper script under `local/<name>/v0.0.0/bin/` so that `buckets run local/<name>` resolves and runs the entrypoint script inside the sandbox.
4. Add comprehensive tests validating parsing, sandboxed build, and execution of Bucketfiles.

## Bugs found and fixed post-implementation (session BUCKETS-9-10-continue, 2026-07-16)

antigravity's initial implementation had 2 real bugs, left half-fixed uncommitted in this
worktree; a 3rd was found and fixed fresh this session. All three verified with live
`buckets build`/`buckets run`, not just unit tests:

- [x] **ENTRYPOINT/CMD parsing used naive `.split_whitespace()`** (`src/main.rs::cmd_run`) — a
  quoted arg like `node -e "console.log(1)"` passed literal quote characters through to the
  runtime instead of being shell-split, silently changing behavior. Fixed with `shlex::split()`.
  Live-verified: `ENTRYPOINT node -e "console.log('shlex-quote-test-ok')"` built and ran,
  printing `shlex-quote-test-ok` (not garbled, not silent).
- [x] **`COPY <src> <dest>` resolved `src` against the `buckets` process's cwd instead of the
  Bucketfile's own directory** (`src/bucketfile.rs::build_bucketfile`) — `buckets build
  <other-dir>` from a different cwd than `<other-dir>` failed with "No such file or directory"
  on a relative `COPY` line. Fixed by anchoring relative `src` to `bucketfile_path.parent()`.
  Live-verified: `buckets build /tmp/bf-test3 -t copy-test` (invoked from a different cwd) with
  a `COPY test.js test.js` line succeeded, and `buckets run local/copy-test` printed
  `copy-path-resolution-ok` from the copied file.
