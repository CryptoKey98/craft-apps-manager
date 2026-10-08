use crate::{jobs::Job, model::Paths};
use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};
/// Apps opened from Finder do not inherit the shell PATH, so include Homebrew.
fn search_path() -> Vec<PathBuf> {
    let mut dirs: Vec<_> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
        let extra = PathBuf::from(extra);
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    dirs
}
pub fn find_in(folder: &Path, exe: &str) -> Option<PathBuf> {
    let exe = exe.strip_suffix(".exe").unwrap_or(exe);
    walkdir::WalkDir::new(folder)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .find(|e| e.file_type().is_file() && e.file_name() == exe)
        .map(|e| e.into_path())
}
pub fn system(exe: &str) -> Option<PathBuf> {
    let exe = exe.strip_suffix(".exe").unwrap_or(exe);
    search_path()
        .into_iter()
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
}
pub fn find(paths: &Paths, folder: &str, exe: &str) -> Option<PathBuf> {
    find_in(&paths.tools.join(folder), exe).or_else(|| system(exe))
}
pub fn cargo(paths: &Paths) -> Option<PathBuf> {
    let local = paths.tools.join("cargo/bin/cargo");
    local
        .is_file()
        .then_some(local)
        .or_else(|| system("cargo"))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".cargo/bin/cargo"))
                .filter(|p| p.is_file())
        })
}
pub fn seven(paths: &Paths) -> Option<PathBuf> {
    find(paths, "7zip", "7zz").or_else(|| system("7z"))
}
pub fn visual_cpp() -> Result<Option<PathBuf>> {
    Ok(None)
}
fn command_line_tools() -> bool {
    Command::new("/usr/bin/xcode-select")
        .arg("-p")
        .output()
        .is_ok_and(|o| o.status.success())
}
pub fn environment(paths: &Paths) -> Result<BTreeMap<String, String>> {
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    if paths.tools.join("cargo/bin/cargo").is_file() {
        env.insert(
            "CARGO_HOME".into(),
            paths.tools.join("cargo").display().to_string(),
        );
        env.insert(
            "RUSTUP_HOME".into(),
            paths.tools.join("rustup").display().to_string(),
        );
    }
    let mut bins = Vec::new();
    if let Some(cargo) = cargo(paths) {
        bins.push(cargo.parent().context("Cargo has no parent")?.to_path_buf());
    }
    bins.extend(search_path());
    env.insert(
        "PATH".into(),
        std::env::join_paths(bins)?
            .into_string()
            .map_err(|_| anyhow::anyhow!("PATH is not UTF-8"))?,
    );
    for (key, value) in [
        ("NO_COLOR", "1"),
        ("FORCE_COLOR", "0"),
        ("TERM", "dumb"),
        ("CARGO_TERM_COLOR", "never"),
    ] {
        env.insert(key.into(), value.into());
    }
    Ok(env)
}
pub fn preflight(paths: &Paths, app: &str) -> Result<()> {
    cargo(paths).context("Rust is missing; install Rust with rustup")?;
    if !command_line_tools() {
        bail!("Xcode Command Line Tools are missing. Use Set up build tools or run xcode-select --install.");
    }
    if app == "artcraftx" {
        for command in ["node", "npm", "cmake", "perl", "nasm", "clang", "git"] {
            system(command).with_context(|| format!("ArtCraft X build tool missing: {command}"))?;
        }
    }
    Ok(())
}
pub fn setup(paths: &Paths, app: &str, job: &Job) -> Result<()> {
    crate::model::valid_app(app)?;
    job.check()?;
    if !command_line_tools() {
        let _ = Command::new("/usr/bin/xcode-select")
            .arg("--install")
            .status();
        bail!("Install the Xcode Command Line Tools from the dialog that opened, then run Set up build tools again.");
    }
    let packages = prerequisite_packages(app);
    let missing: Vec<_> = packages
        .iter()
        .filter(|(command, _)| system(command).is_none())
        .map(|(_, package)| *package)
        .collect();
    if !missing.is_empty() {
        let brew = system("brew").context(format!(
            "Install Homebrew from https://brew.sh, or install these tools manually: {}",
            missing.join(", ")
        ))?;
        job.stage("Installing package", None, "Installing Homebrew packages");
        job.log(&format!(
            "Required packages for {}: {}",
            crate::model::title(app),
            missing.join(", ")
        ));
        job.run(
            Command::new(brew)
                .arg("install")
                .args(&missing)
                .env("HOMEBREW_NO_AUTO_UPDATE", "1"),
            false,
        )?;
    }
    job.check()?;
    if cargo(paths).is_none() {
        install_rust(paths, job)?;
    }
    preflight(paths, app)?;
    job.log("Build tools are ready");
    Ok(())
}
/// Command to look for and the Homebrew package that provides it.
fn prerequisite_packages(app: &str) -> Vec<(&'static str, &'static str)> {
    let mut packages = vec![("7zz", "sevenzip")];
    if app == "artcraftx" {
        packages.extend([
            ("node", "node"),
            ("npm", "node"),
            ("cmake", "cmake"),
            ("nasm", "nasm"),
        ]);
    }
    packages.dedup_by_key(|(_, package)| *package);
    packages
}
fn install_rust(paths: &Paths, job: &Job) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let host = match std::env::consts::ARCH {
        "aarch64" => "aarch64-apple-darwin",
        "x86_64" => "x86_64-apple-darwin",
        _ => bail!("Rust setup does not support this Mac architecture"),
    };
    let network = crate::network::Network::new(&paths.root)?;
    let url = format!("https://static.rust-lang.org/rustup/dist/{host}/rustup-init");
    let installer = paths.at("runtime/downloads/rustup-init");
    network.download(&url, &installer, job)?;
    let checksum = network.text(&format!("{url}.sha256"))?;
    let hash = checksum
        .split_whitespace()
        .next()
        .context("Missing rustup checksum")?;
    if hash.len() != 64
        || !hash.bytes().all(|c| c.is_ascii_hexdigit())
        || !crate::files::hash(&installer)?.eq_ignore_ascii_case(hash)
    {
        bail!("Rust installer checksum mismatch");
    }
    std::fs::set_permissions(&installer, std::fs::Permissions::from_mode(0o755))?;
    job.stage(
        "Installing Rust",
        None,
        "Setting up the stable Rust toolchain",
    );
    job.run(
        Command::new(&installer)
            .args(["-y", "--profile", "minimal", "--no-modify-path"])
            .env("CARGO_HOME", paths.tools.join("cargo"))
            .env("RUSTUP_HOME", paths.tools.join("rustup")),
        false,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prerequisite_setup_only_installs_selected_apps_extra_tools() {
        let normal = prerequisite_packages("filmcraft");
        assert!(!normal.iter().any(|(_, p)| *p == "node"));
        let desktop = prerequisite_packages("artcraftx");
        assert_eq!(desktop.iter().filter(|(_, p)| *p == "node").count(), 1);
        assert!(desktop.iter().any(|(_, p)| *p == "nasm"));
    }
}
