//! Signed, platform-specific manager updates. Package URLs are taken from the
//! verified GitHub release rather than from executable manifest instructions.
use crate::{
    model::{Asset, Release},
    self_update::Available,
    updates,
};
use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

pub const BUILD_REVISION: u32 = match u32::from_str_radix(env!("CRAFT_BUILD_REVISION"), 10) {
    Ok(value) => value,
    Err(_) => panic!("Invalid compiled build revision"),
};

pub fn display_version(version: &str, revision: u32) -> String {
    if revision >= 2 {
        format!("{version} · Rev. {revision}")
    } else {
        version.to_owned()
    }
}

#[derive(Deserialize, Serialize)]
pub struct Envelope {
    pub payload: String,
    pub signature: String,
}

#[derive(Deserialize, Serialize)]
pub struct Manifest {
    pub schema: u32,
    pub repository: String,
    pub version: String,
    pub expires: i64,
    pub packages: Vec<Package>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Package {
    pub platform: String,
    pub architecture: String,
    pub format: String,
    pub revision: u32,
    pub asset: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default)]
    pub package_version: Option<String>,
}

fn hex<const N: usize>(text: &str) -> Result<[u8; N]> {
    anyhow::ensure!(
        text.len() == N * 2 && text.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid update signature encoding"
    );
    let mut bytes = [0; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)?;
    }
    Ok(bytes)
}

pub fn verify(
    envelope: &Envelope,
    public_key: &str,
    repository: &str,
    version: &str,
    now: i64,
) -> Result<Manifest> {
    anyhow::ensure!(
        envelope.payload.len() <= 128 * 1024,
        "Update manifest is too large"
    );
    let key = VerifyingKey::from_bytes(&hex::<32>(public_key.trim())?)?;
    let signature = Signature::from_bytes(&hex::<64>(&envelope.signature)?);
    key.verify_strict(envelope.payload.as_bytes(), &signature)
        .context("Update manifest signature did not verify")?;
    let manifest: Manifest = serde_json::from_str(&envelope.payload)?;
    anyhow::ensure!(
        manifest.schema == 1
            && manifest.repository == repository
            && manifest.version == version
            && manifest.expires > now,
        "Update manifest is expired or belongs to another release"
    );
    anyhow::ensure!(manifest.packages.len() <= 64, "Too many update packages");
    let mut keys = std::collections::BTreeSet::new();
    for package in &manifest.packages {
        anyhow::ensure!(
            package.revision > 0 && package.size > 0,
            "Invalid update package revision or size"
        );
        hex::<32>(&package.sha256)?;
        anyhow::ensure!(
            keys.insert((&package.platform, &package.architecture, &package.format)),
            "Duplicate update target"
        );
        crate::files::safe_relative(&package.asset)?;
        anyhow::ensure!(
            !package.asset.contains(['/', '\\']),
            "Update asset must be a filename"
        );
    }
    Ok(manifest)
}

