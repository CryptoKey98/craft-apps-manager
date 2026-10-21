//! Source-build configuration only. Never reads installed-user settings or secrets.
mod export;
mod resolve;
mod schema;
pub use export::View;
pub use resolve::{resolve, ConfigOverrides, ResolvedConfig};
pub use schema::*;
