//! Conservative matching for user-added release downloads.
use crate::model::{Asset, Preferences, Release};
#[cfg(target_os = "windows")]
use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub template: String,
    pub os: String,
    pub architecture: String,
    pub format: String,
    #[serde(default)]
    pub architecture_confirmed: bool,
}
impl Rule {
    pub fn matches_preferences(&self, prefs: &Preferences) -> bool {
        self.os == crate::model::release_os()
            && self.architecture == prefs.architecture
            && self.format == prefs.release_format
    }
    pub fn same_target(&self, other: &Self) -> bool {
        self.os == other.os
            && self.architecture == other.architecture
            && self.format == other.format
    }
    pub fn new(asset: &Asset, version: &str, prefs: &Preferences) -> Self {
        Self {
            template: asset.name.replace(version, "{version}"),
            architecture_confirmed: architecture_unspecified(&asset.name),
            os: crate::model::release_os().into(),
            architecture: prefs.architecture.clone(),
            format: prefs.release_format.clone(),
        }
    }
}

pub fn candidates<'a>(release: &'a Release, prefs: &Preferences) -> Vec<&'a Asset> {
    release
        .assets
        .iter()
        .filter(|asset| compatible_with_unknown(&asset.name, prefs, true))
        .collect()
}
#[cfg(all(test, target_os = "windows"))]
fn compatible(name: &str, prefs: &Preferences) -> bool {
    compatible_with_unknown(name, prefs, false)
}
pub fn architecture_unspecified(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let tokens: Vec<_> = name.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    !tokens.iter().any(|token| {
        [
            "x64",
            "amd64",
            "win64",
            "64bit",
            "arm64",
            "aarch64",
            "i386",
            "i686",
            "win32",
            "32bit",
            "x86",
            "universal",
            "arm",
            "armv6",
            "armv7",
            "armv7l",
            "armhf",
            "armel",
            "riscv64",
            "mips",
            "mips64",
            "ppc64",
            "ppc64le",
            "s390x",
        ]
        .contains(token)
    }) && !name.contains("x86_64")
        && !name.contains("x86-64")
}
pub fn needs_confirmation(asset: &Asset, prefs: &Preferences) -> bool {
    let name = asset.name.to_ascii_lowercase();
    architecture_unspecified(&name)
        || (prefs.release_format == "installer" && name.ends_with(".exe") && !installer_name(&name))
}
fn installer_name(name: &str) -> bool {
    name.contains("setup")
        || name
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|token| ["installer", "install"].contains(&token))
}
fn compatible_with_unknown(name: &str, prefs: &Preferences, allow_unknown: bool) -> bool {
    let lower = name.to_ascii_lowercase();
    let tokens: Vec<_> = lower.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    let has = |words: &[&str]| tokens.iter().any(|token| words.contains(token));
    let library_prefix = name
        .strip_prefix("Lib")
        .and_then(|rest| rest.chars().next())
        .is_some_and(|next| next.is_ascii_uppercase());
    if library_prefix
        || tokens
            .iter()
            .any(|token| token.len() > 3 && token.ends_with("cli"))
    {
        return false;
    }
    if has(&[
        "cli",
        "console",
        "commandline",
        "library",
        "libraries",
        "lib",
        "headers",
        "debug",
        "symbols",
        "pdb",
        "pdbs",
        "dsym",
        "dsyms",
        "dbsym",
        "source",
        "sources",
        "sdk",
        "dev",
        "development",
    ]) || lower.contains("sourcecode")
        || lower.contains("checksums")
    {
        return false;
    }
    let x64 = has(&["x64", "amd64", "win64", "64bit"])
        || lower.contains("x86_64")
        || lower.contains("x86-64");
    let arm64 = has(&["arm64", "aarch64"]);
    let x86 = has(&["i386", "i686", "win32", "32bit"]) || (has(&["x86"]) && !x64);
    let universal = has(&["universal"]);
    let wanted = match prefs.architecture.as_str() {
        "x86" => x86 && !x64 && !arm64,
        "arm64" => arm64 && !x64 && !x86,
        _ => x64 && !arm64 && !x86,
    };
    if !(wanted
        || (allow_unknown && architecture_unspecified(name))
        || (cfg!(target_os = "macos") && universal))
    {
        return false;
    }
    let windows = has(&["windows", "win", "win32", "win64"]);
    let mac = has(&["macos", "mac", "osx", "darwin"]);
    let linux = has(&["linux", "ubuntu", "debian", "fedora"]);
    if cfg!(target_os = "windows") {
        if mac || linux {
            return false;
        }
        if prefs.release_format == "installer" {
            if has(&["portable"]) || lower.contains("no-installer") {
                return false;
            }
            // EXE is a Windows package; the architecture and exclusions above
            // still prevent wrong-system, portable, CLI and development downloads.
            lower.ends_with(".msi") || lower.ends_with(".exe")
        } else {
            lower.ends_with(".zip") && (windows || has(&["portable"]))
        }
    } else if cfg!(target_os = "macos") {
        !windows && !linux && lower.ends_with(".dmg")
    } else {
        if windows || mac {
            return false;
        }
        if prefs.release_format == "installer" {
            crate::installers::installer_extension()
                .is_ok_and(|ext| lower.ends_with(&ext.to_ascii_lowercase()))
        } else {
            lower.ends_with(".appimage")
        }
    }
}

