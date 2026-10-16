//! Repository-bound identities learned from selected, verified packages.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::RwLock,
};
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<DesktopLaunch>,
    #[serde(default)]
    pub family: String,
    pub platform: String,
    pub package: String,
    pub executable: String,
}
/// Literal desktop launch arguments and per-process environment, never shell code.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct DesktopLaunch {
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
}
impl DesktopLaunch {
    pub fn valid(&self) -> bool {
        self.arguments.len() <= 128
            && self.environment.len() <= 64
            && self
                .arguments
                .iter()
                .all(|value| value.len() <= 16384 && !value.contains(['\0', '\r', '\n']))
            && self.environment.iter().all(|(key, value)| {
                !key.is_empty()
                    && key.bytes().enumerate().all(|(i, c)| {
                        c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
                    })
                    && value.len() <= 16384
                    && !value.contains(['\0', '\r', '\n'])
            })
    }
    pub fn apply(&self, command: &mut std::process::Command) {
        command.args(&self.arguments).envs(&self.environment);
    }
}
#[derive(Default)]
struct Loaded {
    root: Option<PathBuf>,
    records: BTreeMap<String, Vec<NativeIdentity>>,
}
static CURRENT: RwLock<Loaded> = RwLock::new(Loaded {
    root: None,
    records: BTreeMap::new(),
});
const FILE: &str = "runtime/app-identities.json";
const PORTABLE: &str = ".craft-app-executable.json";
#[derive(Serialize, Deserialize)]
struct Portable {
    repository: String,
    executable: PathBuf,
}
fn repository(app: &str) -> String {
    crate::model::repository(app).to_ascii_lowercase()
}
pub fn load(root: &Path) {
    let records = crate::files::read_or_default(&root.join(FILE)).unwrap_or_default();
    *CURRENT.write().unwrap() = Loaded {
        root: Some(root.to_path_buf()),
        records,
    };
}
pub fn native(app: &str) -> Vec<NativeIdentity> {
    CURRENT
        .read()
        .unwrap()
        .records
        .get(&repository(app))
        .into_iter()
        .flatten()
        .filter(|identity| valid(identity))
        .cloned()
        .collect()
}
/// Executable basenames used by process guards, including per-copy portable choices.
pub fn running_executables(app: &str) -> Vec<String> {
    let mut names: Vec<String> = native(app)
        .into_iter()
        .filter(|identity| identity.platform == crate::model::release_os())
        .filter_map(|identity| {
            Path::new(&identity.executable)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect();
    let root = CURRENT.read().unwrap().root.clone();
    if let Some(root) = root {
        if let Ok(config) =
            crate::files::read_json::<serde_json::Value>(&root.join("settings.json"))
        {
            for record in ["apps", "installations"]
                .into_iter()
                .filter_map(|key| config.get(key).and_then(|value| value.as_array()))
                .flatten()
            {
                if record.get("name").and_then(|value| value.as_str()) != Some(app) {
                    continue;
                }
                if let Some(path) = record.get("path").and_then(|value| value.as_str()) {
                    if let Some(executable) = portable(Path::new(path), app) {
                        if let Some(name) = executable.file_name() {
                            names.push(name.to_string_lossy().into_owned());
                        }
                    }
                }
            }
        }
    }
    names.sort();
    names.dedup();
    names
}
pub fn remember_current(app: &str, identity: NativeIdentity) -> Result<()> {
    let root = CURRENT
        .read()
        .unwrap()
        .root
        .clone()
        .context("Application identity library is not initialized")?;
    remember(&root, app, identity)
}
/// Preflight ownership without changing the last successfully installed identity.
pub fn check_current_claim(app: &str, identity: &NativeIdentity) -> Result<()> {
    anyhow::ensure!(valid(identity), "Invalid package identity");
    let root = CURRENT
        .read()
        .unwrap()
        .root
        .clone()
        .context("Application identity library is not initialized")?;
    let records: BTreeMap<String, Vec<NativeIdentity>> =
        crate::files::read_or_default(&root.join(FILE))?;
    check_owner(&records, app, identity)
}
fn check_owner(
    records: &BTreeMap<String, Vec<NativeIdentity>>,
    app: &str,
    identity: &NativeIdentity,
) -> Result<()> {
    let owner = repository(app);
    anyhow::ensure!(
        !records
            .iter()
            .any(|(repository, values)| repository != &owner
                && values
                    .iter()
                    .any(|known| known.platform == identity.platform
                        && (known.package.eq_ignore_ascii_case(&identity.package)
                            || (!identity.family.is_empty()
                                && known.family.eq_ignore_ascii_case(&identity.family))))),
        "This package identity is already associated with another managed application"
    );
    Ok(())
}
pub fn remember(root: &Path, app: &str, identity: NativeIdentity) -> Result<()> {
    anyhow::ensure!(valid(&identity), "Invalid package identity");
    let _lock = crate::platform::Lock::take("Local\\CraftAppsManagerIdentity")?;
    let mut records: BTreeMap<String, Vec<NativeIdentity>> =
        crate::files::read_or_default(&root.join(FILE))?;
    check_owner(&records, app, &identity)?;
    let owner = repository(app);
    let values = records.entry(owner).or_default();
    values.retain(|existing| {
        !(existing.platform == identity.platform && existing.package == identity.package)
    });
    values.push(identity);
    crate::files::write_json(&root.join(FILE), &records)?;
    *CURRENT.write().unwrap() = Loaded {
        root: Some(root.to_path_buf()),
        records,
    };
    Ok(())
}
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().any(char::is_control)
        && !name.contains(['/', '\\', ':'])
        && name != "."
        && name != ".."
}
fn valid(identity: &NativeIdentity) -> bool {
    if identity
        .launch
        .as_ref()
        .is_some_and(|launch| identity.platform != "linux" || !launch.valid())
    {
        return false;
    }
    if !identity.family.is_empty()
        && (identity.platform != "windows" || uuid::Uuid::parse_str(&identity.family).is_err())
    {
        return false;
    }
    if identity.package.is_empty() || identity.package.chars().any(char::is_control) {
        return false;
    }
    match identity.platform.as_str() {
        "windows" => {
            safe_name(&identity.executable)
                && identity.executable.to_ascii_lowercase().ends_with(".exe")
                && (uuid::Uuid::parse_str(&identity.package).is_ok()
                    || identity.package.strip_prefix("exe:").is_some_and(|value| {
                        serde_json::from_str::<[String; 3]>(value).is_ok_and(
                            |[scope, key, folder]| {
                                ["HKCU/64", "HKCU/32", "HKLM/64", "HKLM/32"]
                                    .contains(&scope.as_str())
                                    && safe_name(&key)
                                    && !folder.is_empty()
                            },
                        )
                    }))
        }
        "macos" => {
            safe_name(&identity.executable)
                && identity.executable.ends_with(".app")
                && identity
                    .package
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
        }
        "linux" => {
            identity.package.as_bytes()[0].is_ascii_alphanumeric()
                && identity
                    .package
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"+.-_".contains(&c))
                && identity.executable.starts_with('/')
                && !identity.executable.contains(['\\', '\0', '\r', '\n'])
                && identity
                    .executable
                    .split('/')
                    .skip(1)
                    .all(|part| !part.is_empty() && part != "." && part != "..")
        }
        _ => false,
    }
}
pub fn remember_portable(folder: &Path, app: &str, executable: &Path) -> Result<()> {
    let relative = executable
        .strip_prefix(folder)
        .context("Executable is outside the package")?;
    anyhow::ensure!(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
        "Invalid executable path"
    );
    crate::files::write_json(
        &folder.join(PORTABLE),
        &Portable {
            repository: repository(app),
            executable: relative.into(),
        },
    )
}
pub fn portable(folder: &Path, app: &str) -> Option<PathBuf> {
    let marker = folder.join(PORTABLE);
    if crate::files::linked(&marker).ok()? {
        return None;
    }
    let identity: Portable = crate::files::read_json(&marker).ok()?;
    if identity.repository != repository(app)
        || identity.executable.as_os_str().is_empty()
        || !identity
            .executable
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        return None;
    }
    let path = folder.join(identity.executable);
    let canonical = path.canonicalize().ok()?;
    if !canonical.starts_with(folder.canonicalize().ok()?)
        || !crate::model::is_executable(&canonical)
    {
        return None;
    }
    Some(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_launch_metadata_is_backward_compatible_validated_and_persistent() {
        let legacy =
            r#"{"platform":"linux","package":"different-package","executable":"/opt/app/bin/app"}"#;
        let mut identity: NativeIdentity = serde_json::from_str(legacy).unwrap();
        assert!(identity.launch.is_none());
        assert!(valid(&identity));
        let mut launch = DesktopLaunch::default();
        launch.arguments.push("--safe".into());
        launch
            .environment
            .insert("APP_MODE".into(), "desktop".into());
        identity.launch = Some(launch);
        let root =
            std::env::temp_dir().join(format!("craft-launch-metadata-{}", uuid::Uuid::new_v4()));
        remember(&root, "photocraft", identity.clone()).unwrap();
        load(&root);
        assert_eq!(native("photocraft"), vec![identity.clone()]);
        identity
            .launch
            .as_mut()
            .unwrap()
            .environment
            .insert("BAD=KEY".into(), "value".into());
        assert!(!valid(&identity));
        assert!(remember(&root, "photocraft", identity).is_err());
        load(&root.join("empty"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn linux_identity_accepts_owned_absolute_path_but_rejects_traversal() {
        let root =
            std::env::temp_dir().join(format!("craft-identity-linux-{}", uuid::Uuid::new_v4()));
        let identity = NativeIdentity {
            launch: None,
            family: String::new(),
            platform: "linux".into(),
            package: "different-package".into(),
            executable: "/opt/vendor/bin/main-app".into(),
        };
        assert!(remember(&root, "photocraft", identity.clone()).is_ok());
        assert!(remember(
            &root,
            "photocraft",
            NativeIdentity {
                launch: None,
                family: String::new(),
                executable: "/opt/../etc/shadow".into(),
                ..identity
            }
        )
        .is_err());
        load(&root.join("unused"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_persisted_identity_is_not_returned_as_verified() {
        let root =
            std::env::temp_dir().join(format!("craft-identity-invalid-{}", uuid::Uuid::new_v4()));
        let records = BTreeMap::from([(
            repository("photocraft"),
            vec![NativeIdentity {
                launch: None,
                family: String::new(),
                platform: "windows".into(),
                package: "not-a-product-code".into(),
                executable: "../foreign.exe".into(),
            }],
        )]);
        crate::files::write_json(&root.join(FILE), &records).unwrap();
        load(&root);
        assert!(native("photocraft").is_empty());
        load(&root.join("unused"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_identity_survives_reload_and_stays_scoped_to_repository() {
        let root =
            std::env::temp_dir().join(format!("craft-native-identity-{}", uuid::Uuid::new_v4()));
        let identity = NativeIdentity {
            launch: None,
            family: String::new(),
            platform: "windows".into(),
            package: "{01234567-89AB-CDEF-0123-456789ABCDEF}".into(),
            executable: "DifferentName.exe".into(),
        };
        remember(&root, "photocraft", identity.clone()).unwrap();
        remember(&root, "photocraft", identity.clone()).unwrap();
        load(&root);
        assert_eq!(native("photocraft"), vec![identity]);
        assert!(native("filmcraft").is_empty());
        let changed = NativeIdentity {
            launch: None,
            family: String::new(),
            executable: "Chosen.exe".into(),
            ..native("photocraft")[0].clone()
        };
        remember(&root, "photocraft", changed.clone()).unwrap();
        load(&root);
        assert_eq!(native("photocraft"), vec![changed]);
        load(&root.join("unused"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn portable_identity_survives_moves_and_is_repository_bound() {
        let root = std::env::temp_dir().join(format!("craft-identity-{}", uuid::Uuid::new_v4()));
        let first = root.join("first");
        std::fs::create_dir_all(first.join("bin")).unwrap();
        let executable = first
            .join("bin")
            .join(crate::model::executable_name("differentname"));
        std::fs::write(&executable, b"fixture").unwrap();
        remember_portable(&first, "photocraft", &executable).unwrap();
        assert_eq!(portable(&first, "photocraft"), Some(executable));
        assert!(portable(&first, "filmcraft").is_none());
        let moved = root.join("moved");
        std::fs::rename(&first, &moved).unwrap();
        assert!(portable(&moved, "photocraft").is_some());
        #[cfg(target_os = "windows")]
        assert!(crate::model::installed_executable(&moved, "photocraft").is_some());
        crate::files::write_json(
            &moved.join(PORTABLE),
            &Portable {
                repository: repository("photocraft"),
                executable: "../outside.exe".into(),
            },
        )
        .unwrap();
        assert!(portable(&moved, "photocraft").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
