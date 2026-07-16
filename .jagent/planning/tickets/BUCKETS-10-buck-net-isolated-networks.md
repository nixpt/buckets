# BUCKETS-10 — buck-net: Isolated Virtual Networks

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-10 |
| **Priority** | P2 |
| **Status** | Done |
| **Phase** | M5 |
| **Assignee** | antigravity, cece-buckets |
| **Dependencies** | none |
| **Estimated effort** | M |

## Problem

Buckets are otherwise network-isolated (`--unshare-net` in the sandbox by default), but there
was no way to let two or more buckets talk to each other on a private network without exposing
them to the real host network. We need a named, isolated virtual network that multiple buckets
can join and communicate over, with no route out to the host NIC unless explicitly forwarded.

## Success criteria

- [x] `buckets net create <name>` creates a persistent, named network namespace (rootless, via
  `unshare --user --net`), backed by a `sleep infinity` keeper process whose PID is persisted to
  `{nets_dir}/{name}/info.json`.
- [x] `buckets net rm <name>` kills the keeper process and removes the namespace state.
- [x] `buckets net ls` lists active networks, sweeping stale entries whose keeper has died.
- [x] `buckets net run <name> <spec> -- <cmd>` (and `buckets run --net <name> ...`) joins a bucket
  to the named net namespace via bwrap's `--netns /proc/{pid}/ns/net`, sharing loopback with any
  other bucket in the same net.
- [x] `expose_port` forwards a host-side TCP port into the namespace via `socat`/`nsenter`, for the
  cases where host access to a bucket-side service is actually wanted.
- [x] No new Cargo dependencies — implemented against `unshare`/`nsenter`/`socat` (util-linux /
  widely packaged), matching the rest of the project's standalone/no-daemon stance.

## Technical approach

1. `src/net.rs`: `NetSession` — `create`/`load`/`destroy`/`list_all`/`expose_port`, keeper-PID
   liveness checks (`pid_alive`), namespace-ready polling (`wait_for_ns`).
2. Wire `buckets net {create,rm,ls,run}` subcommands and `buckets run --net <name>` in
   `src/main.rs`.
3. `unshare --user --net` pairs a user namespace with the net namespace so `CAP_NET_ADMIN` isn't
   required — the keeper appears as root inside its own user namespace only.

## Bug found and fixed post-implementation (session BUCKETS-9-10-continue, 2026-07-16)

- [x] **Missing `--map-root-user`** — `unshare --user --net -- sleep infinity` without
  `--map-root-user` leaves the keeper at UID `nobody` (65534) with `CapEff` all zeros instead of
  root-in-its-own-namespace, so anything the keeper or a joined bucket does that needs
  `CAP_NET_ADMIN` inside the namespace (e.g. `ip link set lo up`) fails with "Operation not
  permitted". Confirmed directly: `unshare --user --net -- id` printed `uid=65534(nobody)`.
  Fixed by adding `--map-root-user` to the `unshare` invocation in `NetSession::create`.
  Live-verified this session: `buckets net create test-net-verify` → succeeds, no permission
  error; `buckets net rm test-net-verify` → keeper killed, state removed cleanly.

## Known gaps (not addressed this session, out of scope for BUCKETS-9-10-continue)

- No internet/NAT access from inside a buck-net by design (documented in `src/net.rs` module
  doc as future work — `slirp4netns` integration).
- No automated test exercises `expose_port`'s `socat`/`nsenter` path live (unit tests cover only
  `pid_alive`/`list_all` bookkeeping, not the actual namespace networking) — worth a live-test
  pass if `expose_port` sees real use.
