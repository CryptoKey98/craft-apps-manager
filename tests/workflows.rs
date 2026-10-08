use anyhow::{bail, Result};
use craft_apps_updater::{
    backups, builder, files,
    jobs::Job,
    model::{Asset, BuilderPreferences, Paths, Preferences, Release, Source, APPS},
    network::{verify_asset, Network},
    platform, updates,
};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::Ordering,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("craft-rust-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn paths(&self) -> Paths {
        Paths::new(self.0.clone(), None)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn zip(path: &Path, fileset: &[(&str, &str)]) {
    let mut w = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, text) in fileset {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(text.as_bytes()).unwrap();
    }
    w.finish().unwrap();
}
fn job(root: &Path) -> Job {
    Job::new(root.join("logs/test.log"), &BuilderPreferences::default())
}
#[test]
fn portable_photo_profile_survives_uninstall_and_reinstall() {
    let f = Fixture::new();
    let paths = f.paths();
    let prefs = Preferences {
        release_format: "portable".into(),
        ..Default::default()
    };
    files::write_json(&paths.at("updater-settings.json"), &prefs).unwrap();
    let target = paths.at("releases/photocraft");
    fs::create_dir_all(target.join("PhotoCraftData")).unwrap();
    fs::write(target.join("photocraft.exe"), "fixture").unwrap();
    fs::write(
        target.join("PhotoCraftData/preferences.json"),
        "saved profile",
    )
    .unwrap();
    let mut config = paths.config().unwrap();
    let record = config
        .apps
        .iter_mut()
        .find(|a| a.name == "photocraft")
        .unwrap();
    record.path = target.to_string_lossy().into_owned();
    record.version = "0.3.0".into();
    record.install_kind = "portable".into();
    paths.save_config(&config).unwrap();
    let preserved = craft_apps_updater::profiles::preserve_portable(&paths, "photocraft")
        .unwrap()
        .unwrap();
    assert!(preserved.1.join("preferences.json").exists());
    assert!(!target.join("PhotoCraftData").exists());
    fs::remove_dir_all(&target).unwrap();
    fs::create_dir_all(&target).unwrap();
    craft_apps_updater::profiles::restore_portable(&paths, "photocraft", &target).unwrap();
    assert_eq!(
        fs::read_to_string(target.join("PhotoCraftData/preferences.json")).unwrap(),
        "saved profile"
    );
    assert!(!preserved.1.exists());
}
#[test]
fn app_backup_delete_and_restore_are_scoped() {
    let f = Fixture::new();
    let paths = f.paths();
    let prefs = Preferences {
        release_format: "portable".into(),
        ..Default::default()
    };
    files::write_json(&paths.at("updater-settings.json"), &prefs).unwrap();
    let film = paths.at("backups/releases/filmcraft-0.1.0-11111111111111111111111111111111");
    let other = paths.at("backups/releases/designcraft-0.1.0-22222222222222222222222222222222");
    fs::create_dir_all(&film).unwrap();
    fs::create_dir_all(&other).unwrap();
    fs::write(film.join("filmcraft.exe"), "backup executable").unwrap();
    let target = paths.at("releases/filmcraft");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("filmcraft.exe"), "current executable").unwrap();
    let choices = backups::list(&paths, "filmcraft").unwrap();
    assert_eq!(choices.len(), 1);
    backups::restore(&paths, "filmcraft", &choices[0], &job(&f.0)).unwrap();
    assert_eq!(
        fs::read_to_string(target.join("filmcraft.exe")).unwrap(),
        "backup executable"
    );
    assert_eq!(
        paths
            .config()
            .unwrap()
            .apps
            .iter()
            .find(|a| a.name == "filmcraft")
            .unwrap()
            .version,
        "0.1.0"
    );
    assert!(film.exists());
    assert!(backups::delete_selected(
        &paths,
        "filmcraft",
        &[backups::Backup {
            path: other.clone(),
            source: false
        }]
    )
    .is_err());
    assert!(other.exists());
    backups::delete_selected(&paths, "filmcraft", &choices).unwrap();
    assert!(!film.exists());
    backups::clear_app(&paths, "filmcraft").unwrap();
    assert!(!film.exists());
    assert!(other.exists());
    assert!(target.exists());
}
#[test]
fn switching_release_format_remembers_both_installations() {
    let f = Fixture::new();
    let paths = f.paths();
    let mut prefs = Preferences::default();
    files::write_json(&paths.at("updater-settings.json"), &prefs).unwrap();
    let mut config = paths.config().unwrap();
    config.apps[0].name = "craft-test-missing-app".into();
    let app = config
        .apps
        .iter_mut()
        .find(|a| a.name == "craft-test-missing-app")
        .unwrap();
    app.path = paths
        .at("system/craft-test-missing-app")
        .display()
        .to_string();
    app.version = "2.0.0".into();
    app.install_kind = "installer".into();
    paths.save_config(&config).unwrap();
    prefs.release_format = "portable".into();
    files::write_json(&paths.at("updater-settings.json"), &prefs).unwrap();
    let mut config = paths.config().unwrap();
    let app = config
        .apps
        .iter_mut()
        .find(|a| a.name == "craft-test-missing-app")
        .unwrap();
    app.path = paths
        .at("releases/craft-test-missing-app")
        .display()
        .to_string();
    app.version = "1.0.0".into();
    app.install_kind = "portable".into();
    fs::create_dir_all(&app.path).unwrap();
    fs::write(
        Path::new(&app.path).join("craft-test-missing-app.exe"),
        "fixture",
    )
    .unwrap();
    paths.save_config(&config).unwrap();
    for (format, expected) in [("installer", ""), ("portable", "1.0.0"), ("installer", "")] {
        prefs.release_format = format.into();
        files::write_json(&paths.at("updater-settings.json"), &prefs).unwrap();
        let config = paths.config().unwrap();
        let app = config
            .apps
            .iter()
            .find(|a| a.name == "craft-test-missing-app")
            .unwrap();
        assert_eq!(app.version, expected);
        assert_eq!(app.install_kind, format);
        assert_eq!(
            config
                .installations
                .iter()
                .filter(|a| a.name == "craft-test-missing-app")
                .count(),
            if format == "installer" { 1 } else { 2 }
        );
    }
}
#[test]
fn release_and_source_selections_are_independent() {
    let f = Fixture::new();
    let paths = f.paths();
    files::write_json(&paths.at("updater-settings.json"), &serde_json::json!({"selectedApps":["filmcraft"],"selectedSources":["artcraftx","artcraftx","unknown"]})).unwrap();
    let p = paths.preferences().unwrap();
    assert_eq!(p.selected_apps, ["filmcraft"]);
    assert_eq!(p.selected_sources, ["artcraftx"]);
    files::write_json(
        &paths.at("updater-settings.json"),
        &serde_json::json!({"selectedApps":["filmcraft"],"selectedSources":[]}),
    )
    .unwrap();
    let p = paths.preferences().unwrap();
    updates::sources(&paths, &p.selected_sources, &job(&f.0)).unwrap();
    assert!(!paths.at("runtime/api-cache").exists());
    files::write_json(
        &paths.at("updater-settings.json"),
        &serde_json::json!({"selectedApps":[]}),
    )
    .unwrap();
    assert_eq!(paths.preferences().unwrap().selected_sources.len(), 8);
}
#[test]
fn individual_check_detects_newer_release_without_downloading() {
    use sha2::{Digest, Sha256};
    let f = Fixture::new();
    let paths = f.paths();
    let mut config = paths.config().unwrap();
    let app = config
        .apps
        .iter_mut()
        .find(|a| a.name == "filmcraft")
        .unwrap();
    app.path = paths.at("releases/filmcraft").display().to_string();
    app.version = "1.0.0".into();
    app.install_kind = "portable".into();
    fs::create_dir_all(&app.path).unwrap();
    fs::write(Path::new(&app.path).join("filmcraft.exe"), "fixture").unwrap();
    files::write_json(
        &paths.at("updater-settings.json"),
        &Preferences {
            release_format: "portable".into(),
            ..Default::default()
        },
    )
    .unwrap();
    files::write_json(&paths.at("settings.json"), &config).unwrap();
    let endpoint = "https://api.github.com/repos/storytold/filmcraft/releases/latest";
    let cache = paths.at(format!(
        "runtime/api-cache/{:x}.json",
        Sha256::digest(endpoint.as_bytes())
    ));
    for (version, expected) in [("1.0.0", None), ("1.1.0", Some("1.1.0".to_string()))] {
        files::write_json(&cache, &serde_json::json!({"at":chrono::Utc::now().timestamp(),"etag":null,"value":{
            "tag_name":format!("v{version}"), "draft":false,"prerelease":false,
            "assets":[{"name":format!("filmcraft-{version}-windows-x64-portable.zip"),"size":1,"browser_download_url":"https://github.com/storytold/filmcraft/releases/download/test.zip"}]
        }})).unwrap();
        assert_eq!(updates::check_app(&paths, "filmcraft").unwrap(), expected);
    }
    assert!(!paths.at("runtime/downloads").exists());
    assert!(paths.at("releases/filmcraft/filmcraft.exe").exists());
}
#[test]
fn app_management_preserves_other_data_and_launch_settings() {
    let f = Fixture::new();
    let paths = f.paths();
    files::write_json(
        &paths.at("updater-settings.json"),
        &Preferences {
            release_format: "portable".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let folder = paths.at("releases/filmcraft");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("filmcraft.exe"), "fixture").unwrap();
    fs::create_dir_all(paths.at("sources")).unwrap();
    fs::write(paths.at("sources/filmcraft-source.zip"), "keep").unwrap();
    let mut config = paths.config().unwrap();
    let app = config
        .apps
        .iter_mut()
        .find(|a| a.name == "filmcraft")
        .unwrap();
    app.path = folder.display().to_string();
    app.version = "1.0.0".into();
    files::write_json(&paths.at("settings.json"), &config).unwrap();
    let value = craft_apps_updater::apps::LaunchSettings {
        executable: "filmcraft.exe".into(),
        arguments: vec!["file with spaces.mov".into(), "--test".into()],
    };
    craft_apps_updater::apps::save(&paths, "filmcraft", &value).unwrap();
    assert_eq!(
        craft_apps_updater::apps::settings(&paths, "filmcraft")
            .unwrap()
            .arguments,
        value.arguments
    );
    assert_eq!(
        craft_apps_updater::apps::executables(&paths, "filmcraft").unwrap(),
        ["filmcraft.exe"]
    );
    craft_apps_updater::apps::uninstall(&paths, "filmcraft").unwrap();
    assert!(!folder.exists());
    assert!(paths.at("sources/filmcraft-source.zip").exists());
    assert!(craft_apps_updater::apps::installed(&paths, "filmcraft").is_err());
    assert_eq!(
        craft_apps_updater::apps::settings(&paths, "filmcraft")
            .unwrap()
            .arguments,
        value.arguments
    );
}
#[test]
fn pdfcraft_repository_rename_preserves_existing_app_identity() {
    assert_eq!(
        craft_apps_updater::model::repository("printcraft"),
        "pdfcraft"
    );
    assert_eq!(craft_apps_updater::model::title("printcraft"), "PDFCraft");
    assert_eq!(
        craft_apps_updater::model::repository("filmcraft"),
        "filmcraft"
    );
    let f = Fixture::new();
    let sha = "a".repeat(40);
    for root in ["printcraft", "pdfcraft"] {
        let archive = f.0.join(format!("{root}.zip"));
        zip(
            &archive,
            &[(&format!("{root}-{sha}/Cargo.toml"), "[workspace]")],
        );
        files::verify_source(&archive, "printcraft", &sha).unwrap();
    }
    let archive = f.0.join("wrong.zip");
    zip(
        &archive,
        &[(&format!("other-{sha}/Cargo.toml"), "[workspace]")],
    );
    assert!(files::verify_source(&archive, "printcraft", &sha).is_err());
    let release = Release {
        tag_name: "v0.2.1".into(), draft: false, prerelease: false,
        assets: vec![Asset { name: "printcraft-0.2.1-windows-x64-portable.zip".into(), size: 1,
            browser_download_url: "https://github.com/storytold/pdfcraft/releases/download/v0.2.1/printcraft-0.2.1-windows-x64-portable.zip".into(), digest: None }],
    };
    let prefs = Preferences {
        release_format: "portable".into(),
        ..Default::default()
    };
    assert!(updates::select_asset(&release, "printcraft", &prefs).is_ok());
}
#[test]
fn settings_compatible_with_powershell_and_empty_selection() {
    let f = Fixture::new();
    let paths = f.paths();
    fs::write(
        paths.at("updater-settings.json"),
        "\u{feff}{\"selectedApps\":[],\"keepAppBackups\":false,\"architecture\":\"x86\"}",
    )
    .unwrap();
    let p = paths.preferences().unwrap();
    assert!(p.selected_apps.is_empty());
    assert!(!p.keep_app_backups);
    assert_eq!(p.architecture, "x86");
    let mut p = p;
    p.selected_apps = vec!["filmcraft".into(), "wordcraft".into(), "filmcraft".into()];
    p.validate().unwrap();
    assert_eq!(p.selected_apps, ["filmcraft"]);
    files::write_json(&paths.at("updater-settings.json"), &p).unwrap();
    assert_eq!(paths.preferences().unwrap().selected_apps, ["filmcraft"]);
}
#[test]
fn architectures_and_installer_selection() {
    let r = Release {
        tag_name: "v1.2.3".into(),
        draft: false,
        prerelease: false,
        assets: vec![
            Asset {
                name: "filmcraft-1.2.3-windows-x86-portable.zip".into(),
                size: 1,
                browser_download_url: "".into(),
                digest: None,
            },
            Asset {
                name: "filmcraft-1.2.3-windows-x64.msi".into(),
                size: 1,
                browser_download_url: "".into(),
                digest: None,
            },
        ],
    };
    let mut p = Preferences {
        release_format: "portable".into(),
        ..Default::default()
    };
    assert!(updates::select_asset(&r, "filmcraft", &p).is_err());
    p.architecture = "x86".into();
    assert!(updates::select_asset(&r, "filmcraft", &p)
        .unwrap()
        .name
        .ends_with("portable.zip"));
    p.architecture = "x64".into();
    p.release_format = "installer".into();
    assert!(updates::select_asset(&r, "filmcraft", &p)
        .unwrap()
        .name
        .ends_with(".msi"));
}
#[test]
fn source_validation_and_safe_extraction() {
    let f = Fixture::new();
    let sha = "a".repeat(40);
    let name = format!("filmcraft-{sha}/Cargo.toml");
    let archive = f.0.join("source.zip");
    zip(&archive, &[(&name, "[package]")]);
    files::verify_source(&archive, "filmcraft", &sha).unwrap();
    assert!(files::verify_source(&archive, "photocraft", &sha).is_err());
    files::extract_zip(&archive, &f.0.join("extracted"), &job(&f.0)).unwrap();
    let unsafe_zip = f.0.join("bad.zip");
    zip(&unsafe_zip, &[("../escape.txt", "bad")]);
    assert!(files::extract_zip(&unsafe_zip, &f.0.join("bad-extracted"), &job(&f.0)).is_err());
    assert!(!f.0.join("escape.txt").exists());
}
#[test]
fn checksum_refuses_corruption() {
    let f = Fixture::new();
    let file = f.0.join("asset.zip");
    fs::write(&file, "payload").unwrap();
    let mut a = Asset {
        name: "asset.zip".into(),
        size: 7,
        browser_download_url: "".into(),
        digest: Some(format!("sha256:{}", files::hash(&file).unwrap())),
    };
    verify_asset(&file, &a).unwrap();
    a.digest = Some(format!("sha256:{}", "0".repeat(64)));
    assert!(verify_asset(&file, &a).is_err());
    a.digest = None;
    assert!(verify_asset(&file, &a).is_err());
}
#[test]
fn atomic_metadata_and_rollback() {
    let f = Fixture::new();
    let dest = f.0.join("app");
    let stage = f.0.join("new");
    let backup = f.0.join("backup");
    fs::create_dir_all(&dest).unwrap();
    fs::create_dir_all(&stage).unwrap();
    fs::write(dest.join("app.exe"), "old").unwrap();
    fs::write(stage.join("app.exe"), "new").unwrap();
    assert!(
        updates::replace_transaction(&stage, &dest, Some(&backup), || bail!("commit failure"))
            .is_err()
    );
    assert_eq!(fs::read_to_string(dest.join("app.exe")).unwrap(), "old");
    updates::replace_transaction(&stage, &dest, Some(&backup), || {
        files::write_json(
            &f.0.join("settings.json"),
            &serde_json::json!({"version":"new"}),
        )
    })
    .unwrap();
    assert_eq!(fs::read_to_string(dest.join("app.exe")).unwrap(), "new");
    assert_eq!(fs::read_to_string(backup.join("app.exe")).unwrap(), "old");
}
#[test]
fn backups_retention_and_disabled() {
    let f = Fixture::new();
    let paths = f.paths();
    let root = paths.at("backups/releases");
    fs::create_dir_all(&root).unwrap();
    let mut p = Preferences {
        compress_backups: false,
        backup_versions: 1,
        ..Default::default()
    };
    let old = root.join(format!("filmcraft-1.0.0-{}", uuid::Uuid::new_v4().simple()));
    let current = root.join(format!("filmcraft-1.1.0-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&old).unwrap();
    fs::create_dir_all(&current).unwrap();
    backups::finish(&paths, &p, Some(&current), "filmcraft", false, &job(&f.0)).unwrap();
    assert!(!old.exists());
    assert!(current.exists());
    p.keep_app_backups = false;
    backups::finish(&paths, &p, Some(&current), "filmcraft", false, &job(&f.0)).unwrap();
    assert!(!current.exists());
}
#[test]
fn logs_are_plain_unicode_and_rotate() {
    let f = Fixture::new();
    let j = Job {
        max_bytes: 64,
        archives: 2,
        ..job(&f.0)
    };
    j.log("\x1b[32mBuilt ✓ 日本語\x1b[0m\x07");
    let text = fs::read_to_string(&j.log_path).unwrap();
    assert!(text.contains("✓ 日本語"));
    assert!(!text.contains('\x1b'));
    assert!(!text.contains('\x07'));
    for _ in 0..8 {
        j.log("A sufficiently long line to rotate the log.");
    }
    assert!(j.log_path.with_extension("log.1").exists());
    assert!(j.log_path.with_extension("log.2").exists());
    assert!(!j.log_path.with_extension("log.3").exists());
}
#[test]
fn cleanup_boundary_and_junction_target() {
    let f = Fixture::new();
    let root = f.0.join("workspace");
    let managed = root.join("filmcraft");
    let outside = f.0.join("outside");
    fs::create_dir_all(&managed).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("keep.txt"), "keep").unwrap();
    assert!(files::remove_managed(&outside, &root).is_err());
    assert!(files::remove_managed(&root, &root).is_err());
    let link = managed.join("external");
    let out = platform::output(
        Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&link)
            .arg(&outside),
    )
    .unwrap();
    assert!(out.status.success());
    files::remove_managed(&managed, &root).unwrap();
    assert!(outside.join("keep.txt").exists());
}
#[test]
fn api_cache_avoids_repeat_request() {
    let f = Fixture::new();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api", server.local_addr().unwrap());
    let t = std::thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        let mut b = [0; 2048];
        let received = stream.read(&mut b).unwrap();
        assert!(received > 0);
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nETag: \"fixture\"\r\nConnection: close\r\n\r\n{{\"ok\":true}}").unwrap();
    });
    let n = Network::new(&f.0).unwrap();
    assert_eq!(n.json::<serde_json::Value>(&url).unwrap()["ok"], true);
    t.join().unwrap();
    assert_eq!(n.json::<serde_json::Value>(&url).unwrap()["ok"], true);
}
#[test]
fn empty_selection_does_not_contact_github() {
    let f = Fixture::new();
    let paths = f.paths();
    let mut p = Preferences::default();
    p.selected_apps.clear();
    files::write_json(&paths.at("updater-settings.json"), &p).unwrap();
    updates::releases(&paths, &job(&f.0), false).unwrap();
    assert!(!paths.at("runtime/api-cache").exists());
}
#[test]
fn child_process_capture_and_cancellation() {
    let f = Fixture::new();
    let j = job(&f.0);
    j.run(
        Command::new("cmd.exe").args(["/d", "/c", "echo fixture-output"]),
        false,
    )
    .unwrap();
    assert!(j.state.lock().unwrap().log.contains("fixture-output"));
    let cancel = j.cancel.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        cancel.store(true, Ordering::Relaxed);
    });
    let start = std::time::Instant::now();
    assert!(j
        .run(
            Command::new("cmd.exe").args(["/d", "/c", "ping -n 30 127.0.0.1 >nul"]),
            false
        )
        .is_err());
    t.join().unwrap();
    assert!(start.elapsed().as_secs() < 10);
}
#[test]
fn source_backup_7zip_roundtrip() {
    let Ok(tools) = std::env::var("CRAFT_TEST_TOOLS") else {
        return;
    };
    let f = Fixture::new();
    let paths = Paths::new(f.0.clone(), Some(tools.into()));
    let root = paths.at("backups/sources");
    fs::create_dir_all(&root).unwrap();
    let source = root.join(format!(
        "filmcraft-source-aaaaaaa-{}.zip",
        uuid::Uuid::new_v4().simple()
    ));
    let sha = "a".repeat(40);
    zip(
        &source,
        &[
            (
                &format!("filmcraft-{sha}/Cargo.toml"),
                "[package]\nname='filmcraft'\nversion='0.1.0'\n",
            ),
            (
                &format!("filmcraft-{sha}/src/main.rs"),
                &"fn main() {}\n".repeat(10000),
            ),
        ],
    );
    let p = Preferences::default();
    backups::finish(&paths, &p, Some(&source), "filmcraft", true, &job(&f.0)).unwrap();
    assert!(!source.exists());
    assert!(PathBuf::from(format!("{}.7z", source.display())).exists());
    let backups = backups::list(&paths, "filmcraft").unwrap();
    backups::restore(&paths, "filmcraft", &backups[0], &job(&f.0)).unwrap();
    let restored = paths.at("sources/filmcraft-source.zip");
    files::verify_source(&restored, "filmcraft", &sha).unwrap();
    let index: BTreeMap<String, Source> =
        files::read_json(&paths.at("sources/source-index.json")).unwrap();
    assert_eq!(index["filmcraft"].sha, sha);
    assert_eq!(
        index["filmcraft"].archive_sha256,
        files::hash(&restored).unwrap()
    );
}
#[test]
fn real_rust_fixture_build() -> Result<()> {
    let Ok(tools) = std::env::var("CRAFT_TEST_TOOLS") else {
        return Ok(());
    };
    let f = Fixture::new();
    let paths = Paths::new(f.0.clone(), Some(tools.into()));
    let sha = "b".repeat(40);
    let archive = paths.at("sources/designcraft-source.zip");
    fs::create_dir_all(archive.parent().unwrap())?;
    let manifest = "[package]\nname = \"designcraft\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
    let lock = "version = 4\n[[package]]\nname = \"designcraft\"\nversion = \"0.1.0\"\n";
    zip(
        &archive,
        &[
            (&format!("designcraft-{sha}/Cargo.toml"), manifest),
            (&format!("designcraft-{sha}/Cargo.lock"), lock),
            (
                &format!("designcraft-{sha}/src/main.rs"),
                "fn main(){println!(\"rust-builder-fixture\");}",
            ),
        ],
    );
    let mut index = BTreeMap::new();
    index.insert(
        "designcraft",
        Source {
            sha,
            branch: "main".into(),
            repository: "storytold/designcraft".into(),
            archive_sha256: files::hash(&archive)?,
            downloaded_at: chrono::Utc::now().to_rfc3339(),
        },
    );
    files::write_json(&paths.at("sources/source-index.json"), &index)?;
    let j = job(&f.0);
    builder::build(&paths, "designcraft", false, &j)?;
    let out = j.state.lock().unwrap().output.clone().unwrap();
    assert!(out.join("build-info.json").exists());
    assert_eq!(builder::history(&paths, "designcraft"), Some(out.clone()));
    assert!(!paths.at("workspace/cache/designcraft").exists());
    assert!(!paths.at("workspace/designcraft").exists());
    let result = platform::output(&mut Command::new(out.join("designcraft.exe")))?;
    assert!(String::from_utf8_lossy(&result.stdout).contains("rust-builder-fixture"));
    assert!(APPS.contains(&"designcraft"));
    Ok(())
}