pub fn select<'a>(
    release: &'a Release,
    rule: &Rule,
    prefs: &Preferences,
) -> anyhow::Result<&'a Asset> {
    anyhow::ensure!(
        rule.matches_preferences(prefs),
        "Choose a download for the current system and package type."
    );
    let version = crate::updates::release_version(&release.tag_name)?;
    let expected = rule.template.replace("{version}", &version);
    let matches: Vec<_> = candidates(release, prefs)
        .into_iter()
        .filter(|asset| {
            asset.name.eq_ignore_ascii_case(&expected)
                && (!architecture_unspecified(&asset.name) || rule.architecture_confirmed)
        })
        .collect();
    anyhow::ensure!(
        matches.len() == 1,
        "The download naming changed. Open Install or Update to choose a compatible download."
    );
    Ok(matches[0])
}

pub fn select_for_entry<'a>(
    release: &'a Release,
    entry: &crate::catalog::Entry,
    prefs: &Preferences,
) -> anyhow::Result<&'a Asset> {
    if let Some(rule) = entry
        .download_rules
        .iter()
        .chain(entry.download_rule.iter())
        .find(|rule| rule.matches_preferences(prefs))
    {
        return select(release, rule, prefs);
    }
    let assets = candidates(release, prefs);
    anyhow::ensure!(
        !assets.is_empty(),
        "No compatible {} download for {}",
        prefs.release_format,
        prefs.architecture
    );
    anyhow::ensure!(
        assets.len() == 1 && !needs_confirmation(assets[0], prefs),
        "Choose a download for this package type in the Install or Update window."
    );
    Ok(assets[0])
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    #[test]
    fn sampled_desktop_releases_preserve_package_choices() {
        let installer = Preferences {
            architecture: "x64".into(),
            release_format: "installer".into(),
            ..Default::default()
        };
        let portable = Preferences {
            release_format: "portable".into(),
            ..installer.clone()
        };
        // Release filenames sampled from the publishers' GitHub releases.
        for name in [
            "OBS-Studio-32.2.2-Windows-x64-Installer.exe",
            "npp.8.9.8.1.Installer.x64.exe",
            "npp.8.9.8.1.Installer.x64.msi",
            "rustdesk-1.5.0-x86_64.msi",
            "dbeaver-ce-26.2.2-windows-x86_64.exe",
        ] {
            assert!(compatible(name, &installer), "{name}");
            assert!(!compatible(name, &portable), "{name}");
        }
        for name in [
            "OBS-Studio-32.2.2-Windows-x64.zip",
            "npp.8.9.8.1.portable.x64.zip",
            "dbeaver-ce-26.2.2-windows-x86_64.zip",
        ] {
            assert!(compatible(name, &portable), "{name}");
            assert!(!compatible(name, &installer), "{name}");
        }
        assert!(!compatible(
            "OBS-Studio-32.2.2-Windows-x64-PDBs.zip",
            &portable
        ));
        assert!(!compatible("npp.8.9.8.1.portable.arm64.zip", &portable));
        let uncertain = Asset {
            name: "rustdesk-1.5.0-x86_64.exe".into(),
            size: 1,
            digest: None,
            browser_download_url: String::new(),
        };
        assert!(needs_confirmation(&uncertain, &installer));
    }
    #[test]
    fn explicitly_unsupported_architectures_are_not_unknown_downloads() {
        let prefs = Preferences {
            architecture: "x64".into(),
            release_format: "installer".into(),
            ..Default::default()
        };
        for name in [
            "Example-armv7-setup.exe",
            "Example-armhf-setup.exe",
            "Example-riscv64-setup.exe",
            "Example-mips64-setup.exe",
        ] {
            assert!(!compatible_with_unknown(name, &prefs, true), "{name}");
        }
    }
    #[test]
    fn common_names_reject_wrong_architecture_source_and_other_systems() {
        let prefs = Preferences {
            release_format: "installer".into(),
            architecture: "x64".into(),
            ..Default::default()
        };
        for name in [
            "OBS-Studio-32.2.2-Windows-x64-Installer.exe",
            "ExampleSetup-amd64.exe",
            "Example-win64.msi",
            "Example-x86_64-setup.exe",
        ] {
            assert!(compatible(name, &prefs), "{name}");
        }
        for name in [
            "Example-arm64-setup.exe",
            "Example-x86-setup.exe",
            "Example-linux-x64.exe",
            "Example-Windows-x64-debug.exe",
            "Example-source-x64-setup.exe",
            "Example-setup.exe",
            "Example-x64.zip.sig",
        ] {
            assert!(!compatible(name, &prefs), "{name}");
        }
        let portable = Preferences {
            release_format: "portable".into(),
            ..prefs
        };
        assert!(compatible("OBS-Studio-32.2.2-Windows-x64.zip", &portable));
        assert!(!compatible("Example-linux-x64.zip", &portable));
        assert!(!compatible("Example-Windows-x64-PDBs.zip", &portable));
    }
    #[test]
    fn generic_packages_accept_architecture_only_exe_and_exclude_cli_libraries() {
        let prefs = Preferences {
            architecture: "x64".into(),
            release_format: "installer".into(),
            ..Default::default()
        };
        assert!(compatible("Example-x86_64.exe", &prefs));
        assert!(!compatible("Example-CLI-windows-x86-64.exe", &prefs));
        assert!(!compatible("Example-macos-x86_64.exe", &prefs));
        assert!(!compatible("Example-portable-x86_64.exe", &prefs));
        let portable = Preferences {
            release_format: "portable".into(),
            ..prefs
        };
        assert!(compatible("Example-1.2.3-Win_GUI-x64.zip", &portable));
        assert!(!compatible("ExampleCLI-1.2.3-win-x64.zip", &portable));
        assert!(!compatible("LibExample-1.2.3-win-x64.zip", &portable));
        assert!(!compatible("Example-library-win-x64.zip", &portable));
        assert!(compatible("LibreOffice-portable-x64.zip", &portable));
    }
    #[test]
    fn unlabeled_download_requires_selection_and_remembers_architecture() {
        let prefs = Preferences {
            architecture: "x64".into(),
            release_format: "installer".into(),
            ..Default::default()
        };
        let asset = Asset {
            name: "ExampleSetup.exe".into(),
            size: 1,
            digest: None,
            browser_download_url: String::new(),
        };
        let release = Release {
            tag_name: "v1.2.3".into(),
            name: None,
            draft: false,
            prerelease: false,
            assets: vec![asset.clone()],
        };
        assert_eq!(candidates(&release, &prefs).len(), 1);
        assert!(needs_confirmation(&asset, &prefs));
        let mut entry = crate::catalog::builtin().remove(0);
        assert!(select_for_entry(&release, &entry, &prefs).is_err());
        entry.download_rules = vec![Rule::new(&asset, "1.2.3", &prefs)];
        assert_eq!(
            select_for_entry(&release, &entry, &prefs).unwrap().name,
            asset.name
        );
        let arm = Preferences {
            architecture: "arm64".into(),
            ..prefs.clone()
        };
        assert!(select_for_entry(&release, &entry, &arm).is_err());
        entry.download_rules[0].architecture_confirmed = false;
        assert!(select_for_entry(&release, &entry, &prefs).is_err());
    }
    #[test]
    fn padded_versions_follow_future_download_names() {
        let prefs = Preferences {
            architecture: "x64".into(),
            release_format: "installer".into(),
            ..Default::default()
        };
        let asset = Asset {
            name: "ExampleSetup-1.135.06055-x64.exe".into(),
            size: 1,
            digest: None,
            browser_download_url: String::new(),
        };
        let rule = Rule::new(
            &asset,
            &crate::updates::release_version("1.135.06055").unwrap(),
            &prefs,
        );
        assert!(rule.template.contains("{version}"));
        let release = Release {
            tag_name: "1.136.00001".into(),
            name: None,
            draft: false,
            prerelease: false,
            assets: vec![Asset {
                name: "ExampleSetup-1.136.00001-x64.exe".into(),
                ..asset
            }],
        };
        assert!(select(&release, &rule, &prefs).is_ok());
    }
    #[test]
    fn choices_are_scoped_to_architecture_and_format() {
        let prefs = Preferences {
            architecture: "x64".into(),
            release_format: "installer".into(),
            ..Default::default()
        };
        let asset = Asset {
            name: "Example-1.2.3-Windows-x64-Installer.exe".into(),
            size: 5,
            digest: None,
            browser_download_url: String::new(),
        };
        let rule = Rule::new(&asset, "1.2.3", &prefs);
        let x86 = Preferences {
            architecture: "x86".into(),
            ..prefs.clone()
        };
        let portable = Preferences {
            release_format: "portable".into(),
            ..prefs.clone()
        };
        assert!(rule.matches_preferences(&prefs));
        assert!(!rule.matches_preferences(&x86));
        assert!(!rule.matches_preferences(&portable));
        let mut entry = crate::catalog::builtin().remove(0);
        entry.download_rules = vec![rule];
        let mut release = Release {
            tag_name: "v1.2.3".into(),
            name: None,
            draft: false,
            prerelease: false,
            assets: vec![
                asset.clone(),
                Asset {
                    name: "Example-1.2.3-Windows-x86-Installer.exe".into(),
                    ..asset.clone()
                },
                Asset {
                    name: "Example-1.2.3-Windows-x64.zip".into(),
                    ..asset.clone()
                },
            ],
        };
        assert_eq!(
            select_for_entry(&release, &entry, &prefs).unwrap().name,
            asset.name
        );
        assert!(select_for_entry(&release, &entry, &x86)
            .unwrap()
            .name
            .contains("x86"));
        assert!(select_for_entry(&release, &entry, &portable)
            .unwrap()
            .name
            .ends_with(".zip"));
        release.assets.push(Asset {
            name: "Example-1.2.3-Windows-x86-Setup.exe".into(),
            ..asset
        });
        assert!(select_for_entry(&release, &entry, &x86).is_err());
        assert!(select_for_entry(&release, &entry, &prefs).is_ok());
        assert!(!compatible(
            "Example-1.2.3-Windows-x64-Portable.exe",
            &prefs
        ));
    }
    #[test]
    fn saved_choice_follows_versions_without_switching_downloads() {
        let prefs = Preferences {
            release_format: "installer".into(),
            architecture: "x64".into(),
            ..Default::default()
        };
        let asset = Asset {
            name: "Example-1.2.3-Windows-x64-Installer.exe".into(),
            size: 5,
            digest: None,
            browser_download_url: String::new(),
        };
        let rule = Rule::new(&asset, "1.2.3", &prefs);
        let mut release = Release {
            tag_name: "v1.2.4".into(),
            name: None,
            draft: false,
            prerelease: false,
            assets: vec![Asset {
                name: "Example-1.2.4-Windows-x64-Installer.exe".into(),
                ..asset
            }],
        };
        assert!(select(&release, &rule, &prefs).is_ok());
        release.assets[0].name = "Example-1.2.4-Windows-x64-Portable.exe".into();
        assert!(select(&release, &rule, &prefs).is_err());
    }
}

