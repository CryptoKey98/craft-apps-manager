use crate::{jobs::Job, model::Installed};
use anyhow::{bail, Context, Result};
use std::{
    path::{Path, PathBuf},
    process::Command,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageKind {
    Debian,
    Rpm,
    Arch,
}
pub fn upstream_version(version: &str) -> &str {
    let version = version.split_once(':').map_or(version, |(_, value)| value);
    version.rsplit_once('-').map_or(version, |(value, _)| value)
}
pub fn arch_metadata(file: &Path) -> Result<(String, String, String)> {
    let out = Command::new("bsdtar")
        .args(["-xOf"])
        .arg(file)
        .arg(".PKGINFO")
        .output()?;
    if !out.status.success() {
        bail!("Could not read Arch package metadata");
    }
    parse_arch_metadata(&String::from_utf8(out.stdout)?)
}
fn parse_arch_metadata(text: &str) -> Result<(String, String, String)> {
    let field = |key: &str| -> Result<String> {
        let values: Vec<_> = text
            .lines()
            .filter_map(|line| line.split_once(" = "))
            .filter(|(name, _)| *name == key)
            .map(|(_, value)| value)
            .collect();
        if values.len() != 1 || values[0].is_empty() {
            bail!("Invalid Arch package metadata: {key}");
        }
        Ok(values[0].to_string())
    };
    Ok((field("pkgname")?, field("pkgver")?, field("arch")?))
}
fn install_arch(file: &Path, app: &str, job: Option<&Job>) -> Result<Installed> {
    if let Some(job) = job {
        job.check()?;
        job.stage("Installing package", None, "Approve the administrator prompt. The package transaction must finish before another operation.");
    }
    let status = Command::new("pkexec")
        .args(["/usr/bin/pacman", "-U", "--noconfirm"])
        .arg(file)
        .status()?;
    if !status.success() {
        bail!("Arch package installation failed or authorization was cancelled ({status})");
    }
    detect(app)?.context("Package command completed, but the app executable could not be detected")
}
fn kind_from_os_release(text: &str) -> Result<PackageKind> {
    let mut ids = Vec::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once('=') {
            if matches!(key, "ID" | "ID_LIKE") {
                ids.extend(value.trim_matches(['\"', '\'']).split_whitespace());
            }
        }
    }
    if ids
        .iter()
        .any(|id| ["debian", "ubuntu", "linuxmint"].contains(id))
    {
        return Ok(PackageKind::Debian);
    }
    if ids
        .iter()
        .any(|id| ["fedora", "rhel", "centos", "rocky", "almalinux"].contains(id))
    {
        return Ok(PackageKind::Rpm);
    }
    if ids.contains(&"arch") {
        return Ok(PackageKind::Arch);
    }
    bail!(
        "Installer mode supports Debian, RPM and Arch packages. Use AppImage on this distribution."
    )
}
pub fn package_kind() -> Result<PackageKind> {
    kind_from_os_release(&std::fs::read_to_string("/etc/os-release")?)
}
pub fn installer_extension() -> Result<&'static str> {
    Ok(match package_kind()? {
        PackageKind::Debian => ".deb",
        PackageKind::Rpm => ".rpm",
        PackageKind::Arch => ".pkg.tar.zst",
    })
}
pub fn installer_label() -> &'static str {
    match package_kind() {
        Ok(PackageKind::Debian) => "Debian",
        Ok(PackageKind::Rpm) => "RPM",
        Ok(PackageKind::Arch) => "Arch",
        Err(_) => "Linux",
    }
}
fn manager(kind: PackageKind) -> &'static str {
    match kind {
        PackageKind::Debian => "apt-get",
        PackageKind::Rpm => "dnf",
        PackageKind::Arch => "pacman",
    }
}
pub fn detect(app: &str) -> Result<Option<Installed>> {
    if !crate::model::apps().iter().any(|a| a == app) {
        return Ok(None);
    }
    // Newest package names first, like the repository name the app now uses.
    let mut names: Vec<String> = crate::catalog::names(app)
        .iter()
        .map(|n| n.to_lowercase())
        .collect();
    names.reverse();
    names.dedup();
    for package in &names {
        if let Some(installed) = detect_package(app, package)? {
            return Ok(Some(installed));
        }
    }
    Ok(None)
}
/// Whether a package name belongs to the app under any of its names.
fn owned_package(app: &str, package: &str) -> bool {
    crate::catalog::names(app)
        .iter()
        .any(|name| name.eq_ignore_ascii_case(package))
}
fn detect_package(app: &str, package: &str) -> Result<Option<Installed>> {
    let kind = package_kind()?;
    let (version, architecture, list) = match kind {
        PackageKind::Arch => {
            let out = Command::new("pacman").args(["-Q", package]).output()?;
            if !out.status.success() {
                return Ok(None);
            }
            let text = String::from_utf8(out.stdout)?;
            let version = upstream_version(
                text.split_whitespace()
                    .nth(1)
                    .context("Package has no version")?,
            )
            .to_string();
            let metadata = Command::new("pacman")
                .env("LC_ALL", "C")
                .args(["-Qi", package])
                .output()?;
            if !metadata.status.success() {
                bail!("Could not read installed package metadata");
            }
            let text = String::from_utf8(metadata.stdout)?;
            let arch = text
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(key, _)| key.trim() == "Architecture")
                        .map(|(_, value)| value.trim().to_string())
                })
                .context("Package has no architecture")?;
            (
                version,
                arch,
                Command::new("pacman").args(["-Qql", package]).output()?,
            )
        }
        PackageKind::Debian => {
            let out = Command::new("dpkg-query")
                .args(["-W", "-f=${Status}\n${Version}\n${Architecture}\n", package])
                .output()?;
            if !out.status.success() {
                return Ok(None);
            }
            let text = String::from_utf8(out.stdout)?;
            let mut lines = text.lines();
            if lines.next() != Some("install ok installed") {
                return Ok(None);
            }
            let version = lines
                .next()
                .context("Package has no version")?
                .split('-')
                .next()
                .unwrap_or_default()
                .to_string();
            let arch = lines
                .next()
                .context("Package has no architecture")?
                .to_string();
            (
                version,
                arch,
                Command::new("dpkg-query").args(["-L", package]).output()?,
            )
        }
        PackageKind::Rpm => {
            let out = Command::new("rpm")
                .args(["-q", "--queryformat", "%{VERSION}\n%{ARCH}\n", package])
                .output()?;
            if !out.status.success() {
                return Ok(None);
            }
            let text = String::from_utf8(out.stdout)?;
            let mut lines = text.lines();
            let version = lines.next().context("Package has no version")?.to_string();
            let arch = lines
                .next()
                .context("Package has no architecture")?
                .to_string();
            (
                version,
                arch,
                Command::new("rpm").args(["-ql", package]).output()?,
            )
        }
    };
    if !list.status.success() {
        bail!("Could not list installed package files");
    }
    let files = String::from_utf8(list.stdout)?;
    let Some(executable) = files.lines().map(PathBuf::from).find(|p| {
        p.file_name().is_some_and(|n| {
            crate::model::executable_names(app)
                .iter()
                .any(|name| n == name.as_str())
        }) && p.is_file()
    }) else {
        return Ok(None);
    };
    let architecture = match architecture.as_str() {
        "amd64" | "x86_64" => "x64",
        "arm64" | "aarch64" => "arm64",
        "i386" | "i686" => "x86",
        _ => bail!("Unsupported installed package architecture"),
    };
    Ok(Some(Installed {
        name: app.into(),
        version,
        path: executable
            .parent()
            .context("No executable parent")?
            .display()
            .to_string(),
        architecture: architecture.into(),
        install_kind: "installer".into(),
        product_code: package.into(),
    }))
}
pub fn run(file: &Path, app: &str) -> Result<Installed> {
    run_with_job(file, app, None)
}
pub fn run_with_job(file: &Path, app: &str, job: Option<&Job>) -> Result<Installed> {
    crate::model::valid_app(app)?;
    let kind = package_kind()?;
    let file = file.canonicalize()?;
    if !file.file_name().is_some_and(|name| {
        name.to_string_lossy().ends_with(match kind {
            PackageKind::Debian => ".deb",
            PackageKind::Rpm => ".rpm",
            PackageKind::Arch => ".pkg.tar.zst",
        })
    }) {
        bail!("Package format does not match this distribution");
    }
    let package = match kind {
        PackageKind::Arch => {
            let (name, _, _) = arch_metadata(&file)?;
            if !owned_package(app, &name) {
                bail!("Package identity does not match the selected app");
            }
            return install_arch(&file, app, job);
        }
        PackageKind::Debian => Command::new("dpkg-deb")
            .arg("-f")
            .arg(&file)
            .arg("Package")
            .output()?,
        PackageKind::Rpm => Command::new("rpm")
            .args(["-qp", "--queryformat", "%{NAME}"])
            .arg(&file)
            .output()?,
    };
    if !package.status.success()
        || !owned_package(app, String::from_utf8_lossy(&package.stdout).trim())
    {
        bail!("Package identity does not match the selected app");
    }
    if let Some(job) = job {
        job.check()?;
        job.stage("Installing package",None,"Approve the desktop authorization prompt. Package installation must finish before another operation.");
        job.log(&format!(
            "Approve the desktop authorization prompt to install this {} package",
            installer_label()
        ));
    }
    let status = Command::new("pkexec")
        .args([manager(kind), "install", "-y"])
        .arg(&file)
        .status()?;
    if !status.success() {
        bail!("Package installation failed or authorization was cancelled ({status})");
    }
    detect(app)?.context("Package command completed, but the app executable could not be detected")
}
pub fn uninstall(app: &Installed) -> Result<()> {
    crate::model::valid_app(&app.name)?;
    if !owned_package(&app.name, &app.product_code) {
        bail!("Installed package identity mismatch");
    }
    let kind = package_kind()?;
    let args = if kind == PackageKind::Arch {
        vec![manager(kind), "-R", "--noconfirm", &app.product_code]
    } else {
        vec![manager(kind), "remove", "-y", &app.product_code]
    };
    let status = Command::new("pkexec").args(args).status()?;
    if !status.success() {
        bail!("Package uninstall failed or authorization was cancelled ({status})");
    }
    if detect_package(&app.name, &app.product_code)?.is_some() {
        bail!("Package is still installed");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arch_metadata_requires_unique_identity_fields() {
        let info = "pkgname = pdfcraft\npkgver = 1:0.4.1-2\narch = x86_64\ndepend = glibc\n";
        let (name, version, arch) = parse_arch_metadata(info).unwrap();
        assert_eq!(name, "pdfcraft");
        assert_eq!(upstream_version(&version), "0.4.1");
        assert_eq!(arch, "x86_64");
        assert!(parse_arch_metadata("pkgname = pdfcraft\narch = x86_64").is_err());
        assert!(parse_arch_metadata(&format!("{info}pkgname = other\n")).is_err());
    }
    #[test]
    fn distro_identity_determines_package_backend() {
        assert_eq!(
            kind_from_os_release("ID=fedora\n").unwrap(),
            PackageKind::Rpm
        );
        assert_eq!(
            kind_from_os_release("ID=rocky\nID_LIKE=\"rhel centos fedora\"\n").unwrap(),
            PackageKind::Rpm
        );
        assert_eq!(
            kind_from_os_release("ID=ubuntu\nID_LIKE=debian\n").unwrap(),
            PackageKind::Debian
        );
        assert_eq!(
            kind_from_os_release("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n").unwrap(),
            PackageKind::Debian
        );
        assert_eq!(
            kind_from_os_release("ID=arch\n").unwrap(),
            PackageKind::Arch
        );
        assert_eq!(
            kind_from_os_release("ID=manjaro\nID_LIKE=arch\n").unwrap(),
            PackageKind::Arch
        );
        assert!(kind_from_os_release("ID=notfedora\n").is_err());
    }
}