#[test]
fn stale_installation_records_do_not_mark_apps_installed() {
    use craft_apps_updater::model::{Config, Installed};
    let f = Fixture::new();
    let paths = f.paths();
    let record = Installed {
        name: "craft-test-missing-app".into(),
        version: "1.0.0".into(),
        path: f.0.join("missing").display().to_string(),
        architecture: "x64".into(),
        install_kind: "installer".into(),
        product_code: "{00000000-0000-0000-0000-000000000000}".into(),
    };
    let config = Config {
        apps_root: f.0.display().to_string(),
        apps: vec![record.clone()],
        installations: vec![record],
    };
    files::write_json(&paths.at("settings.json"), &config).unwrap();
    let refreshed = paths.config().unwrap();
    assert!(refreshed.apps[0].path.is_empty());
    assert!(refreshed.apps[0].product_code.is_empty());
    assert!(refreshed.installations.is_empty());
    let prefs = Preferences {
        release_format: "portable".into(),
        ..Default::default()
    };
    files::write_json(&paths.at("updater-settings.json"), &prefs).unwrap();
    let mut config = config;
    config.apps[0].install_kind = "portable".into();
    config.installations[0].install_kind = "portable".into();
    files::write_json(&paths.at("settings.json"), &config).unwrap();
    assert!(paths.config().unwrap().apps[0].path.is_empty());
}
