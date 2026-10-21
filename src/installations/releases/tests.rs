#[test]
fn release_url_matches_repository_case_only() {
    let url = "https://github.com/EcoPasteHub/EcoPaste/releases/download/v1.1.0/EcoPaste_1.1.0_x64-setup.exe";
    assert!(super::expected_asset_url(url, "ecopastehub/ecopaste"));
    assert!(!super::expected_asset_url(url, "another/ecopaste"));
    for invalid in [
        url.replace("github.com", "github.com.example.org"),
        url.replace("https://", "http://"),
        url.replace("/releases/", "/Releases/"),
        url.replace("github.com/", "github.com@evil.example/"),
    ] {
        assert!(!super::expected_asset_url(&invalid, "ecopastehub/ecopaste"));
    }
}
#[cfg(target_os = "linux")]
#[test]
fn tauri_linux_assets_match_distribution_and_architecture() {
    use crate::installers::PackageKind;
    for (architecture, deb, native, image) in [
        ("x64", "amd64", "x86_64", "amd64"),
        ("x86", "i386", "i686", "i386"),
        ("arm64", "arm64", "aarch64", "aarch64"),
    ] {
        let mut prefs = Preferences {
            architecture: architecture.into(),
            release_format: "installer".into(),
            ..Default::default()
        };
        for (kind, name) in [
            (PackageKind::Debian, format!("ArtCraft_0.41.0_{deb}.deb")),
            (PackageKind::Rpm, format!("ArtCraft-0.41.0-1.{native}.rpm")),
            (
                PackageKind::Arch,
                format!("ArtCraft-0.41.0-1-{native}.pkg.tar.zst"),
            ),
        ] {
            let patterns = tauri_linux_patterns("ArtCraft", "0\\.41\\.0", &prefs, kind);
            let regex = regex::Regex::new(&patterns[0]).unwrap();
            assert!(regex.is_match(&name));
            assert!(!regex.is_match("ArtCraft_0.41.0_amd64.AppImage"));
            if architecture != "x64" {
                assert!(!regex.is_match("ArtCraft_0.41.0_amd64.deb"));
            }
        }
        prefs.release_format = "portable".into();
        let patterns = tauri_linux_patterns("ArtCraft", "0\\.41\\.0", &prefs, PackageKind::Arch);
        let regex = regex::Regex::new(&patterns[0]).unwrap();
        assert!(regex.is_match(&format!("ArtCraft_0.41.0_{image}.AppImage")));
        assert!(!regex.is_match(&format!("ArtCraft_0.41.0_{deb}.deb")));
    }
}
use super::*;
#[test]
fn new_apps_select_matching_release_formats_and_reject_missing_architectures() {
    for app in [
        "wordcraft",
        "gridcraft",
        "deckcraft",
        "cadcraft",
        "soundcraft",
    ] {
        for format in ["portable", "installer"] {
            let preferences = Preferences {
                architecture: "x64".into(),
                release_format: format.into(),
                ..Default::default()
            };
            let suffix = if cfg!(target_os = "macos") {
                ".dmg"
            } else if format == "installer" {
                crate::installers::installer_extension().unwrap()
            } else if cfg!(target_os = "windows") {
                "-portable.zip"
            } else {
                ".AppImage"
            };
            let name = format!(
                "{app}-0.3.0-{}-{}{suffix}",
                crate::model::release_os(),
                crate::model::release_arch("x64")
            );
            let release = Release {
                tag_name: "v0.3.0".into(),
                name: None,
                draft: false,
                prerelease: false,
                assets: vec![crate::model::Asset {
                    name: name.clone(),
                    size: 1,
                    digest: None,
                    browser_download_url: format!(
                        "https://github.com/storytold/{app}/releases/download/v0.3.0/{name}"
                    ),
                }],
            };
            assert_eq!(
                select_asset(&release, app, &preferences).unwrap().name,
                name
            );
            if !cfg!(target_os = "macos") {
                let other = Preferences {
                    architecture: "x86".into(),
                    ..preferences
                };
                assert!(select_asset(&release, app, &other).is_err());
            }
        }
    }
}
#[cfg(target_os = "windows")]
#[test]
fn third_party_assets_match_case_and_architecture_aliases() {
    let preferences = Preferences {
        architecture: "x64".into(),
        release_format: "installer".into(),
        ..Default::default()
    };
    let release = |app: &str, tag: &str, asset: &str| Release {
        tag_name: tag.into(),
        name: None,
        draft: false,
        prerelease: false,
        assets: vec![crate::model::Asset {
            name: asset.into(),
            size: 1,
            digest: None,
            browser_download_url: format!(
                "https://github.com/{}/releases/download/{tag}/{asset}",
                crate::model::repository(app)
            ),
        }],
    };
    // Uppercase file names and the `x86_64` spelling still match.
    let photocraft = release("photocraft", "v0.1.2", "PhotoCraft-0.1.2-windows-x64.exe");
    assert_eq!(
        select_asset(&photocraft, "photocraft", &preferences)
            .unwrap()
            .name,
        "PhotoCraft-0.1.2-windows-x64.exe"
    );
    let filmcraft = release("filmcraft", "v0.2.6", "FilmCraft-0.2.6-windows-x86_64.msi");
    assert_eq!(
        select_asset(&filmcraft, "filmcraft", &preferences)
            .unwrap()
            .name,
        "FilmCraft-0.2.6-windows-x86_64.msi"
    );
    // A built-in repository can adopt a different naming convention without
    // needing an app-specific patch. Ambiguity and missing architecture still
    // require an explicit choice rather than silently installing a wrong file.
    let mut changed = release("photocraft", "v0.1.2", "PhotoStudioSetup-amd64.exe");
    assert_eq!(
        select_asset(&changed, "photocraft", &preferences)
            .unwrap()
            .name,
        "PhotoStudioSetup-amd64.exe"
    );
    changed.assets.push(changed.assets[0].clone());
    changed.assets[1].name = "PhotoStudioSetup-x64.msi".into();
    assert!(select_asset(&changed, "photocraft", &preferences).is_err());
    changed.assets.truncate(1);
    changed.assets[0].name = "PhotoStudioSetup.exe".into();
    assert!(select_asset(&changed, "photocraft", &preferences).is_err());
    changed.assets[0].name = "PhotoStudioSetup-arm64.exe".into();
    assert!(select_asset(&changed, "photocraft", &preferences).is_err());
}
#[test]
fn rollback() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&root).unwrap();
    let dest = root.join("app");
    let stage = root.join("new");
    let backup = root.join("backup");
    fs::write(&dest, "old").unwrap();
    fs::write(&stage, "new").unwrap();
    assert!(
        replace_transaction(&stage, &dest, Some(&backup), || bail!("metadata failure")).is_err()
    );
    assert_eq!(fs::read_to_string(&dest).unwrap(), "old");
    assert_eq!(fs::read_to_string(&stage).unwrap(), "new");
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn versions() {
    assert_eq!(version("1.68").unwrap(), version("1.68.0").unwrap());
    assert_eq!(release_version("v1.68").unwrap(), "1.68");
    assert_eq!(release_version("App-1.068").unwrap(), "1.068");
    assert!(version("1.69").unwrap() > version("1.68.9").unwrap());
    assert!(version("1.68-beta").is_err());
    assert!(version("v0.10.0").unwrap() > version("0.9.0").unwrap());
    assert!(version("1.2.3-beta").is_err());
    assert_eq!(version("1.2.3").unwrap(), version("v1.2.3.0").unwrap());
    assert!(version("8.9.8.1").unwrap() > version("8.9.8").unwrap());
    assert!(version("8.9.8.10").unwrap() > version("8.9.8.2").unwrap());
    assert!(version("8.9.9").unwrap() > version("8.9.8.99").unwrap());
    assert_eq!(release_version("v8.9.8.1").unwrap(), "8.9.8.1");
    assert_eq!(release_version("app-v1.2.3.0").unwrap(), "1.2.3.0");
    assert_eq!(release_version("v1.2.3").unwrap(), "1.2.3");
    assert_eq!(release_version("release-5.2.4").unwrap(), "5.2.4");
    assert_eq!(release_version("Audacity-4.0.1").unwrap(), "4.0.1");
    assert_eq!(release_version("some-app_v1.2.3.4").unwrap(), "1.2.3.4");
    assert!(version("release-5.2.4-beta").is_err());
    assert!(version("App-1.2.3-rc1").is_err());
    assert!(version("-1.2.3").is_err());
    assert_eq!(release_version("1.135.06055").unwrap(), "1.135.06055");
    assert_eq!(version("1.2.0003").unwrap(), version("1.2.3").unwrap());
    assert_eq!(release_version("3.6.1rel").unwrap(), "3.6.1");
    assert_eq!(
        release_version("20240203-110809-5046fc22").unwrap(),
        "20240203-110809-5046fc22"
    );
    assert!(
        version("20240203-110810-abcdef12").unwrap() > version("20240203-110809-5046fc22").unwrap()
    );
    assert!(version("20240231-110809-5046fc22").is_err());

    for invalid in [
        "1",
        "1.2.3.4.5",
        "1.2.3.",
        "1.2.3.-1",
        "1.2.3.4-beta",
        "1.2.3.4+build",
        "1.2.3.+1",
        "1.2.3.18446744073709551616",
    ] {
        assert!(version(invalid).is_err(), "{invalid}");
    }
}
