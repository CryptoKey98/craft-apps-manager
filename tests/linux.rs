#![cfg(target_os = "linux")]
use craft_apps_manager::{
    files,
    jobs::Job,
    model::{Asset, BuilderPreferences, Preferences, Release},
    platform, updates,
};
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn linux_x86_selects_i686_appimage_without_accepting_x64() {
    let release = Release {
        tag_name: "v1.2.3".into(), draft: false, prerelease: false,
        assets: vec![Asset {
            name: "filmcraft-1.2.3-linux-i686.AppImage".into(), size: 1,
            browser_download_url: "https://github.com/storytold/filmcraft/releases/download/v1.2.3/filmcraft-1.2.3-linux-i686.AppImage".into(), digest: None,
        }],
    };
    let mut preferences = Preferences {
        architecture: "x86".into(),
        release_format: "portable".into(),
        ..Default::default()
    };
    assert!(updates::select_asset(&release, "filmcraft", &preferences)
        .unwrap()
        .name
        .contains("i686"));
    preferences.architecture = "x64".into();
    assert!(updates::select_asset(&release, "filmcraft", &preferences).is_err());
}

#[test]
fn linux_releases_match_appimage_and_debian_architecture() {
    let release = Release {tag_name:"v1.2.3".into(),draft:false,prerelease:false,assets:vec![
        Asset{name:"filmcraft-1.2.3-linux-x86_64.AppImage".into(),size:1,browser_download_url:"https://github.com/storytold/filmcraft/releases/download/v1.2.3/filmcraft-1.2.3-linux-x86_64.AppImage".into(),digest:None},
        Asset{name:"filmcraft-1.2.3-linux-aarch64.deb".into(),size:1,browser_download_url:"https://github.com/storytold/filmcraft/releases/download/v1.2.3/filmcraft-1.2.3-linux-aarch64.deb".into(),digest:None},
        Asset{name:"filmcraft-1.2.3-linux-aarch64.rpm".into(),size:1,browser_download_url:"https://github.com/storytold/filmcraft/releases/download/v1.2.3/test.rpm".into(),digest:None},
        Asset{name:"filmcraft-1.2.3-windows-x64.msi".into(),size:1,browser_download_url:"https://github.com/storytold/filmcraft/releases/download/v1.2.3/test.msi".into(),digest:None}
    ]};
    let mut prefs = Preferences {
        architecture: "x64".into(),
        ..Default::default()
    };
    assert!(updates::select_asset(&release, "filmcraft", &prefs)
        .unwrap()
        .name
        .ends_with(".AppImage"));
    prefs.release_format = "installer".into();
    assert!(updates::select_asset(&release, "filmcraft", &prefs).is_err());
    prefs.architecture = "arm64".into();
    assert!(updates::select_asset(&release, "filmcraft", &prefs)
        .unwrap()
        .name
        .ends_with(&format!(
            "aarch64{}",
            craft_apps_manager::installers::installer_extension().unwrap()
        )));
}

#[test]
fn operation_lock_is_exclusive_and_released_on_drop() {
    let name = format!("test-{}", uuid::Uuid::new_v4());
    let lock = platform::Lock::take(&name).unwrap();
    assert!(platform::Lock::take(&name).is_err());
    drop(lock);
    assert!(platform::Lock::take(&name).is_ok());
}

#[test]
fn extracted_source_retains_executable_bits_without_special_permissions() {
    use std::io::Write;
    let root = std::env::temp_dir().join(format!("craft-linux-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let archive = root.join("source.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    zip.start_file(
        "build.sh",
        zip::write::SimpleFileOptions::default().unix_permissions(0o4755),
    )
    .unwrap();
    zip.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
    zip.finish().unwrap();
    let job = Job::new(root.join("build.log"), &BuilderPreferences::default());
    files::extract_zip(&archive, &root.join("extracted"), &job).unwrap();
    assert_eq!(
        fs::metadata(root.join("extracted/build.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o755
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn normal_uninstall_does_not_require_profile_cleanup_or_reacquire_lock() {
    use craft_apps_manager::{apps, model::Paths};
    let root = std::env::temp_dir().join(format!("craft-uninstall-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root.clone(), None);
    let app = paths.at("releases/filmcraft");
    fs::create_dir_all(&app).unwrap();
    fs::write(app.join("filmcraft"), "fixture").unwrap();
    let mut config = paths.config().unwrap();
    let record = config
        .apps
        .iter_mut()
        .find(|a| a.name == "filmcraft")
        .unwrap();
    record.version = "0.2.1".into();
    record.path = app.display().to_string();
    record.install_kind = "portable".into();
    paths.save_config(&config).unwrap();
    assert!(apps::uninstall_with_profile(&paths, "filmcraft", true).is_err());
    assert!(app.is_dir());
    apps::uninstall_with_profile(&paths, "filmcraft", false).unwrap();
    assert!(!app.exists());
    assert!(apps::installed(&paths, "filmcraft").is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn linux_app_shortcuts_use_desktop_extension_and_repair_old_names() {
    use craft_apps_manager::{apps, model::Paths};
    let root = std::env::temp_dir().join(format!("craft-shortcuts-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root.clone(), None);
    let app = paths.at("releases/filmcraft/filmcraft");
    fs::create_dir_all(app.parent().unwrap()).unwrap();
    fs::write(&app, "fixture").unwrap();
    let old = paths.at("releases/filmcraft.lnk");
    fs::write(
        &old,
        format!(
            "[Desktop Entry]\nName=Craft Apps Manager\nExec=\"{}\" \n",
            app.display()
        ),
    )
    .unwrap();
    let other = paths.at("releases/unrelated.lnk");
    fs::write(&other, "unrelated file").unwrap();
    apps::repair_linux_shortcuts(&paths).unwrap();
    let shortcut = paths.at("releases/filmcraft.desktop");
    let text = fs::read_to_string(&shortcut).unwrap();
    assert_eq!(platform::shortcut_extension(), "desktop");
    assert!(text.contains("Name=FilmCraft\n"));
    assert!(text.contains("APPIMAGE_EXTRACT_AND_RUN=1"));
    assert_eq!(
        fs::metadata(shortcut).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(!old.exists());
    assert!(other.exists());
    apps::repair_linux_shortcuts(&paths).unwrap();
    fs::remove_dir_all(root).unwrap();
}
