# TASKS — buckets

Every open item below represents a planned task or issue. See `.jagent/planning/tickets/` for full details on each `BUCKETS-N` ID.

---

## P0 — Build & Core Health ✅

- [x] Standard compilation with default features (`cargo build`)
- [x] Full unit and binary tests passing (`cargo test` - 146 tests green)
- [x] Binary size under 2MB via LTO and symbol stripping
- [x] Zero `cargo doc` warnings or dependency build issues

---

## M2 — Sandbox Portability (PRoot Fallback)

Fallback to ptrace-based `proot` interception for environments without namespace namespace capability.

- [x] **BUCKETS-1** (S): **Spike PRoot on Developer Box** — Verify basic command execution, path mapping, and standard streams/sentinel behaviors under `proot`.
- [x] **BUCKETS-2** (M): **Implement ProotBackend in sandbox.rs** — Implement `build_proot_args` mapping `SandboxProfile` fields (`project_dir`, `extra_ro_binds`, `allow_network`) to proot arguments. Warn on lack of real namespace-enforced network/PID isolation.
- [ ] **BUCKETS-3** (M): **Android/Termux Verification** — Test and resolve Yama `ptrace_scope` and SELinux LSM restrictions on an actual Android node (`phone-claude`).

---

## M3 — Cargo Spec Resolution

Bridge the distribution gap for Rust-based tools by downloading and building them from crates.io natively.

- [ ] **BUCKETS-4** (L): **Cargo Spec Type Engine** — Implement parsing, version resolving, fetching, building, and caching cargo packages (e.g. `buckets run cargo:crush-ast@0.2.0`).
- [x] **BUCKETS-8** (M): **Local Path Spec Support** — Support running local project source code directly in buckets via a `path:` spec prefix (e.g. `path:.`).
- [ ] **BUCKETS-5** (M): **Local Pantry Overrides** — Create configuration to override package distributions with local directories or custom manifests.

---

## M4 — Fleet Concurrency & Optimization

Ensure concurrent performance and resource sharing for parallel fleet agents.

- [ ] **BUCKETS-6** (M): **Cellar Cache Locking** — Optimize multi-agent write locks to prevent corrupted/concurrent extractions in the shared cache.
- [ ] **BUCKETS-7** (S): **CLI Diagnostic Cleanups** — Polish error outputs when remote connections fail or index versions are not found.
