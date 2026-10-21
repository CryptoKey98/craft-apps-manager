use super::*;
use crate::installers::{package_kind, PackageKind};

pub(super) fn asset_name(version: &str, arch: &str, kind: PackageKind) -> Result<String> {
    Ok(match (kind, arch) {
        (PackageKind::Debian, "x64") => format!("craft-apps-manager_{version}_amd64.deb"),
        (PackageKind::Debian, "x86") => format!("craft-apps-manager_{version}_i386.deb"),
        (PackageKind::Debian, "arm64") => format!("craft-apps-manager_{version}_arm64.deb"),
        (PackageKind::Rpm, "x64") => format!("craft-apps-manager-{version}-0.2.x86_64.rpm"),
        (PackageKind::Rpm, "x86") => format!("craft-apps-manager-{version}-0.2.i686.rpm"),
        (PackageKind::Rpm, "arm64") => format!("craft-apps-manager-{version}-0.2.aarch64.rpm"),
        (PackageKind::Arch, "x64") => format!("craft-apps-manager-{version}-1-x86_64.pkg.tar.zst"),
        _ => bail!("Unsupported manager package architecture"),
    })
}

fn validate_metadata(output: &str, version: &str, arch: &str, kind: PackageKind) -> Result<()> {
    let expected = match (kind, arch) {
        (PackageKind::Debian, "x64") => "amd64",
        (PackageKind::Debian, "x86") => "i386",
        (PackageKind::Debian, "arm64") => "arm64",
        (PackageKind::Rpm, "x64") => "x86_64",
        (PackageKind::Rpm, "x86") => "i686",
        (PackageKind::Rpm, "arm64") => "aarch64",
        (PackageKind::Arch, "x64") => "x86_64",
        _ => bail!("Unsupported manager architecture"),
    };
    if output.split_whitespace().collect::<Vec<_>>() != ["craft-apps-manager", version, expected] {
        bail!("Downloaded manager package has the wrong identity, version, or architecture");
    }
    Ok(())
}

fn metadata(file: &Path, kind: PackageKind, full: bool) -> Result<String> {
    if kind == PackageKind::Arch {
        let (name, version, arch) = crate::installers::arch_metadata(file)?;
        return Ok(format!(
            "{name} {} {arch}",
            if full {
                version.as_str()
            } else {
                crate::installers::upstream_version(&version)
            }
        ));
    }
    let output = match kind {
        PackageKind::Debian => Command::new("dpkg-deb")
            .args(["-f"])
            .arg(file)
            .args(["Package", "Version", "Architecture"])
            .output()?,
        PackageKind::Rpm => Command::new("rpm")
            .args([
                "-qp",
                "--queryformat",
                if full {
                    "%{NAME} %{VERSION}-%{RELEASE} %{ARCH}"
                } else {
                    "%{NAME} %{VERSION} %{ARCH}"
                },
            ])
            .arg(file)
            .output()?,
        PackageKind::Arch => unreachable!("Arch metadata handled above"),
    };
    if !output.status.success() {
        bail!("Could not read manager package metadata");
    }
    let text = String::from_utf8(output.stdout)?;
    Ok(if kind == PackageKind::Debian {
        text.lines()
            .map(|line| line.split_once(": ").map_or(line, |(_, v)| v))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        text
    })
}

fn create_stage_root(paths: &Paths) -> Result<PathBuf> {
    fs::create_dir_all(&paths.root)?;
    for directory in [
        paths.root.clone(),
        paths.at("runtime"),
        paths.at("runtime/self-update"),
    ] {
        if !directory.exists() {
            fs::create_dir(&directory)?;
        }
        if files::linked(&directory)? {
            bail!("Manager update directory is linked");
        }
    }
    Ok(paths.at("runtime/self-update"))
}