/// Inspect the PE headers without loading or running the downloaded executable.
#[cfg(target_os = "windows")]
fn pe_info(path: &std::path::Path) -> anyhow::Result<(&'static str, u16)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos)?;
    anyhow::ensure!(&dos[..2] == b"MZ", "Download is not a Windows executable");
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    file.seek(SeekFrom::Start(offset))?;
    let mut pe = [0u8; 24];
    file.read_exact(&mut pe)?;
    anyhow::ensure!(&pe[..4] == b"PE\0\0", "Invalid Windows executable");
    let mut architecture = match u16::from_le_bytes([pe[4], pe[5]]) {
        0x014c => "x86",
        0x8664 => "x64",
        0xaa64 => "arm64",
        _ => anyhow::bail!("Unsupported executable architecture"),
    };
    anyhow::ensure!(
        u16::from_le_bytes([pe[20], pe[21]]) >= 70,
        "Missing executable metadata"
    );
    let optional_size = u16::from_le_bytes([pe[20], pe[21]]) as usize;
    anyhow::ensure!(optional_size <= 4096, "Invalid executable metadata size");
    let mut optional = vec![0u8; optional_size];
    file.read_exact(&mut optional)?;
    anyhow::ensure!(
        [0x10b, 0x20b].contains(&u16::from_le_bytes([optional[0], optional[1]])),
        "Invalid executable metadata"
    );
    // IL-only PE32 assemblies can run in a 64-bit CLR even though COFF says
    // I386. Honor CLR Requires32Bit/Prefers32Bit, rather than the installer
    // registry view or the bootstrap executable's apparent machine type.
    if architecture == "x86" && optional.len() >= 216 {
        let clr_rva = u32::from_le_bytes(optional[208..212].try_into().unwrap());
        let directories = u32::from_le_bytes(optional[92..96].try_into().unwrap());
        if directories >= 15 && clr_rva != 0 {
            let sections = u16::from_le_bytes([pe[6], pe[7]]);
            anyhow::ensure!(sections <= 96, "Invalid executable section count");
            let mut clr_offset = None;
            for _ in 0..sections {
                let mut section = [0u8; 40];
                file.read_exact(&mut section)?;
                let rva = u32::from_le_bytes(section[12..16].try_into().unwrap());
                let raw_size = u32::from_le_bytes(section[16..20].try_into().unwrap());
                if clr_rva
                    .checked_sub(rva)
                    .is_some_and(|delta| delta <= raw_size.saturating_sub(20))
                    && raw_size >= 20
                {
                    let raw = u32::from_le_bytes(section[20..24].try_into().unwrap());
                    clr_offset = Some(u64::from(raw) + u64::from(clr_rva - rva));
                }
            }
            let clr_offset = clr_offset.context("Invalid CLR metadata location")?;
            file.seek(SeekFrom::Start(clr_offset))?;
            let mut clr = [0u8; 20];
            file.read_exact(&mut clr)?;
            let flags = u32::from_le_bytes(clr[16..20].try_into().unwrap());
            if flags & 1 != 0 && flags & (2 | 0x20000) == 0 {
                let host = std::env::var("PROCESSOR_ARCHITEW6432")
                    .or_else(|_| std::env::var("PROCESSOR_ARCHITECTURE"))
                    .unwrap_or_default();
                architecture = if host.eq_ignore_ascii_case("ARM64") {
                    "arm64"
                } else if cfg!(target_arch = "x86_64") || host.eq_ignore_ascii_case("AMD64") {
                    "x64"
                } else {
                    "x86"
                };
            }
        }
    }
    Ok((
        architecture,
        u16::from_le_bytes([optional[68], optional[69]]),
    ))
}
#[cfg(target_os = "windows")]
pub(crate) fn windows_executable_architecture(
    path: &std::path::Path,
) -> anyhow::Result<&'static str> {
    pe_info(path).map(|(architecture, _)| architecture)
}
#[cfg(target_os = "windows")]
pub fn verify_desktop_executable(path: &std::path::Path, architecture: &str) -> anyhow::Result<()> {
    let (actual, subsystem) = pe_info(path)?;
    anyhow::ensure!(
        subsystem == 2,
        "This download is a command-line tool, not a Windows desktop application"
    );
    anyhow::ensure!(actual == architecture, "The download is {actual}, but {architecture} was selected. Choose the matching architecture.");
    Ok(())
}
/// Do not run an arbitrary EXE as an installer merely because of its extension.
#[cfg(target_os = "windows")]
pub fn verify_installer_executable(path: &std::path::Path) -> anyhow::Result<()> {
    use std::io::Read;
    pe_info(path)?;
    let identifiers = [
        "Inno Setup Setup Data",
        "InnoSetup",
        "NullsoftInst",
        "WixBundle",
        "WiX Burn",
        "InstallShield",
        "Advanced Installer",
        "Squirrel.Update",
    ];
    let markers: Vec<Vec<u8>> = identifiers
        .iter()
        .flat_map(|text| {
            [
                text.as_bytes().to_vec(),
                text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            ]
        })
        .collect();
    let mut file = std::fs::File::open(path)?;
    let mut tail = Vec::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        tail.extend_from_slice(&buffer[..read]);
        if markers
            .iter()
            .any(|marker| tail.windows(marker.len()).any(|window| window == marker))
        {
            return Ok(());
        }
        let keep = tail.len().saturating_sub(128);
        tail.drain(..keep);
    }
    anyhow::bail!("This EXE is a standalone program or an unrecognized installer. It was not run. Choose a supported installer package instead.")
}

