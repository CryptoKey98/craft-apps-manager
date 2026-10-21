use super::*;

pub(super) fn expected_asset_url(value: &str, repository: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    let Some((owner, repo)) = repository.split_once('/') else {
        return false;
    };
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return false;
    }
    let mut segments = url.path().trim_start_matches('/').split('/');
    segments
        .next()
        .is_some_and(|s| s.eq_ignore_ascii_case(owner))
        && segments
            .next()
            .is_some_and(|s| s.eq_ignore_ascii_case(repo))
        && segments.next() == Some("releases")
        && segments.next() == Some("download")
        && segments.next().is_some_and(|s| !s.is_empty())
        && segments.next().is_some_and(|s| !s.is_empty())
}

/// Compare stable numeric or timestamp-based release versions.
pub fn version(v: &str) -> Result<[u64; 4]> {
    let text = version_text(v)?;
    if let Some((date, rest)) = text.split_once('-') {
        let time = rest.split('-').next().context("Missing release time")?;
        chrono::NaiveDateTime::parse_from_str(&format!("{date}{time}"), "%Y%m%d%H%M%S")
            .context("Invalid release timestamp")?;
        return Ok([date.parse()?, time.parse()?, 0, 0]);
    }
    let parts: Vec<u64> = text
        .split('.')
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    Ok([
        parts[0],
        parts[1],
        *parts.get(2).unwrap_or(&0),
        *parts.get(3).unwrap_or(&0),
    ])
}

pub(super) fn version_text(v: &str) -> Result<&str> {
    static TAG: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = TAG.get_or_init(|| {
        regex::Regex::new(
            r"^(?:[A-Za-z][A-Za-z0-9_-]*[-_])?v?([0-9]+\.[0-9]+(?:\.[0-9]+){0,2})(?:rel)?$",
        )
        .unwrap()
    });
    if let Some(captures) = pattern.captures(v) {
        return Ok(captures.get(1).unwrap().as_str());
    }
    static STAMP: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    if STAMP
        .get_or_init(|| regex::Regex::new(r"^[0-9]{8}-[0-9]{6}-[0-9a-fA-F]{7,40}$").unwrap())
        .is_match(v)
    {
        return Ok(v);
    }
    bail!("Unsupported version: {v}")
}

/// Preserve version spelling (including leading zeros) for download patterns.
pub fn release_version(tag: &str) -> Result<String> {
    version(tag)?;
    Ok(version_text(tag)?.to_string())
}

/// Unknown metadata stays actionable; only a confirmed incompatible release disables Install.
pub fn cached_download_available(paths: &Paths, app: &str, prefs: &Preferences) -> Option<bool> {
    let entry = crate::catalog::get(app)?;
    let release: Release = Network::cached_json(
        &paths.root,
        &format!(
            "https://api.github.com/repos/{}/releases/latest",
            crate::model::repository(app)
        ),
    )?;
    if release.draft || release.prerelease || release_version(&release.tag_name).is_err() {
        return None;
    }
    let _ = entry;
    Some(
        !crate::install_options::available(&release, app, prefs).is_empty()
            || !crate::downloads::candidates(&release, prefs).is_empty(),
    )
}

pub fn check_app(paths: &Paths, app: &str) -> Result<Option<String>> {
    crate::model::valid_app(app)?;
    let installed = crate::apps::installed(paths, app)?;
    let release: Release = Network::new(&paths.root)?.json(&format!(
        "https://api.github.com/repos/{}/releases/latest",
        crate::model::repository(app)
    ))?;
    check_installed_release(&release, app, &installed, &paths.read_preferences()?)
}

/// Recompare known latest-release metadata against the actual installed version, without HTTP.
pub fn cached_installation_check(
    paths: &Paths,
    installed: &crate::model::Installed,
    prefs: &Preferences,
) -> Option<Result<Option<String>>> {
    let release: Release = Network::cached_json(
        &paths.root,
        &format!(
            "https://api.github.com/repos/{}/releases/latest",
            crate::model::repository(&installed.name)
        ),
    )?;
    Some(check_installed_release(
        &release,
        &installed.name,
        installed,
        prefs,
    ))
}

