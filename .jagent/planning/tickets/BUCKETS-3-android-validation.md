# BUCKETS-3 — Android/Termux Verification

| Field | Value |
|-------|-------|
| **ID** | BUCKETS-3 |
| **Priority** | P2 |
| **Status** | Backlog |
| **Phase** | M2 |
| **Assignee** | unassigned |
| **Dependencies** | BUCKETS-2 |
| **Estimated effort** | M |

## Problem

Android and Termux nodes run custom vendor kernels that have strict LSM (Linux Security Modules) security policies. Specifically, Yama `ptrace_scope` and SELinux can block child process ptrace attachment, which could break the `proot` backend. We need to verify the implementation on a real Android node.

## Success criteria

- [ ] Execute `buckets` under a Termux environment on `phone-claude` or a similar test device.
- [ ] Confirm `buckets run` executes successfully using the `proot` fallback.
- [ ] Verify there are no ptrace denial logs or crashes under the target platform.
- [ ] Resolve any signal propagation issues (such as SIGTERM forwarding gaps under PRoot).

## Technical approach

- Deploy the compiled binary to the Termux test target.
- Run tests and trace system calls if blocked.
