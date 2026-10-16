//! Portable storage is opt-in and only rewrites known manager-owned path fields.
//! Execution plans, approved manifests, arguments, credentials and app data are untouched.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::{OnceLock, RwLock},
};

pub const MARKER: &str = "portable.txt";
const ORIGIN: &str = "portable-state.json";
const PREFIX: &str = "@portable/";

#[derive(Clone)]
struct Storage {
    base: PathBuf,
    previous: PathBuf,
    aliases: Vec<PathBuf>,
    fixed_base: Option<PathBuf>,
    libraries: Vec<PathBuf>,
}
#[derive(Serialize, Deserialize)]
struct Origin {
    schema: u32,
    base: PathBuf,
    #[serde(default)]
    previous_bases: Vec<PathBuf>,
}
static STORAGE: OnceLock<RwLock<Vec<Storage>>> = OnceLock::new();

/// On macOS mutable data lives outside the signed application bundle.
pub fn executable_folder(executable: &Path) -> Result<PathBuf> {
    if let Some(bundle) = executable
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
    {
        return Ok(bundle
            .parent()
            .context("Application bundle has no parent")?
            .to_path_buf());
    }
    Ok(executable
        .parent()
        .context("Executable has no parent")?
        .to_path_buf())
}

pub fn enabled(home: &Path) -> bool {
    local_storage(home) && !fixed_storage(home)
}

pub fn local_storage(home: &Path) -> bool {
    home.join(MARKER).is_file()
}

fn fixed_storage(home: &Path) -> bool {
    fs::read(home.join(MARKER))
        .is_ok_and(|bytes| bytes == b"Craft Apps Manager fixed local storage\n")
}

/// Fix managed metadata to the current location without moving the library.
pub fn deactivate(base: &Path, busy: bool) -> Result<()> {
    anyhow::ensure!(
        !busy,
        "Finish active operations before disabling portable mode."
    );
    anyhow::ensure!(enabled(base), "Portable mode is not enabled.");
    migrate(base, true)
}

pub fn activate(base: &Path, library: &Path, busy: bool) -> Result<()> {
    anyhow::ensure!(
        !busy,
        "Finish active operations before enabling portable mode."
    );
    anyhow::ensure!(
        library.starts_with(base),
        "Use a library inside the manager folder for portable mode."
    );
    anyhow::ensure!(!enabled(base), "Portable mode is already enabled.");
    // Activation changes no paths or application files; migration happens on restart.
    crate::files::write_bytes(&base.join(MARKER), b"Craft Apps Manager portable mode\n")
        .context("Cannot enable portable mode. Choose a writable portable manager folder.")
}

fn register(base: &Path, previous: &Path, aliases: &[PathBuf]) {
    let mut stores = STORAGE.get_or_init(Default::default).write().unwrap();
    stores.retain(|s| s.base != base);
    stores.push(Storage {
        base: base.to_owned(),
        previous: previous.to_owned(),
        aliases: aliases.to_vec(),
        fixed_base: None,
        libraries: vec![base.join("data"), base.to_owned()],
    });
}

/// Explicit library boundaries take precedence over cache-directory names.
pub fn register_library(base: &Path, library: &Path) {
    let mut stores = STORAGE.get_or_init(Default::default).write().unwrap();
    if let Some(storage) = stores.iter_mut().find(|s| s.base == base) {
        if library.starts_with(base) && !storage.libraries.contains(&library.to_path_buf()) {
            storage.libraries.push(library.to_path_buf());
        }
    }
}

fn metadata_relative(file: &Path, storage: &Storage) -> Result<PathBuf> {
    let library = storage
        .libraries
        .iter()
        .filter(|root| file.starts_with(root))
        .max_by_key(|root| root.components().count());
    Ok(file
        .strip_prefix(library.unwrap_or(&storage.base))?
        .to_path_buf())
}

pub fn initialized(home: &Path) -> bool {
    store(&home.join("data-root.json")).is_some()
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn base_for_library(library: &Path) -> Option<PathBuf> {
    store(library).map(|storage| storage.base)
}

/// The approved worker only attaches a decoder. It must never run migration
/// or write location metadata with administrator privileges.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn attach_existing(base: &Path, library: &Path) -> Result<()> {
    anyhow::ensure!(
        base.is_absolute() && library.starts_with(base),
        "Invalid portable batch storage"
    );
    crate::files::no_links(base)?;
    crate::files::no_links(library)?;
    anyhow::ensure!(local_storage(base), "Portable storage marker is missing");
    let origin: Origin = serde_json::from_slice(&fs::read(base.join(ORIGIN))?)?;
    anyhow::ensure!(
        origin.schema == 1
            && origin.base == base
            && origin.previous_bases.len() <= 128
            && origin.previous_bases.iter().all(|path| path.is_absolute()),
        "Portable storage changed; review a new batch"
    );
    register(base, &origin.base, &origin.previous_bases);
    register_library(base, library);
    if fixed_storage(base) {
        STORAGE
            .get()
            .unwrap()
            .write()
            .unwrap()
            .iter_mut()
            .find(|s| s.base == base)
            .unwrap()
            .fixed_base = Some(origin.base);
    }
    Ok(())
}

