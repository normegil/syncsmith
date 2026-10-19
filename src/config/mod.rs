//! Configuration model, environment expansion, and file I/O.

pub mod discovery;
pub mod env;
pub mod io;
pub mod model;
pub mod path;

pub use model::{Config, Mode, Policy, Source, SyncEntry};

/// Name of the configuration file kept at the target repository root.
pub const CONFIG_FILE_NAME: &str = ".syncsmith.toml";
