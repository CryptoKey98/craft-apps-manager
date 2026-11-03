//! App registry, discovery, metadata and release records.
pub mod discovery;
pub mod metadata;
mod registry;
mod supported;
pub mod types;
pub use registry::*;
pub use supported::repository_slug;
pub(crate) use supported::{ExternalApp, EXTERNAL_APPS};