pub(super) fn check_installed_release(
    release: &Release,
    app: &str,
    installed: &crate::model::Installed,
    prefs: &Preferences,
) -> Result<Option<String>> {
    if release.draft || release.prerelease {
        bail!("No stable release available");
    }
    if version(&release.tag_name)? > version(&installed.version)? {
        select_asset(
            release,
            app,
            &crate::install_options::for_installed(prefs, installed)?,
        )?;
        Ok(Some(release_version(&release.tag_name)?))
    } else {
        Ok(None)
    }
}

pub fn installed_check_targets(config: &crate::model::Config) -> Vec<crate::model::Installed> {
    let apps = crate::model::apps();
    config
        .apps
        .iter()
        .filter(|app| apps.contains(&app.name) && !app.path.is_empty() && !app.version.is_empty())
        .cloned()
        .collect()
}

/// Chooses the release file for this system, architecture and release format.
pub fn select_asset<'a>(r: &'a Release, name: &str, p: &Preferences) -> Result<&'a Asset> {
    let release = release_version(&r.tag_name)?;
    let entry = crate::catalog::get(name).context("Unknown app")?;
    if entry.scheme == crate::catalog::Scheme::Generic {
        return crate::downloads::select_for_entry(r, &entry, p);
    }
    if entry.scheme == crate::catalog::Scheme::Tauri {
        return tauri_asset(r, &entry, &release, p)
            .or_else(|_| crate::downloads::select_for_entry(r, &entry, p));
    }
    // Newest names first, like the repository name the release now uses.
    let mut known = crate::catalog::names(name);
    known.reverse();
    if let Some(asset) = asset_named(r, &known, p)? {
        return Ok(asset);
    }
    // A release from the app's own repository whose files use one new name is
    // the same app renamed (printcraft → pdfcraft).
    let renamed: std::collections::BTreeSet<String> = r
        .assets
        .iter()
        .filter_map(|a| crate::catalog::parse_asset(&a.name))
        .filter(|a| {
            a.scheme == crate::catalog::Scheme::Craft
                && a.version == release
                && a.os == crate::model::release_os()
                && !known.iter().any(|k| k.eq_ignore_ascii_case(&a.prefix))
        })
        .map(|a| a.prefix)
        .collect();
    if renamed.len() == 1 {
        if let Some(asset) = asset_named(r, &renamed.into_iter().collect::<Vec<_>>(), p)? {
            return Ok(asset);
        }
    }
    // Published naming conventions can change. Use the same conservative
    // package/OS/architecture matching as custom repositories, never an arbitrary
    // first asset when multiple downloads or an unlabeled architecture exist.
    crate::downloads::select_for_entry(r, &entry, p)
}

/// Release file whose name starts with one of `prefixes`.
pub(super) fn asset_named<'a>(
    r: &'a Release,
    prefixes: &[String],
    p: &Preferences,
) -> Result<Option<&'a Asset>> {
    let release = release_version(&r.tag_name)?;
    // Different projects spell the same target differently: `x64`, `x86_64`
    // and `amd64` all denote a 64-bit Intel build.
    let os = crate::model::release_os();
    let architecture = crate::model::release_arch(&p.architecture);
    let architectures: Vec<&str> = match architecture {
        "x64" => vec!["x64", "x86_64", "amd64"],
        "x86" => vec!["x86", "i686", "i386"],
        "arm64" => vec!["arm64", "aarch64"],
        other => vec![other],
    };
    let mut names = Vec::new();
    for asset_name in prefixes {
        for arch in &architectures {
            let prefix = format!("{asset_name}-{release}-{os}-{arch}");
            let package_names = if cfg!(target_os = "macos") {
                // The same DMG serves both formats: portable copies its app bundle
                // into the library, installer copies it into Applications.
                vec![format!("{prefix}.dmg")]
            } else if cfg!(target_os = "linux") {
                vec![format!(
                    "{prefix}{}",
                    if p.release_format == "installer" {
                        crate::installers::installer_extension()?
                    } else {
                        ".AppImage"
                    }
                )]
            } else if p.release_format == "installer" {
                vec![format!("{prefix}.msi"), format!("{prefix}.exe")]
            } else {
                vec![format!("{prefix}-portable.zip")]
            };
            names.extend(package_names);
        }
        #[cfg(target_os = "linux")]
        if p.release_format == "installer"
            && crate::installers::package_kind()? == crate::installers::PackageKind::Arch
        {
            let start = format!("{asset_name}-{release}-");
            let end = format!(
                "-{}.pkg.tar.zst",
                crate::model::release_arch(&p.architecture)
            );
            let mut candidates = Vec::new();
            for asset in &r.assets {
                if let Some(release) = asset
                    .name
                    .strip_prefix(&start)
                    .and_then(|value| value.strip_suffix(&end))
                {
                    if release.split('.').all(|part| {
                        !part.is_empty() && part.bytes().all(|value| value.is_ascii_digit())
                    }) {
                        candidates.push(asset.name.clone());
                    }
                }
            }
            if candidates.len() > 1 {
                bail!("Ambiguous Arch release assets");
            }
            names.extend(candidates);
        }
    }
    names.dedup();
    for n in names {
        let assets: Vec<_> = r
            .assets
            .iter()
            .filter(|a| a.name.eq_ignore_ascii_case(&n))
            .collect();
        if assets.len() > 1 {
            bail!("Ambiguous release assets");
        }
        if let Some(a) = assets.first() {
            return Ok(Some(a));
        }
    }
    Ok(None)
}

