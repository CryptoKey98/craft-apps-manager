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

/// Path passed to compilers, which can have stricter limits than file operations.
pub fn build_path(path: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        compiler_path(path)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(path.to_path_buf())
    }
}

pub fn configure_build_paths(
    paths: &crate::model::Paths,
    project: &std::path::Path,
    env: &mut std::collections::BTreeMap<String, String>,
) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        windows::configure_rust_paths(paths, project, env)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (paths, project, env);
        Ok(())
    }
}
