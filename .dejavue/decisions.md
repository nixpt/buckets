# Decisions


## 2026-07-10T08:59:00-05:00 — [STRATEGIC] [ADOPTED] [ARCHITECTURAL] Standalone, sync, no-daemon design — deliberately not touching exosphere

Reason:
Third crate in a pkgx-derived provisioning lineage (exosphere's exo-hydra → exo-light's exo-hydra → here). The first two are async and produce a CapsuleManifest for a daemon to spawn under full isolation. buckets is a separate throwaway-runtime surface for AI agents that just need 'run this with node@20' without any daemon/capsule machinery — sync throughout, composes a plain shell env instead of a manifest, and was built explicitly without modifying exosphere at all.

Artifacts: src/lib.rs,README.md

Rejected alternatives:
- **fork exo-hydra directly**: would inherit async/tokio + daemon-manifest output neither of which this surface needs
- **extend exo-light's exo-hydra with a sync facade**: keeps two runtime models in one crate instead of a clean standalone tool

Outcome:
cargo lib+bin crate, 8 modules (types/index/resolve/cellar/inventory/install/env/main), 23 tests, zero cargo-doc warnings, no path-deps on any peer project


## 2026-07-10T09:31:04-05:00 — [STRATEGIC] [VERIFIED] [ARCHITECTURAL] Live-tested against dist.pkgx.dev — found and fixed 4 real bugs (URL format, unpinned companions, non-semver versions, wrong jq alias)

Reason:
The README/index/pipeline looked complete and cargo test was green, but nothing had ever actually hit the real dist server end-to-end. First live run (node@20) failed at every stage in sequence as each bug was fixed: (1) URL path order was {platform}/{project} instead of pkgx's real {project}/{platform}/{arch}, and arch used 'x86_64' instead of pkgx's 'x86-64' (hyphen) — both verified via curl against a real pkgx source read + live 404s/200s. (2) nodejs.org had an empty companions list, but a real downloaded bottle's ldd showed it needs libcrypto.so.1.1/libssl.so.1.1 + libicu*.so.73 — and companions had no version-pinning mechanism at all (always resolved *,  and never alias-resolved the companion's own project name). (3) openssl's 1.1.1x releases use a letter-suffixed non-semver scheme (1.1.1w) that Version::parse silently drops, making the entire 1.1.1 line invisible to the resolver even after companions were fixed. (4) jq's alias pointed at 'jqlang.org', which 404s — the real project is 'stedolan.github.io/jq'.

Artifacts: src/config.rs,src/types.rs,src/inventory.rs,src/cellar.rs,src/install.rs,src/index.rs,src/resolve.rs,src/env.rs,src/main.rs

Rejected alternatives:
- **trust cargo test's 23 passing unit tests as sufficient**: none of them made a real network call, so all 4 bugs were invisible to the suite

Outcome:
node@20 and jq@latest both verified working end-to-end (resolve -> download -> install -> compose env -> exec), rerun confirmed cache-hit path skips re-download. Added dist_version_string() as the one place that knows how to round-trip a letter-suffixed version back to the dist server's real string form. NOT exhaustively re-verified: the other ~55 aliases in index.rs may have similar wrong-project-name bugs to jq's — only node/openssl/icu4c/jq were confirmed live.


## 2026-07-10T10:15:03-05:00 — [STRATEGIC] [VERIFIED] [ARCHITECTURAL] Real bwrap sandboxing + git-clone/build/run pipeline — found 3 more real bugs via live testing

Reason:
User wanted buckets expanded toward a full ephemeral dev-environment platform (build/run arbitrary source without touching the host, worktree buckets, per-agent buckets, capsule mapping, X11 GUI buckets). Sequenced: real process isolation first (bwrap — several of the other ideas assume containment that didn't exist; run/shell were plain unsandboxed subprocess exec before this), then a git-clone->detect->build->run pipeline as the first concrete phase (no new isolation needed, reuses the existing resolve engine, testable on real projects immediately). Standalone (no exosphere/exo-light dependency), consistent with buckets' earlier founding decision. x11docker's own design confirmed GUI sandboxing needs real containment to exist first, so it's explicitly deferred, not built here.

Artifacts: src/sandbox.rs,src/project.rs,src/main.rs,src/resolve.rs,src/index.rs

Rejected alternatives:
- **route sandboxing through exo-light's ExoLightRuntime**: exo-light's own subprocess backend documents itself as having no namespace/cgroup dependency anyway (the real isolation code is exosphere's crates/exo/container, a heavier crate) — would add a peer dependency for no real containment benefit
- **build real isolation AND worktree/per-agent/capsule features in one pass**: too much surface for one verified pass; those are natural follow-ons once build proves out

Outcome:
sandbox.rs (bwrap-based, unit-tested arg-building + live-verified: normal exec works, write outside grants fails, write inside cwd succeeds), project.rs (source resolution + 5-language build detection), buckets build subcommand — all wired through the same resolve/install engine. Found 3 more real bugs via live testing (matching this project's now-established pattern of cargo test passing while real network/filesystem behavior was still broken): (1) --chdir failed because run/shell never bound the invocation cwd into the sandbox: fixed by treating cwd as a rw project_dir bind. (2) companion resolution was only one level deep, so rust's companion-of-a-companion (cargo needs openssl@^1.1, but cargo itself is only reachable as rust's companion) was silently dropped: fixed by making companion collection a proper transitive worklist/BFS, with an honestly-documented known gap (no constraint-intersection when two paths want the same companion under different constraints). (3) sharing the network namespace (omitting --unshare-net) doesn't give working DNS on its own: cargo's crates.io fetch failed with 'Could not resolve host' because /etc/resolv.conf wasn't bound into the fresh mount namespace — fixed by ro-binding resolv.conf/hosts/nsswitch.conf/ssl/ca-certificates when allow_network is set. Verified end-to-end: a real Rust crate (exo-light's capsule-contract) built AND tested successfully via local path; a real 16-crate workspace (interactd) cloned fresh via git URL and built successfully, all inside the sandbox. 55 tests, cargo doc clean.

