use crate::model::*;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

fn default_arch() -> String {
    MANAGER_ARCH.into()
}

#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub tools: PathBuf,
}

impl Paths {
    pub fn new(root: PathBuf, tools: Option<PathBuf>) -> Self {
        let tools = tools.unwrap_or_else(|| root.join("workspace/tools"));
        Self { root, tools }
    }
    pub fn at(&self, name: impl AsRef<Path>) -> PathBuf {
        self.root.join(name)
    }
    pub fn config(&self) -> Result<Config> {
        #[cfg(target_os = "windows")]
        {
            let preferences = self.read_preferences()?;
            self.config_with_detector(|app| {
                crate::installers::detect_for_architecture(
                    app,
                    Some(&crate::install_options::preferred(&preferences, app).architecture),
                )?
                .map_or_else(
                    || crate::installers::detect(app),
                    |installed| Ok(Some(installed)),
                )
            })
        }
        #[cfg(not(target_os = "windows"))]
        self.config_with_detector(crate::installers::detect)
    }
    fn config_with_detector(
        &self,
        detect: impl Fn(&str) -> Result<Option<Installed>>,
    ) -> Result<Config> {
        if self.at("settings.json").exists() {
            let config: Config = crate::files::read_json(&self.at("settings.json"))?;
            if !Path::new(&config.apps_root).eq(&self.root) {
                bail!("settings.json points to a different data folder. Select that folder in Settings.");
            }
            return self.refresh_config(config, &detect);
        }
        let mut apps = Vec::new();
        for name in self::apps() {
            let name = name.as_str();
            let mut found: Vec<_> = std::fs::read_dir(self.at("releases"))
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| {
                    e.path().is_dir()
                        && e.file_name()
                            .to_string_lossy()
                            .starts_with(&format!("{name}-"))
                })
                .filter_map(|e| {
                    let label = e.file_name().to_string_lossy().into_owned();
                    let version = label
                        .strip_prefix(&format!("{name}-"))?
                        .split(&format!("-{}-", release_os()))
                        .next()?
                        .to_string();
                    crate::updates::version(&version)
                        .ok()
                        .map(|v| (v, version, e.path()))
                })
                .collect();
            found.sort_by_key(|v| v.0);
            let (version, path) = found
                .pop()
                .map(|(_, v, p)| (v, p.to_string_lossy().into_owned()))
                .unwrap_or_default();
            let direct = self.at(format!("releases/{name}"));
            apps.push(Installed {
                installation_label: Default::default(),
                package_type: Default::default(),
                name: name.into(),
                version,
                path: if path.is_empty() && installed_executable(&direct, name).is_some() {
                    direct.to_string_lossy().into_owned()
                } else {
                    path
                },
                architecture: String::new(),
                install_kind: String::new(),
                product_code: String::new(),
            });
        }
        self.refresh_config(
            Config {
                portable_label_counters: Default::default(),
                apps_root: self.root.to_string_lossy().into_owned(),
                apps,
                installations: Vec::new(),
            },
            &detect,
        )
    }
    fn refresh_config(
        &self,
        mut config: Config,
        detect: &impl Fn(&str) -> Result<Option<Installed>>,
    ) -> Result<Config> {
        let preferences = self.read_preferences()?;

        // Add catalog entries when upgrading an existing library without
        // changing saved selections, installations, or the user's app order.
        for name in apps() {
            if !config.apps.iter().any(|app| app.name == name) {
                config.apps.push(Installed {
                    name,
                    architecture: default_arch(),
                    ..Default::default()
                });
            }
        }
        for app in &config.apps {
            if !app.path.is_empty()
                && !config.installations.iter().any(|a| {
                    a.name == app.name
                        && crate::installation_labels::path_identity(&a.path)
                            == crate::installation_labels::path_identity(&app.path)
                        && (a.install_kind == "installer") == (app.install_kind == "installer")
                })
            {
                config.installations.push(app.clone());
            }
        }
        for app in &mut config.apps {
            let selected_path = app.path.clone();
            let selected_kind = app.install_kind.clone();
            let preferred = crate::install_options::preferred(&preferences, &app.name);
            let has_portable = config.installations.iter().any(|copy| {
                copy.name == app.name
                    && copy.install_kind != "installer"
                    && installed_executable(Path::new(&copy.path), &copy.name).is_some()
            });
            let detected = detect(&app.name)?;
            if let Some(native) = &detected {
                config
                    .installations
                    .retain(|copy| !(copy.name == app.name && copy.install_kind == "installer"));
                config.installations.push(native.clone());
            }
            // Legacy discovery supplied a path but no package type/version. Recover
            // only verified portable files: unknown external folders stay unknown,
            // and a detected native installation must never become portable.
            for copy in config.installations.iter_mut().filter(|copy| {
                copy.name == app.name
                    && !copy.path.is_empty()
                    && (copy.install_kind == "portable"
                        || (copy.install_kind.is_empty()
                            && Path::new(&copy.path).starts_with(self.at("releases"))))
                    && detected
                        .as_ref()
                        .is_none_or(|native| Path::new(&native.path) != Path::new(&copy.path))
            }) {
                if let Some(executable) = installed_executable(Path::new(&copy.path), &copy.name) {
                    copy.install_kind = "portable".into();
                    if copy.version.is_empty() {
                        copy.version = crate::platform::executable_version(&executable)
                            .unwrap_or_else(|| "0.0.0".into());
                    }
                    if copy.architecture.is_empty() {
                        copy.architecture =
                            crate::install_options::executable_architecture(&executable)
                                .unwrap_or_default();
                    }
                }
            }
            let library = self.at(format!("releases/{}", app.name));
            if !config
                .installations
                .iter()
                .any(|copy| copy.name == app.name && Path::new(&copy.path) == library)
            {
                if let Some(executable) = installed_executable(&library, &app.name) {
                    config.installations.push(Installed {
                        installation_label: Default::default(),
                        package_type: Default::default(),
                        name: app.name.clone(),
                        path: library.display().to_string(),
                        version: crate::platform::executable_version(&executable)
                            .unwrap_or_else(|| "0.0.0".into()),
                        architecture: crate::install_options::executable_architecture(&executable)
                            .unwrap_or_default(),
                        install_kind: "portable".into(),
                        product_code: String::new(),
                    });
                }
            }
            let library_portable = detected.is_none()
                && installed_executable(&self.at(format!("releases/{}", app.name)), &app.name)
                    .is_some();
            // Native inventory must reflect detection even when a portable
            // installation is selected for display.
            if detected.is_none() {
                config.installations.retain(|record| {
                    !(record.name == app.name && record.install_kind == "installer")
                });
            }
            // The selected copy is installation identity, not pending package preferences.
            // Changing install options must never silently switch an existing copy.
            if let Some(selected) = config.installations.iter().find(|copy| {
                !selected_kind.is_empty()
                    && copy.name == app.name
                    && crate::installation_labels::path_identity(&copy.path)
                        == crate::installation_labels::path_identity(&selected_path)
                    && copy.install_kind == selected_kind
                    && !copy.path.is_empty()
                    && installed_executable(Path::new(&copy.path), &copy.name).is_some()
            }) {
                *app = selected.clone();
                continue;
            }
            let installer =
                preferred.release_format == "installer" || (!has_portable && detected.is_some());
            if installer && (detected.is_some() || (!has_portable && !library_portable)) {
                if let Some(record) = detected {
                    config.installations.retain(|a| {
                        !(a.name == app.name
                            && a.install_kind == "installer"
                            && (a.architecture == record.architecture || a.path == record.path))
                    });
                    config.installations.push(record.clone());
                    *app = record;
                } else {
                    config.installations.retain(|record| {
                        !(record.name == app.name && record.install_kind == "installer")
                    });
                    app.path.clear();
                    app.version.clear();
                    app.product_code.clear();
                    app.install_kind = "installer".into();
                }
            } else if let Some(record) = config
                .installations
                .iter()
                .filter(|a| {
                    a.name == app.name
                        && a.install_kind != "installer"
                        && installed_executable(Path::new(&a.path), &a.name).is_some()
                })
                .max_by_key(|a| installed_architecture_matches(a, &preferred))
            {
                *app = record.clone();
            } else {
                let root = self.at(format!("releases/{}", app.name));
                if let Some(executable) = installed_executable(&root, &app.name) {
                    app.path = root.display().to_string();
                    app.version = crate::platform::executable_version(&executable)
                        .unwrap_or_else(|| "0.0.0".into());
                    app.install_kind = "portable".into();
                    app.product_code.clear();
                    config.installations.push(app.clone());
                } else {
                    app.path.clear();
                    app.version.clear();
                    app.install_kind = "portable".into();
                    app.product_code.clear();
                }
            }
        }
        #[cfg(target_os = "linux")]
        for copy in config.apps.iter_mut().chain(&mut config.installations) {
            if copy.install_kind != "installer" && !copy.path.is_empty() {
                copy.package_type = installed_executable(Path::new(&copy.path), &copy.name)
                    .filter(|executable| is_appimage(executable))
                    .map(|_| "appimage".to_owned())
                    .unwrap_or_default();
            }
        }
        for app in &mut config.apps {
            if app.architecture.is_empty() && !app.path.is_empty() {
                app.architecture = installed_executable(Path::new(&app.path), &app.name)
                    .and_then(|exe| crate::install_options::executable_architecture(&exe))
                    .unwrap_or_default();
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        config.installations.retain(|copy| {
            seen.insert((
                copy.name.clone(),
                copy.install_kind == "installer",
                crate::installation_labels::path_identity(&copy.path),
            ))
        });
        crate::installation_labels::assign(&mut config)?;
        Ok(config)
    }
    pub fn save_config(&self, config: &Config) -> Result<()> {
        let mut config = config.clone();
        for app in &config.apps {
            let installer = app.install_kind == "installer";
            config.installations.retain(|a| {
                !(a.name == app.name
                    && (a.install_kind == "installer") == installer
                    && crate::installation_labels::path_identity(&a.path)
                        == crate::installation_labels::path_identity(&app.path))
            });
            if !app.path.is_empty() {
                config.installations.push(app.clone());
            }
        }
        crate::installation_labels::assign(&mut config)?;
        crate::files::write_json(&self.at("settings.json"), &config)
    }
    /// Read preferences without migrating or changing any file.
    pub fn read_preferences(&self) -> Result<Preferences> {
        let target = self.at("manager-settings.json");
        let legacy = self.at("updater-settings.json");
        let mut p =
            crate::files::read_or_default::<Preferences>(if !target.exists() && legacy.exists() {
                &legacy
            } else {
                &target
            })?;
        p.validate()?;
        Ok(p)
    }
    /// Verified alternate-format facts for display only; action inventory remains unchanged.
    pub fn alternate_installations(&self, config: &Config) -> Result<Vec<Installed>> {
        self.alternates_with_detector(config, crate::installers::detect)
    }
    fn alternates_with_detector(
        &self,
        config: &Config,
        detect: impl Fn(&str) -> Result<Option<Installed>>,
    ) -> Result<Vec<Installed>> {
        let mut records = Vec::new();
        for name in apps() {
            let name = name.as_str();
            let installer = config.apps.iter().any(|app| {
                app.name == name && !app.path.is_empty() && app.install_kind == "installer"
            });
            if !installer {
                if let Some(record) = detect(name)? {
                    records.push(record);
                }
            } else {
                // Native installers may legitimately target the portable library.
                // A directory location alone is not evidence of another installation.
                let native_owns = |folder: &Path| {
                    config
                        .apps
                        .iter()
                        .chain(config.installations.iter())
                        .any(|record| {
                            record.name == name
                                && record.install_kind == "installer"
                                && !record.path.is_empty()
                                && (Path::new(&record.path) == folder
                                    || std::fs::canonicalize(&record.path)
                                        .ok()
                                        .zip(std::fs::canonicalize(folder).ok())
                                        .is_some_and(|(native, candidate)| native == candidate))
                        })
                };
                let recorded = config.installations.iter().find(|a| {
                    a.name == name
                        && a.install_kind != "installer"
                        && !a.path.is_empty()
                        && !native_owns(Path::new(&a.path))
                        && installed_executable(Path::new(&a.path), name).is_some()
                });
                if let Some(record) = recorded {
                    records.push(record.clone());
                } else {
                    let folder = self.at(format!("releases/{name}"));
                    if native_owns(&folder) {
                        continue;
                    }
                    if let Some(executable) = installed_executable(&folder, name) {
                        records.push(Installed {
                            name: name.into(),
                            path: folder.display().to_string(),
                            version: crate::platform::executable_version(&executable)
                                .unwrap_or_else(|| "0.0.0".into()),
                            install_kind: "portable".into(),
                            architecture: default_arch(),
                            ..Default::default()
                        });
                    }
                }
            }
        }
        Ok(records)
    }
    pub fn preferences(&self) -> Result<Preferences> {
        let target = self.at("manager-settings.json");
        let legacy = self.at("updater-settings.json");
        let migrate = !target.exists() && legacy.exists();
        let mut p =
            crate::files::read_or_default::<Preferences>(if migrate { &legacy } else { &target })?;
        let adopted = p.adopt_new_apps()?;
        if migrate {
            crate::files::write_json(&target, &p)?;
            std::fs::remove_file(legacy)?;
        } else if adopted && target.exists() {
            // Remember discovered apps without changing update selections.
            crate::files::write_json(&target, &p)?;
        }
        Ok(p)
    }
    pub fn builder_preferences(&self) -> Result<BuilderPreferences> {
        let mut p =
            crate::files::read_or_default::<BuilderPreferences>(&self.at("builder-settings.json"))?;
        p.log_size_mb = p.log_size_mb.clamp(1, 100);
        p.log_archives = p.log_archives.min(5);
        Ok(p)
    }
}

/// AppImages retain their identifying ELF header even when renamed on install.
#[cfg(any(target_os = "linux", test))]
fn is_appimage(path: &Path) -> bool {
    use std::io::Read;
    let mut header = [0u8; 11];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok()
        && &header[..4] == b"\x7fELF"
        && &header[8..10] == b"AI"
        && matches!(header[10], 1 | 2)
}

#[cfg(test)]
#[path = "detection_tests.rs"]
mod detection_tests;

#[cfg(test)]
mod appimage_header_tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn legacy_copies_recover_appimage_labels_without_changing_selection_or_letters() {
        use super::*;
        let root =
            std::env::temp_dir().join(format!("craft-appimage-migration-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let mut header = [0u8; 64];
        header[..4].copy_from_slice(b"\x7fELF");
        header[8..11].copy_from_slice(b"AI\x02");
        let mut copies = Vec::new();
        for (folder, label, bytes) in [
            ("filmcraft", "A", header.as_slice()),
            ("filmcraft-2", "B", b"\x7fELFordinary".as_slice()),
        ] {
            let folder = paths.at(format!("releases/{folder}"));
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join(executable_name("filmcraft")), bytes).unwrap();
            copies.push(Installed {
                name: "filmcraft".into(),
                path: folder.display().to_string(),
                version: "0.8.2".into(),
                architecture: "x64".into(),
                install_kind: "portable".into(),
                installation_label: label.into(),
                ..Default::default()
            });
        }
        let config = Config {
            apps_root: root.display().to_string(),
            apps: vec![copies[0].clone()],
            installations: copies.clone(),
            portable_label_counters: Default::default(),
        };
        let mut legacy = serde_json::to_value(config).unwrap();
        for field in ["apps", "installations"] {
            for copy in legacy[field].as_array_mut().unwrap() {
                assert!(copy
                    .as_object_mut()
                    .unwrap()
                    .remove("package_type")
                    .is_some());
            }
        }
        crate::files::write_json(&paths.at("settings.json"), &legacy).unwrap();
        let refreshed = paths.config_with_detector(|_| Ok(None)).unwrap();
        let selected = refreshed
            .apps
            .iter()
            .find(|copy| copy.name == "filmcraft")
            .unwrap();
        assert_eq!(selected.path, copies[0].path);
        assert_eq!(selected.package_type, "appimage");
        assert_eq!(selected.installation_label, "A");
        assert!(crate::installation_labels::summary(selected).starts_with("AppImage"));
        paths.save_config(&refreshed).unwrap();
        let reopened = paths.config_with_detector(|_| Ok(None)).unwrap();
        for expected in &copies {
            let copy = reopened
                .installations
                .iter()
                .find(|copy| copy.path == expected.path)
                .unwrap();
            assert_eq!(copy.installation_label, expected.installation_label);
            assert_eq!(
                copy.package_type,
                if expected.installation_label == "A" {
                    "appimage"
                } else {
                    ""
                }
            );
            assert_eq!(copy.install_kind, "portable");
        }
        // A later ordinary portable replacement must not retain a stale AppImage label.
        std::fs::write(
            Path::new(&copies[0].path).join(executable_name("filmcraft")),
            b"\x7fELFordinary",
        )
        .unwrap();
        let replaced = paths.config_with_detector(|_| Ok(None)).unwrap();
        let selected = replaced
            .apps
            .iter()
            .find(|copy| copy.name == "filmcraft")
            .unwrap();
        assert_eq!(selected.package_type, "");
        assert_eq!(selected.installation_label, "A");
        assert!(crate::installation_labels::summary(selected).starts_with("Portable"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn renamed_appimages_are_detected_without_mislabeling_other_elf_files() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("filmcraft");
        for kind in [1u8, 2] {
            let mut header = [0u8; 11];
            header[..4].copy_from_slice(b"\x7fELF");
            header[8..10].copy_from_slice(b"AI");
            header[10] = kind;
            std::fs::write(&file, header).unwrap();
            assert!(super::is_appimage(&file));
        }
        std::fs::write(&file, b"\x7fELFordinary").unwrap();
        assert!(!super::is_appimage(&file));
        std::fs::write(&file, b"short").unwrap();
        assert!(!super::is_appimage(&file));
        std::fs::remove_dir_all(root).unwrap();
    }
}
