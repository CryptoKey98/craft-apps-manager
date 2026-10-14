//! Ownership and destination rules for portable apps outside the library.
use crate::{files, model::Paths};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

const MARKER: &str = ".craft-manager-install.json";

/// Remove empty legacy UUID containers, never a user-selected parent.
pub fn cleanup_empty_installation_parent(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    crate::model::valid_app(app)?;
    let Some(parent) = target.parent() else {
        return Ok(());
    };
    let library = paths.at("releases");
    if target.file_name().is_none_or(|name| name != app)
        || parent.parent() != Some(library.as_path())
        || !parent.exists()
    {
        return Ok(());
    }
    let Some(suffix) = parent
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(&format!("{app}-")))
    else {
        return Ok(());
    };
    let id = match suffix.split_once('-') {
        Some(("x64" | "x86" | "arm64", id)) => id,
        None => suffix,
        _ => return Ok(()),
    };
    if id.len() != 32
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || files::linked(parent)?
    {
        return Ok(());
    }
    files::inside(parent, &library)?;
    match std::fs::remove_dir(parent) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}
#[derive(Serialize, Deserialize)]
struct Ownership {
    app: String,
    library: PathBuf,
    #[serde(default)]
    shortcut: Option<(PathBuf, String)>,
}
pub fn destination(parent: &Path, app: &str) -> Result<PathBuf> {
    crate::model::valid_app(app)?;
    if !parent.is_absolute()
        || parent
            .components()
            .any(|c| matches!(c, Component::ParentDir))
    {
        bail!("Choose an absolute folder path without .. components");
    }
    let target = parent.join(app);
    validate_path(&target)?;
    Ok(target)
}

/// Library copies live in flat numbered folders. Custom destinations keep
/// their existing parent/app layout and collision checks.
pub fn additional_destination(paths: &Paths, parent: &Path, app: &str) -> Result<PathBuf> {
    let target = destination(parent, app)?;
    if parent != paths.at("releases") {
        return Ok(target);
    }
    match std::fs::symlink_metadata(&target) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(target),
        Err(error) => return Err(error.into()),
    }
    for number in 2..=u32::MAX {
        let candidate = parent.join(format!("{app}-{number}"));
        validate_path(&candidate)?;
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Err(error) => return Err(error.into()),
        }
    }
    bail!("No unused installation folder is available")
}

