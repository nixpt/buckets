# buckets

Throwaway runtime environments for AI agents — resolve, fetch, and run any CLI tool in an isolated ephemeral environment without installing it globally. Standalone binary + library crate (not a cargo workspace).

## Identity

- **Repository:** buckets (crates.io package: `crush-buckets`)
- **Language:** Rust (edition 2021)
- **Ecosystem:** Core runtime runner for squadron fleet and Exosphere/crush ecosystem.
- **Protocol:** CLI binary (`buckets`) + library API (`crush-buckets`).

**Working this backlog?** Read `.jagent/planning/RULES.md` first — one worktree/branch per ticket/milestone + verify-before-fix.

## Crate Layout

```
buckets/
├── Cargo.toml         # Standalone binary + lib crate
├── README.md          # Multi-subcommand usage documentation
├── CLAUDE.md          # Developer guidelines & build check commands
├── .dejavue/          # Persistent architectural memory
├── .jagent/           # Planning board
└── src/
    ├── main.rs        # CLI command dispatcher & arg parsing
    ├── lib.rs         # Crate docs and top-level exports
    ├── types.rs       # Version, Package, Spec, SandboxProfile, PathSpec types
    ├── index.rs       # Spec parser, alias resolver, index.json parser
    ├── resolve.rs     # Transitive dependency resolution pipeline
    ├── inventory.rs   # Fetch version listings from dist server
    ├── cellar.rs      # Local cache manager (~/.cache/squadron-buckets)
    ├── install.rs     # Bottle download & atomic decompression (tar.xz)
    ├── env.rs         # Composed environment builder (PATH, exports)
    ├── sandbox.rs     # Bubblewrap wrapper for process containment
    ├── project.rs     # Git-clone & local path build detection
    ├── worktree.rs    # Ephemeral git worktree manager (buckets worktree)
    ├── gui.rs         # Sandboxed Xvfb per-session window isolation
    ├── site.rs        # Per-origin sandboxed storage/headless browser isolation
    ├── session.rs     # Persistent session / OverlayFS buckets lifecycle
    └── config.rs      # User settings & cache paths
```
