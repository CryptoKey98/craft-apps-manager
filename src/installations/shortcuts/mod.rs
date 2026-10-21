use super::portable::{validate, Ownership, MARKER};
use crate::{files, model::Paths};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub fn desktop_shortcut(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    validate(paths, app, target)?;
    let marker = target.join(MARKER);
    let owner: Ownership = files::read_json(&marker)?;
    create_installation_shortcut(paths, &marker, owner, target, &desktop()?, "Portable")
}

pub fn refresh_moved_shortcut(
    paths: &Paths,
    app: &str,
    target: &Path,
    installer: bool,
) -> Result<()> {
    let marker = if installer {
        paths.at(format!("runtime/desktop-shortcuts/{app}.json"))
    } else {
        target.join(MARKER)
    };
    if !marker.exists() {
        return Ok(());
    }
    let owner: Ownership = files::read_json(&marker)?;
    if owner.shortcut.is_some() {
        create_installation_shortcut(
            paths,
            &marker,
            owner,
            target,
            &desktop()?,
            if installer { "Installer" } else { "Portable" },
        )?;
    }
    Ok(())
}

pub fn installer_desktop_shortcut(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    crate::model::valid_app(app)?;
    let marker = paths.at(format!("runtime/desktop-shortcuts/{app}.json"));
    let owner: Ownership = if marker.exists() {
        files::read_json(&marker)?
    } else {
        Ownership {
            app: app.into(),
            library: std::path::absolute(&paths.root)?,
            shortcut: None,
        }
    };
    anyhow::ensure!(
        owner.app == app
            && crate::portable_mode::same_library(
                &owner.library,
                &std::path::absolute(&paths.root)?
            ),
        "Shortcut record belongs to another app library"
    );
    create_installation_shortcut(paths, &marker, owner, target, &desktop()?, "Installer")
}

/// Upgrade only unchanged, manager-owned portable desktop shortcuts.
pub fn migrate_installation_labels(paths: &Paths) -> Result<()> {
    let _lock = crate::platform::Lock::wait(r"Local\CraftAppsManager")?;
    let Some(config) = crate::installation_labels::migrate(paths)? else {
        return Ok(());
    };
    migrate_shortcut_records(&config, |copy| migrate_installation_shortcut(paths, copy))
}

pub(super) fn migrate_shortcut_records(
    config: &crate::model::Config,
    mut migrate: impl FnMut(&crate::model::Installed) -> Result<()>,
) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    let mut errors = Vec::new();
    for copy in config.installations.iter().chain(&config.apps) {
        if copy.install_kind == "installer"
            || copy.path.is_empty()
            || !seen.insert((
                copy.name.clone(),
                crate::installation_labels::path_identity(&copy.path),
            ))
        {
            continue;
        }
        if let Err(error) = migrate(copy) {
            errors.push(format!("{} ({}): {error:#}", copy.name, copy.path));
        }
    }
    anyhow::ensure!(
        errors.is_empty(),
        "Some installation shortcuts could not be migrated:\n{}",
        errors.join("\n")
    );
    Ok(())
}

pub(super) fn migrate_installation_shortcut(
    paths: &Paths,
    copy: &crate::model::Installed,
) -> Result<()> {
    let target = Path::new(&copy.path);
    if !target.exists() {
        return Ok(());
    }
    let marker = target.join(MARKER);
    if !marker.exists() {
        return Ok(());
    }
    validate(paths, &copy.name, target)?;
    let owner: Ownership = files::read_json(&marker)?;
    let Some((shortcut, proof)) = &owner.shortcut else {
        return Ok(());
    };
    let desktop = desktop()?;
    let wanted = installation_shortcut_title(
        &copy.name,
        target,
        "Portable",
        &copy.architecture,
        &copy.installation_label,
    )?;
    if shortcut.file_stem().and_then(|name| name.to_str()) == Some(&wanted)
        || owned_installation_title(&copy.name, shortcut, &desktop).is_none()
        || !shortcut_proof(shortcut).is_ok_and(|actual| actual == *proof)
    {
        return Ok(());
    }
    create_installation_shortcut(paths, &marker, owner, target, &desktop, "Portable")?;
    Ok(())
}

pub(super) fn installation_shortcut_title(
    app: &str,
    target: &Path,
    kind: &str,
    architecture: &str,
    label: &str,
) -> Result<String> {
    let _ = target;
    let architecture = match architecture {
        "x64" | "x86" | "arm64" => format!(" {architecture}"),
        _ => String::new(),
    };
    if kind == "Installer" {
        let title = crate::model::title(app);
        return Ok(if architecture.is_empty() {
            title
        } else {
            format!("{title} ({})", architecture.trim())
        });
    }
    anyhow::ensure!(
        crate::installation_labels::valid_label(label),
        "Portable installation label is missing"
    );
    Ok(format!(
        "{} ({kind}{architecture} · {})",
        crate::model::title(app),
        label
    ))
}

