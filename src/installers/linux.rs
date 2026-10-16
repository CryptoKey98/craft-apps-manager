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
    for identity in crate::app_identity::native(app) {
        if identity.platform == "linux"
            && safe_package(&identity.package)
            && safe_linux_path(&identity.executable)
        {
            if let Some(installed) = detect_package(app, &identity.package)? {
                return Ok(Some(installed));
            }
        }
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
    safe_package(package)
        && (crate::app_identity::native(app).iter().any(|identity| {
            identity.platform == "linux"
                && identity.package == package
                && safe_linux_path(&identity.executable)
        }) || (crate::catalog::names(app)
            .iter()
            .any(|name| name.eq_ignore_ascii_case(package))))
}
fn detect_package(app: &str, package: &str) -> Result<Option<Installed>> {
    detect_package_with_identity(app, package, None)
}
fn detect_package_with_identity(
    app: &str,
    package: &str,
    pending: Option<&crate::app_identity::NativeIdentity>,
) -> Result<Option<Installed>> {
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
    let learned = pending.cloned().or_else(|| {
        crate::app_identity::native(app)
            .into_iter()
            .find(|identity| identity.platform == "linux" && identity.package == package)
    });
    let executable = if let Some(identity) = learned {
        if !safe_linux_path(&identity.executable) {
            return Ok(None);
        }
        let path = PathBuf::from(&identity.executable);
        if !files.lines().any(|line| line == identity.executable) || !linux_executable(&path) {
            return Ok(None);
        }
        path
    } else {
        let Some(path) = files.lines().map(PathBuf::from).find(|p| {
            p.file_name().is_some_and(|n| {
                crate::model::executable_names(app)
                    .iter()
                    .any(|name| n == name.as_str())
            }) && safe_linux_path(&p.to_string_lossy())
                && linux_executable(p)
        }) else {
            return Ok(None);
        };
        path
    };
    let architecture = match architecture.as_str() {
        "amd64" | "x86_64" => "x64",
        "arm64" | "aarch64" => "arm64",
        "i386" | "i686" => "x86",
        _ => bail!("Unsupported installed package architecture"),
    };
    Ok(Some(Installed {
        installation_label: Default::default(),
        package_type: Default::default(),
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
    run_selected_with_job(file, app, job, false)
}
pub fn run_selected_with_job(
    file: &Path,
    app: &str,
    job: Option<&Job>,
    downgrade: bool,
) -> Result<Installed> {
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
    let package = package_name(&file, kind)?;
    let pending = if custom_app(app) {
        let (executable, launch) = package_launch(&file, kind, app, &package, job)?;
        Some(crate::app_identity::NativeIdentity {
            launch: Some(launch),
            family: String::new(),
            platform: "linux".into(),
            package: package.clone(),
            executable,
        })
    } else {
        anyhow::ensure!(
            owned_package(app, &package),
            "Package identity does not match the selected app"
        );
        None
    };
    with_package_identity(app, pending.as_ref(), || {
        if let Some(job) = job {
            job.check()?;
            job.stage("Installing package", None, installation_notice());
            job.log(installation_notice());
        }
        let status = package_status(&file, kind, downgrade, job)?;
        anyhow::ensure!(
            status.success(),
            "Package installation failed or authorization was cancelled ({status})"
        );
        detect_package_with_identity(app, &package, pending.as_ref())?
            .context("Package command completed, but the app executable could not be detected")
    })
}
fn with_package_identity(
    app: &str,
    pending: Option<&crate::app_identity::NativeIdentity>,
    install: impl FnOnce() -> Result<Installed>,
) -> Result<Installed> {
    if let Some(identity) = pending {
        crate::app_identity::check_current_claim(app, identity)?;
    }
    let installed = install()?;
    if let Some(identity) = pending {
        crate::app_identity::remember_current(app, identity.clone())?;
    }
    Ok(installed)
}

fn custom_app(app: &str) -> bool {
    crate::catalog::get(app).is_some_and(|entry| entry.repository.contains('/'))
}
fn linux_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}
pub fn registered_launch_executable(folder: &Path, app: &str) -> Option<PathBuf> {
    crate::app_identity::native(app)
        .into_iter()
        .find_map(|identity| {
            if identity.platform != "linux"
                || !safe_package(&identity.package)
                || !safe_linux_path(&identity.executable)
            {
                return None;
            }
            let path = PathBuf::from(identity.executable);
            (path.parent() == Some(folder) && linux_executable(&path)).then_some(path)
        })
}
fn safe_package(name: &str) -> bool {
    !name.is_empty()
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+.-_".contains(&b))
}
fn safe_linux_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains(['\\', '\0', '\n', '\r'])
        && path
            .split('/')
            .skip(1)
            .all(|part| !part.is_empty() && part != "." && part != "..")
}
fn package_name(file: &Path, kind: PackageKind) -> Result<String> {
    let name = match kind {
        PackageKind::Arch => arch_metadata(file)?.0,
        PackageKind::Debian | PackageKind::Rpm => {
            let output = if kind == PackageKind::Debian {
                Command::new("dpkg-deb")
                    .arg("-f")
                    .arg(file)
                    .arg("Package")
                    .output()?
            } else {
                Command::new("rpm")
                    .args(["-qp", "--queryformat", "%{NAME}"])
                    .arg(file)
                    .output()?
            };
            anyhow::ensure!(output.status.success(), "Could not read package identity");
            String::from_utf8(output.stdout)?.trim().to_string()
        }
    };
    anyhow::ensure!(safe_package(&name), "Invalid package name");
    Ok(name)
}
fn desktop_launch(text: &str) -> Option<(String, crate::app_identity::DesktopLaunch)> {
    let mut desktop = false;
    let mut exec = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            desktop = line == "[Desktop Entry]";
            continue;
        }
        if !desktop {
            continue;
        }
        if line
            .strip_prefix("Type=")
            .is_some_and(|value| value != "Application")
            || line == "Terminal=true"
            || line == "Hidden=true"
        {
            return None;
        }
        if let Some(value) = line.strip_prefix("Exec=") {
            if exec.is_some() {
                return None;
            }
            exec = Some(value);
        }
    }
    let tokens = desktop_tokens(exec?)?;
    let mut cursor = 0;
    let mut launch = crate::app_identity::DesktopLaunch::default();
    if matches!(tokens.first()?.as_str(), "env" | "/usr/bin/env") {
        cursor += 1;
        while let Some(token) = tokens.get(cursor) {
            if token == "--" {
                cursor += 1;
                break;
            }
            let Some((key, value)) = token.split_once('=') else {
                break;
            };
            if launch
                .environment
                .insert(key.into(), value.into())
                .is_some()
            {
                return None;
            }
            cursor += 1;
        }
    }
    let executable = tokens.get(cursor)?.clone();
    let basename = Path::new(&executable).file_name()?.to_str()?;
    if executable.starts_with('-')
        || executable.contains(['%', '\0', '\r', '\n'])
        || matches!(
            basename,
            "env" | "sh" | "bash" | "dash" | "zsh" | "fish" | "csh" | "ksh"
        )
    {
        return None;
    }
    for token in tokens.into_iter().skip(cursor + 1) {
        if matches!(token.as_str(), "%u" | "%U" | "%f" | "%F") {
            continue;
        }
        let decoded = token.replace("%%", "\0");
        if decoded.contains('%') {
            return None;
        }
        launch.arguments.push(decoded.replace('\0', "%"));
    }
    launch.valid().then_some((executable, launch))
}
fn desktop_tokens(exec: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut characters = exec.chars();
    while let Some(c) = characters.next() {
        if c.is_control() && c != '\t' {
            return None;
        }
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' => {
                token.push(characters.next().filter(|c| !c.is_control())?);
                started = true;
            }
            ' ' | '\t' if !quoted => {
                if started {
                    tokens.push(std::mem::take(&mut token));
                    started = false;
                }
            }
            '$' | '`' => return None,
            ';' | '&' | '|' | '<' | '>' | '\'' if !quoted => return None,
            _ => {
                token.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if started {
        tokens.push(token);
    }
    (!tokens.is_empty()).then_some(tokens)
}
fn desktop_launch_candidates(
    files: &[String],
    desktops: &[String],
) -> Vec<(String, crate::app_identity::DesktopLaunch)> {
    let mut result = Vec::new();
    for text in desktops {
        if let Some((exec, launch)) = desktop_launch(text) {
            for file in files {
                if safe_linux_path(file)
                    && (file == &exec
                        || (!exec.contains('/') && file == &format!("/usr/bin/{exec}")))
                {
                    result.push((file.clone(), launch.clone()));
                }
            }
        }
    }
    result.sort();
    result.dedup();
    result
}
#[cfg(test)]
fn desktop_exec(text: &str) -> Option<String> {
    desktop_launch(text).map(|(exe, _)| exe)
}
#[cfg(test)]
fn desktop_candidates(files: &[String], desktops: &[String]) -> Vec<String> {
    desktop_launch_candidates(files, desktops)
        .into_iter()
        .map(|(exe, _)| exe)
        .collect()
}
#[cfg(test)]
fn package_executable(
    file: &Path,
    kind: PackageKind,
    app: &str,
    package: &str,
    job: Option<&Job>,
) -> Result<String> {
    package_launch(file, kind, app, package, job).map(|(exe, _)| exe)
}
fn package_launch(
    file: &Path,
    kind: PackageKind,
    app: &str,
    package: &str,
    job: Option<&Job>,
) -> Result<(String, crate::app_identity::DesktopLaunch)> {
    // Inspect archive members without extracting untrusted paths onto the filesystem.
    let temporary = std::env::temp_dir().join(format!("craft-package-{}", uuid::Uuid::new_v4()));
    let archive = if kind == PackageKind::Arch {
        file.to_path_buf()
    } else {
        let payload = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let status = if kind == PackageKind::Debian {
            Command::new("dpkg-deb")
                .arg("--fsys-tarfile")
                .arg(file)
                .stdout(payload)
                .status()
        } else {
            Command::new("rpm2cpio").arg(file).stdout(payload).status()
        };
        if !status.as_ref().is_ok_and(|status| status.success()) {
            let _ = std::fs::remove_file(&temporary);
            anyhow::bail!(
                "Could not inspect package payload: {}",
                status
                    .map(|s| s.to_string())
                    .unwrap_or_else(|e| e.to_string())
            );
        }
        temporary.clone()
    };
    let result = (|| -> Result<(String, crate::app_identity::DesktopLaunch)> {
        let output = Command::new("bsdtar").arg("-tf").arg(&archive).output()?;
        anyhow::ensure!(output.status.success(), "Could not list package payload");
        let members: Vec<String> = String::from_utf8(output.stdout)?
            .lines()
            .map(str::to_string)
            .collect();
        let files: Vec<String> = members
            .iter()
            .map(|name| {
                format!(
                    "/{}",
                    name.strip_prefix("./")
                        .unwrap_or(name)
                        .trim_start_matches('/')
                )
            })
            .collect();
        let mut desktops = Vec::new();
        for (member, path) in members.iter().zip(&files) {
            if safe_linux_path(path)
                && path.starts_with("/usr/share/applications/")
                && path.ends_with(".desktop")
            {
                let output = Command::new("bsdtar")
                    .arg("-xOf")
                    .arg(&archive)
                    .arg("--")
                    .arg(member)
                    .output()?;
                anyhow::ensure!(
                    output.status.success(),
                    "Could not read package desktop entry"
                );
                desktops.push(String::from_utf8(output.stdout)?);
            }
        }
        let candidates = desktop_launch_candidates(&files, &desktops);
        anyhow::ensure!(
            !candidates.is_empty(),
            "Package has no verifiable desktop executable"
        );
        if let Some(identity) = crate::app_identity::native(app)
            .into_iter()
            .find(|identity| {
                identity.platform == "linux"
                    && identity.package == package
                    && candidates
                        .iter()
                        .any(|(exe, _)| exe == &identity.executable)
            })
        {
            let matching: Vec<_> = candidates
                .iter()
                .filter(|(exe, _)| exe == &identity.executable)
                .collect();
            if matching.len() == 1 {
                return Ok(matching[0].clone());
            }
            if let Some(saved) = identity.launch {
                if let Some(found) = matching.into_iter().find(|(_, launch)| launch == &saved) {
                    return Ok(found.clone());
                }
            }
        }
        if candidates.len() == 1 {
            return Ok(candidates[0].clone());
        }
        let index = job
            .context("Package has multiple desktop executables; choose one in the desktop app")?
            .choose_identity(
                "Choose the app executable",
                candidates
                    .iter()
                    .map(|(exe, launch)| {
                        format!(
                            "{} {} - environment: {}",
                            exe,
                            launch.arguments.join(" "),
                            launch
                                .environment
                                .keys()
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })
                    .collect(),
            )?;
        candidates
            .get(index)
            .cloned()
            .context("Invalid executable selection")
    })();
    if archive == temporary {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
fn installation_notice() -> &'static str {
    #[cfg(target_os = "linux")]
    if crate::batch_admin::active() {
        return "Installing an approved batch package. Cancellation stops remaining packages after this transaction finishes.";
    }
    "Approve the desktop authorization prompt. Package installation must finish before another operation."
}
fn installation_arguments(kind: PackageKind, downgrade: bool) -> Vec<&'static str> {
    match (kind, downgrade) {
        (PackageKind::Debian, true) => vec!["apt-get", "install", "-y", "--allow-downgrades"],
        (PackageKind::Rpm, true) => vec!["dnf", "downgrade", "-y"],
        (PackageKind::Arch, _) => vec!["pacman", "-U", "--noconfirm"],
        _ => vec![manager(kind), "install", "-y"],
    }
}
fn package_status(
    file: &Path,
    kind: PackageKind,
    downgrade: bool,
    job: Option<&Job>,
) -> Result<std::process::ExitStatus> {
    #[cfg(target_os = "linux")]
    if let Some(result) = crate::batch_admin::install_package(file, downgrade, job) {
        use std::os::unix::process::ExitStatusExt;
        return result.map(|code| std::process::ExitStatus::from_raw(code << 8));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = job;
    Ok(Command::new("pkexec")
        .args(installation_arguments(kind, downgrade))
        .arg(file)
        .status()?)
}
/// Called only by the approved root helper, with its private verified package copy.
pub(crate) fn approved_package_command(kind: PackageKind, file: &Path) -> Command {
    let args = installation_arguments(kind, false);
    let mut command = Command::new(format!("/usr/bin/{}", args[0]));
    command
        .args(&args[1..])
        .arg(file)
        .current_dir("/")
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("HOME", "/root")
        .env("LANG", "C.UTF-8")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(std::process::Stdio::null());
    command
}
pub fn uninstall(app: &Installed) -> Result<()> {
    crate::model::valid_app(&app.name)?;
    if !owned_package(&app.name, &app.product_code) {
        bail!("Installed package identity mismatch");
    }
    let kind = package_kind()?;
    let current = detect_package(&app.name, &app.product_code)?
        .context("Installed package identity could not be verified")?;
    if current.path != app.path {
        bail!("Installed executable identity mismatch");
    }
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
    #[cfg(target_os = "linux")]
    struct PackageFixture(PathBuf);
    #[cfg(target_os = "linux")]
    impl PackageFixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("craft-linux-fixture-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn write(&self, path: &str, content: &str) {
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        fn payload(&self) {
            self.write("usr/bin/different-name", "#!/bin/sh\nexit 0\n");
            self.write("usr/bin/second-app", "#!/bin/sh\nexit 0\n");
            self.write(
                "usr/share/applications/app.desktop",
                "[Desktop Entry]\nType=Application\nName=Fixture\nExec=different-name %U\n",
            );
        }
    }
    #[cfg(target_os = "linux")]
    impl Drop for PackageFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn real_deb_metadata_selects_package_owned_desktop_executable() {
        if Command::new("dpkg-deb").arg("--version").output().is_err() {
            eprintln!("DEB fixture skipped: dpkg-deb is unavailable on this distribution");
            return;
        }
        let fixture = PackageFixture::new();
        fixture.payload();
        fixture.write("DEBIAN/control", "Package: unrelated-repo-package\nVersion: 1.0\nArchitecture: all\nMaintainer: Fixture <fixture@example.invalid>\nDescription: Fixture only\n");
        let output = PackageFixture::new();
        let archive = output.0.join("fixture.deb");
        assert!(Command::new("dpkg-deb")
            .arg("--build")
            .arg(&fixture.0)
            .arg(&archive)
            .status()
            .unwrap()
            .success());
        assert_eq!(
            package_name(&archive, PackageKind::Debian).unwrap(),
            "unrelated-repo-package"
        );
        assert_eq!(
            package_executable(
                &archive,
                PackageKind::Debian,
                "photocraft",
                "unrelated-repo-package",
                None
            )
            .unwrap(),
            "/usr/bin/different-name"
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn real_arch_metadata_rejects_unowned_exec_and_reuses_learned_choice() {
        let fixture = PackageFixture::new();
        fixture.payload();
        fixture.write(
            ".PKGINFO",
            "pkgname = unrelated-repo-package\npkgver = 1.0-1\narch = x86_64\n",
        );
        let archive = fixture.0.join("fixture.pkg.tar");
        let build = || {
            assert!(Command::new("bsdtar")
                .arg("-cf")
                .arg(&archive)
                .arg("-C")
                .arg(&fixture.0)
                .args([".PKGINFO", "usr"])
                .status()
                .unwrap()
                .success());
        };
        build();
        assert_eq!(arch_metadata(&archive).unwrap().0, "unrelated-repo-package");
        assert_eq!(
            package_executable(
                &archive,
                PackageKind::Arch,
                "photocraft",
                "unrelated-repo-package",
                None
            )
            .unwrap(),
            "/usr/bin/different-name"
        );
        fixture.write(
            "usr/share/applications/second.desktop",
            "[Desktop Entry]\nType=Application\nExec=second-app\n",
        );
        build();
        assert!(package_executable(
            &archive,
            PackageKind::Arch,
            "photocraft",
            "unrelated-repo-package",
            None
        )
        .is_err());
        crate::app_identity::remember(
            &fixture.0,
            "photocraft",
            crate::app_identity::NativeIdentity {
                launch: None,
                family: String::new(),
                platform: "linux".into(),
                package: "unrelated-repo-package".into(),
                executable: "/usr/bin/second-app".into(),
            },
        )
        .unwrap();
        assert_eq!(
            package_executable(
                &archive,
                PackageKind::Arch,
                "photocraft",
                "unrelated-repo-package",
                None
            )
            .unwrap(),
            "/usr/bin/second-app"
        );
        fixture.write(
            "usr/share/applications/app.desktop",
            "[Desktop Entry]\nType=Application\nExec=/opt/not-owned\n",
        );
        std::fs::remove_file(fixture.0.join("usr/share/applications/second.desktop")).unwrap();
        build();
        assert!(package_executable(
            &archive,
            PackageKind::Arch,
            "photocraft",
            "unrelated-repo-package",
            None
        )
        .is_err());
        crate::app_identity::load(&fixture.0.join("empty"));
    }
    #[test]
    fn failed_package_transaction_preserves_previous_launch_identity() {
        let root =
            std::env::temp_dir().join(format!("craft-package-identity-{}", uuid::Uuid::new_v4()));
        let old = crate::app_identity::NativeIdentity {
            platform: "linux".into(),
            package: "verified-package".into(),
            executable: "/usr/bin/old-app".into(),
            ..Default::default()
        };
        crate::app_identity::remember(&root, "photocraft", old.clone()).unwrap();
        let pending = crate::app_identity::NativeIdentity {
            executable: "/opt/new-app".into(),
            ..old.clone()
        };
        assert!(
            with_package_identity("photocraft", Some(&pending), || Err(anyhow::anyhow!(
                "authorization cancelled"
            )))
            .is_err()
        );
        crate::app_identity::load(&root);
        assert_eq!(crate::app_identity::native("photocraft"), vec![old]);
        crate::app_identity::load(&root.join("empty"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn desktop_launch_keeps_arguments_and_environment_without_shell_evaluation() {
        let text = "[Desktop Entry]\nExec=env APP_MODE=desktop \"APP_LABEL=two words\" /opt/app --safe \"two words\" %% %U";
        let (executable, launch) = desktop_launch(text).unwrap();
        assert_eq!(executable, "/opt/app");
        assert_eq!(launch.arguments, ["--safe", "two words", "%"]);
        assert_eq!(launch.environment["APP_LABEL"], "two words");
        let mut command = Command::new(&executable);
        launch.apply(&mut command);
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["--safe", "two words", "%"]
        );
        assert!(command
            .get_envs()
            .any(|(name, value)| name == "APP_MODE"
                && value == Some(std::ffi::OsStr::new("desktop"))));
        for exec in [
            "env -S 'app --safe'",
            "sh -c app",
            "env 9INVALID=x /opt/app",
            "env BAD=x BAD=y /opt/app",
            "/opt/app %k",
            "/opt/app $HOME",
            "\"unterminated",
        ] {
            assert!(
                desktop_launch(&format!("[Desktop Entry]\nExec={exec}")).is_none(),
                "{exec}"
            );
        }
        let files = vec!["/opt/app".into()];
        let desktops = vec![
            "[Desktop Entry]\nExec=env MODE=one /opt/app".into(),
            "[Desktop Entry]\nExec=env MODE=two /opt/app".into(),
        ];
        assert_eq!(desktop_launch_candidates(&files, &desktops).len(), 2);
        assert!(desktop_launch_candidates(&[], &desktops).is_empty());
    }

    #[test]
    fn desktop_env_launcher_resolves_the_application_instead_of_env() {
        assert_eq!(desktop_exec("[Desktop Entry]\nExec=/usr/bin/env APP_MODE=desktop \"/opt/App Name/app\" --safe %U"), Some("/opt/App Name/app".into()));
        assert!(desktop_exec("[Desktop Entry]\nExec=sh -c \"/opt/app; touch /tmp/no\"").is_none());
    }

    #[test]
    fn desktop_identity_requires_exact_package_owned_command() {
        let files = vec!["/usr/bin/different-name".into(), "/usr/bin/helper".into()];
        assert_eq!(
            desktop_candidates(&files, &["[Desktop Entry]\nExec=different-name %U".into()]),
            vec!["/usr/bin/different-name"]
        );
        assert!(
            desktop_candidates(&files, &["[Desktop Entry]\nExec=/opt/unowned".into()]).is_empty()
        );
        assert!(
            desktop_candidates(&files, &["[Desktop Action Help]\nExec=helper".into()]).is_empty()
        );
        assert_eq!(
            desktop_exec("[Desktop Entry]\nExec=\"/opt/App Name/app\" %F"),
            Some("/opt/App Name/app".into())
        );
        assert!(desktop_exec("[Desktop Entry]\nType=Link\nExec=helper").is_none());
        assert!(desktop_exec("[Desktop Entry]\nExec=helper\nTerminal=true").is_none());
        assert!(!safe_linux_path("/usr/bin/../app"));
        assert!(!safe_package("--remove"));
        assert!(!safe_package("app\nother"));
        assert!(safe_package("different-name+desktop"));
    }
    #[test]
    fn explicit_downgrades_use_native_package_manager_support() {
        assert_eq!(
            installation_arguments(PackageKind::Debian, true),
            ["apt-get", "install", "-y", "--allow-downgrades"]
        );
        assert_eq!(
            installation_arguments(PackageKind::Rpm, true),
            ["dnf", "downgrade", "-y"]
        );
        assert_eq!(
            installation_arguments(PackageKind::Rpm, false),
            ["dnf", "install", "-y"]
        );
        assert_eq!(
            installation_arguments(PackageKind::Arch, true),
            ["pacman", "-U", "--noconfirm"]
        );
    }
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
