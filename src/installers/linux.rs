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
    bail!("Installer mode currently supports Ubuntu/Debian and Fedora/RHEL-family distributions. Use AppImage on this distribution.")
}
pub fn package_kind() -> Result<PackageKind> {
    kind_from_os_release(&std::fs::read_to_string("/etc/os-release")?)
}
pub fn installer_extension() -> Result<&'static str> {
    Ok(match package_kind()? {
        PackageKind::Debian => ".deb",
        PackageKind::Rpm => ".rpm",
    })
}
pub fn installer_label() -> &'static str {
    match package_kind() {
        Ok(PackageKind::Debian) => "Debian",
        Ok(PackageKind::Rpm) => "RPM",
        Err(_) => "Linux",
    }
}
fn manager(kind: PackageKind) -> &'static str {
    match kind {
        PackageKind::Debian => "apt-get",
        PackageKind::Rpm => "dnf",
    }
}
pub fn detect(app: &str) -> Result<Option<Installed>> {
    if !crate::model::APPS.contains(&app) {
        return Ok(None);
    }
    if let Some(installed) = detect_package(app, crate::model::repository(app))? {
        return Ok(Some(installed));
    }
    if crate::model::repository(app) != app {
        return detect_package(app, app);
    }
    Ok(None)
}
fn detect_package(app: &str, package: &str) -> Result<Option<Installed>> {
    let kind = package_kind()?;
    let (version, architecture, list) = match kind {
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
    if file.extension().is_none_or(|s| {
        s != match kind {
            PackageKind::Debian => "deb",
            PackageKind::Rpm => "rpm",
        }
    }) {
        bail!("Package format does not match this distribution");
    }
    let package = match kind {
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
        || ![app, crate::model::repository(app)]
            .contains(&String::from_utf8_lossy(&package.stdout).trim())
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
    if ![app.name.as_str(), crate::model::repository(&app.name)]
        .contains(&app.product_code.as_str())
    {
        bail!("Installed package identity mismatch");
    }
    let kind = package_kind()?;
    let status = Command::new("pkexec")
        .args([manager(kind), "remove", "-y", &app.product_code])
        .status()?;
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
        assert!(kind_from_os_release("ID=arch\n").is_err());
        assert!(kind_from_os_release("ID=notfedora\n").is_err());
    }
}
