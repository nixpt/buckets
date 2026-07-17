//! buckets — throwaway runtime environments for AI agents.
//!
//! Resolve, fetch, and run any CLI tool in an isolated ephemeral environment,
//! without installing it globally. Borrows its bottle format, distribution
//! protocol, and env-composition approach from [pkgx](https://pkgx.dev) (see
//! the top-level README's "Features borrowed from pkgx" section for the
//! full list) and the resolve→install→compose→exec provisioning shape from
//! exosphere's `exo-hydra` crate — but as a deliberately standalone surface,
//! not a fork of either: sync (not async), no daemon/manifest handoff, just
//! resolve a spec, install to `~/.buckets/`, and run.
//!
//! ## Pipeline
//!
//! [`resolve::resolve`] parses a spec (`node@20`) → [`cellar`] checks the
//! local cache → [`inventory`] picks a version from the remote index if
//! nothing cached → [`install`] downloads and extracts the bottle →
//! [`env::compose_env`] builds the `PATH`/`LD_LIBRARY_PATH`/etc. environment →
//! [`sandbox::sandboxed_command`] wraps the actual exec under `bwrap` (real
//! process/mount isolation, not just an isolated toolchain version) → the
//! CLI (`main.rs`, via [`index::Index`] for alias resolution) execs or
//! prints it.

pub mod cellar;
pub mod config;
pub mod env;
pub mod gui;
pub mod index;
pub mod install;
pub mod inventory;
pub mod project;
pub mod resolve;
pub mod sandbox;
pub mod site;
pub mod types;
pub mod worktree;
pub mod bucketfile;
pub mod net;
pub mod herd;

pub use config::Config;
pub use index::Index;
pub use resolve::resolve;
pub use types::{Package, PackageReq, ResolvedEnvironment, Installation};
