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
#[path = "selection/tests.rs"]
mod tests;

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
#[path = "selection/executable_validation_tests.rs"]
mod executable_validation_tests;