#[cfg(all(test, target_os = "windows"))]
mod executable_validation_tests {
    use super::*;
    #[test]
    fn managed_anycpu_is_not_confused_with_required_or_preferred_x86() {
        let path = std::env::temp_dir().join(format!("craft-clr-{}.exe", uuid::Uuid::new_v4()));
        let mut bytes = vec![0; 512];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[68..70].copy_from_slice(&0x14cu16.to_le_bytes());
        bytes[70..72].copy_from_slice(&1u16.to_le_bytes());
        bytes[84..86].copy_from_slice(&224u16.to_le_bytes());
        bytes[88..90].copy_from_slice(&0x10bu16.to_le_bytes());
        bytes[156..158].copy_from_slice(&2u16.to_le_bytes());
        bytes[180..184].copy_from_slice(&16u32.to_le_bytes());
        bytes[296..300].copy_from_slice(&0x2000u32.to_le_bytes());
        bytes[324..328].copy_from_slice(&0x2000u32.to_le_bytes());
        bytes[328..332].copy_from_slice(&160u32.to_le_bytes());
        bytes[332..336].copy_from_slice(&352u32.to_le_bytes());
        for flags in [3u32, 0x20001, 0] {
            bytes[368..372].copy_from_slice(&flags.to_le_bytes());
            std::fs::write(&path, &bytes).unwrap();
            assert_eq!(windows_executable_architecture(&path).unwrap(), "x86");
        }
        bytes[368..372].copy_from_slice(&1u32.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        let actual = windows_executable_architecture(&path).unwrap();
        if cfg!(target_arch = "x86_64") {
            assert_eq!(actual, "x64");
        }
        assert!(verify_desktop_executable(&path, actual).is_ok());
        bytes[296..300].copy_from_slice(&0xfffffff0u32.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert!(windows_executable_architecture(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
    fn fixture(machine: u16, subsystem: u16, extra: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("craft-exe-{}.exe", uuid::Uuid::new_v4()));
        let mut bytes = vec![0; 158];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[68..70].copy_from_slice(&machine.to_le_bytes());
        bytes[84..86].copy_from_slice(&70u16.to_le_bytes());
        bytes[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        bytes[156..158].copy_from_slice(&subsystem.to_le_bytes());
        bytes.extend_from_slice(extra);
        std::fs::write(&path, bytes).unwrap();
        path
    }
    #[test]
    fn actual_binary_type_blocks_console_apps_and_wrong_architecture() {
        let console = fixture(0x8664, 3, b"ordinary program");
        assert!(verify_installer_executable(&console).is_err());
        assert!(verify_desktop_executable(&console, "x64").is_err());
        let gui = fixture(0x8664, 2, b"ordinary desktop program");
        assert!(verify_desktop_executable(&gui, "x64").is_ok());
        assert!(verify_desktop_executable(&gui, "arm64").is_err());
        assert!(verify_installer_executable(&gui).is_err());
        std::fs::remove_file(console).unwrap();
        std::fs::remove_file(gui).unwrap();
    }
    #[test]
    fn installer_markers_cross_scan_boundaries_and_do_not_execute_files() {
        let mut extra = vec![0; 65530 - 158];
        extra.extend_from_slice(b"Inno Setup Setup Data (6.4.2)");
        let installer = fixture(0x014c, 2, &extra);
        assert!(verify_installer_executable(&installer).is_ok());
        std::fs::remove_file(installer).unwrap();
    }
}