pub(super) fn create_installation_shortcut(
    paths: &Paths,
    marker: &Path,
    owner: Ownership,
    target: &Path,
    desktop: &Path,
    kind: &str,
) -> Result<()> {
    let mut config: Option<crate::model::Config> = if paths.at("settings.json").exists() {
        Some(files::read_json(&paths.at("settings.json"))?)
    } else {
        None
    };
    if let Some(config) = &mut config {
        crate::installation_labels::assign(config)?;
    }
    let target_identity = crate::installation_labels::path_identity(&target.to_string_lossy());
    let copy = config
        .iter()
        .flat_map(|config| config.apps.iter().chain(&config.installations))
        .find(|copy| {
            copy.name == owner.app
                && crate::installation_labels::path_identity(&copy.path) == target_identity
        });
    let architecture = copy
        .map(|copy| copy.architecture.as_str())
        .unwrap_or_default();
    let label = copy
        .map(|copy| copy.installation_label.as_str())
        .unwrap_or_default();
    // Keep the identity across relocation; legacy names are upgraded on creation.
    let previous = owner.shortcut.clone();
    let title = installation_shortcut_title(&owner.app, target, kind, architecture, label)?;
    let desired = desktop.join(format!("{title}.{}", crate::platform::shortcut_extension()));
    anyhow::ensure!(
        !previous
            .as_ref()
            .is_some_and(|(old, _)| *old != desired && desired.exists()),
        "An existing shortcut with the new name was left unchanged"
    );
    let app = owner.app.clone();
    create_named_shortcut(marker, owner, target, desktop, &title)?;
    if let Some((old, proof)) = previous {
        let new = desktop.join(format!("{title}.{}", crate::platform::shortcut_extension()));
        if old != new
            && owned_installation_title(&app, &old, desktop).is_some()
            && shortcut_proof(&old).is_ok_and(|actual| actual == proof)
        {
            std::fs::remove_file(old)?;
        }
    }
    Ok(())
}

pub(super) fn owned_installation_title(app: &str, path: &Path, desktop: &Path) -> Option<String> {
    if path.parent() != Some(desktop)
        || path.extension()?.to_str()? != crate::platform::shortcut_extension()
    {
        return None;
    }
    let title = path.file_stem()?.to_str()?;
    let base = crate::model::title(app);
    if title == base {
        return Some(title.into());
    }
    let suffix = title
        .strip_prefix(&format!("{base} ("))?
        .strip_suffix(')')?;
    if matches!(
        suffix,
        "x64"
            | "x86"
            | "arm64"
            | "Installer"
            | "Installer x64"
            | "Installer x86"
            | "Installer arm64"
    ) {
        return Some(title.into());
    }
    let (label, digest) = suffix.rsplit_once(" · ")?;
    if !matches!(
        label,
        "Portable"
            | "Portable x64"
            | "Portable x86"
            | "Portable arm64"
            | "Installer"
            | "Installer x64"
            | "Installer x86"
            | "Installer arm64"
    ) || !(crate::installation_labels::valid_label(digest)
        || digest.len() == 8 && digest.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return None;
    }
    Some(title.into())
}

#[derive(Serialize, Deserialize)]
struct BuildShortcutOwnership {
    #[serde(flatten)]
    owner: Ownership,
    #[serde(default)]
    build_folder: Option<PathBuf>,
    #[serde(default)]
    shortcut_title: Option<String>,
}

pub(super) fn build_shortcut_identity(
    paths: &Paths,
    app: &str,
    target: &Path,
) -> Result<(PathBuf, String)> {
    use sha2::{Digest, Sha256};
    crate::model::valid_app(app)?;
    let target = std::path::absolute(target)?;
    let digest = format!(
        "{:x}",
        Sha256::digest(crate::portable_mode::identity(&target)?.as_bytes())
    );
    let info: crate::model::BuildInfo = files::read_json(&target.join("build-info.json"))?;
    anyhow::ensure!(info.app == app, "The build belongs to another app");
    let date = chrono::DateTime::parse_from_rfc3339(&info.built_at)?
        .with_timezone(&chrono::Local)
        .format("%b %d %Y %H-%M-%S")
        .to_string();
    let commit: String = info
        .commit
        .chars()
        .take(7)
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    let title = format!(
        "{} Build ({date} · {commit} · {})",
        crate::model::title(app),
        &digest[..8]
    );
    Ok((
        paths.at(format!("runtime/build-shortcuts/{app}/{digest}.json")),
        title,
    ))
}

pub fn build_desktop_shortcut(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    build_desktop_shortcut_at(paths, app, target, &desktop()?)
}

