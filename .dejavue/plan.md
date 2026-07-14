# Plan

Captured by agents as they work. An unchecked box is an open item;
nobody has committed to doing it, only to not losing it.

- [ ] **opportunity** — CRUSH DISTRIBUTION GAP (verified s382): buckets resolves specs against pkgx's index (dist.pkgx.dev) — 'buckets run crush' fails with 'Failed to fetch versions from https://dist.pkgx.dev/crush/linux/x86-64/versions.txt'. So buckets can provision the RUNTIMES crush scripts need (node/python/bun/deno — crush-pkg's runners.rs already routes these through buckets, on main @4e54caf) but CANNOT provision crush ITSELF. runners.rs already names this: 'Sona has no bottle in buckets index — it's a crush-specific tool, not something pkgx/buckets distributes.' THE FIX IS THE DISTRIBUTION STORY: buckets needs a spec backend that is not pkgx. Crush crates are published to crates.io, so a 'cargo:' spec type (buckets run cargo:crush-ast@0.2.0) would give us crush distribution immediately AND every other Rust tool for free — a strictly bigger win than a crush-only bottle. Alternative: a local/custom pantry so we can ship bottles without an upstream pkgx PR.  _(kai, 2026-07-14)_
