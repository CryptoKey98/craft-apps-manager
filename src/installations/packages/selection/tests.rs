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
