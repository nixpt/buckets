# buckets — Gaps & Opportunities

Snapshot: 2026-08-26 · 19 source files, ~7,400 LOC · 95 tests · pkgx-derived lineage (exo-hydra → buckets)

## Module maturity map

| Module | LOC | Maturity | Notes |
|---|---|---|---|
| `types.rs` | 244 | **mature** | Well-tested parsing; `dist_version_string` is load-bearing |
| `config.rs` | 226 | **mature** | env-var config, pantry.toml overrides, tested |
| `index.rs` | 456 | **mature** | ~100 curated aliases/companions; live-verified pins (node→openssl@^1.1) |
| `inventory.rs` | 261 | **mature** | remote/cargo/path versions; letter-suffix parse; good error mapping |
| `resolve.rs` | 309 | **mature** | transitive companion BFS; one documented limitation (constraint conflict) |
| `install.rs` | 552 | **mature** | atomic rename, flock, parallel; path:/cargo:/pantry support; tested |
| `env.rs` | 257 | **mature** | PATH/LD/C PATH composition; shell+json output |
| `cellar.rs` | 175 | **mature** | v*/v\<major\>/v\<mm\> symlinks; 2 dead tombstone fns |
| `sandbox.rs` | 405 | **solid, narrow** | real bwrap; bwrap/proot/none fallback; no caps/seccomp/cgroups |
| `gui.rs` | 248 | **solid** | real Xvfb lifecycle; clean Drop; VNC; tested |
| `site.rs` | 133 | **solid, thin** | just resolves storage dir; enforcement is delegated to sandbox |
| `worktree.rs` | 236 | **mature** | real git wrapper; sibling-default; 6 tests incl. regression |
| `net.rs` | 320 | **solid** | real unshare/netns + socat expose; no NAT |
| `session.rs` | 597 | **real but buggy** | genuine OverlayFS; /session/ bind is broken; --zram is a no-op |
| `herd.rs` | 551 | **real, partly unwired** | real reconciler; scale() dead code; stop/scale can't reach running deploy |
| `bucketfile.rs` | 284 | **functional, limited** | FROM/ENV/COPY/RUN/WORKDIR/ENTRYPOINT; no multi-stage/globs |
| `main.rs` | 1760 | **mature CLI** | 10 subcommands; one ignored flag (gui --web) |

## High — correctness (claimed feature doesn't work)

### 1. session.rs `/session/` bwrap bind is broken

`session_start` (line ~347) and `session_exec` (line ~451) call `sandboxed_command(...)`
and *then* append `.arg("--bind").arg(&mount).arg("/session/")`. But
`sandboxed_command`'s bwrap path already terminated its arg list with
`-- <program> <args…>` — so `--bind /session` gets passed as arguments to the
*program*, not to bwrap. The overlay is mounted on the host but never reaches
the sandbox's mount namespace.

The headline feature ("share a writable FS across execs") doesn't actually
work under bwrap. Fix: pass the session mount through
`SandboxProfile.extra_rw_binds` (or a new bind-at-path field) *before*
`sandboxed_command` builds the arg list.

### 2. `gui --web` is a no-op

`cmd_gui(..., _web: bool, ...)` in main.rs — the flag is accepted and
immediately discarded. Either implement the noVNC web-UI serving (the VNC
server part already works) or remove the flag.

### 3. `session --zram` is a silent no-op

`session_start` only does `if use_tmpfs { tmpfs_mount(...) }`;
`use_zram` is recorded in `SessionConfig.upper_is_zram` but no zram device
is ever provisioned. The flag lies.

### 4. herd stop/scale can't reach a running deploy (BUCKETS-12)

`HerdController::scale()` is `#[allow(dead_code)]` and unreachable from the
`buckets herd scale` CLI — it just prints "requires the controlling process."
`herd stop` (separate process) SIGTERMs the replica PIDs, but the still-running
`herd deploy` reconciler thread sees dead replicas and **respawns them**. No
IPC channel between the two processes. `InstanceState.last_exit_code` is
always `None` (never populated — dead field).

## Medium — containment depth

