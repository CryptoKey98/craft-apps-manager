use super::unix_tools;
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
    bins.extend(unix_tools::tool_bins(paths));
    if let Some(cmake) = find(paths, "cmake", "cmake") {
        bins.push(cmake.parent().context("CMake has no parent")?.to_path_buf());
    }
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
    if let Some(libclang) = apple_libclang() {
        env.insert("LIBCLANG_PATH".into(), libclang.display().to_string());
    }
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
        unix_tools::node(paths)
            .context("ArtCraft X requires Node.js 20+; use Set up build tools")?;
        unix_tools::npm(paths).context("ArtCraft X requires npm; use Set up build tools")?;
        find(paths, "cmake", "cmake")
            .context("ArtCraft X requires CMake; use Set up build tools")?;
        apple_libclang().context("Apple libclang is missing; repair Xcode Command Line Tools")?;
        for command in ["perl", "make", "clang", "git"] {
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
    if app == "artcraftx" {
        unix_tools::ensure_node(paths, job)?;
        install_cmake(paths, job)?;
    }
    job.check()?;
    if cargo(paths).is_none() {
        install_rust(paths, job)?;
    }
    preflight(paths, app)?;
    job.log("Build tools are ready");
    Ok(())
}
fn apple_libclang() -> Option<PathBuf> {
    let output = Command::new("/usr/bin/xcrun")
        .args(["--find", "clang"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let clang = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    let library = clang.parent()?.parent()?.join("lib");
    library.join("libclang.dylib").is_file().then_some(library)
}

fn install_cmake(paths: &Paths, job: &Job) -> Result<()> {
    if find(paths, "cmake", "cmake").is_some() {
        return Ok(());
    }
    download_cmake(paths, job)
}

fn download_cmake(paths: &Paths, job: &Job) -> Result<()> {
    let version = "3.31.12";
    let name = format!("cmake-{version}-macos-universal.tar.gz");
    let base = format!("https://github.com/Kitware/CMake/releases/download/v{version}");
    let network = crate::network::Network::new(&paths.root)?;
    let sums = network.text(&format!("{base}/cmake-{version}-SHA-256.txt"))?;
    let hash = sums
        .lines()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            let hash = fields.next()?;
            (fields.next()?.trim_start_matches('*') == name).then_some(hash)
        })
        .context("CMake checksum is missing")?;
    job.stage(
        "Installing CMake",
        None,
        "Downloading verified macOS build tools",
    );
    unix_tools::install_tar_gz(
        paths,
        &network,
        job,
        "cmake",
        &format!("{base}/{name}"),
        hash,
    )
}

#[cfg(test)]
#[test]
#[ignore = "Downloads and executes the official universal CMake package in an isolated directory"]
fn verified_universal_cmake_runs_without_homebrew() {
    let root = std::env::temp_dir().join(format!("craft-cmake-smoke-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let paths = Paths::new(root.clone(), None);
    let job = Job::new(root.join("test.log"), &Default::default());
    let result = (|| -> Result<()> {
        download_cmake(&paths, &job)?;
        let cmake =
            find_in(&paths.tools.join("cmake"), "cmake").context("Downloaded CMake is missing")?;
        anyhow::ensure!(
            Command::new(cmake).arg("--version").status()?.success(),
            "Downloaded CMake did not start"
        );
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(root);
    result.unwrap();
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
