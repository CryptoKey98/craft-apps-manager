use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    pub name: String,
    pub version: String,
    pub path: String,
    #[serde(default)]
    pub architecture: String,
    #[serde(default)]
    pub install_kind: String,
    #[serde(default)]
    pub product_code: String,
    /// Verified Windows Installer context; absent in legacy/non-MSI records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_msi_scope: Option<String>,
    #[serde(default)]
    pub installation_label: String,
    #[serde(default)]
    pub package_type: String,
}

impl Installed {
    /// Refresh the app version independently of package registration. Some
    /// per-user Craft MSIs carry portable.txt and update their payload in place.
    pub(crate) fn refresh_self_updated_version(&mut self, executable: Option<&std::path::Path>) {
        use std::path::Path;

        let folder = Path::new(&self.path);
        let replaces_payload = self.install_kind == "portable"
            || (cfg!(target_os = "windows")
                && self.install_kind == "installer"
                && self.windows_msi_scope.as_deref() == Some("user")
                && folder.join("portable.txt").is_file());
        if self.path.is_empty()
            || !replaces_payload
            || !crate::catalog::get(&self.name)
                .is_some_and(|entry| entry.release && !entry.repository.contains('/'))
        {
            return;
        }

        let resolved;
        let executable = if let Some(executable) = executable {
            executable
        } else {
            resolved = crate::model::installed_executable(folder, &self.name);
            let Some(executable) = resolved.as_deref() else {
                return;
            };
            executable
        };
        if crate::files::inside(executable, folder).is_err() {
            return;
        }
        #[cfg(target_os = "linux")]
        let observed = crate::platform::portable_version(folder, executable, &self.name);
        #[cfg(not(target_os = "linux"))]
        let observed = crate::platform::executable_version(executable);
        let Some(observed) = observed else {
            return;
        };
        let Ok(version) = crate::updates::version(&observed) else {
            return;
        };
        if version != [0; 4]
            && crate::updates::version(&self.version).is_ok_and(|saved| version > saved)
        {
            self.version = observed;
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub apps_root: String,
    pub apps: Vec<Installed>,
    #[serde(default)]
    pub installations: Vec<Installed>,
    #[serde(default)]
    pub portable_label_counters: std::collections::BTreeMap<String, u32>,
}

#[cfg(test)]
mod scope_compatibility_tests {
    use super::*;

    #[test]
    fn older_records_load_without_inventing_an_msi_scope() {
        let old =
            serde_json::json!({"name":"filmcraft","version":"1.0","path":"C:/Apps/FilmCraft"});
        let installed: Installed = serde_json::from_value(old).unwrap();
        assert!(installed.windows_msi_scope.is_none());
        assert!(serde_json::to_value(installed)
            .unwrap()
            .get("windows_msi_scope")
            .is_none());
    }

    #[test]
    fn verified_scope_survives_inventory_round_trip() {
        let installed = Installed {
            windows_msi_scope: Some("machine".into()),
            ..Default::default()
        };
        let restored: Installed =
            serde_json::from_value(serde_json::to_value(installed).unwrap()).unwrap();
        assert_eq!(restored.windows_msi_scope.as_deref(), Some("machine"));
    }
}
