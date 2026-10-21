//! Manager update facade plus optional service integration.
pub mod api;
pub mod coordinator;
#[cfg(not(feature = "service-updater"))]
mod disabled;
#[cfg(feature = "service-updater")]
mod service_coordinator;
#[cfg(not(feature = "service-updater"))]
pub use disabled::{bridge, check_state, native_admin, transaction};
#[cfg(feature = "service-updater")]
pub mod bridge;
#[cfg(feature = "service-updater")]
pub mod catalog;
#[cfg(feature = "service-updater")]
pub mod check_state;
#[cfg(feature = "service-updater")]
pub mod handlers;
#[cfg(feature = "service-updater")]
pub mod model;
#[cfg(feature = "service-updater")]
pub mod native_admin;
#[cfg(feature = "service-updater")]
pub mod routing;
#[cfg(feature = "service-updater")]
pub mod transaction;
#[cfg(feature = "service-updater")]
pub mod transport;
#[cfg(feature = "service-updater")]
pub mod trust;
