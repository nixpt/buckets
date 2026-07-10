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
//! the CLI (`main.rs`, via [`index::Index`] for alias resolution) execs or
//! prints it.

pub mod cellar;
pub mod config;
pub mod env;
pub mod index;
pub mod install;
pub mod inventory;
pub mod resolve;
pub mod types;

pub use config::Config;
pub use index::Index;
pub use resolve::resolve;
pub use types::{Package, PackageReq, ResolvedEnvironment, Installation};