pub fn select(
    manifest: &Manifest,
    release: &Release,
    target: (&str, &str, &str),
    current: (&str, u32),
) -> Result<Option<Available>> {
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let Some(package) = manifest.packages.iter().find(|package| {
        (
            package.platform.as_str(),
            package.architecture.as_str(),
            package.format.as_str(),
        ) == target
    }) else {
        return Ok(None);
    };
    let latest_version = updates::version(&manifest.version)?;
    let installed_version = updates::version(current.0)?;
    if latest_version < installed_version
        || latest_version == installed_version && package.revision <= current.1
    {
        return Ok(None);
    }
    let mut asset: Asset = release
        .assets
        .iter()
        .find(|asset| asset.name == package.asset)
        .cloned()
        .context("Revision package is not published yet")?;
    anyhow::ensure!(
        asset.size == package.size
            && asset.browser_download_url.starts_with(&format!(
                "https://github.com/{}/releases/download/{}/",
                manifest.repository, release.tag_name
            )),
        "Revision asset has an unexpected size or location"
    );
    let digest = format!("sha256:{}", package.sha256.to_ascii_lowercase());
    if let Some(published) = &asset.digest {
        anyhow::ensure!(
            published.eq_ignore_ascii_case(&digest),
            "Revision asset checksum disagrees with signed metadata"
        );
    }
    asset.digest = Some(digest);
    let extension = match target.2 {
        "zip" => ".zip",
        "msi" => ".msi",
        "deb" => ".deb",
        "rpm" => ".rpm",
        "arch" => ".pkg.tar.zst",
        _ => bail!("Unsupported update format"),
    };
    anyhow::ensure!(
        asset.name.ends_with(extension),
        "Revision package format does not match this installation"
    );
    Ok(Some(Available {
        version: manifest.version.clone(),
        revision: package.revision,
        package_version: package.package_version.clone(),
        asset,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    #[test]
    fn version_display_omits_the_baseline_revision() {
        assert_eq!(display_version("0.8.3", 1), "0.8.3");
        assert_eq!(display_version("0.8.3", 2), "0.8.3 · Rev. 2");
    }
    #[test]
    fn friendly_revision_filenames_are_selected_from_signed_metadata() {
        let (mut manifest, mut release) = fixture();
        let name = "Craft Apps Manager-0.8.3-Rev-2-linux-x64.zip";
        manifest.packages[0].asset = name.into();
        release.assets[0].name = name.into();
        release.assets[0].browser_download_url = format!(
            "https://github.com/owner/manager/releases/download/v0.8.3/{}",
            name.replace(' ', "%20")
        );
        let offered = select(&manifest, &release, ("linux", "x64", "zip"), ("0.8.3", 1))
            .unwrap()
            .unwrap();
        assert_eq!(offered.asset.name, name);
        assert_eq!(offered.revision, 2);
    }
    fn hex_text(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
    fn fixture() -> (Manifest, Release) {
        let asset = Asset {
            name: "manager-r2-linux-x64.zip".into(),
            size: 123,
            digest: Some(format!("sha256:{}", "ab".repeat(32))),
            browser_download_url:
                "https://github.com/owner/manager/releases/download/v0.8.3/manager-r2-linux-x64.zip"
                    .into(),
        };
        let manifest = Manifest {
            schema: 1,
            repository: "owner/manager".into(),
            version: "0.8.3".into(),
            expires: 1000,
            packages: vec![Package {
                platform: "linux".into(),
                architecture: "x64".into(),
                format: "zip".into(),
                revision: 2,
                asset: asset.name.clone(),
                size: asset.size,
                sha256: "ab".repeat(32),
                package_version: None,
            }],
        };
        let release = Release {
            tag_name: "v0.8.3".into(),
            assets: vec![asset],
            name: None,
            draft: false,
            prerelease: false,
        };
        (manifest, release)
    }
    #[test]
    fn signed_metadata_rejects_tampering_expiration_and_wrong_repository() {
        let (manifest, _) = fixture();
        let key = SigningKey::from_bytes(&[42; 32]);
        let payload = serde_json::to_string(&manifest).unwrap();
        let mut envelope = Envelope {
            signature: hex_text(&key.sign(payload.as_bytes()).to_bytes()),
            payload,
        };
        let public = hex_text(&key.verifying_key().to_bytes());
        assert!(verify(&envelope, &public, "owner/manager", "0.8.3", 999).is_ok());
        assert!(verify(&envelope, &public, "owner/manager", "0.8.3", 1000).is_err());
        assert!(verify(&envelope, &public, "another/manager", "0.8.3", 999).is_err());
        envelope.payload.push(' ');
        assert!(verify(&envelope, &public, "owner/manager", "0.8.3", 999).is_err());
    }
    #[test]
    fn revisions_skip_intermediate_builds_and_leave_other_platforms_alone() {
        let (manifest, release) = fixture();
        let linux = ("linux", "x64", "zip");
        assert_eq!(
            select(&manifest, &release, linux, ("0.8.2", 1))
                .unwrap()
                .unwrap()
                .revision,
            2
        );
        assert_eq!(
            select(&manifest, &release, linux, ("0.8.3", 1))
                .unwrap()
                .unwrap()
                .revision,
            2
        );
        assert!(select(&manifest, &release, linux, ("0.8.3", 2))
            .unwrap()
            .is_none());
        assert!(select(&manifest, &release, linux, ("0.8.4", 1))
            .unwrap()
            .is_none());
        assert!(
            select(&manifest, &release, ("windows", "x64", "zip"), ("0.8.3", 1))
                .unwrap()
                .is_none()
        );
        assert!(select(
            &manifest,
            &release,
            ("macos", "universal", "zip"),
            ("0.8.3", 1)
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn windows_installer_and_macos_revisions_match_only_their_package_targets() {
        let (mut manifest, mut release) = fixture();
        for (platform, architecture, format) in
            [("windows", "x64", "msi"), ("macos", "universal", "zip")]
        {
            let name = format!("manager-{platform}-{architecture}-r2.{format}");
            let mut package = manifest.packages[0].clone();
            package.platform = platform.into();
            package.architecture = architecture.into();
            package.format = format.into();
            package.asset = name.clone();
            let mut asset = release.assets[0].clone();
            asset.name = name.clone();
            asset.browser_download_url =
                format!("https://github.com/owner/manager/releases/download/v0.8.3/{name}");
            manifest.packages.push(package);
            release.assets.push(asset);
            assert_eq!(
                select(
                    &manifest,
                    &release,
                    (platform, architecture, format),
                    ("0.8.3", 1)
                )
                .unwrap()
                .unwrap()
                .revision,
                2
            );
        }
        assert!(
            select(&manifest, &release, ("windows", "x86", "msi"), ("0.8.3", 1))
                .unwrap()
                .is_none()
        );
        assert!(
            select(&manifest, &release, ("windows", "x64", "zip"), ("0.8.3", 1))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn missing_or_changed_assets_fail_without_offering_an_update() {
        let (manifest, mut release) = fixture();
        release.assets[0].size += 1;
        assert!(select(&manifest, &release, ("linux", "x64", "zip"), ("0.8.3", 1)).is_err());
        release.assets[0].size -= 1;
        release.assets[0].digest = Some(format!("sha256:{}", "00".repeat(32)));
        assert!(select(&manifest, &release, ("linux", "x64", "zip"), ("0.8.3", 1)).is_err());
        release.assets.clear();
        assert!(select(&manifest, &release, ("linux", "x64", "zip"), ("0.8.3", 1)).is_err());
    }
}
