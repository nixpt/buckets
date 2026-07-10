//! buckets — throwaway runtime environments for AI agents.
//!
//! Resolve, fetch, and run any CLI tool in an isolated ephemeral environment.
//! ```

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
