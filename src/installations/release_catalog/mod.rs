//! Signed public application metadata; never installation authority.
mod api_observation;
mod types;
pub(crate) use api_observation::observe_api;
pub mod verification;
pub use types::*;
pub use verification::{verify_app, verify_index};
#[cfg(test)]
mod tests;

pub mod client;
pub mod provider;
pub mod store;

pub mod inspection;

#[cfg(any(test, feature = "app-catalog-tools"))]
pub mod publisher;
