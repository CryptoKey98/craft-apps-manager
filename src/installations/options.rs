//! Per-application intent, kept separate from verified installed facts.
use crate::model::{Installed, Preferences, Release};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOptions {
    pub release_format: String,
    pub architecture: String,
}
impl InstallOptions {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            ["portable", "installer"].contains(&self.release_format.as_str())
                && ["x64", "x86", "arm64"].contains(&self.architecture.as_str()),
            "Invalid application installation options"
        );
        Ok(())
    }
    pub fn installed(app: &Installed) -> Option<Self> {
        if app.path.is_empty()
            || !["portable", "installer"].contains(&app.install_kind.as_str())
            || !["x64", "x86", "arm64", "mac", "universal"].contains(&app.architecture.as_str())
        {
            return None;
        }
        Some(Self {
            release_format: app.install_kind.clone(),
            architecture: if ["mac", "universal"].contains(&app.architecture.as_str()) {
                host_architecture()
            } else {
                app.architecture.clone()
            },
        })
    }
}
pub fn identity(app: &str) -> String {
    crate::model::repository(app).to_ascii_lowercase()
}
pub fn host_architecture() -> String {
    if cfg!(target_os = "windows") {
        let architecture = std::env::var("PROCESSOR_ARCHITEW6432")
            .or_else(|_| std::env::var("PROCESSOR_ARCHITECTURE"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        if architecture.contains("arm64") {
            return "arm64".into();
        }
        if architecture.contains("amd64") {
            return "x64".into();
        }
    }
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "x64"
    }
    .into()
}
pub fn host_supports(architecture: &str) -> bool {
    let host = host_architecture();
    architecture == host || (cfg!(target_os = "windows") && host == "x64" && architecture == "x86")
}
pub fn apply(base: &Preferences, options: &InstallOptions) -> Preferences {
    let mut prefs = base.clone();
    prefs.release_format = options.release_format.clone();
    prefs.architecture = options.architecture.clone();
    prefs
}
pub fn preferred(base: &Preferences, app: &str) -> Preferences {
    match base.app_install_options.get(&identity(app)) {
        Some(options) => apply(base, options),
        None => {
            let mut prefs = base.clone();
            prefs.architecture = host_architecture();
            prefs.release_format = if cfg!(target_os = "linux") {
                "portable"
            } else {
                "installer"
            }
            .into();
            prefs
        }
    }
}
pub fn for_installed(base: &Preferences, app: &Installed) -> anyhow::Result<Preferences> {
    let options = InstallOptions::installed(app).ok_or_else(|| anyhow::anyhow!("Installation format or architecture is unknown for {}. Confirm its installation options before updating.", app.name))?;
    Ok(apply(base, &options))
}
/// Only proven combinations are offered; unlabeled assets require the existing explicit download confirmation.
pub fn available(release: &Release, app: &str, base: &Preferences) -> Vec<InstallOptions> {
    let mut options = Vec::new();
    for format in ["installer", "portable"] {
        for architecture in ["x64", "x86", "arm64"] {
            if !host_supports(architecture) {
                continue;
            }
            let option = InstallOptions {
                release_format: format.into(),
                architecture: architecture.into(),
            };
            let prefs = apply(base, &option);
            if crate::updates::select_asset(release, app, &prefs).is_ok()
                || crate::downloads::candidates(release, &prefs)
                    .iter()
                    .any(|asset| !crate::downloads::architecture_unspecified(&asset.name))
            {
                options.push(option);
            }
        }
    }
    options
}
pub fn preserve_or_default(
    options: &[InstallOptions],
    selected: &InstallOptions,
) -> Option<InstallOptions> {
    options
        .iter()
        .find(|option| *option == selected)
        .or_else(|| {
            options.iter().find(|option| {
                option.architecture == host_architecture()
                    && option.release_format == selected.release_format
            })
        })
        .or_else(|| {
            options
                .iter()
                .find(|option| option.architecture == host_architecture())
        })
        .or_else(|| options.first())
        .cloned()
}

/// Recover legacy binary architecture without borrowing a global preference.
pub fn executable_architecture(path: &std::path::Path) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        crate::downloads::windows_executable_architecture(path)
            .ok()
            .map(String::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        use std::io::Read;
        let mut header = [0u8; 20];
        std::fs::File::open(path)
            .ok()?
            .read_exact(&mut header)
            .ok()?;
        if &header[..4] != b"\x7fELF" {
            return None;
        }
        let machine = if header[5] == 1 {
            u16::from_le_bytes([header[18], header[19]])
        } else if header[5] == 2 {
            u16::from_be_bytes([header[18], header[19]])
        } else {
            return None;
        };
        match machine {
            3 => Some("x86".into()),
            62 => Some("x64".into()),
            183 => Some("arm64".into()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_builtin_and_custom_options_follow_assets_and_host() {
        let architecture = host_architecture();
        let extension = crate::installers::installer_extension().unwrap();
        let asset = |name: String| crate::model::Asset {
            name,
            browser_download_url: "https://github.com/example/app/releases/download/v1.2.3/package"
                .into(),
            size: 1,
            digest: Some(format!("sha256:{}", "a".repeat(64))),
        };
        let mut release = Release {
            tag_name: "v1.2.3".into(),
            name: None,
            draft: false,
            prerelease: false,
            assets: vec![
                asset(format!("photocraft-1.2.3-linux-{architecture}.AppImage")),
                asset(format!("photocraft-1.2.3-linux-{architecture}{extension}")),
                asset("Example-Windows-x64.exe".into()),
                asset("Example-linux-mips.AppImage".into()),
            ],
        };
        let prefs = Preferences::default();
        let expected = vec![
            InstallOptions {
                release_format: "installer".into(),
                architecture: architecture.clone(),
            },
            InstallOptions {
                release_format: "portable".into(),
                architecture: architecture.clone(),
            },
        ];
        assert_eq!(available(&release, "photocraft", &prefs), expected);
        release.assets = vec![
            asset(format!("Example-{architecture}.AppImage")),
            asset(format!("Example-{architecture}{extension}")),
        ];
        assert_eq!(available(&release, "photocraft", &prefs), expected);
        release.assets.remove(1);
        assert_eq!(
            available(&release, "photocraft", &prefs),
            vec![expected[1].clone()]
        );
    }
    #[test]
    fn options_are_independent_and_survive_restart_without_legacy_authority() {
        let mut preferences: Preferences = serde_json::from_str(
            r#"{"releaseFormat":"portable","architecture":"x86","notifyUpdates":false}"#,
        )
        .unwrap();
        preferences.app_install_options.insert(
            identity("photocraft"),
            InstallOptions {
                release_format: "portable".into(),
                architecture: "x64".into(),
            },
        );
        preferences.app_install_options.insert(
            identity("filmcraft"),
            InstallOptions {
                release_format: "installer".into(),
                architecture: "x86".into(),
            },
        );
        let serialized = serde_json::to_vec(&preferences).unwrap();
        let reopened: Preferences = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(preferred(&reopened, "photocraft").architecture, "x64");
        assert_eq!(
            preferred(&reopened, "filmcraft").release_format,
            "installer"
        );
        assert!(!reopened.notify_updates);
        let mut unrelated = reopened.clone();
        unrelated.release_format = "installer".into();
        unrelated.architecture = "arm64".into();
        assert_eq!(preferred(&unrelated, "photocraft").architecture, "x64");
        assert_eq!(preferred(&unrelated, "filmcraft").architecture, "x86");
    }
    #[test]
    fn installed_facts_override_pending_intent_and_unknown_facts_block_updates() {
        let prefs = Preferences::default();
        let mut app = Installed {
            name: "photocraft".into(),
            path: "existing".into(),
            install_kind: "portable".into(),
            architecture: "x86".into(),
            ..Default::default()
        };
        let effective = for_installed(&prefs, &app).unwrap();
        assert_eq!(effective.release_format, "portable");
        assert_eq!(effective.architecture, "x86");
        app.architecture.clear();
        assert!(for_installed(&prefs, &app).is_err());
        app.architecture = "x64".into();
        app.install_kind.clear();
        assert!(for_installed(&prefs, &app).is_err());
    }
    #[test]
    fn selection_preserves_valid_choices_and_rejects_missing_pairs() {
        let options = vec![
            InstallOptions {
                release_format: "portable".into(),
                architecture: "x64".into(),
            },
            InstallOptions {
                release_format: "installer".into(),
                architecture: "x64".into(),
            },
            InstallOptions {
                release_format: "portable".into(),
                architecture: "arm64".into(),
            },
        ];
        assert_eq!(
            preserve_or_default(&options, &options[2]),
            Some(options[2].clone())
        );
        let missing = InstallOptions {
            release_format: "installer".into(),
            architecture: "arm64".into(),
        };
        assert!(!options.contains(&missing));
        assert!(options.contains(&preserve_or_default(&options, &missing).unwrap()));
        assert!(preserve_or_default(&[], &missing).is_none());
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn builtin_and_custom_asset_names_produce_only_real_combinations() {
        use crate::model::Asset;
        let asset = |name: &str| Asset {
            name: name.into(),
            browser_download_url: "https://github.com/example/app/releases/download/v1.2.3/package"
                .into(),
            size: 1,
            digest: Some(format!("sha256:{}", "a".repeat(64))),
        };
        let release = Release {
            name: Some("Example".into()),
            tag_name: "v1.2.3".into(),
            draft: false,
            prerelease: false,
            assets: vec![
                asset("photocraft-1.2.3-windows-x64.zip"),
                asset("photocraft-1.2.3-windows-x64.msi"),
                asset("photocraft-1.2.3-windows-arm64.zip"),
            ],
        };
        let prefs = Preferences::default();
        let result = available(&release, "photocraft", &prefs);
        assert!(result
            .iter()
            .any(|o| o.release_format == "portable" && o.architecture == "x64"));
        assert!(result
            .iter()
            .any(|o| o.release_format == "installer" && o.architecture == "x64"));
        assert!(!result
            .iter()
            .any(|o| o.release_format == "installer" && o.architecture == "arm64"));
        let custom = Release {
            assets: vec![
                asset("Example-Setup-Windows-x64.exe"),
                asset("Example-Windows-x64.zip"),
            ],
            ..release.clone()
        };
        assert_eq!(available(&custom, "photocraft", &prefs).len(), 2);
        let incomplete = Release {
            assets: vec![asset("Example-Setup.exe")],
            ..release
        };
        assert!(available(&incomplete, "photocraft", &prefs).is_empty());
    }
}
