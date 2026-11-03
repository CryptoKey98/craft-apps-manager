use super::*;
use anyhow::{ensure, Context, Result};
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
pub const DOMAIN: &[u8] = b"craft-apps-manager/app-release-catalog/v1\n";
pub const MAX_ROOT: usize = 256 * 1024;
pub const MAX_APP: usize = 8 * 1024 * 1024;
/// Reject user-controlled links, while accepting the root-owned aliases macOS
/// uses for ordinary temporary folders. Never canonicalize away a link below
/// the selected folder before inspecting it.
pub(super) fn unlinked_ancestors(path: &std::path::Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => {
                if !crate::files::linked(ancestor)? {
                    continue;
                }
                #[cfg(target_os = "macos")]
                {
                    use std::os::unix::fs::MetadataExt;
                    let expected = match ancestor.to_str() {
                        Some("/var") => Some(std::path::Path::new("/private/var")),
                        Some("/tmp") => Some(std::path::Path::new("/private/tmp")),
                        _ => None,
                    };
                    if expected.is_some_and(|expected| {
                        std::fs::symlink_metadata(ancestor)
                            .is_ok_and(|metadata| metadata.uid() == 0)
                            && std::fs::read_link(ancestor).is_ok_and(|target| {
                                let target = if target.is_absolute() {
                                    target
                                } else {
                                    ancestor
                                        .parent()
                                        .unwrap_or(std::path::Path::new("/"))
                                        .join(target)
                                };
                                target == expected
                            })
                    }) {
                        continue;
                    }
                }
                anyhow::bail!("Linked catalog path is not allowed: {}", ancestor.display());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
pub fn decode<const N: usize>(text: &str) -> Result<[u8; N]> {
    ensure!(
        text.len() == N * 2 && text.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid hex length or characters"
    );
    let mut bytes = [0; N];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16)?;
    }
    Ok(bytes)
}
pub(crate) fn unique_map<'de, D, V>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    V: serde::Deserialize<'de>,
{
    struct Visitor<V>(std::marker::PhantomData<V>);
    impl<'de, V: serde::Deserialize<'de>> serde::de::Visitor<'de> for Visitor<V> {
        type Value = BTreeMap<String, V>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a map with unique keys")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut access: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut result = BTreeMap::new();
            while let Some((key, value)) = access.next_entry::<String, V>()? {
                if result.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("Duplicate map key"));
                }
                if result.len() > 10000 {
                    return Err(serde::de::Error::custom("Oversized map"));
                }
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(Visitor(std::marker::PhantomData))
}
pub fn canonical_repo(repo: &str) -> Result<String> {
    let parts: Vec<_> = repo.split('/').collect();
    ensure!(
        parts.len() == 2
            && parts.iter().all(|p| !p.is_empty()
                && p.len() <= 100
                && *p != "."
                && *p != ".."
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))),
        "Invalid repository"
    );
    Ok(repo.to_ascii_lowercase())
}
pub fn validate_index(index: &Index) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    ensure!(
        index.schema == 1
            && index.purpose == "app-release-catalog"
            && index.sequence > 0
            && index.sequence <= i64::MAX as u64,
        "Invalid index schema/purpose/sequence"
    );
    ensure!(
        (0..=now + 300).contains(&index.generated_at) && index.repositories.len() <= 1024,
        "Invalid index time or repository count"
    );
    let mut total = 0u64;
    for (repo, entry) in &index.repositories {
        ensure!(canonical_repo(repo)? == *repo, "Noncanonical repository");
        decode::<32>(&entry.sha256)?;
        ensure!(
            entry.sha256 == entry.sha256.to_ascii_lowercase()
                && entry.size > 0
                && entry.size <= MAX_APP as u64
                && entry.checked_at >= 0
                && entry.checked_at <= index.generated_at + 300
                && entry
                    .discovery_added_at
                    .is_none_or(|added| added > 0 && added <= index.generated_at),
            "Invalid app reference"
        );
        total = total
            .checked_add(entry.size)
            .context("Catalog size overflow")?;
    }
    ensure!(
        total <= 128 * 1024 * 1024,
        "Catalog exceeds total size budget"
    );
    Ok(())
}
pub fn verify_index(bytes: &[u8], source: &Source, floor: u64) -> Result<VerifiedIndex> {
    ensure!(
        bytes.len() <= MAX_ROOT && source.schema == 1,
        "Oversized index or unsupported source"
    );
    let envelope: Envelope = serde_json::from_slice(bytes)?;
    let public = source
        .keys
        .get(&envelope.key_id)
        .context("Untrusted catalog key")?;
    let key = VerifyingKey::from_bytes(&decode::<32>(public)?)?;
    let signature = Signature::from_bytes(&decode::<64>(&envelope.signature)?);
    let mut message = DOMAIN.to_vec();
    message.extend_from_slice(envelope.payload.as_bytes());
    key.verify_strict(&message, &signature)
        .context("Invalid catalog signature")?;
    let index: Index = serde_json::from_str(&envelope.payload)?;
    validate_index(&index)?;
    ensure!(index.sequence >= floor, "Catalog sequence rollback");
    Ok(VerifiedIndex { envelope, index })
}
pub fn validate_app(app: &AppCatalog) -> Result<()> {
    ensure!(
        app.schema == 1
            && canonical_repo(&app.repository)? == app.repository
            && app.releases.len() <= 10000
            && app.scopes.len() <= 100000,
        "Invalid app catalog"
    );
    let mut tags = BTreeSet::new();
    for release in &app.releases {
        ensure!(
            !release.draft
                && !release.prerelease
                && !release.tag_name.is_empty()
                && release.tag_name.len() <= 256
                && !release.tag_name.chars().any(char::is_control)
                && tags.insert(&release.tag_name)
                && release.assets.len() <= 256,
            "Invalid/duplicate release"
        );
        let mut names = BTreeSet::new();
        for asset in &release.assets {
            ensure!(
                !asset.name.is_empty()
                    && asset.name.len() <= 256
                    && !asset.name.chars().any(char::is_control)
                    && names.insert(&asset.name)
                    && asset.browser_download_url.len() <= 2048
                    && crate::updates::expected_asset_url(
                        &asset.browser_download_url,
                        &app.repository
                    ),
                "Invalid/duplicate release asset"
            );
            if let Some(digest) = &asset.digest {
                decode::<32>(
                    digest
                        .strip_prefix("sha256:")
                        .context("Invalid asset digest")?,
                )?;
            }
        }
    }
    ensure!(
        app.latest_tag.as_ref().is_none_or(|tag| tags.contains(tag)),
        "Latest tag missing from history"
    );
    let mut evidence = BTreeSet::new();
    for scope in &app.scopes {
        ensure!(
            evidence.insert((
                &scope.tag,
                &scope.asset_name,
                &scope.os,
                &scope.architecture
            )),
            "Duplicate scope evidence"
        );
        ensure!(
            scope.os == "windows"
                && matches!(scope.architecture.as_str(), "x64" | "x86" | "arm64")
                && scope.inspector_version > 0,
            "Unsupported scope target"
        );
        let asset = app
            .releases
            .iter()
            .find(|r| r.tag_name == scope.tag)
            .and_then(|r| r.assets.iter().find(|a| a.name == scope.asset_name))
            .context("Scope evidence has no matching asset")?;
        ensure!(
            asset.name.to_ascii_lowercase().ends_with(".msi")
                && asset.size == scope.size
                && asset.digest.as_deref() == Some(&scope.digest),
            "Scope evidence does not match package bytes"
        );
    }
    Ok(())
}
pub fn verify_app(bytes: &[u8], repository: &str, reference: &AppReference) -> Result<AppCatalog> {
    ensure!(
        bytes.len() <= MAX_APP
            && bytes.len() as u64 == reference.size
            && hash(bytes) == reference.sha256,
        "App catalog size/hash mismatch"
    );
    let app: AppCatalog = serde_json::from_slice(bytes)?;
    ensure!(
        app.repository == canonical_repo(repository)?,
        "App catalog repository mismatch"
    );
    validate_app(&app)?;
    Ok(app)
}