/// ArtCraft-style releases: `ArtCraft_0.41.0_x64_en-US.msi`, `ArtCraft_0.41.0_universal.dmg`.
pub(super) fn tauri_asset<'a>(
    r: &'a Release,
    entry: &crate::catalog::Entry,
    release: &str,
    p: &Preferences,
) -> Result<&'a Asset> {
    let prefix = regex::escape(&entry.asset_prefix);
    let version = regex::escape(release);
    let arch = match p.architecture.as_str() {
        "x86" => "x86",
        "arm64" => "(?:arm64|aarch64)",
        _ => "x64",
    };
    let patterns: Vec<String> = if cfg!(target_os = "macos") {
        vec![
            format!("^{prefix}_{version}_universal\\.dmg$"),
            format!("^{prefix}_{version}_{arch}\\.dmg$"),
        ]
    } else if cfg!(target_os = "windows") {
        if p.release_format != "installer" {
            bail!(
                "{} is only published as an installer. Choose Installer in Settings.",
                entry.title
            );
        }
        vec![
            format!("^{prefix}_{version}_{arch}_[A-Za-z]{{2}}-[A-Za-z]{{2}}\\.msi$"),
            format!("^{prefix}_{version}_{arch}-setup\\.exe$"),
        ]
    } else {
        #[cfg(target_os = "linux")]
        {
            let kind = if p.release_format == "installer" {
                crate::installers::package_kind()?
            } else {
                crate::installers::PackageKind::Debian
            };
            tauri_linux_patterns(&prefix, &version, p, kind)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Vec::new()
        }
    };
    for pattern in patterns {
        let pattern = regex::Regex::new(&pattern)?;
        let assets: Vec<_> = r
            .assets
            .iter()
            .filter(|a| pattern.is_match(&a.name))
            .collect();
        if assets.len() > 1 {
            bail!("Ambiguous release assets");
        }
        if let Some(asset) = assets.first() {
            return Ok(asset);
        }
    }
    bail!(
        "This {} release has no {} package for this system.",
        entry.title,
        p.release_format
    )
}

/// Finds the release file matching Craft-style naming
/// (`prefix-version-os-arch`) for the given prefixes, format and
/// architecture, without needing a catalog entry.
pub fn find_asset<'a>(
    release: &'a Release,
    prefixes: &[String],
    release_format: &str,
    architecture: &str,
) -> Option<&'a Asset> {
    let prefs = Preferences {
        release_format: release_format.into(),
        architecture: architecture.into(),
        ..Preferences::default()
    };
    asset_named(release, prefixes, &prefs).ok().flatten()
}

