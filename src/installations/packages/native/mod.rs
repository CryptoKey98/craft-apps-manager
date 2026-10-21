#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

// Exercise Linux package command contracts on Windows CI as well.
#[cfg(all(test, target_os = "windows"))]
#[allow(dead_code)]
#[path = "linux.rs"]
mod linux_contract_tests;

// Exercise bundle identity and rollback contracts without executing macOS tools.
#[cfg(all(test, not(target_os = "macos")))]
#[allow(dead_code)]
#[path = "macos.rs"]
mod macos_contract_tests;