fn numbered_library_copy(paths: &Paths, app: &str, target: &Path) -> bool {
    target.parent() == Some(paths.at("releases").as_path())
        && target
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(&format!("{app}-")))
            .is_some_and(|number| {
                number
                    .parse::<u32>()
                    .is_ok_and(|value| value >= 2 && number == value.to_string())
            })
}
fn validate_path(target: &Path) -> Result<()> {
    let parent = target.parent().context("Missing app parent folder")?;
    files::inside(target, parent)?;
    let mut cursor = parent;
    loop {
        if std::fs::symlink_metadata(cursor).is_ok()
            && files::linked(cursor)?
            && !system_directory_alias(cursor)
        {
            bail!("Choose a folder without linked parent directories");
        }
        if let Some(next) = cursor.parent() {
            cursor = next;
        } else {
            break;
        }
    }
    Ok(())
}
fn system_directory_alias(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        // macOS exposes these OS-owned locations through fixed root aliases.
        // No user-controlled link below them is accepted.
        let expected = match path.to_str() {
            Some("/var") => Path::new("/private/var"),
            Some("/tmp") => Path::new("/private/tmp"),
            _ => return false,
        };
        std::fs::read_link(path).is_ok_and(|target| {
            let resolved = if target.is_absolute() {
                target
            } else {
                path.parent().unwrap_or(Path::new("/")).join(target)
            };
            resolved == expected
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}
pub fn validate(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    crate::model::valid_app(app)?;
    if target.starts_with(paths.at("releases")) {
        return files::inside(target, &paths.at("releases"));
    }
    validate_path(target)?;
    if target.file_name() != Some(std::ffi::OsStr::new(app)) {
        bail!("Portable apps must use a separate app folder");
    }
    let marker = target.join(MARKER);
    files::inside(&marker, target)?;
    let owner: Ownership =
        files::read_json(&marker).context("This folder is not managed by this library")?;
    if owner.app != app
        || !crate::portable_mode::same_library(&owner.library, &std::path::absolute(&paths.root)?)
    {
        bail!("This app folder belongs to another library");
    }
    Ok(())
}
pub fn validate_install(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    let parent = target.parent().context("Missing app parent folder")?;
    if numbered_library_copy(paths, app, target) {
        validate_path(target)?;
    }
    if target.starts_with(&paths.root) && !target.starts_with(paths.at("releases")) {
        bail!("Choose a folder outside the manager's sources, builds and runtime folders");
    }
    if (!target.starts_with(paths.at("releases")) || !target.exists())
        && !numbered_library_copy(paths, app, target)
    {
        anyhow::ensure!(destination(parent, app)? == target, "Invalid app folder");
    }
    // Never replace a pre-existing folder merely because the user chose its parent.
    if std::fs::symlink_metadata(target).is_ok() {
        let record = paths.config()?.apps.into_iter().find(|a| a.name == app);
        anyhow::ensure!(
            record.is_some_and(|a| a.install_kind != "installer" && Path::new(&a.path) == target),
            "That app folder already exists. Choose another parent folder to keep its files safe."
        );
        validate(paths, app, target)?;
    }
    Ok(())
}
pub fn mark(paths: &Paths, app: &str, stage: &Path) -> Result<()> {
    mark_copy(paths, app, stage, false)
}
pub fn mark_additional(paths: &Paths, app: &str, stage: &Path) -> Result<()> {
    mark_copy(paths, app, stage, true)
}
fn mark_copy(paths: &Paths, app: &str, stage: &Path, additional: bool) -> Result<()> {
    let marker = stage.join(MARKER);
    anyhow::ensure!(!marker.exists(), "Release includes a reserved manager file");
    let shortcut = paths
        .config()?
        .apps
        .into_iter()
        .find(|a| a.name == app)
        .and_then(|a| files::read_json::<Ownership>(&PathBuf::from(a.path).join(MARKER)).ok())
        .and_then(|owner| owner.shortcut);
    files::write_json(
        &marker,
        &Ownership {
            app: app.into(),
            library: std::path::absolute(&paths.root)?,
            shortcut: if additional { None } else { shortcut },
        },
    )
}
pub fn mark_restored(paths: &Paths, app: &str, stage: &Path) -> Result<()> {
    let marker = stage.join(MARKER);
    if marker.exists() {
        files::inside(&marker, stage)?;
        let owner: Ownership = files::read_json(&marker)?;
        anyhow::ensure!(
            owner.app == app
                && crate::portable_mode::same_library(
                    &owner.library,
                    &std::path::absolute(&paths.root)?
                ),
            "Backup belongs to another library"
        );
        std::fs::remove_file(marker)?;
    }
    mark(paths, app, stage)
}
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
pub fn mark_moved(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    let marker = target.join(MARKER);
    if marker.exists() {
        let owner: Ownership = files::read_json(&marker)?;
        anyhow::ensure!(
            owner.app == app
                && crate::portable_mode::same_library(
                    &owner.library,
                    &std::path::absolute(&paths.root)?
                ),
            "App folder belongs to another library"
        );
        return Ok(());
    }
    mark(paths, app, target)
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

fn migrate_shortcut_records(
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

fn migrate_installation_shortcut(paths: &Paths, copy: &crate::model::Installed) -> Result<()> {
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

fn installation_shortcut_title(
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

fn create_installation_shortcut(
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

fn owned_installation_title(app: &str, path: &Path, desktop: &Path) -> Option<String> {
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
fn build_shortcut_identity(paths: &Paths, app: &str, target: &Path) -> Result<(PathBuf, String)> {
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
fn build_desktop_shortcut_at(
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
fn remove_build_shortcut_at(paths: &Paths, app: &str, folder: &Path, desktop: &Path) -> Result<()> {
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
fn legacy_build_shortcut_matches(shortcut: &Path, folder: &Path, app: &str) -> bool {
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
fn create_shortcut(marker: &Path, owner: Ownership, target: &Path, desktop: &Path) -> Result<()> {
    let title = crate::model::title(&owner.app);
    create_named_shortcut(marker, owner, target, desktop, &title)
}
fn create_named_shortcut(
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

fn repairable_shortcut(shortcut: &Path, executable: &Path) -> bool {
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
fn same_shortcut_target(existing: &str, executable: &Path) -> bool {
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
fn remove_owned_shortcut(owner: Ownership, desktop: &Path) -> Result<()> {
    let title = owner
        .shortcut
        .as_ref()
        .and_then(|(path, _)| owned_installation_title(&owner.app, path, desktop));
    if let Some(title) = title {
        remove_owned_named_shortcut(owner, desktop, &title)?;
    }
    Ok(())
}
fn remove_owned_named_shortcut(owner: Ownership, desktop: &Path, title: &str) -> Result<()> {
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
fn shortcut_proof(path: &Path) -> Result<String> {
    if files::linked(path)? {
        Ok(format!("link:{}", std::fs::read_link(path)?.display()))
    } else {
        files::hash(path)
    }
}
fn desktop() -> Result<PathBuf> {
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
pub fn pick_folder(initial: &Path) -> Result<Option<PathBuf>> {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::{System::Com::*, UI::Shell::*};
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let result = (|| -> Result<Option<PathBuf>> {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
            if initial.is_dir() {
                let text: Vec<u16> = initial
                    .as_os_str()
                    .to_string_lossy()
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
                if let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                    windows::core::PCWSTR(text.as_ptr()),
                    None,
                ) {
                    let _ = dialog.SetFolder(&folder);
                }
            }
            if let Err(error) = dialog.Show(None) {
                if error.code().0 as u32 == 0x800704c7 {
                    return Ok(None);
                }
                return Err(error.into());
            }
            let item = dialog.GetResult()?;
            let value = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let text = value.to_string();
            CoTaskMemFree(Some(value.0.cast()));
            Ok(Some(PathBuf::from(text?)))
        })();
        CoUninitialize();
        result
    }
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("zenity")
            .args([
                "--file-selection",
                "--directory",
                "--title=Choose install location",
            ])
            .arg(format!("--filename={}/", initial.display()))
            .output()
            .or_else(|_| {
                std::process::Command::new("kdialog")
                    .arg("--getexistingdirectory")
                    .arg(initial)
                    .output()
            })
            .context("Folder picker unavailable. Enter the folder path directly.")?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(PathBuf::from(
            String::from_utf8(output.stdout)?.trim(),
        )))
    }
    #[cfg(target_os = "macos")]
    {
        let _ = initial;
        let output = std::process::Command::new("/usr/bin/osascript")
            .args([
                "-e",
                "POSIX path of (choose folder with prompt \"Choose install location\")",
            ])
            .output()?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(PathBuf::from(
            String::from_utf8(output.stdout)?.trim(),
        )))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shortcut_migration_continues_after_errors_and_visits_each_copy_once() {
        let copy = |path: &str| crate::model::Installed {
            name: "filmcraft".into(),
            path: path.into(),
            install_kind: "portable".into(),
            ..Default::default()
        };
        let config = crate::model::Config {
            apps_root: String::new(),
            installations: vec![copy("damaged"), copy("conflict"), copy("healthy")],
            apps: vec![copy("healthy")],
            portable_label_counters: Default::default(),
        };
        let mut visited = Vec::new();
        let result = super::migrate_shortcut_records(&config, |copy| {
            visited.push(copy.path.clone());
            if copy.path != "healthy" {
                anyhow::bail!("fixture migration failure");
            }
            Ok(())
        });
        assert_eq!(visited, ["damaged", "conflict", "healthy"]);
        let message = result.unwrap_err().to_string();
        assert!(message.contains("damaged") && message.contains("conflict"));
    }

    #[test]
    fn empty_generated_installation_parents_are_removed_but_user_folders_are_kept() {
        let (root, paths) = fixture();
        let parent = paths.at(format!(
            "releases/filmcraft-x64-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&parent).unwrap();
        cleanup_empty_installation_parent(&paths, "filmcraft", &parent.join("filmcraft")).unwrap();
        assert!(!parent.exists());
        assert!(paths.at("releases").exists());

        // The install dialog historically omitted the architecture component.
        let dialog_parent = paths.at(format!(
            "releases/filmcraft-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dialog_parent).unwrap();
        cleanup_empty_installation_parent(&paths, "filmcraft", &dialog_parent.join("filmcraft"))
            .unwrap();
        assert!(!dialog_parent.exists());

        std::fs::create_dir_all(&parent).unwrap();
        std::fs::write(parent.join("notes.txt"), b"keep").unwrap();
        cleanup_empty_installation_parent(&paths, "filmcraft", &parent.join("filmcraft")).unwrap();
        assert_eq!(std::fs::read(parent.join("notes.txt")).unwrap(), b"keep");

        let custom = root.join("my-apps");
        std::fs::create_dir_all(&custom).unwrap();
        cleanup_empty_installation_parent(&paths, "filmcraft", &custom.join("filmcraft")).unwrap();
        assert!(custom.exists());
        let ordinary = paths.at("releases/my-folder");
        std::fs::create_dir_all(&ordinary).unwrap();
        cleanup_empty_installation_parent(&paths, "filmcraft", &ordinary.join("filmcraft"))
            .unwrap();
        assert!(ordinary.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installation_shortcut_names_distinguish_copies_and_validate_ownership() {
        let root = std::env::temp_dir().join("shortcut-names");
        let desktop = root.join("desktop");
        let first =
            installation_shortcut_title("filmcraft", &root.join("first"), "Portable", "x64", "A")
                .unwrap();
        let second =
            installation_shortcut_title("filmcraft", &root.join("second"), "Portable", "x64", "B")
                .unwrap();
        let installer =
            installation_shortcut_title("filmcraft", &root.join("first"), "Installer", "x64", "")
                .unwrap();
        assert_ne!(first, second);
        assert_ne!(first, installer);
        assert_eq!(installer, "FilmCraft (x64)");
        for title in ["FilmCraft (x64)", "FilmCraft (Installer x64)"] {
            let shortcut =
                desktop.join(format!("{title}.{}", crate::platform::shortcut_extension()));
            assert_eq!(
                owned_installation_title("filmcraft", &shortcut, &desktop).as_deref(),
                Some(title)
            );
        }
        let shortcut = desktop.join(format!("{first}.{}", crate::platform::shortcut_extension()));
        assert_eq!(
            owned_installation_title("filmcraft", &shortcut, &desktop),
            Some(first)
        );
        assert!(owned_installation_title("photocraft", &shortcut, &desktop).is_none());
        assert!(owned_installation_title("filmcraft", &shortcut, &root).is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn multiple_installation_shortcuts_survive_updates_moves_and_individual_removal() {
        let (root, paths) = fixture();
        let desktop = root.join("desktop");
        let mut copies = Vec::new();
        let mut config = paths.config().unwrap();
        for (name, kind) in [
            ("first", "Portable"),
            ("second", "Portable"),
            ("native", "Installer"),
        ] {
            let target = root.join(name);
            std::fs::create_dir_all(&target).unwrap();
            std::fs::write(target.join("filmcraft.exe"), b"fixture").unwrap();
            let record = crate::model::Installed {
                name: "filmcraft".into(),
                path: target.display().to_string(),
                install_kind: if kind == "Installer" {
                    "installer"
                } else {
                    "portable"
                }
                .into(),
                architecture: "x64".into(),
                ..Default::default()
            };
            *config
                .apps
                .iter_mut()
                .find(|copy| copy.name == "filmcraft")
                .unwrap() = record.clone();
            config.installations.push(record);
            paths.save_config(&config).unwrap();
            config = files::read_json(&paths.at("settings.json")).unwrap();
            let marker = target.join(MARKER);
            let owner = Ownership {
                app: "filmcraft".into(),
                library: paths.root.clone(),
                shortcut: None,
            };
            let owner = if name == "first" {
                create_named_shortcut(
                    &marker,
                    owner,
                    &target,
                    &desktop,
                    "FilmCraft (Portable x64 · 1234abcd)",
                )
                .unwrap();
                files::read_json(&marker).unwrap()
            } else {
                owner
            };
            create_installation_shortcut(&paths, &marker, owner, &target, &desktop, kind).unwrap();
            let owner: Ownership = files::read_json(&marker).unwrap();
            let shortcut = owner.shortcut.as_ref().unwrap().0.clone();
            assert_eq!(
                PathBuf::from(crate::platform::shortcut_target(&shortcut).unwrap())
                    .canonicalize()
                    .unwrap(),
                target.join("filmcraft.exe").canonicalize().unwrap()
            );
            copies.push((target, marker, shortcut));
        }
        assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 3);
        let (target, marker, shortcut) = &copies[0];
        let owner: Ownership = files::read_json(marker).unwrap();
        create_installation_shortcut(&paths, marker, owner, target, &desktop, "Portable").unwrap();
        assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 3);
        let moved = root.join("moved");
        std::fs::rename(target, &moved).unwrap();
        config
            .installations
            .iter_mut()
            .find(|copy| Path::new(&copy.path) == target)
            .unwrap()
            .path = moved.display().to_string();
        paths.save_config(&config).unwrap();
        let marker = moved.join(MARKER);
        let owner: Ownership = files::read_json(&marker).unwrap();
        create_installation_shortcut(&paths, &marker, owner, &moved, &desktop, "Portable").unwrap();
        assert_eq!(
            PathBuf::from(crate::platform::shortcut_target(shortcut).unwrap())
                .canonicalize()
                .unwrap(),
            moved.join("filmcraft.exe").canonicalize().unwrap()
        );
        let owner: Ownership = files::read_json(&marker).unwrap();
        remove_owned_shortcut(owner, &desktop).unwrap();
        assert!(!shortcut.exists());
        assert!(copies[1].2.exists() && copies[2].2.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn legacy_shortcut_is_replaced_only_when_still_owned() {
        for edited in [false, true] {
            let (root, paths) = fixture();
            let target = root.join("app");
            let desktop = root.join("desktop");
            std::fs::create_dir_all(&target).unwrap();
            std::fs::write(target.join("filmcraft.exe"), b"fixture").unwrap();
            // A pre-label installation record must still be usable.
            files::write_json(&paths.at("settings.json"), &serde_json::json!({"appsRoot":paths.root, "apps":[{"name":"filmcraft","path":target,"version":"0.1.0","architecture":"x64","install_kind":"portable"}]})).unwrap();
            let marker = target.join(MARKER);
            create_shortcut(
                &marker,
                Ownership {
                    app: "filmcraft".into(),
                    library: paths.root.clone(),
                    shortcut: None,
                },
                &target,
                &desktop,
            )
            .unwrap();
            let legacy = desktop.join("FilmCraft.lnk");
            if edited {
                std::fs::write(&legacy, b"user edit").unwrap();
            }
            let owner: Ownership = files::read_json(&marker).unwrap();
            create_installation_shortcut(&paths, &marker, owner, &target, &desktop, "Portable")
                .unwrap();
            assert_eq!(legacy.exists(), edited);
            if edited {
                assert_eq!(std::fs::read(&legacy).unwrap(), b"user edit");
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn installer_shortcuts_are_tracked_without_writing_to_app_folder() {
        use super::*;
        let root = std::env::temp_dir().join(format!("craft-shortcut-{}", uuid::Uuid::new_v4()));
        let target = root.join("installed-app");
        let desktop = root.join("desktop");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("photocraft.exe"), b"fixture").unwrap();
        let marker = root.join("runtime/desktop-shortcuts/photocraft.json");
        let owner = Ownership {
            app: "photocraft".into(),
            library: root.clone(),
            shortcut: None,
        };
        create_shortcut(&marker, owner, &target, &desktop).unwrap();
        assert!(!target.join(MARKER).exists());
        let shortcut = desktop.join("PhotoCraft.lnk");
        assert!(shortcut.is_file());
        let owner: Ownership = files::read_json(&marker).unwrap();
        create_shortcut(&marker, owner, &target, &desktop).unwrap();
        std::fs::write(&shortcut, b"user-edited shortcut").unwrap();
        let owner: Ownership = files::read_json(&marker).unwrap();
        remove_owned_shortcut(owner, &desktop).unwrap();
        assert_eq!(std::fs::read(&shortcut).unwrap(), b"user-edited shortcut");
        let owner: Ownership = files::read_json(&marker).unwrap();
        assert!(create_shortcut(&marker, owner, &target, &desktop).is_err());
        std::fs::remove_file(&shortcut).unwrap();
        let owner: Ownership = files::read_json(&marker).unwrap();
        create_shortcut(&marker, owner, &target, &desktop).unwrap();
        let owner: Ownership = files::read_json(&marker).unwrap();
        remove_owned_shortcut(owner, &desktop).unwrap();
        assert!(!shortcut.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn shortcuts_repair_when_the_target_still_points_at_the_app() {
        use super::*;
        let root = std::env::temp_dir().join(format!("craft-shortcut-{}", uuid::Uuid::new_v4()));
        let target = root.join("installed-app");
        let desktop = root.join("desktop");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("photocraft.exe"), b"fixture").unwrap();
        let marker = root.join("runtime/desktop-shortcuts/photocraft.json");
        let owner = Ownership {
            app: "photocraft".into(),
            library: root.clone(),
            shortcut: None,
        };
        create_shortcut(&marker, owner, &target, &desktop).unwrap();
        let shortcut = desktop.join("PhotoCraft.lnk");
        // Rewriting the shortcut with different metadata keeps the same target
        // but breaks the recorded proof hash, as an update or repair would.
        crate::platform::shortcut(
            &shortcut,
            &target.join("photocraft.exe"),
            "",
            &target.join("elsewhere"),
        )
        .unwrap();
        let owner: Ownership = files::read_json(&marker).unwrap();
        create_shortcut(&marker, owner, &target, &desktop).unwrap();
        let owner: Ownership = files::read_json(&marker).unwrap();
        remove_owned_shortcut(owner, &desktop).unwrap();
        assert!(!shortcut.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn installer_cache_path_alone_does_not_prove_shortcut_ownership() {
        use super::*;
        let root = std::env::temp_dir().join(format!("craft-shortcut-{}", uuid::Uuid::new_v4()));
        let target = root.join("installed-app");
        let desktop = root.join("desktop");
        std::fs::create_dir_all(&target).unwrap();
        let exe = target.join("photocraft.exe");
        std::fs::write(&exe, b"fixture").unwrap();
        // Installer updates leave advertised shortcuts whose target lives in
        // the Windows Installer cache and breaks on the next major upgrade.
        let advertised = PathBuf::from(
            r"C:\Windows\Installer\{01234567-89AB-CDEF-0123-456789ABCDEF}\PhotocraftIcon.ico",
        );
        std::fs::create_dir_all(&desktop).unwrap();
        let shortcut = desktop.join("PhotoCraft.lnk");
        crate::platform::shortcut(&shortcut, &advertised, "", &target).unwrap();
        let marker = root.join("runtime/desktop-shortcuts/photocraft.json");
        let owner = Ownership {
            app: "photocraft".into(),
            library: root.clone(),
            shortcut: None,
        };
        let before = std::fs::read(&shortcut).unwrap();
        assert!(create_shortcut(&marker, owner, &target, &desktop).is_err());
        assert_eq!(std::fs::read(&shortcut).unwrap(), before);
        assert!(crate::platform::advertised_shortcut_target(&shortcut).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn shortcuts_pointing_elsewhere_are_left_unchanged() {
        use super::*;
        let root = std::env::temp_dir().join(format!("craft-shortcut-{}", uuid::Uuid::new_v4()));
        let target = root.join("installed-app");
        let desktop = root.join("desktop");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("photocraft.exe"), b"fixture").unwrap();
        std::fs::write(target.join("other.exe"), b"other").unwrap();
        std::fs::create_dir_all(&desktop).unwrap();
        let shortcut = desktop.join("PhotoCraft.lnk");
        crate::platform::shortcut(&shortcut, &target.join("other.exe"), "", &target).unwrap();
        let before = std::fs::read(&shortcut).unwrap();
        let marker = root.join("runtime/desktop-shortcuts/photocraft.json");
        let owner = Ownership {
            app: "photocraft".into(),
            library: root.clone(),
            shortcut: None,
        };
        assert!(create_shortcut(&marker, owner, &target, &desktop).is_err());
        assert_eq!(std::fs::read(&shortcut).unwrap(), before);
        crate::platform::shortcut(
            &shortcut,
            &target.join("photocraft.exe"),
            "--user-option",
            &target,
        )
        .unwrap();
        let before = std::fs::read(&shortcut).unwrap();
        let owner = Ownership {
            app: "photocraft".into(),
            library: root.clone(),
            shortcut: None,
        };
        assert!(create_shortcut(&marker, owner, &target, &desktop).is_err());
        assert_eq!(std::fs::read(&shortcut).unwrap(), before);
        std::fs::remove_dir_all(root).unwrap();
    }

    use super::*;
    use crate::model::Preferences;
    #[cfg(unix)]
    #[test]
    fn destination_rejects_custom_links_even_below_system_temp_directory() {
        let root = std::env::temp_dir().join(format!("craft-linked-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("linked")).unwrap();
        assert!(destination(&root.join("real"), "photocraft").is_ok());
        assert!(destination(&root.join("linked"), "photocraft").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    fn fixture() -> (PathBuf, Paths) {
        let root = std::env::temp_dir().join(format!("craft-custom-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.join("library"), None);
        files::write_json(
            &paths.at("manager-settings.json"),
            &Preferences {
                release_format: "portable".into(),
                ..Default::default()
            },
        )
        .unwrap();
        (root, paths)
    }
    #[test]
    fn destinations_reject_relative_traversal_internal_and_existing_unmanaged_folders() {
        let (root, paths) = fixture();
        assert!(destination(Path::new("relative"), "photocraft").is_err());
        assert!(destination(&root.join(".."), "photocraft").is_err());
        assert!(validate_install(&paths, "photocraft", &paths.at("sources/photocraft")).is_err());
        let target = destination(&root.join("apps"), "photocraft").unwrap();
        assert!(!target.exists());
        validate_install(&paths, "photocraft", &target).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("work.png"), b"personal").unwrap();
        assert!(validate_install(&paths, "photocraft", &target).is_err());
        assert_eq!(std::fs::read(target.join("work.png")).unwrap(), b"personal");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn additional_library_copies_are_flat_numbered_and_never_replace_existing_files() {
        let (root, paths) = fixture();
        let library = paths.at("releases");
        assert_eq!(
            additional_destination(&paths, &library, "filmcraft").unwrap(),
            library.join("filmcraft")
        );
        std::fs::create_dir_all(library.join("filmcraft")).unwrap();
        std::fs::write(library.join("filmcraft/keep.txt"), b"original").unwrap();
        let second = additional_destination(&paths, &library, "filmcraft").unwrap();
        assert_eq!(second, library.join("filmcraft-2"));
        validate_install(&paths, "filmcraft", &second).unwrap();
        std::fs::write(&second, b"unrelated file").unwrap();
        assert!(validate_install(&paths, "filmcraft", &second).is_err());
        assert_eq!(
            additional_destination(&paths, &library, "filmcraft").unwrap(),
            library.join("filmcraft-3")
        );
        let custom_parent = root.join("custom");
        assert_eq!(
            additional_destination(&paths, &custom_parent, "filmcraft").unwrap(),
            custom_parent.join("filmcraft")
        );
        assert!(validate_install(&paths, "filmcraft", &custom_parent.join("filmcraft-2")).is_err());
        assert_eq!(
            std::fs::read(library.join("filmcraft/keep.txt")).unwrap(),
            b"original"
        );
        assert_eq!(std::fs::read(&second).unwrap(), b"unrelated file");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn custom_folder_ownership_is_bound_to_app_and_library() {
        let (root, paths) = fixture();
        let target = destination(&root.join("apps"), "photocraft").unwrap();
        std::fs::create_dir_all(&target).unwrap();
        mark(&paths, "photocraft", &target).unwrap();
        validate(&paths, "photocraft", &target).unwrap();
        assert!(validate(&paths, "filmcraft", &target).is_err());
        let other = Paths::new(root.join("other-library"), None);
        assert!(validate(&other, "photocraft", &target).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn custom_portable_uninstall_keeps_projects_and_retains_photo_data() {
        let (root, paths) = fixture();
        let parent = root.join("apps");
        let target = destination(&parent, "photocraft").unwrap();
        std::fs::create_dir_all(target.join("PhotoCraftData")).unwrap();
        let exe = target.join(crate::model::executable_name("photocraft"));
        std::fs::write(&exe, b"fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(target.join("PhotoCraftData/recovery.pcraft"), b"recovery").unwrap();
        std::fs::write(parent.join("my-image.png"), b"image").unwrap();
        mark(&paths, "photocraft", &target).unwrap();
        let mut config = paths.config().unwrap();
        let app = config
            .apps
            .iter_mut()
            .find(|a| a.name == "photocraft")
            .unwrap();
        app.path = target.display().to_string();
        app.version = "0.3.0".into();
        app.install_kind = "portable".into();
        paths.save_config(&config).unwrap();
        assert_eq!(
            crate::apps::executables(&paths, "photocraft")
                .unwrap()
                .len(),
            1
        );
        validate_install(&paths, "photocraft", &target).unwrap();
        crate::apps::uninstall_with_profile(&paths, "photocraft", false).unwrap();
        assert!(!target.exists());
        assert_eq!(
            std::fs::read(parent.join("my-image.png")).unwrap(),
            b"image"
        );
        assert_eq!(
            std::fs::read(paths.at("runtime/app-profiles/photocraft/recovery.pcraft")).unwrap(),
            b"recovery"
        );
        assert!(!crate::apps::installed(&paths, "photocraft").is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, target_os = "windows"))]
mod build_shortcut_tests {
    use super::*;
    #[test]
    fn legacy_build_shortcut_cleanup_survives_portable_library_move() {
        let temp =
            std::env::temp_dir().join(format!("craft-portable-shortcut-{}", uuid::Uuid::new_v4()));
        let base = temp.join("original");
        let paths = Paths::new(base.join("data"), None);
        let folder = paths.at("builds/filmcraft/first");
        let desktop = temp.join("desktop");
        files::write_json(
            &folder.join("build-info.json"),
            &crate::model::BuildInfo {
                app: "filmcraft".into(),
                commit: "a".repeat(40),
                source_branch: "main".into(),
                built_at: "2026-10-12T12:00:00Z".into(),
                profile: "release".into(),
                log: String::new(),
            },
        )
        .unwrap();
        std::fs::write(folder.join("filmcraft.exe"), b"test build").unwrap();
        // Create the shortcut before portable mode, exercising the old absolute digest.
        build_desktop_shortcut_at(&paths, "filmcraft", &folder, &desktop).unwrap();
        assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 1);
        std::fs::write(base.join(crate::portable_mode::MARKER), b"").unwrap();
        crate::portable_mode::initialize(&base).unwrap();
        let moved = temp.join("moved");
        std::fs::rename(&base, &moved).unwrap();
        crate::portable_mode::initialize(&moved).unwrap();
        let paths = Paths::new(moved.join("data"), None);
        remove_build_shortcut_at(
            &paths,
            "filmcraft",
            &paths.at("builds/filmcraft/first"),
            &desktop,
        )
        .unwrap();
        assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 0);
        std::fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn multiple_build_shortcuts_coexist_and_remove_independently() {
        let root = std::env::temp_dir().join(format!(
            "craft-build-shortcuts-many-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = Paths::new(root.join("library"), None);
        let desktop = root.join("desktop");
        let make = |name: &str| {
            let folder = paths.at(format!("builds/filmcraft/{name}"));
            files::write_json(
                &folder.join("build-info.json"),
                &crate::model::BuildInfo {
                    app: "filmcraft".into(),
                    commit: "a".repeat(40),
                    source_branch: "main".into(),
                    built_at: "2026-10-12T12:00:00Z".into(),
                    profile: "release".into(),
                    log: String::new(),
                },
            )
            .unwrap();
            std::fs::write(folder.join("filmcraft.exe"), b"test build").unwrap();
            build_desktop_shortcut_at(&paths, "filmcraft", &folder, &desktop).unwrap();
            let (marker, title) = build_shortcut_identity(&paths, "filmcraft", &folder).unwrap();
            (folder, marker, desktop.join(format!("{title}.lnk")))
        };
        // Even the same commit and timestamp need separate shortcuts.
        let first = make("first");
        let second = make("second");
        assert_ne!(first.2, second.2);
        assert!(first.2.is_file() && second.2.is_file());
        assert_eq!(
            PathBuf::from(crate::platform::shortcut_target(&first.2).unwrap())
                .canonicalize()
                .unwrap(),
            first.0.join("filmcraft.exe").canonicalize().unwrap()
        );
        assert_eq!(
            PathBuf::from(crate::platform::shortcut_target(&second.2).unwrap())
                .canonicalize()
                .unwrap(),
            second.0.join("filmcraft.exe").canonicalize().unwrap()
        );
        build_desktop_shortcut_at(&paths, "filmcraft", &first.0, &desktop).unwrap();
        assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 2);
        remove_build_shortcut_at(&paths, "filmcraft", &first.0, &desktop).unwrap();
        assert!(!first.2.exists() && !first.1.exists());
        assert!(second.2.exists() && second.1.exists());
        std::fs::write(&second.2, b"User edited shortcut").unwrap();
        remove_build_shortcut_at(&paths, "filmcraft", &second.0, &desktop).unwrap();
        assert_eq!(std::fs::read(&second.2).unwrap(), b"User edited shortcut");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removing_build_shortcuts_preserves_other_builds_and_user_edits() {
        for (legacy, other_build, edited) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, true, false),
            (false, false, true),
        ] {
            let root =
                std::env::temp_dir().join(format!("craft-build-shortcut-{}", uuid::Uuid::new_v4()));
            let paths = Paths::new(root.join("library"), None);
            let desktop = root.join("desktop");
            let selected = root.join("selected");
            let other = root.join("other");
            std::fs::create_dir_all(&selected).unwrap();
            std::fs::create_dir_all(&other).unwrap();
            for folder in [&selected, &other] {
                std::fs::write(folder.join("filmcraft.exe"), b"test build").unwrap();
            }
            let marker = paths.at("runtime/build-shortcuts/filmcraft.json");
            let target = if other_build { &other } else { &selected };
            create_named_shortcut(
                &marker,
                Ownership {
                    app: "filmcraft".into(),
                    library: std::path::absolute(&paths.root).unwrap(),
                    shortcut: None,
                },
                target,
                &desktop,
                "FilmCraft Build",
            )
            .unwrap();
            if !legacy {
                let owner = files::read_json(&marker).unwrap();
                files::write_json(
                    &marker,
                    &BuildShortcutOwnership {
                        owner,
                        build_folder: Some(target.clone()),
                        shortcut_title: None,
                    },
                )
                .unwrap();
            }
            let shortcut = desktop.join("FilmCraft Build.lnk");
            if edited {
                std::fs::write(&shortcut, b"user changed shortcut").unwrap();
            }
            let original = std::fs::read(&shortcut).unwrap();
            remove_build_shortcut_at(&paths, "filmcraft", &selected, &desktop).unwrap();
            if other_build || edited {
                assert_eq!(std::fs::read(&shortcut).unwrap(), original);
            } else {
                assert!(!shortcut.exists());
            }
            if other_build {
                assert!(marker.exists());
            }
            assert!(other.join("filmcraft.exe").exists());
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