### 5. No capability drop, seccomp, or cgroups

The bwrap invocation never emits `--cap-drop` / `--seccomp` / resource limits.
Isolation is "fresh mount ns + RO system," not least-privilege.

### 6. Host /usr is bound RO unconditionally

A `run` with `allow_network:false` and no `project_dir` can still read and
exec any host binary under `/usr /bin /sbin /lib /lib64`. There's no
restriction on which host programs are reachable.

### 7. proot fallback silently widens the blast radius

`build_proot_args` binds `extra_ro_binds` with `-b` (read-**write**), so the
RO toolchain installs become writable, and proot provides no net/PID ns at
all. The warning message calls out the missing namespaces but doesn't call
out the RW-bind downgrade.

### 8. buck-net has no NAT/internet

A bucket in a buck-net has no route to the outside. `expose_port` is
foreground-only (blocks forever, no daemonization/pidfile). Host side is
hardcoded to 127.0.0.1 — can't expose to an interface/LAN.

## Low — hygiene

### 9. Dead code

- `cellar.rs::has_lookup_tombstone` / `mark_lookup_tombstone` — both
  `#[allow(dead_code)]`, never called. "Remember this project doesn't exist"
  feature is unwired.
- `index.rs::is_known` / `iter_aliases` — `#[allow(dead_code)]`
- `types.rs::PackageReq::matches` — `#[allow(dead_code)]`
- `resolve.rs::resolve` (single, not `resolve_multi`) — `#[allow(dead_code)]`
- `herd.rs::InstanceState.last_exit_code` — always `None`, never populated

### 10. Crash recovery

If a process dies between `session start` and `session stop`, the
`/tmp/buckets-session-*` overlay mount leaks. `list_sessions` reads
`config.toml` but never sweeps stale/unmounted entries.

### 11. install_local_dir edge cases

- `source_dir.file_name().unwrap()` panics if path ends in `/`
- `.ok()` on `detect()` failure silently skips toolchain resolution instead
  of surfacing the error

### 12. resolve.rs error message

`resolve_version` (line ~199) maps `list_remote_versions` error to
`"unknown"` in the message, discarding the real cause.

## Distribution gap (strategic)

### 13. crush can't provision itself through buckets

buckets resolves specs against pkgx's index (dist.pkgx.dev). `buckets run
crush` fails with "Failed to fetch versions from
https://dist.pkgx.dev/crush/linux/x86-64/versions.txt". buckets can
provision the RUNTIMES crush scripts need (node/python/bun/deno —
crush-pkg's runners.rs already routes these through buckets) but CANNOT
provision crush ITSELF.

**The fix is the distribution story:** buckets needs a spec backend that is
not pkgx. Crush crates are published to crates.io, so a `cargo:` spec type
(`buckets run cargo:crush-ast@0.2.0`) would give crush distribution
immediately AND every other Rust tool for free — a strictly bigger win than
a crush-only bottle. Alternative: a local/custom pantry so bottles can be
shipped without an upstream pkgx PR.

### 14. proot backend for non-bwrap hosts (BUCKETS-3)

Design doc filed:
`workspace-meta/plans/2026-07-16-proot-backend-for-buckets-flame-exosphere.md`.
Add a proot-based `ProotBackend` to sandbox.rs as a 3rd rung (bwrap → proot
→ bare exec) for hosts without kernel namespace support (Android/Termux
fleet nodes specifically, general hardened-kernel Linux secondary). proot is
portability-only, NOT security-equivalent to bwrap (no network/PID isolation,
ptrace-based) — must be labeled honestly wherever selected. Spike-first per
crush-ast's BUCKETSPIKE-1/2 precedent.

## What's working well

The codebase is unusually honest — every stub is *documented* as a stub, and
the git history shows live-verified fixes (openssl/icu4c pins, DNS binding,
sibling-worktree default, SIGINT handler, atomic-rename install) rather than
green-test-but-broken code. The biggest actual risk is `session.rs`, where a
real feature is wired incorrectly and would only surface in live use (which
CLAUDE.md itself warns `cargo test` can't catch).