/// Preserve the old library before the first launch of a portable update.
/// Old update helpers only replace the binary, so startup calls this too.
pub fn adopt_updated_portable(base: &Path, library: &Path, tools: Option<&Path>) -> Result<()> {
    anyhow::ensure!(
        base.is_absolute() && library.is_absolute(),
        "Invalid portable update location"
    );
    // This may be /Applications, not a manager-owned tree. Validate only
    // the directory and metadata we touch; updater staging/bundle checks
    // independently validate the manager's executable files.
    anyhow::ensure!(
        !crate::files::linked(base)?,
        "Linked portable update folder"
    );
    for name in [MARKER, "data-root.json"] {
        let path = base.join(name);
        match fs::symlink_metadata(&path) {
            Ok(_) => anyhow::ensure!(
                !crate::files::linked(&path)?,
                "Linked portable update metadata: {}",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if local_storage(base) {
        return Ok(());
    }
    let mut writes = Vec::new();
    if !base.join("data-root.json").exists() {
        writes.push((
            base.join("data-root.json"),
            serde_json::to_vec_pretty(&serde_json::json!({"root":library,"tools":tools}))?,
        ));
    }
    writes.push((
        base.join(MARKER),
        b"Craft Apps Manager portable mode\n".to_vec(),
    ));
    crate::settings::commit_metadata(&writes)
}

/// Legacy external portable folders retain their ownership after their library moves.
/// No external file is rewritten, and the recorded library must be a known prior location.
pub(crate) fn same_library(recorded: &Path, current: &Path) -> bool {
    if recorded == current {
        return true;
    }
    let Some(storage) = store(current) else {
        return false;
    };
    storage
        .aliases
        .iter()
        .chain(std::iter::once(&storage.previous))
        .any(|base| {
            recorded.strip_prefix(base).is_ok_and(|relative| {
                storage
                    .fixed_base
                    .as_ref()
                    .unwrap_or(&storage.base)
                    .join(relative)
                    == current
            })
        })
}

fn store(file: &Path) -> Option<Storage> {
    STORAGE
        .get()?
        .read()
        .ok()?
        .iter()
        .filter(|s| file.starts_with(&s.base))
        .max_by_key(|s| s.base.components().count())
        .cloned()
}

/// Resolve user-entered relative folders against the portable folder, never the CWD.
pub fn resolve_folder(home: &Path, folder: &Path) -> Result<PathBuf> {
    if folder.is_absolute() {
        return Ok(folder.to_owned());
    }
    anyhow::ensure!(
        enabled(home),
        "Folder paths must be absolute outside portable mode."
    );
    // Explicit external ../ locations are supported; normalize them before use.
    let mut resolved = home.to_path_buf();
    for part in folder.components() {
        match part {
            Component::Normal(p) => resolved.push(p),
            Component::CurDir => {}
            Component::ParentDir => {
                anyhow::ensure!(
                    resolved.pop(),
                    "Folder path goes above the filesystem root."
                );
            }
            _ => bail!("Invalid relative folder path"),
        }
    }
    Ok(resolved)
}

fn path_value(value: &mut Value, storage: &Storage, encode: bool) -> Result<()> {
    let path_base = storage.fixed_base.as_ref().unwrap_or(&storage.base);
    let Some(text) = value.as_str() else {
        return Ok(());
    };
    if text.is_empty() {
        return Ok(());
    }
    let path = if let Some(relative) = text.strip_prefix(PREFIX) {
        let relative = Path::new(relative);
        anyhow::ensure!(
            relative
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
            "Invalid portable path"
        );
        let mut resolved = path_base.clone();
        for component in relative.components() {
            if let Component::Normal(part) = component {
                resolved.push(part);
            }
        }
        resolved
    } else {
        let path = PathBuf::from(text);
        if !path.is_absolute() {
            return Ok(());
        }
        if path.starts_with(path_base) {
            path
        } else if let Some(relative) = storage
            .aliases
            .iter()
            .chain(std::iter::once(&storage.previous))
            .filter_map(|base| {
                path.strip_prefix(base)
                    .ok()
                    .map(|relative| (base.components().count(), relative))
            })
            .max_by_key(|(length, _)| *length)
            .map(|(_, relative)| relative)
        {
            path_base.join(relative)
        } else {
            path
        }
    };
    // No traversal or linked paths are made portable. External absolute paths stay absolute.
    anyhow::ensure!(
        !path.components().any(|c| matches!(c, Component::ParentDir)),
        "Unnormalized stored path"
    );
    if encode {
        if let Ok(relative) = path.strip_prefix(&storage.base) {
            if path != storage.base {
                crate::files::inside(&path, &storage.base)?;
            }
            let relative = if relative.as_os_str().is_empty() {
                ".".to_owned()
            } else {
                relative.to_string_lossy().replace('\\', "/")
            };
            *value = Value::String(format!("{PREFIX}{relative}"));
        }
    } else {
        *value = Value::String(path.to_string_lossy().into_owned());
    }
    Ok(())
}

fn field(object: &mut Value, key: &str, storage: &Storage, encode: bool) -> Result<()> {
    if let Some(value) = object.get_mut(key) {
        path_value(value, storage, encode)?;
    }
    Ok(())
}

fn metadata_name(name: &str) -> bool {
    if name.split('/').any(|part| {
        matches!(
            part,
            "self-update"
                | "admin-batches"
                | "portable-migration"
                | "workspace"
                | "sources"
                | "backups"
        )
    }) {
        return false;
    }
    name == "data-root.json"
        || name == "settings.json"
        || name.ends_with("/settings.json")
            && !name.contains("runtime/")
            && !name.contains("releases/")
            && !name.contains("builds/")
        || name.ends_with("runtime/launch-settings.json")
        || name.ends_with("runtime/build-launch-settings.json")
        || name.contains("runtime/build-locations/")
        || name.contains("runtime/build-shortcuts/")
        || name.contains("runtime/desktop-shortcuts/")
        || name.contains("runtime/operations/")
        || name.ends_with("/build-info.json") && !name.contains("releases/")
        || name.ends_with("/.craft-manager-install.json")
}

pub(crate) fn applies(file: &Path) -> bool {
    store(file).is_some_and(|storage| {
        metadata_relative(file, &storage)
            .is_ok_and(|path| metadata_name(&path.to_string_lossy().replace('\\', "/")))
    })
}

/// Paths are transformed only at known persistence boundaries, not in arbitrary JSON.
fn transform(relative: &Path, value: &mut Value, storage: &Storage, encode: bool) -> Result<()> {
    let name = relative.to_string_lossy().replace('\\', "/");
    if !metadata_name(&name) {
        return Ok(());
    }
    match name.as_str() {
        "data-root.json" => {
            field(value, "root", storage, encode)?;
            field(value, "tools", storage, encode)?;
        }
        "settings.json" | "data/settings.json" => {
            field(value, "appsRoot", storage, encode)?;
            for key in ["apps", "installations"] {
                if let Some(apps) = value.get_mut(key).and_then(Value::as_array_mut) {
                    for app in apps {
                        // Native installers are machine-local, even when their directory is nearby.
                        if app.get("install_kind").and_then(Value::as_str) != Some("installer") {
                            field(app, "path", storage, encode)?;
                        }
                    }
                }
            }
        }
        _ if name.ends_with("/runtime/launch-settings.json")
            || name == "runtime/launch-settings.json" =>
        {
            if let Some(map) = value.as_object_mut() {
                let old = std::mem::take(map);
                for (key, mut settings) in old {
                    field(&mut settings, "executable", storage, encode)?;
                    let key = if let Some((app, folder)) = key.split_once('|') {
                        let mut folder = Value::String(folder.into());
                        path_value(&mut folder, storage, encode)?;
                        format!("{app}|{}", folder.as_str().unwrap())
                    } else {
                        key
                    };
                    anyhow::ensure!(
                        map.insert(key, settings).is_none(),
                        "Duplicate portable launch configuration"
                    );
                }
            }
        }
        _ if name.ends_with("/runtime/build-launch-settings.json")
            || name == "runtime/build-launch-settings.json" =>
        {
            if let Some(map) = value.as_object_mut() {
                for settings in map.values_mut() {
                    field(settings, "selectedBuild", storage, encode)?;
                }
            }
        }
        _ if name.contains("runtime/build-locations/") && name.ends_with(".json") => {
            path_value(value, storage, encode)?
        }
        _ if (name.contains("runtime/build-shortcuts/")
            || name.contains("runtime/desktop-shortcuts/"))
            && name.ends_with(".json") =>
        {
            field(value, "library", storage, encode)?;
            field(value, "build_folder", storage, encode)?;
            if let Some(shortcut) = value
                .get_mut("shortcut")
                .and_then(Value::as_array_mut)
                .and_then(|a| a.first_mut())
            {
                path_value(shortcut, storage, encode)?;
            }
        }
        _ if name.ends_with("/.craft-manager-install.json") => {
            field(value, "library", storage, encode)?;
            if let Some(shortcut) = value
                .get_mut("shortcut")
                .and_then(Value::as_array_mut)
                .and_then(|a| a.first_mut())
            {
                path_value(shortcut, storage, encode)?;
            }
        }
        _ if name.contains("runtime/operations/") && name.ends_with(".json") => {
            if let Some(facts) = value.get_mut("facts") {
                for key in [
                    "Log file",
                    "Output location",
                    "Installation location",
                    "Source location",
                    "Destination",
                ] {
                    field(facts, key, storage, encode)?;
                }
            }
        }
        _ if name.ends_with("/build-info.json")
            && value.get("builtAt").is_some()
            && value.get("commit").is_some() =>
        {
            field(value, "log", storage, encode)?
        }
        // Libraries can have a user-chosen folder name within the portable package.
        _ if name.ends_with("/settings.json")
            && !name.contains("runtime/")
            && value.get("appsRoot").is_some() =>
        {
            transform(Path::new("settings.json"), value, storage, encode)?;
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn convert(file: &Path, value: &mut Value, encode: bool) -> Result<()> {
    if let Some(storage) = store(file) {
        transform(
            &metadata_relative(file, &storage)?,
            value,
            &storage,
            encode && !fixed_storage(&storage.base),
        )?;
    }
    Ok(())
}

/// Stable shortcut identities for managed builds survive a move of the portable folder.
pub(crate) fn identity(path: &Path) -> Result<String> {
    let mut value = Value::String(path.to_string_lossy().into_owned());
    if let Some(storage) = store(path) {
        path_value(&mut value, &storage, true)?;
    }
    Ok(value.as_str().unwrap().to_owned())
}

/// Called before reading settings. A bounded migration backs up only manager metadata.
/// Approved elevated manifests and recovery plans deliberately retain their exact paths.
pub fn initialize(base: &Path) -> Result<()> {
    migrate(base, false)
}

fn migrate(base: &Path, disabling: bool) -> Result<()> {
    anyhow::ensure!(local_storage(base), "Local storage marker is missing.");
    let base = std::path::absolute(base)?;
    anyhow::ensure!(
        !crate::files::linked(&base)?,
        "Choose a portable folder that is not a symbolic link."
    );
    let origin_path = base.join(ORIGIN);
    let origin: Option<Origin> = if origin_path.exists() {
        Some(serde_json::from_slice(&fs::read(&origin_path)?)?)
    } else {
        None
    };
    if let Some(origin) = &origin {
        anyhow::ensure!(
            origin.schema == 1 && origin.base.is_absolute(),
            "Unsupported portable metadata."
        );
    }
    let previous = origin
        .as_ref()
        .map(|o| o.base.clone())
        .unwrap_or_else(|| base.clone());
    let mut aliases = origin
        .as_ref()
        .map(|o| o.previous_bases.clone())
        .unwrap_or_default();
    if previous != base && !aliases.contains(&previous) {
        aliases.push(previous.clone());
    }
    anyhow::ensure!(
        aliases.len() <= 128 && aliases.iter().all(|p| p.is_absolute()),
        "Invalid portable location history"
    );
    register(&base, &previous, &aliases);
    let fixed = !disabling && fixed_storage(&base);
    if fixed {
        let mut stores = STORAGE.get_or_init(Default::default).write().unwrap();
        stores
            .iter_mut()
            .find(|s| s.base == base)
            .unwrap()
            .fixed_base = Some(previous.clone());
    }
    let mut storage = store(&base.join("data-root.json")).unwrap();
    if base.join("data-root.json").is_file() {
        let bytes = fs::read(base.join("data-root.json"))?;
        let mut locations: Value =
            serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))?;
        field(&mut locations, "root", &storage, false)?;
        if let Some(root) = locations.get("root").and_then(Value::as_str) {
            let root = PathBuf::from(root);
            let root = if root.is_absolute() {
                root
            } else {
                resolve_folder(&base, &root)?
            };
            register_library(&base, &root);
        }
        storage = store(&base.join("data-root.json")).unwrap();
    }
    if !disabling && (fixed_storage(&base) || origin.as_ref().is_some_and(|o| o.base == base)) {
        return Ok(());
    }
    let _lock = crate::platform::Lock::take("Local\\CraftAppsManager")?;
    let mut writes = Vec::new();
    // Skip application files, tool caches, source trees, backups and approved helpers.
    for entry in walkdir::WalkDir::new(&base)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 || !entry.file_type().is_dir() {
                return true;
            }
            if crate::files::linked(entry.path()).unwrap_or(true) {
                return false;
            }
            // Follow the configured library even beneath a directory called workspace,
            // backups, or sources; exclusions apply inside that library instead.
            if storage
                .libraries
                .iter()
                .any(|root| root != &base && root.starts_with(entry.path()))
            {
                return true;
            }
            let relative = metadata_relative(entry.path(), &storage).unwrap();
            let name = relative.to_string_lossy().replace('\\', "/");
            let parts: Vec<_> = name.split('/').collect();
            if parts.iter().any(|part| part.ends_with(".app")) {
                return false;
            }
            if let Some(index) = parts.iter().position(|part| *part == "releases") {
                if parts.len() > index + 2 {
                    return false;
                }
            }
            if let Some(index) = parts.iter().position(|part| *part == "builds") {
                if parts.len() > index + 3 {
                    return false;
                }
            }
            !name.split('/').any(|part| {
                matches!(
                    part,
                    "sources"
                        | "workspace"
                        | "backups"
                        | "target"
                        | ".git"
                        | "portable-migration"
                        | "self-update"
                        | "admin-batches"
                )
            })
        })
        .enumerate()
    {
        let (visited, entry) = entry;
        anyhow::ensure!(
            visited < 100_000,
            "Too many files to migrate safely; no metadata was changed."
        );
        let entry = entry?;
        if !entry.file_type().is_file() || entry.path().extension().is_none_or(|e| e != "json") {
            continue;
        }
        anyhow::ensure!(
            writes.len() < 2000,
            "Too many metadata files to migrate automatically."
        );
        // Only known schemas qualify; unrelated app JSON must never be parsed or rewritten.
        let relative = metadata_relative(entry.path(), &storage)?;
        let name = relative.to_string_lossy().replace('\\', "/");
        if !metadata_name(&name) {
            continue;
        }
        anyhow::ensure!(
            entry.metadata()?.len() <= 8 * 1024 * 1024,
            "Metadata file is too large: {}",
            entry.path().display()
        );
        let old = fs::read(entry.path())?;
        let mut value: Value =
            serde_json::from_slice(old.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&old))
                .with_context(|| format!("Cannot migrate {}", entry.path().display()))?;
        let original = value.clone();
        if name.contains("runtime/operations/") {
            let active = value
                .get("lifecycle")
                .and_then(Value::as_str)
                .is_some_and(|state| {
                    matches!(state, "Queued" | "Running" | "CancellationRequested")
                });
            let recent = value
                .get("heartbeat")
                .and_then(Value::as_i64)
                .is_some_and(|time| chrono::Utc::now().timestamp_millis() - time < 30_000);
            anyhow::ensure!(!(active && recent), "Close the manager and Builder after active operations finish, then restart to apply portable mode.");
        }
        transform(&relative, &mut value, &storage, !disabling)?;
        if original != value {
            writes.push((
                entry.path().to_path_buf(),
                serde_json::to_vec_pretty(&value)?,
            ));
        }
    }
    if disabling && !base.join("data-root.json").exists() {
        let library = default_library(&base);
        let tools = if base.join("workspace/tools").is_dir() {
            base.join("workspace/tools")
        } else {
            library.join("workspace/tools")
        };
        writes.push((
            base.join("data-root.json"),
            serde_json::to_vec_pretty(&serde_json::json!({"root":library,"tools":tools}))?,
        ));
    }
    // Backups finish before any original is touched; the existing rollback-protected writer
    // restores originals if one replacement fails. The origin is committed last.
    let backup = base
        .join("runtime/portable-migration")
        .join(uuid::Uuid::new_v4().to_string());
    crate::files::inside(&backup, &base)?;
    for (path, _) in &writes {
        if !path.exists() {
            continue;
        }
        let target = backup.join(path.strip_prefix(&base)?);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(path, target)?;
    }
    writes.push((
        origin_path,
        serde_json::to_vec_pretty(&Origin {
            schema: 1,
            base: base.clone(),
            previous_bases: aliases.clone(),
        })?,
    ));
    if disabling {
        writes.push((
            base.join(MARKER),
            b"Craft Apps Manager fixed local storage\n".to_vec(),
        ));
    }
    crate::settings::commit_metadata(&writes)
        .context("Portable migration failed; previous metadata was preserved.")?;
    if disabling {
        STORAGE
            .get_or_init(Default::default)
            .write()
            .unwrap()
            .iter_mut()
            .find(|s| s.base == base)
            .unwrap()
            .fixed_base = Some(base.clone());
    }
    Ok(())
}

/// Portable launch defaults preserve an existing Windows library beside the executable.
/// New downloads use a separate data folder so manager updates cannot overwrite data.
pub fn default_library(base: &Path) -> PathBuf {
    if [
        "settings.json",
        "manager-settings.json",
        "builder-settings.json",
        "releases",
        "builds",
        "sources",
    ]
    .iter()
    .any(|name| base.join(name).exists())
    {
        base.to_path_buf()
    } else {
        base.join("data")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_adoption_ignores_unrelated_link_but_rejects_linked_metadata() {
        let temp =
            std::env::temp_dir().join(format!("craft-adoption-links-{}", uuid::Uuid::new_v4()));
        let base = temp.join("Applications");
        let other = temp.join("unrelated");
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&other).unwrap();
        let link = base.join("Unrelated-Test-Link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&other, &link).unwrap();
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&other)
            .output()
            .unwrap()
            .status
            .success());
        // Reproduce the old whole-folder rejection before testing scoped adoption.
        assert!(crate::files::no_links(&base).is_err());
        adopt_updated_portable(&base, &temp.join("library"), None).unwrap();
        assert!(local_storage(&base));
        assert!(link.exists());
        fs::remove_file(base.join(MARKER)).unwrap();
        fs::remove_file(base.join("data-root.json")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&other, base.join("data-root.json")).unwrap();
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(base.join("data-root.json"))
            .arg(&other)
            .output()
            .unwrap()
            .status
            .success());
        assert!(adopt_updated_portable(&base, &temp.join("library"), None).is_err());
        assert!(!local_storage(&base));
        #[cfg(unix)]
        fs::remove_file(base.join("data-root.json")).unwrap();
        #[cfg(windows)]
        fs::remove_dir(base.join("data-root.json")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&other, base.join(MARKER)).unwrap();
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(base.join(MARKER))
            .arg(&other)
            .output()
            .unwrap()
            .status
            .success());
        assert!(adopt_updated_portable(&base, &temp.join("library"), None).is_err());
        // The containing folder itself must also remain unlinked.
        assert!(adopt_updated_portable(&link, &temp.join("library"), None).is_err());
        assert!(fs::read_dir(&other).unwrap().next().is_none());
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn old_portable_update_preserves_library_and_migrates_on_restart() {
        let temp = std::env::temp_dir().join(format!("craft-old-update-{}", uuid::Uuid::new_v4()));
        let base = temp.join("manager");
        let library = base.join("legacy-library");
        let tools = base.join("workspace").join("tools");
        fs::create_dir_all(&library).unwrap();
        let settings = library.join("settings.json");
        fs::write(
            &settings,
            serde_json::to_vec(
                &serde_json::json!({"appsRoot":library,"apps":[],"installations":[]}),
            )
            .unwrap(),
        )
        .unwrap();
        let old = fs::read(&settings).unwrap();
        adopt_updated_portable(&base, &library, Some(&tools)).unwrap();
        assert_eq!(fs::read(&settings).unwrap(), old);
        initialize(&base).unwrap();
        let moved = temp.join("moved-manager");
        fs::rename(&base, &moved).unwrap();
        initialize(&moved).unwrap();
        let locations: Value = crate::files::read_json(&moved.join("data-root.json")).unwrap();
        assert_eq!(
            locations["root"],
            moved.join("legacy-library").display().to_string()
        );
        assert_eq!(
            locations["tools"],
            moved.join("workspace").join("tools").display().to_string()
        );
        let config: crate::model::Config =
            crate::files::read_json(&moved.join("legacy-library/settings.json")).unwrap();
        assert_eq!(
            config.apps_root,
            moved.join("legacy-library").display().to_string()
        );
        // Existing marker/location choices are never overwritten by another update.
        let old_locations = fs::read(moved.join("data-root.json")).unwrap();
        adopt_updated_portable(&moved, &temp.join("another-library"), None).unwrap();
        assert_eq!(
            fs::read(moved.join("data-root.json")).unwrap(),
            old_locations
        );
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn old_portable_update_keeps_external_library_absolute() {
        let temp =
            std::env::temp_dir().join(format!("craft-external-update-{}", uuid::Uuid::new_v4()));
        let base = temp.join("manager");
        let library = temp.join("external-library");
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&library).unwrap();
        let settings = library.join("settings.json");
        fs::write(&settings, b"external settings are never rewritten").unwrap();
        adopt_updated_portable(&base, &library, None).unwrap();
        initialize(&base).unwrap();
        let moved = temp.join("moved");
        fs::rename(&base, &moved).unwrap();
        initialize(&moved).unwrap();
        let locations: Value = crate::files::read_json(&moved.join("data-root.json")).unwrap();
        assert_eq!(locations["root"], library.display().to_string());
        assert_eq!(
            fs::read(&settings).unwrap(),
            b"external settings are never rewritten"
        );
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn portable_worker_attachment_decodes_without_migrating_or_writing() {
        let temp =
            std::env::temp_dir().join(format!("craft-worker-storage-{}", uuid::Uuid::new_v4()));
        let library = temp.join("custom").join("library");
        fs::create_dir_all(&library).unwrap();
        fs::write(temp.join(MARKER), b"").unwrap();
        fs::write(
            temp.join("data-root.json"),
            serde_json::to_vec(&serde_json::json!({"root":library})).unwrap(),
        )
        .unwrap();
        initialize(&temp).unwrap();
        let settings = library.join("settings.json");
        crate::files::write_json(
            &settings,
            &serde_json::json!({"appsRoot":library,"apps":[],"installations":[]}),
        )
        .unwrap();
        let before = fs::read(&settings).unwrap();
        let origin = fs::read(temp.join(ORIGIN)).unwrap();
        assert_eq!(base_for_library(&library), Some(temp.clone()));
        STORAGE
            .get()
            .unwrap()
            .write()
            .unwrap()
            .retain(|store| store.base != temp);
        let undecoded: crate::model::Config = crate::files::read_json(&settings).unwrap();
        assert!(undecoded.apps_root.starts_with(PREFIX));
        attach_existing(&temp, &library).unwrap();
        let decoded: crate::model::Config = crate::files::read_json(&settings).unwrap();
        assert_eq!(decoded.apps_root, library.display().to_string());
        assert_eq!(fs::read(&settings).unwrap(), before);
        assert_eq!(fs::read(temp.join(ORIGIN)).unwrap(), origin);
        assert!(attach_existing(&temp, &temp.parent().unwrap().join("external")).is_err());
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn disabled_mode_retains_external_ownership_and_restores_tagged_backups() {
        let temp =
            std::env::temp_dir().join(format!("craft-fixed-compat-{}", uuid::Uuid::new_v4()));
        let first = temp.join("first");
        fs::create_dir_all(first.join("data")).unwrap();
        fs::write(first.join(MARKER), b"").unwrap();
        initialize(&first).unwrap();
        let external = temp.join("external/filmcraft");
        fs::create_dir_all(&external).unwrap();
        fs::write(
            external.join(".craft-manager-install.json"),
            serde_json::to_vec(
                &serde_json::json!({"app":"filmcraft","library":first.join("data")}),
            )
            .unwrap(),
        )
        .unwrap();
        let moved = temp.join("moved");
        fs::rename(&first, &moved).unwrap();
        initialize(&moved).unwrap();
        let library = moved.join("data");
        let paths = crate::model::Paths::new(library.clone(), None);
        crate::files::write_json(
            &paths.at("settings.json"),
            &crate::model::Config {
                portable_label_counters: Default::default(),
                apps_root: library.display().to_string(),
                apps: Vec::new(),
                installations: Vec::new(),
            },
        )
        .unwrap();
        crate::portable::validate(&paths, "filmcraft", &external).unwrap();
        deactivate(&moved, false).unwrap();
        initialize(&moved).unwrap();
        crate::portable::validate(&paths, "filmcraft", &external).unwrap();
        assert!(!same_library(&temp.join("unrelated/data"), &library));
        let stage = paths.at("runtime/downloads/restore/contents/filmcraft");
        fs::create_dir_all(&stage).unwrap();
        // An archived ownership marker must remain decodable after Disable and restart.
        fs::write(
            stage.join(".craft-manager-install.json"),
            br#"{"app":"filmcraft","library":"@portable/data"}"#,
        )
        .unwrap();
        crate::portable::mark_restored(&paths, "filmcraft", &stage).unwrap();
        let raw: Value =
            serde_json::from_slice(&fs::read(stage.join(".craft-manager-install.json")).unwrap())
                .unwrap();
        assert_eq!(Path::new(raw["library"].as_str().unwrap()), library);
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn deeply_nested_library_under_excluded_name_migrates_completely() {
        let temp =
            std::env::temp_dir().join(format!("craft-deep-library-{}", uuid::Uuid::new_v4()));
        let first = temp.join("first");
        let relative = Path::new("workspace/one/two/three/four/five/six/seven/eight/library");
        let library = first.join(relative);
        fs::create_dir_all(&library).unwrap();
        fs::write(first.join(MARKER), b"").unwrap();
        fs::write(
            first.join("data-root.json"),
            serde_json::to_vec(&serde_json::json!({"root":library})).unwrap(),
        )
        .unwrap();
        crate::files::write_json(
            &library.join("settings.json"),
            &serde_json::json!({"appsRoot":library,"apps":[],"installations":[]}),
        )
        .unwrap();
        let chosen = library.join("builds/filmcraft/selected");
        crate::files::write_json(
            &library.join("runtime/build-launch-settings.json"),
            &serde_json::json!({"filmcraft":{"selectedBuild":chosen}}),
        )
        .unwrap();
        let unrelated = first.join("workspace/tools/settings.json");
        fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
        fs::write(&unrelated, b"unrelated application settings").unwrap();
        initialize(&first).unwrap();
        let raw: Value =
            serde_json::from_slice(&fs::read(library.join("settings.json")).unwrap()).unwrap();
        assert!(raw["appsRoot"].as_str().unwrap().starts_with(PREFIX));
        let moved = temp.join("moved");
        fs::rename(&first, &moved).unwrap();
        initialize(&moved).unwrap();
        let library = moved.join(relative);
        let config: Value = crate::files::read_json(&library.join("settings.json")).unwrap();
        assert_eq!(Path::new(config["appsRoot"].as_str().unwrap()), library);
        deactivate(&moved, false).unwrap();
        initialize(&moved).unwrap();
        let raw: Value = serde_json::from_slice(
            &fs::read(library.join("runtime/build-launch-settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            Path::new(raw["filmcraft"]["selectedBuild"].as_str().unwrap()),
            library.join("builds/filmcraft/selected")
        );
        assert_eq!(
            fs::read(moved.join("workspace/tools/settings.json")).unwrap(),
            b"unrelated application settings"
        );
        fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn disabling_after_move_fixes_paths_and_preserves_library() {
        let temp = std::env::temp_dir().join(format!("craft-disable-{}", uuid::Uuid::new_v4()));
        let first = temp.join("first");
        fs::create_dir_all(first.join("data")).unwrap();
        fs::write(first.join(MARKER), b"").unwrap();
        fs::write(
            first.join("data-root.json"),
            serde_json::to_vec(&serde_json::json!({"root": first.join("data")})).unwrap(),
        )
        .unwrap();
        initialize(&first).unwrap();
        let moved = temp.join("moved");
        fs::rename(&first, &moved).unwrap();
        initialize(&moved).unwrap();
        assert!(deactivate(&moved, true).is_err());
        assert!(enabled(&moved));
        deactivate(&moved, false).unwrap();
        assert!(!enabled(&moved));
        assert!(local_storage(&moved));
        assert!(initialized(&moved)); // Compatibility decoding survives disabling.
        let value: Value = crate::files::read_or_default(&moved.join("data-root.json")).unwrap();
        assert_eq!(
            Path::new(value["root"].as_str().unwrap()),
            moved.join("data")
        );
        assert!(moved.join("data").is_dir());
        initialize(&moved).unwrap();
        let next = temp.join("next");
        fs::rename(&moved, &next).unwrap();
        initialize(&next).unwrap();
        let value: Value = crate::files::read_or_default(&next.join("data-root.json")).unwrap();
        assert_eq!(
            Path::new(value["root"].as_str().unwrap()),
            moved.join("data")
        );
        activate(&next, &next.join("data"), false).unwrap();
        initialize(&next).unwrap();
        let value: Value = crate::files::read_or_default(&next.join("data-root.json")).unwrap();
        assert_eq!(
            Path::new(value["root"].as_str().unwrap()),
            next.join("data")
        );
        fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn builds_operations_and_transactional_settings_survive_repeated_moves() {
        let temp =
            std::env::temp_dir().join(format!("craft-portable-state-{}", uuid::Uuid::new_v4()));
        let first = temp.join("first");
        fs::create_dir_all(&first).unwrap();
        fs::write(first.join(MARKER), b"").unwrap();
        initialize(&first).unwrap();
        let library = first.join("data");
        let paths = crate::model::Paths::new(library.clone(), None);
        let output = paths.at("builds/filmcraft/selected");
        fs::create_dir_all(&output).unwrap();
        fs::write(
            output.join(crate::model::build_executable_name("filmcraft")),
            b"test build",
        )
        .unwrap();
        crate::files::write_json(
            &output.join("build-info.json"),
            &crate::model::BuildInfo {
                app: "filmcraft".into(),
                commit: "a".repeat(40),
                source_branch: "main".into(),
                built_at: "2026-10-12T12:00:00Z".into(),
                profile: "release".into(),
                log: paths.at("logs/build.log").display().to_string(),
            },
        )
        .unwrap();
        crate::builder::save_build_options(
            &paths,
            "filmcraft",
            &crate::builder::BuildLaunchOptions {
                selected_build: Some(output.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let id_before = identity(&output).unwrap();
        let operation =
            crate::operations::Manager::new(&library).register("Build", "filmcraft", None);
        operation.report(
            "Log file",
            &paths.at("logs/build.log").display().to_string(),
        );
        operation.finish(
            crate::operations::Lifecycle::Completed,
            "Build completed".into(),
        );
        crate::settings::save(
            &paths,
            &first.join("data-root.json"),
            &serde_json::json!({"root":library,"tools":paths.tools}),
            &paths.root,
            &paths.tools,
            None,
            None,
        )
        .unwrap();
        let raw: Value =
            serde_json::from_slice(&fs::read(first.join("data-root.json")).unwrap()).unwrap();
        assert_eq!(raw["root"], "@portable/data");
        let second = temp.join("second");
        fs::rename(&first, &second).unwrap();
        initialize(&second).unwrap();
        let third = temp.join("third");
        fs::rename(&second, &third).unwrap();
        initialize(&third).unwrap();
        let moved = crate::model::Paths::new(third.join("data"), None);
        let selected = crate::builder::selected_build(&moved, "filmcraft")
            .unwrap()
            .unwrap();
        assert_eq!(selected, moved.at("builds/filmcraft/selected"));
        assert_eq!(identity(&selected).unwrap(), id_before);
        let info: crate::model::BuildInfo =
            crate::files::read_json(&selected.join("build-info.json")).unwrap();
        assert_eq!(Path::new(&info.log), moved.at("logs/build.log"));
        let records = crate::operations::Manager::new(&moved.root).list();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].lifecycle,
            crate::operations::Lifecycle::Completed
        );
        assert_eq!(records[0].facts["Log file"], info.log);
        assert!(same_library(&library, &moved.root));
        assert!(!same_library(&temp.join("unrelated"), &moved.root));
        assert_eq!(
            resolve_folder(&third, Path::new("./data")).unwrap(),
            moved.root
        );
        assert_eq!(
            resolve_folder(&third, Path::new("../external")).unwrap(),
            temp.join("external")
        );
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn failed_migration_preserves_originals_and_does_not_parse_application_data() {
        let temp =
            std::env::temp_dir().join(format!("craft-portable-failure-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(temp.join("data/releases/filmcraft")).unwrap();
        fs::write(temp.join(MARKER), b"").unwrap();
        fs::write(
            temp.join("data/releases/filmcraft/settings.json"),
            b"not manager JSON",
        )
        .unwrap();
        crate::files::write_json(
            &temp.join("settings.json"),
            &serde_json::json!({"appsRoot":temp,"apps":[],"installations":[]}),
        )
        .unwrap();
        let original = fs::read(temp.join("settings.json")).unwrap();
        fs::create_dir_all(temp.join("runtime")).unwrap();
        fs::write(
            temp.join("runtime/launch-settings.json"),
            b"corrupt manager JSON",
        )
        .unwrap();
        assert!(initialize(&temp).is_err());
        assert_eq!(fs::read(temp.join("settings.json")).unwrap(), original);
        assert!(!temp.join(ORIGIN).exists());
        fs::remove_file(temp.join("runtime/launch-settings.json")).unwrap();
        initialize(&temp).unwrap();
        assert_eq!(
            fs::read(temp.join("data/releases/filmcraft/settings.json")).unwrap(),
            b"not manager JSON"
        );
        fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn portable_paths_survive_moves_without_touching_external_installs_or_arguments() {
        let temp = std::env::temp_dir().join(format!("craft-portable-{}", uuid::Uuid::new_v4()));
        let old = temp.join("original");
        fs::create_dir_all(old.join("data/releases/filmcraft")).unwrap();
        fs::write(old.join(MARKER), b"").unwrap();
        let config = serde_json::json!({"appsRoot": old.join("data"), "apps": [
            {"name":"filmcraft", "path":old.join("data/releases/filmcraft"), "install_kind":"portable"},
            {"name":"designcraft", "path":temp.join("external"), "install_kind":"installer"}
        ], "installations": []});
        crate::files::write_json(&old.join("data/settings.json"), &config).unwrap();
        let settings = serde_json::json!({format!("filmcraft|{}",old.join("data/releases/filmcraft").display()): {
            "executable":old.join("data/releases/filmcraft/filmcraft.exe"), "arguments":[old.display().to_string()]
        }});
        crate::files::write_json(&old.join("data/runtime/launch-settings.json"), &settings)
            .unwrap();
        let plan = serde_json::json!({"root":old, "target":old.join("data/releases/filmcraft")});
        crate::files::write_json(&old.join("runtime/self-update/plan.json"), &plan).unwrap();
        initialize(&old).unwrap();
        let raw: Value =
            serde_json::from_slice(&fs::read(old.join("data/settings.json")).unwrap()).unwrap();
        assert_eq!(raw["appsRoot"], "@portable/data");
        assert!(old.join("runtime/portable-migration").is_dir());
        let new = temp.join("moved folder");
        fs::rename(&old, &new).unwrap();
        initialize(&new).unwrap();
        let config: Value = crate::files::read_json(&new.join("data/settings.json")).unwrap();
        assert_eq!(
            Path::new(config["apps"][0]["path"].as_str().unwrap()),
            new.join("data/releases/filmcraft")
        );
        assert_eq!(
            config["apps"][1]["path"],
            temp.join("external").display().to_string()
        );
        let settings: Value =
            crate::files::read_json(&new.join("data/runtime/launch-settings.json")).unwrap();
        let settings =
            &settings[format!("filmcraft|{}", config["apps"][0]["path"].as_str().unwrap())];
        assert_eq!(settings["arguments"][0], old.display().to_string());
        assert_eq!(
            Path::new(settings["executable"].as_str().unwrap()),
            new.join("data/releases/filmcraft/filmcraft.exe")
        );
        assert_eq!(
            crate::files::read_json::<Value>(&new.join("runtime/self-update/plan.json")).unwrap(),
            plan
        );
        fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn tagged_paths_reject_traversal_and_mac_data_stays_outside_bundle() {
        let base = PathBuf::from(if cfg!(windows) {
            "C:\\portable"
        } else {
            "/portable"
        });
        let storage = Storage {
            base: base.clone(),
            previous: base.clone(),
            aliases: Vec::new(),
            fixed_base: None,
            libraries: vec![base.clone()],
        };
        for path in ["@portable/../external", "@portable//absolute"] {
            assert!(path_value(&mut Value::String(path.into()), &storage, false).is_err());
        }
        assert_eq!(
            executable_folder(&base.join("Manager.app/Contents/MacOS/manager")).unwrap(),
            base
        );
    }
}
