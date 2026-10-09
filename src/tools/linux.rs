use crate::{jobs::Job, model::Paths};
use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
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
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(exe))
            .find(|p| p.is_file())
    })
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
    if let Some(cargo) = cargo(paths) {
        let mut bins = vec![cargo.parent().context("Cargo has no parent")?.to_path_buf()];
        bins.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        env.insert(
            "PATH".into(),
            std::env::join_paths(bins)?
                .into_string()
                .map_err(|_| anyhow::anyhow!("PATH is not UTF-8"))?,
        );
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
    for command in ["cc", "pkg-config"] {
        system(command).with_context(|| format!("Build tool missing: {command}"))?;
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
    let kind = crate::installers::package_kind()?;
    let manager = match kind {
        crate::installers::PackageKind::Debian => "apt-get",
        crate::installers::PackageKind::Rpm => "dnf",
        crate::installers::PackageKind::Arch => "pacman",
    };
    if system(manager).is_none() || system("pkexec").is_none() {
        bail!("Automatic prerequisite setup requires {manager} and PolicyKit. Install Rust and the development packages listed in docs/linux.md.");
    }
    let seven_package = if kind != crate::installers::PackageKind::Debian
        || std::process::Command::new("apt-cache")
            .args(["show", "7zip"])
            .output()?
            .status
            .success()
    {
        "7zip"
    } else {
        "p7zip-full"
    };
    let packages = prerequisite_packages(kind, app, seven_package);
    job.stage(
        "Installing package",
        None,
        "Authorize installation of Linux development packages",
    );
    job.log(&format!(
        "Required packages for {}: {}",
        crate::model::title(app),
        packages.join(", ")
    ));
    let mut command = std::process::Command::new("pkexec");
    if kind == crate::installers::PackageKind::Arch {
        command.args([manager, "-S", "--needed", "--noconfirm"]);
    } else {
        command.args([manager, "install", "-y"]);
    }
    if kind == crate::installers::PackageKind::Debian {
        command.arg("--no-install-recommends");
    }
    let out = command.args(packages).output()?;
    for line in String::from_utf8_lossy(&out.stdout)
        .lines()
        .chain(String::from_utf8_lossy(&out.stderr).lines())
    {
        job.log(line);
    }
    if !out.status.success() {
        bail!(
            "Development package installation failed or was cancelled. {}",
            out.status
        );
    }
    job.check()?;
    if cargo(paths).is_none() {
        install_rust(paths, job)?;
    }
    preflight(paths, app)?;
    job.log("Build tools are ready");
    Ok(())
}
fn prerequisite_packages<'a>(
    kind: crate::installers::PackageKind,
    app: &str,
    seven: &'a str,
) -> Vec<&'a str> {
    if kind == crate::installers::PackageKind::Arch {
        let mut packages = vec![
            "base-devel",
            "pkgconf",
            "libxkbcommon",
            "wayland",
            "libx11",
            "libxrandr",
            "libxi",
            "libxcursor",
            "mesa",
            "alsa-lib",
            "openssl",
            "systemd",
            "7zip",
            "libarchive",
        ];
        if app == "artcraftx" {
            packages.extend([
                "nodejs",
                "npm",
                "cmake",
                "perl",
                "nasm",
                "clang",
                "git",
                "gtk3",
                "webkit2gtk-4.1",
                "libayatana-appindicator",
                "librsvg",
            ]);
        }
        return packages;
    }
    if kind == crate::installers::PackageKind::Rpm {
        let mut packages = vec![
            "gcc",
            "gcc-c++",
            "pkgconf-pkg-config",
            "libxkbcommon-devel",
            "wayland-devel",
            "libX11-devel",
            "libXrandr-devel",
            "libXi-devel",
            "libXcursor-devel",
            "mesa-libGL-devel",
            "alsa-lib-devel",
            "openssl-devel",
            "systemd-devel",
            "7zip",
        ];
        if app == "artcraftx" {
            packages.extend([
                "/usr/bin/node",
                "/usr/bin/npm",
                "cmake",
                "perl",
                "nasm",
                "clang",
                "clang-devel",
                "git",
                "gtk3-devel",
                "webkit2gtk4.1-devel",
                "libappindicator-gtk3-devel",
                "librsvg2-devel",
            ]);
        }
        return packages;
    }
    let mut packages = vec![
        "build-essential",
        "pkg-config",
        "libxkbcommon-dev",
        "libwayland-dev",
        "libx11-dev",
        "libxrandr-dev",
        "libxi-dev",
        "libgl1-mesa-dev",
        "libasound2-dev",
        "libssl-dev",
        "libudev-dev",
        seven,
    ];
    if app == "artcraftx" {
        packages.extend([
            "nodejs",
            "npm",
            "cmake",
            "perl",
            "nasm",
            "clang",
            "libclang-dev",
            "git",
            "libgtk-3-dev",
            "libwebkit2gtk-4.1-dev",
            "libayatana-appindicator3-dev",
            "librsvg2-dev",
        ]);
    }
    packages
}
fn install_rust(paths: &Paths, job: &Job) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let host = match std::env::consts::ARCH {
        "x86_64" => "x86_64-unknown-linux-gnu",
        "x86" => "i686-unknown-linux-gnu",
        "aarch64" => "aarch64-unknown-linux-gnu",
        _ => bail!("Rust setup does not support this Linux architecture"),
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
        std::process::Command::new(&installer)
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
    use crate::installers::PackageKind;
    #[test]
    fn prerequisite_setup_only_installs_selected_apps_extra_tools() {
        for kind in [PackageKind::Debian, PackageKind::Rpm, PackageKind::Arch] {
            let normal = prerequisite_packages(kind, "filmcraft", "7zip");
            assert!(!normal
                .iter()
                .any(|p| p.contains("node") || p.contains("npm") || p.contains("webkit")));
            let desktop = prerequisite_packages(kind, "artcraftx", "7zip");
            assert!(desktop.iter().any(|p| p.contains("node")));
            assert!(desktop.iter().any(|p| p.contains("npm")));
            assert!(desktop.iter().any(|p| p.contains("webkit")));
            assert!(desktop.contains(&"clang"));
        }
        let fedora = prerequisite_packages(PackageKind::Rpm, "filmcraft", "7zip");
        assert!(fedora.contains(&"gcc"));
        assert!(!fedora.contains(&"build-essential"));
    }
}
