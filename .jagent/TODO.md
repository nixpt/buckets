# TODO

Tracked by agent. One-line items; complex work goes in `.jagent/planning/tickets/`.

## Priority: Sandbox Portability (PRoot Backend)

- [x] Install and verify `proot` behavior on developer host (desktop-Linux spike)
- [x] Add `ProotBackend` logic to `sandbox.rs` as a 3rd execution rung (bwrap -> proot -> bare exec)
- [x] Implement `build_proot_args` mapping the `SandboxProfile` fields (`project_dir`, `extra_ro_binds`, `allow_network`) to proot `-b`/`-w` options
- [x] Emit a warning when using proot since network/PID isolation is not namespace-enforced
- [ ] Verify `proot` behavior on an actual Termux/Android node (`phone-claude` environment) to check Yama ptrace LSM limitations

## Priority: Distribution Gap (Cargo spec resolver)

- [x] Implement `cargo:` scheme spec support (e.g., `buckets run cargo:crush-ast@0.2.0`)
- [x] Design resolve & install pipeline to fetch crate versions from crates.io registry
- [x] Download, build, and cache cargo package binaries into the cellar cache under `cargo/<crate>@<version>`
- [ ] Establish local/custom pantry override mechanism for offline or custom package configurations

## Priority: CLI & Usability Gaps

- [ ] Allow configuring default behavior via `.buckets.toml` or `config.rs` (sandbox selection preference)
- [ ] Address unhandled transitive companion dependency version conflicts (BFS resolver intersection limit)
- [ ] Improve error reporting during connection failures or parsing errors from `dist.pkgx.dev`