fn install_command(kind: PackageKind) -> &'static [&'static str] {
    match kind {
        // Compatible Debian downloads retain the release version even when
        // their executable advances to a newer signed build revision.
        PackageKind::Debian => &["/usr/bin/apt-get", "install", "--reinstall", "-y"],
        PackageKind::Rpm => &["/usr/bin/dnf", "install", "-y"],
        PackageKind::Arch => &["/usr/bin/pacman", "-U", "--noconfirm"],
    }
}
#[cfg(feature = "service-updater")]
pub(super) fn validate_record(
    file: &Path,
    p: &crate::manager_updates::model::PackageRecord,
) -> Result<()> {
    let kind = package_kind()?;
    ensure_native_kind(kind, &p.installation)?;
    validate_metadata(
        &metadata(file, kind, true)?,
        p.native_version
            .as_deref()
            .context("Missing signed native version")?,
        &p.architecture,
        kind,
    )
}
#[cfg(feature = "service-updater")]
fn ensure_native_kind(
    kind: PackageKind,
    installation: &crate::manager_updates::model::InstallationKind,
) -> Result<()> {
    use crate::manager_updates::model::InstallationKind;
    if !matches!(
        (kind, installation),
        (PackageKind::Debian, InstallationKind::Deb)
            | (PackageKind::Rpm, InstallationKind::Rpm)
            | (PackageKind::Arch, InstallationKind::Arch)
    ) {
        bail!("Native package manager changed");
    }
    Ok(())
}
#[cfg(feature = "service-updater")]
pub(super) fn verify_registered(
    target: &Path,
    p: &crate::manager_updates::model::PackageRecord,
) -> Result<()> {
    let kind = package_kind()?;
    ensure_native_kind(kind, &p.installation)?;
    let output = match kind {
        PackageKind::Debian => Command::new("/usr/bin/dpkg-query")
            .args(["-W", "-f=${Version}", "craft-apps-manager"])
            .output()?,
        PackageKind::Rpm => Command::new("/usr/bin/rpm")
            .args([
                "-q",
                "--queryformat",
                "%{VERSION}-%{RELEASE}",
                "craft-apps-manager",
            ])
            .output()?,
        PackageKind::Arch => Command::new("/usr/bin/pacman")
            .args(["-Q", "craft-apps-manager"])
            .output()?,
    };
    let text = String::from_utf8(output.stdout)?;
    let value = if kind == PackageKind::Arch {
        text.split_whitespace().nth(1).unwrap_or_default()
    } else {
        text.trim()
    };
    if !output.status.success()
        || Some(value) != p.native_version.as_deref()
        || !installed_with_linux_package_at(target)
    {
        bail!("Native manager registration/version could not be verified");
    }
    Ok(())
}