/// The Tauri asset prefix (`Foo` in `Foo_1.2.3_x64-setup.exe`) of a release
/// installable on this machine, if the release uses Tauri-style file names.
pub fn tauri_prefix(
    release: &Release,
    version: &str,
    release_format: &str,
    architecture: &str,
) -> Option<String> {
    let arch = match architecture {
        "x86" => "x86",
        "arm64" => "(?:arm64|aarch64)",
        _ => "x64",
    };
    let version = regex::escape(version);
    let patterns: Vec<String> = if cfg!(target_os = "macos") {
        vec![
            format!("^([A-Za-z0-9]+)_{version}_universal\\.dmg$"),
            format!("^([A-Za-z0-9]+)_{version}_{arch}\\.dmg$"),
        ]
    } else if cfg!(target_os = "windows") {
        if release_format != "installer" {
            return None;
        }
        vec![
            format!("^([A-Za-z0-9]+)_{version}_{arch}_[A-Za-z]{{2}}-[A-Za-z]{{2}}\\.msi$"),
            format!("^([A-Za-z0-9]+)_{version}_{arch}-setup\\.exe$"),
        ]
    } else {
        // Linux packaging varies per distribution; verified separately.
        Vec::new()
    };
    for pattern in patterns {
        let pattern = regex::Regex::new(&pattern).ok()?;
        let mut prefix = None;
        for asset in &release.assets {
            if let Some(captured) = pattern.captures(&asset.name) {
                if prefix.replace(captured[1].to_string()).is_some() {
                    // More than one app in this release: not attributable.
                    return None;
                }
            }
        }
        if prefix.is_some() {
            return prefix;
        }
    }
    None
}

#[cfg(target_os = "linux")]
pub(super) fn tauri_linux_patterns(
    prefix: &str,
    version: &str,
    prefs: &Preferences,
    kind: crate::installers::PackageKind,
) -> Vec<String> {
    use crate::installers::PackageKind;
    let (deb, native, image) = match prefs.architecture.as_str() {
        "x86" => ("i386", "i686", "i386"),
        "arm64" => ("arm64", "aarch64", "aarch64"),
        _ => ("amd64", "x86_64", "amd64"),
    };
    if prefs.release_format != "installer" {
        return vec![format!("(?i)^{prefix}_{version}_{image}\\.AppImage$")];
    }
    match kind {
        PackageKind::Debian => vec![format!("(?i)^{prefix}_{version}_{deb}\\.deb$")],
        PackageKind::Rpm => vec![format!(
            "(?i)^{prefix}-{version}-[0-9][A-Za-z0-9.]*\\.{native}\\.rpm$"
        )],
        PackageKind::Arch => vec![format!(
            "(?i)^{prefix}-{version}-[0-9][A-Za-z0-9.]*-{native}\\.pkg\\.tar\\.zst$"
        )],
    }
}

/// Recent published stable releases compatible with the chosen platform and format.
/// Older assets without a published digest remain unavailable for verified installs.
pub fn available_versions(paths: &Paths, app: &str) -> Result<Vec<Release>> {
    crate::model::valid_app(app)?;
    let prefs = crate::install_options::preferred(&paths.read_preferences()?, app);
    let network = Network::new(&paths.root)?;
    let releases: Vec<Release> = network.json(&format!(
        "https://api.github.com/repos/{}/releases?per_page=100",
        crate::model::repository(app)
    ))?;
    Ok(compatible_versions(releases, app, &prefs))
}

pub(super) fn compatible_versions(
    releases: Vec<Release>,
    app: &str,
    prefs: &Preferences,
) -> Vec<Release> {
    let mut versions: Vec<_> = releases
        .into_iter()
        .filter(|r| {
            !r.draft
                && !r.prerelease
                && release_version(&r.tag_name).is_ok()
                && crate::install_options::available(r, app, prefs)
                    .iter()
                    .any(|option| {
                        let context = crate::install_options::apply(prefs, option);
                        crate::downloads::candidates(r, &context).iter().any(|a| {
                            a.digest
                                .as_deref()
                                .and_then(|d| d.strip_prefix("sha256:"))
                                .is_some_and(|d| {
                                    d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit())
                                })
                        }) || select_asset(r, app, &context).is_ok_and(|a| {
                            a.digest
                                .as_deref()
                                .and_then(|d| d.strip_prefix("sha256:"))
                                .is_some_and(|d| {
                                    d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit())
                                })
                        })
                    })
        })
        .collect();
    versions.sort_by_key(|r| std::cmp::Reverse(version(&r.tag_name).unwrap()));
    versions
}

/// Bulk updates operate only on a usable installation in the selected format.
pub fn installed_for_updates(app: &crate::model::Installed, _prefs: &Preferences) -> bool {
    !app.path.is_empty()
        && !app.version.is_empty()
        && crate::model::installed_executable(Path::new(&app.path), &app.name).is_some()
}