pub(super) fn build_desktop_shortcut_at(
    paths: &Paths,
    app: &str,
    target: &Path,
    desktop: &Path,
) -> Result<()> {
    let (marker, title) = build_shortcut_identity(paths, app, target)?;
    let target = std::path::absolute(target)?;
    let owner = if marker.exists() {
        files::read_json(&marker)?
    } else {
        Ownership {
            app: app.into(),
            library: std::path::absolute(&paths.root)?,
            shortcut: None,
        }
    };
    anyhow::ensure!(
        owner.app == app
            && crate::portable_mode::same_library(
                &owner.library,
                &std::path::absolute(&paths.root)?
            ),
        "Build shortcut belongs to another app library"
    );
    create_named_shortcut(&marker, owner, &target, desktop, &title)?;
    let owner = files::read_json(&marker)?;
    files::write_json(
        &marker,
        &BuildShortcutOwnership {
            owner,
            build_folder: Some(target),
            shortcut_title: Some(title),
        },
    )
}

pub fn remove_build_shortcut(paths: &Paths, app: &str, folder: &Path) -> Result<()> {
    // Keep legacy ownership records supported alongside separate records for each build.
    let legacy = paths.at(format!("runtime/build-shortcuts/{app}.json"));
    let directory = paths.at(format!("runtime/build-shortcuts/{app}"));
    if !legacy.exists() && !directory.exists() {
        return Ok(());
    }
    remove_build_shortcut_at(paths, app, folder, &desktop()?)
}

pub(super) fn remove_build_shortcut_at(
    paths: &Paths,
    app: &str,
    folder: &Path,
    desktop: &Path,
) -> Result<()> {
    use sha2::{Digest, Sha256};
    crate::model::valid_app(app)?;
    let folder = std::path::absolute(folder)?;
    let digest = format!(
        "{:x}",
        Sha256::digest(crate::portable_mode::identity(&folder)?.as_bytes())
    );
    let mut markers = vec![
        paths.at(format!("runtime/build-shortcuts/{app}/{digest}.json")),
        paths.at(format!("runtime/build-shortcuts/{app}.json")),
    ];
    // Older builds used an absolute-path digest. Locate their verified ownership
    // records too, including after the portable library has moved.
    let directory = paths.at(format!("runtime/build-shortcuts/{app}"));
    if directory.is_dir() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            if entry.file_type()?.is_file() && entry.path().extension().is_some_and(|e| e == "json")
            {
                let marker = entry.path();
                if !markers.contains(&marker) {
                    markers.push(marker);
                }
            }
        }
    }
    for marker in markers {
        if !marker.exists() {
            continue;
        }
        let record: BuildShortcutOwnership = files::read_json(&marker)?;
        anyhow::ensure!(
            record.owner.app == app
                && crate::portable_mode::same_library(
                    &record.owner.library,
                    &std::path::absolute(&paths.root)?
                ),
            "Build shortcut belongs to another app library"
        );
        let belongs =
            match &record.build_folder {
                Some(owned) => owned == &folder,
                None => record.owner.shortcut.as_ref().is_some_and(|(shortcut, _)| {
                    legacy_build_shortcut_matches(shortcut, &folder, app)
                }),
            };
        if belongs {
            let title = record
                .shortcut_title
                .unwrap_or_else(|| format!("{} Build", crate::model::title(app)));
            anyhow::ensure!(
                Path::new(&title).components().count() == 1 && !title.contains(['/', '\\']),
                "Invalid build shortcut name"
            );
            remove_owned_named_shortcut(record.owner, desktop, &title)?;
            std::fs::remove_file(marker)?;
        }
    }
    Ok(())
}

pub(super) fn legacy_build_shortcut_matches(shortcut: &Path, folder: &Path, app: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        crate::platform::shortcut_target(shortcut).is_some_and(|target| {
            same_shortcut_target(
                &target,
                &folder.join(crate::model::build_executable_name(app)),
            )
        })
    }
    #[cfg(target_os = "macos")]
    {
        std::fs::read_link(shortcut)
            .is_ok_and(|target| target == folder.join(crate::model::build_executable_name(app)))
    }
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        std::fs::read_to_string(shortcut).is_ok_and(|contents| {
            contents.lines().any(|line| {
                line.strip_prefix("Path=")
                    .is_some_and(|working| Path::new(working) == folder)
            })
        })
    }
}

#[cfg(all(test, target_os = "windows"))]
pub(super) fn create_shortcut(
    marker: &Path,
    owner: Ownership,
    target: &Path,
    desktop: &Path,
) -> Result<()> {
    let title = crate::model::title(&owner.app);
    create_named_shortcut(marker, owner, target, desktop, &title)
}

