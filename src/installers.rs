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
#[path = "installers/linux.rs"]
mod linux_contract_tests;
