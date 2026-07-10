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