pub(super) fn create_named_shortcut(
    marker: &Path,
    mut owner: Ownership,
    target: &Path,
    desktop: &Path,
    title: &str,
) -> Result<()> {
    let app = &owner.app;
    let shortcut = desktop.join(format!(
        "{}.{}",
        title,
        crate::platform::shortcut_extension()
    ));
    std::fs::create_dir_all(desktop)?;
    let executable =
        crate::model::installed_executable(target, app).context("Missing installed executable")?;
    if shortcut.exists() {
        let owned = owner
            .shortcut
            .as_ref()
            .is_some_and(|(p, hash)| p == &shortcut && shortcut_proof(p).is_ok_and(|h| &h == hash));
        if !owned && !repairable_shortcut(&shortcut, &executable) {
            anyhow::bail!("An existing desktop shortcut was left unchanged");
        }
    }
    #[cfg(not(target_os = "macos"))]
    crate::platform::shortcut(&shortcut, &executable, "", target)?;
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::symlink;
        if shortcut.exists() {
            std::fs::remove_file(&shortcut)?;
        }
        symlink(&executable, &shortcut)?;
    }
    owner.shortcut = Some((shortcut.clone(), shortcut_proof(&shortcut)?));
    files::write_json(marker, &owner)
}

pub(super) fn repairable_shortcut(shortcut: &Path, executable: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        match crate::platform::shortcut_target(shortcut) {
            Some(existing) => {
                same_shortcut_target(&existing, executable)
                    || crate::platform::advertised_shortcut_target(shortcut)
                        .is_some_and(|target| same_shortcut_target(&target, executable))
            }
            None => false,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (shortcut, executable);
        false
    }
}

#[cfg(target_os = "windows")]
pub(super) fn same_shortcut_target(existing: &str, executable: &Path) -> bool {
    if Path::new(existing)
        .to_string_lossy()
        .eq_ignore_ascii_case(&executable.to_string_lossy())
    {
        return true;
    }
    // Shortcuts store long paths; Windows temp directories may use 8.3 names.
    match (
        std::fs::canonicalize(existing),
        std::fs::canonicalize(executable),
    ) {
        (Ok(a), Ok(b)) => a
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()),
        _ => false,
    }
}

pub fn remove_shortcut(target: &Path) -> Result<()> {
    let marker = target.join(MARKER);
    if !marker.exists() {
        return Ok(());
    }
    let owner: Ownership = files::read_json(&marker)?;
    remove_owned_shortcut(owner, &desktop()?)
}

pub fn remove_installer_shortcut(paths: &Paths, app: &str) -> Result<()> {
    crate::model::valid_app(app)?;
    let marker = paths.at(format!("runtime/desktop-shortcuts/{app}.json"));
    if !marker.exists() {
        return Ok(());
    }
    let owner: Ownership = files::read_json(&marker)?;
    anyhow::ensure!(
        owner.app == app
            && crate::portable_mode::same_library(
                &owner.library,
                &std::path::absolute(&paths.root)?
            ),
        "Shortcut record belongs to another app library"
    );
    remove_owned_shortcut(owner, &desktop()?)?;
    std::fs::remove_file(marker)?;
    Ok(())
}

pub(super) fn remove_owned_shortcut(owner: Ownership, desktop: &Path) -> Result<()> {
    let title = owner
        .shortcut
        .as_ref()
        .and_then(|(path, _)| owned_installation_title(&owner.app, path, desktop));
    if let Some(title) = title {
        remove_owned_named_shortcut(owner, desktop, &title)?;
    }
    Ok(())
}

pub(super) fn remove_owned_named_shortcut(
    owner: Ownership,
    desktop: &Path,
    title: &str,
) -> Result<()> {
    if let Some((path, expected)) = owner.shortcut {
        let known_path = desktop.join(format!(
            "{}.{}",
            title,
            crate::platform::shortcut_extension()
        ));
        // A user-edited shortcut is theirs to keep.
        if path == known_path && shortcut_proof(&path).is_ok_and(|hash| hash == expected) {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub(super) fn shortcut_proof(path: &Path) -> Result<String> {
    if files::linked(path)? {
        Ok(format!("link:{}", std::fs::read_link(path)?.display()))
    } else {
        files::hash(path)
    }
}

pub(super) fn desktop() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        crate::profiles::known_folder(&windows::Win32::UI::Shell::FOLDERID_Desktop)
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = std::process::Command::new("xdg-user-dir")
            .arg("DESKTOP")
            .output()
        {
            if output.status.success() {
                let path = PathBuf::from(String::from_utf8(output.stdout)?.trim());
                if path.is_absolute() {
                    return Ok(path);
                }
            }
        }
        Ok(PathBuf::from(std::env::var_os("HOME").context("Missing home folder")?).join("Desktop"))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(PathBuf::from(std::env::var_os("HOME").context("Missing home folder")?).join("Desktop"))
    }
}

#[cfg(all(test, target_os = "windows"))]
#[path = "build_shortcut_tests.rs"]
mod build_shortcut_tests;
