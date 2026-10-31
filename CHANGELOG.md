# Changelog

## Unreleased

### Security — sandbox hardening (BUCKETS-17)

Requires **zandbox 0.1.0** (crates.io).

- **Fail closed.** Without `bwrap`, `run`/`shell`/`build`/`gui`/`site`/
  `session`/Bucketfile `RUN` refuse with an error naming the fix, instead of
  falling back to proot or to an unsandboxed process with a warning.
  `--allow-weaker-sandbox` (proot) and `--no-sandbox` are the explicit
  opt-outs. A bwrap missing a required flag is also a refusal.
- **Clean environment.** `bwrap --clearenv` + `--setenv` for the composed
  toolchain env, `HOME=/tmp`, and `TERM`/`COLORTERM`/`NO_COLOR`/`LANG`/
  `LANGUAGE`/`LC_*`/`TZ`. The host environment no longer leaks into a
  sandbox. `--no-sandbox` runs are clean-env too. `--inherit-env` restores
  the old behaviour.
- **bwrap flags:** `--die-with-parent`, `--new-session` (not for `shell`),
  `--cap-drop ALL`, `--unshare-ipc` (not for `gui`/`site`), `--unshare-uts`
  + `--hostname bucket`, `--unshare-cgroup-try`, `--unshare-user
  --disable-userns` (not for `site`), on top of the existing mount/PID/net.
- **seccomp** inside bwrap via `--seccomp FD`, with the program built by
  zandbox: `--seccomp off|default|strict` (default `default`: zandbox
  `Application` + a compatibility list − `unshare`, EPERM otherwise).
- **Resource limits:** `--memory`, `--pids`, `--cpu` (cgroup v2 via zandbox,
  joined in the bwrap process's `pre_exec`; needs `--cgroup-parent` /
  `BUCKETS_CGROUP_PARENT`; `memory.swap.max` pinned to 0 with `--memory`) and
  `--timeout` (kills the whole sandbox, exit 124).

### Fixed

- `session start`/`session exec`: the `/session` overlay (and
  `/session-upper`) binds were appended after bwrap's `--` separator, so they
  became the program's arguments and the overlay was never mounted in the
  sandbox. They are now profile binds emitted before `--`.
- `shell`: `BUCKET_PROMPT`/`PS1` are passed through the sandbox env rather
  than set on the bwrap process.

### Library API

- New: `sandbox::try_sandboxed_command` → `Result<Sandbox, SandboxError>`,
  `sandbox::unsandboxed`, `Sandbox`/`SandboxChild`/`RunOutcome`,
  `Fallback`, `EnvPolicy`, `SeccompMode`, `Limits`, `BindMount`,
  `clean_env`, `parse_size`/`parse_cpu_percent`/`parse_duration`.
- `SandboxProfile` gained fields that all default to the strict choice
  (`SandboxProfile { .., ..Default::default() }` still compiles).
- `sandboxed_command` keeps its signature. When the sandbox can't be built
  it now returns a command that runs nothing and exits 126 with the reason
  on stderr (it used to run the program unsandboxed).