pub(super) fn prepare(paths: &Paths, available: &Available, job: &Job) -> Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let kind = package_kind()?;
    let expected = available.asset.name.clone();
    crate::files::safe_relative(&expected)?;
    if available.package_version.is_none()
        && available.asset.name != asset_name(&available.version, crate::model::MANAGER_ARCH, kind)?
    {
        bail!("Update package does not match this installation type");
    }
    let target = std::env::current_exe()?.canonicalize()?;
    let stage_root = create_stage_root(paths)?;
    let stage = stage_root.join(uuid::Uuid::new_v4().simple().to_string());
    fs::DirBuilder::new().mode(0o700).create(&stage)?;
    // Keep failures and their logs for diagnosis; the installed package is untouched until approval.
    let package = stage.join(&expected);
    #[cfg(feature = "service-updater")]
    if let Some(selection) = &available.selection {
        let selected = selection
            .catalog
            .catalog()
            .packages
            .iter()
            .find(|p| p.id == selection.offer.immediate_package_id)
            .context("Missing signed native package")?;
        if !crate::manager_updates::native_admin::stage(paths, selected, &package)? {
            crate::manager_updates::transport::download_asset(selected, &package, job)?;
        }
    } else {
        Network::new(&paths.root)?.asset(&available.asset, &package, job)?;
    }
    #[cfg(not(feature = "service-updater"))]
    Network::new(&paths.root)?.asset(&available.asset, &package, job)?;
    validate_metadata(
        &metadata(&package, kind, available.package_version.is_some())?,
        available
            .package_version
            .as_deref()
            .unwrap_or(&available.version),
        crate::model::MANAGER_ARCH,
        kind,
    )?;
    let hash = files::hash(&package)?;
    fs::copy(&target, stage.join(helper_name()))?;
    job.check()?;
    job.stage("Installing package", None, "Approve the administrator prompt. The package manager must finish before the manager restarts.");
    let log = fs::File::create(stage.join("install.log"))?;
    let args = install_command(kind);
    #[cfg(feature = "service-updater")]
    let _authorization = available
        .selection
        .as_ref()
        .map(|selection| {
            let trust = crate::manager_updates::transaction::production_trust()?;
            let profile = crate::manager_updates::transaction::installed_profile(paths)?;
            crate::manager_updates::transaction::authorize_commit(
                &paths.root,
                selection,
                &profile,
                &trust,
                chrono::Utc::now().timestamp(),
            )
        })
        .transpose()?;
    job.check()?;
    #[cfg(feature = "service-updater")]
    let batch = available
        .selection
        .as_ref()
        .map(|selection| {
            crate::manager_updates::native_admin::install(
                paths,
                selection,
                &crate::manager_updates::transaction::installed_profile(paths)?,
                Some(job),
            )
        })
        .transpose()?
        .flatten();
    #[cfg(not(feature = "service-updater"))]
    let batch: Option<(i32, String)> = None;
    let code = if let Some((code, text)) = batch {
        fs::write(stage.join("install.log"), text)?;
        code
    } else {
        let status = Command::new("pkexec")
            .args(args)
            .arg(&package)
            .stdout(log.try_clone()?)
            .stderr(log)
            .status()?;
        status.code().unwrap_or(-1)
    };
    if code != 0 {
        bail!("Manager package update failed or administrator approval was cancelled ({code}). See {}", stage.join("install.log").display());
    }
    let output = match kind {
        PackageKind::Debian => Command::new("dpkg-query")
            .args(["-W", "-f=${Version}", "craft-apps-manager"])
            .output()?,
        PackageKind::Rpm => Command::new("rpm")
            .args([
                "-q",
                "--queryformat",
                if available.package_version.is_some() {
                    "%{VERSION}-%{RELEASE}"
                } else {
                    "%{VERSION}"
                },
                "craft-apps-manager",
            ])
            .output()?,
        PackageKind::Arch => Command::new("pacman")
            .args(["-Q", "craft-apps-manager"])
            .output()?,
    };
    let installed_text = String::from_utf8_lossy(&output.stdout);
    let installed_version = if kind == PackageKind::Arch && available.package_version.is_some() {
        installed_text.split_whitespace().nth(1).unwrap_or_default()
    } else if kind == PackageKind::Arch {
        crate::installers::upstream_version(
            installed_text.split_whitespace().nth(1).unwrap_or_default(),
        )
    } else {
        installed_text.trim()
    };
    if !output.status.success()
        || installed_version
            != available
                .package_version
                .as_deref()
                .unwrap_or(&available.version)
        || !installed_with_linux_package_at(&target)
    {
        bail!("Package installation completed, but the new manager version could not be verified. See {}", stage.join("install.log").display());
    }
    #[cfg(feature="service-updater")]
    let expected_identity=available.selection.as_ref().map(|selection| {
        let selected=selection.catalog.catalog().packages.iter().find(|p|p.id==selection.offer.immediate_package_id).context("Missing signed package identity")?;
        let expected=crate::manager_updates::transaction::expected(&selection.catalog,selected);
        crate::manager_updates::transaction::verify_executable(&target,&expected,&stage)
            .context("Package installation completed, but the installed manager identity could not be verified")?;
        Ok::<_,anyhow::Error>(expected)
    }).transpose()?;
    #[cfg(feature = "service-updater")]
    if let Some(selection) = &available.selection {
        let package = selection
            .catalog
            .catalog()
            .packages
            .iter()
            .find(|p| p.id == selection.offer.immediate_package_id)
            .context("Missing native package")?;
        crate::manager_updates::native_admin::acknowledge(&paths.root, package)?;
    }
    #[cfg(not(feature = "service-updater"))]
    let expected_identity = None;
    let plan = stage.join("plan.json");
    files::write_json(
        &plan,
        &Plan {
            expected_identity,
            layout: 0,
            msi: false,
            linux_package: true,
            parent_pid: std::process::id(),
            target,
            staged: package,
            hash,
            root: paths.root.clone(),
            tools: paths.tools.clone(),
        },
    )?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debian_revisions_reinstall_while_other_package_managers_keep_upgrade_commands() {
        assert_eq!(
            install_command(PackageKind::Debian),
            ["/usr/bin/apt-get", "install", "--reinstall", "-y"]
        );
        assert_eq!(
            install_command(PackageKind::Rpm),
            ["/usr/bin/dnf", "install", "-y"]
        );
        assert_eq!(
            install_command(PackageKind::Arch),
            ["/usr/bin/pacman", "-U", "--noconfirm"]
        );
    }
    #[test]
    fn staging_works_in_a_fresh_library_and_rejects_links() {
        let root =
            std::env::temp_dir().join(format!("craft-native-stage-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let stage = create_stage_root(&paths).unwrap();
        assert!(stage.is_dir());
        fs::remove_dir(&stage).unwrap();
        let elsewhere = root.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &stage).unwrap();
        assert!(create_stage_root(&paths).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn package_selection_and_validation_preserve_architecture() {
        for (kind, arch, metadata) in [
            (PackageKind::Debian, "x64", "amd64"),
            (PackageKind::Debian, "x86", "i386"),
            (PackageKind::Rpm, "x64", "x86_64"),
            (PackageKind::Rpm, "x86", "i686"),
            (PackageKind::Arch, "x64", "x86_64"),
        ] {
            let name = asset_name("0.4.2", arch, kind).unwrap();
            assert!(name.contains(metadata));
            validate_metadata(
                &format!("craft-apps-manager 0.4.2 {metadata}"),
                "0.4.2",
                arch,
                kind,
            )
            .unwrap();
            for wrong in [
                format!("other 0.4.2 {metadata}"),
                format!("craft-apps-manager 0.4.0 {metadata}"),
                "craft-apps-manager 0.4.2 wrong".into(),
            ] {
                assert!(validate_metadata(&wrong, "0.4.2", arch, kind).is_err());
            }
        }
    }
}
