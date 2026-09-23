//! Transport-agnostic Costlytics backend: request validation, cost queries,
//! source registration and settings management. Called by the Tauri
//! commands (`crates/app`) and the dev/test HTTP harness (`crates/api`).

pub mod cost;
pub mod error;
pub mod register;
pub mod registry;
pub mod sources;

pub use error::{ErrorKind, ServiceError};
pub use registry::SourceRegistry;
