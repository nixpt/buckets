# BUCKETS-17 — Sandbox hardening with zandbox

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-17 |
| **Priority** | P1 |
| **Status** | In review |
| **Phase** | Security |
| **Assignee** | satya |
| **Dependencies** | zandbox 0.1.0 (crates.io) |
| **Estimated effort** | L |

> Note: `9f3ca0c` ("worktree create --path ... (BUCKETS-17)") already used
> this number for an unrelated change, without a ticket file. This file is
> the sandbox-hardening BUCKETS-17 the 2026-10-31 audit asked for.

## Problem (2026-10-31 audit of `src/sandbox.rs` / `src/session.rs` @ 7375f92)

1. Failed open: no bwrap → proot (host `/` visible, no net/pid isolation) →
   plain unsandboxed `Command`, with only a stderr warning.
2. Host environment inherited (no `--clearenv`; env set on the bwrap process).
3. Missing bwrap hardening: `--die-with-parent`, `--new-session`,
   `--cap-drop ALL`, `--unshare-ipc`, `--unshare-uts`.
4. No resource limits.
5. Session bug: `/session` binds appended after `--`, so they were argv.

## Success criteria

- [x] Strict by default: refuse without bwrap; `--allow-weaker-sandbox` (proot)
      and `--no-sandbox` are explicit; no silent downgrade anywhere
- [x] Clean env via `--clearenv` + `--setenv` (composed env + small allowlist,
      `HOME=/tmp`); `--inherit-env` opt-out; `--no-sandbox` clean too
- [x] bwrap flags added, probed from `bwrap --help` (required ones refuse if absent)
- [x] cgroup limits via zandbox in the bwrap process's `pre_exec`
      (`--memory/--pids/--cpu`, `--cgroup-parent`/`BUCKETS_CGROUP_PARENT`),
      `--timeout` kills the whole sandbox
- [x] seccomp via zandbox → memfd → `bwrap --seccomp FD`; `--seccomp off|default|strict`
- [x] Session binds before `--`; test on the generated argv + live bind test
- [x] `sandboxed_command` source-compatible for crush-vm; fallible `try_sandboxed_command`
- [x] README sandbox section, CHANGELOG

## Design points

- zandbox's namespaces/seccomp/landlock are NOT applied before `exec bwrap`
  (bwrap needs mount/pivot_root/unshare). zandbox is used for exactly two
  things: the cgroup (prepare before fork, join in `pre_exec`) and compiling
  the BPF program (`Seccomp::compile` → `CompiledFilter::program`, already
  public in 0.1.0, so zandbox needed no API change).
- The seccomp default profile was picked from measurement. `strace -f` over
  python3 (threads, subprocess, multiprocessing, venv), node (child_process,
  worker_threads), bash, pip, uv, npm and git used 110 distinct syscalls;
  only `copy_file_range` and `io_uring_*` fell outside zandbox `Application`.
  `cargo build` used 116; only `io_uring_*` falls outside the final profile
  (deliberately denied, and cargo/libuv fall back). Default action EPERM, so
  an unexpected call fails rather than killing the process.
- `gui`/`site` keep IPC shared (X11 MIT-SHM) and seccomp off; `site` allows
  nested user namespaces (browser sandboxes); `shell` keeps the terminal
  session (job control). `dev.tty.legacy_tiocsti=0` on zorro anyway.
- `memory.swap.max=0` with `--memory`, otherwise the cap could be bypassed
  by swapping.

## Verification

See the PR body for the exact `test result:` lines and which cgroup tests
ran live (they ran in a `systemd-run --user --scope -p Delegate=yes` scope).
