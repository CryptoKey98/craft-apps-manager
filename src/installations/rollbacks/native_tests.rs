use super::*;

struct NativeFixture {
    paths: Paths,
    previous: Installed,
    current: Installed,
    prepared: native::Prepared,
    job: Job,
}
impl NativeFixture {
    fn new() -> Self {
        let paths = Paths::new(
            std::env::temp_dir().join(format!("craft-native-fixture-{}", uuid::Uuid::new_v4())),
            None,
        );
        let previous = Installed {
            name: "filmcraft".into(),
            path: paths.at("application").display().to_string(),
            version: "1.0.0".into(),
            architecture: "x64".into(),
            install_kind: "installer".into(),
            product_code: "{11111111-1111-4111-8111-111111111111}".into(),
            ..Default::default()
        };
        let mut current = previous.clone();
        current.version = "2.0.0".into();
        current.product_code = "{22222222-2222-4222-8222-222222222222}".into();
        let package = paths.at("fixture/setup.msi");
        files::write_bytes(&package, b"old fixture").unwrap();
        let asset = crate::model::Asset {
            name: "setup.msi".into(),
            size: 11,
            browser_download_url: format!(
                "https://github.com/{}/releases/download/1.0.0/setup.msi",
                crate::model::repository("filmcraft")
            ),
            digest: Some(format!("sha256:{}", files::hash(&package).unwrap())),
        };
        let prepared = native::Prepared {
            #[cfg(target_os = "windows")]
            windows_msi_scope: None,
            asset,
            package,
            tag: "1.0.0".into(),
            identity: native::PackageIdentity {
                format: "msi".into(),
                identity: "family".into(),
                product: previous.product_code.clone(),
                version: previous.version.clone(),
                architecture: "x64".into(),
            },
        };
        let config = crate::model::Config {
            apps_root: paths.root.display().to_string(),
            apps: vec![current.clone()],
            installations: vec![current.clone()],
            portable_label_counters: Default::default(),
        };
        files::write_json(&paths.at("installations.json"), &config).unwrap();
        let job = Job::new(paths.at("logs/test.log"), &Default::default());
        native::publish(&paths, &previous, &current, &prepared, &job).unwrap();
        Self {
            paths,
            previous,
            current,
            prepared,
            job,
        }
    }
    fn point(&self) -> RollbackPoint {
        read_point(&self.paths, &native::key(&self.current).unwrap())
            .unwrap()
            .unwrap()
    }
}
impl Drop for NativeFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.paths.root);
    }
}

#[test]
fn installer_staging_cleanup_preserves_unknown_files_and_prunes_only_empty_owned_parents() {
    let f = NativeFixture::new();
    let owned = f.paths.at("releases/installers/filmcraft/new.msi");
    let unknown = f.paths.at("releases/installers/filmcraft/unknown.txt");
    files::write_bytes(&owned, b"new downloaded package").unwrap();
    files::write_bytes(&unknown, b"keep unknown data").unwrap();
    native::discard_new_download(&f.paths, &owned).unwrap();
    assert!(!owned.exists());
    assert_eq!(fs::read(&unknown).unwrap(), b"keep unknown data");
    assert!(native::discard_new_download(&f.paths, &f.prepared.package).is_err());
    assert!(f.prepared.package.exists());
    fs::remove_file(unknown).unwrap();
    files::write_bytes(&owned, b"new downloaded package").unwrap();
    native::discard_new_download(&f.paths, &owned).unwrap();
    assert!(!f.paths.at("releases/installers").exists());
    fs::create_dir_all(owned.parent().unwrap()).unwrap();
    native::discard_new_download(&f.paths, &owned).unwrap();
    assert!(!f.paths.at("releases/installers").exists());
}

#[test]
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn intel_msi_accepts_verified_anycpu_runtime_but_not_native_or_required_x86() {
    let f = NativeFixture::new();
    let executable =
        Path::new(&f.current.path).join(crate::model::executable_name(&f.current.name));
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
    let identity = native::PackageIdentity {
        product: f.current.product_code.clone(),
        version: f.current.version.clone(),
        architecture: "x86".into(),
        ..f.prepared.identity.clone()
    };
    bytes[368..372].copy_from_slice(&1u32.to_le_bytes());
    files::write_bytes(&executable, &bytes).unwrap();
    assert!(native::validate_identity(&identity, &f.current).is_ok());
    let mut wrong = identity.clone();
    wrong.product = "{33333333-3333-4333-8333-333333333333}".into();
    assert!(native::validate_identity(&wrong, &f.current).is_err());
    wrong = identity.clone();
    wrong.version = "9.0.0".into();
    assert!(native::validate_identity(&wrong, &f.current).is_err());
    wrong = identity.clone();
    wrong.format = "exe".into();
    assert!(native::validate_identity(&wrong, &f.current).is_err());
    wrong = identity.clone();
    wrong.architecture = "arm64".into();
    assert!(native::validate_identity(&wrong, &f.current).is_err());
    for flags in [3u32, 0x20001, 0] {
        bytes[368..372].copy_from_slice(&flags.to_le_bytes());
        files::write_bytes(&executable, &bytes).unwrap();
        assert!(native::validate_identity(&identity, &f.current).is_err());
    }
    bytes[368..372].copy_from_slice(&1u32.to_le_bytes());
    bytes[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
    files::write_bytes(&executable, &bytes).unwrap();
    assert!(native::validate_identity(&identity, &f.current).is_err());
    bytes[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
    files::write_bytes(&executable, &bytes).unwrap();
    assert!(native::validate_identity(&identity, &f.current).is_err());
}

#[test]
fn sidebar_summary_leaves_installer_checksum_verification_to_rollback_review() {
    let f = NativeFixture::new();
    let point = f.point();
    let package = native::verify(&f.paths, &f.current, &point).unwrap();
    files::write_bytes(&package, b"bad fixture").unwrap();
    let prefs = f.paths.read_preferences().unwrap();

    let summary = summary(&f.paths, &f.current, &prefs).unwrap();
    assert_eq!(summary.previous.unwrap().id, point.id);
    assert!(status(&f.paths, &f.current).is_err());

    fs::remove_file(package).unwrap();
    assert!(super::summary(&f.paths, &f.current, &prefs).is_err());
}

#[test]
fn damaged_native_point_can_be_explicitly_discarded_without_touching_current_app() {
    let f = NativeFixture::new();
    let point = f.point();
    let package = native::verify(&f.paths, &f.current, &point).unwrap();
    fs::rename(&package, package.with_extension("held")).unwrap();
    files::write_bytes(&Path::new(&f.current.path).join("keep.txt"), b"current app").unwrap();
    let candidate = removal_candidate(&f.paths, &f.current).unwrap().unwrap();
    assert_eq!(candidate.id, point.id);
    assert!(native::verify(&f.paths, &f.current, &point).is_err());
    let mut changed = candidate.clone();
    changed.version = "unapproved".into();
    assert!(remove_previous(&f.paths, &f.current, &changed).is_err());
    assert!(package.with_extension("held").exists());
    remove_previous(&f.paths, &f.current, &candidate).unwrap();
    assert!(!f.paths.at("rollbacks").exists());
    assert_eq!(
        fs::read(Path::new(&f.current.path).join("keep.txt")).unwrap(),
        b"current app"
    );
}

#[test]
fn removal_authorization_rejects_a_rollback_folder_used_by_another_installation() {
    let f = NativeFixture::new();
    let point = f.point();
    let folder = native::verify(&f.paths, &f.current, &point)
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let mut config = f.paths.read_inventory().unwrap().unwrap();
    let mut other = f.current.clone();
    other.name = "designcraft".into();
    other.path = folder.join("installed-copy").display().to_string();
    config.installations.push(other);
    files::write_json(&f.paths.at("installations.json"), &config).unwrap();
    assert!(removal_candidate(&f.paths, &f.current).is_err());
    assert!(remove_previous(&f.paths, &f.current, &point).is_err());
    assert!(folder.exists());
}

#[test]
fn missing_native_payload_can_be_removed_without_discarding_current_inventory() {
    let f = NativeFixture::new();
    let point = f.point();
    let package = native::verify(&f.paths, &f.current, &point).unwrap();
    fs::remove_file(&package).unwrap();
    fs::remove_dir(package.parent().unwrap()).unwrap();
    remove_previous(&f.paths, &f.current, &point).unwrap();
    assert!(!f.paths.at("rollbacks").exists());
    assert_eq!(
        f.paths.read_inventory().unwrap().unwrap().apps[0].version,
        f.current.version
    );
}

#[test]
fn failed_native_publication_keeps_the_existing_point_and_cleans_staging() {
    let f = NativeFixture::new();
    let original = f.point();
    let error = native::publish_with(
        &f.paths,
        &f.previous,
        &f.current,
        &f.prepared,
        &f.job,
        |_, _| anyhow::bail!("injected publication failure"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("injected"));
    assert_eq!(f.point().id, original.id);
    assert!(native::verify(&f.paths, &f.current, &original).is_ok());
    assert_eq!(
        fs::read_dir(root(&f.paths, &original.installation_id).unwrap())
            .unwrap()
            .count(),
        2
    );
}

#[test]
fn native_removal_preserves_current_installation_and_old_archives() {
    let f = NativeFixture::new();
    let point = f.point();
    let legacy = f.paths.at("backups/releases/old.zip");
    files::write_bytes(&legacy, b"legacy").unwrap();
    remove_previous(&f.paths, &f.current, &point).unwrap();
    assert!(!f.paths.at("rollbacks").exists());
    assert!(legacy.exists());
    assert_eq!(
        f.paths.read_inventory().unwrap().unwrap().apps[0].version,
        "2.0.0"
    );
}

#[test]
fn retained_plan_rejects_changed_assets_and_wrong_installations_without_execution() {
    let f = NativeFixture::new();
    let point = f.point();
    let mut plan:crate::updates::ReleasePlan=serde_json::from_value(serde_json::json!({
        "native_rollback":point.id,"entries":[{"app":"filmcraft","action":"Update","version":"1.0.0","asset":f.prepared.asset,"destination":null,"error":null,"preferences":crate::model::Preferences {release_format:"installer".into(),architecture:"x64".into(),..Default::default()}}],
        "preferences":crate::model::Preferences::default(),"requested_tag":"1.0.0","snapshot":[],"desktop_shortcut":false,"installer_destination":null
    })).unwrap();
    native::validate_plan(&f.paths, &plan).unwrap();
    plan.entries[0].asset.as_mut().unwrap().digest = Some(format!("sha256:{}", "a".repeat(64)));
    assert!(native::validate_plan(&f.paths, &plan).is_err());
    let mut wrong = f.current.clone();
    wrong.path.push_str("-other");
    assert!(native::validate_replacement(&f.paths, &f.current, &point.id, &wrong).is_err());
}

#[test]
fn identical_setup_filenames_have_independent_verified_cache_paths() {
    let f = NativeFixture::new();
    let old =
        crate::updates::installer_package_path(&f.paths, "filmcraft", &f.prepared.asset).unwrap();
    let mut new = f.prepared.asset.clone();
    new.digest = Some(format!("sha256:{}", "b".repeat(64)));
    let replacement = crate::updates::installer_package_path(&f.paths, "filmcraft", &new).unwrap();
    assert_ne!(old, replacement);
    assert_eq!(old.parent(), replacement.parent());
    assert!(old.to_string_lossy().ends_with(".msi"));
}

#[test]
fn native_retention_binds_the_active_copy_and_survives_product_code_changes() {
    let paths = Paths::new(
        std::env::temp_dir().join(format!("craft-native-point-{}", uuid::Uuid::new_v4())),
        None,
    );
    let previous = Installed {
        name: "filmcraft".into(),
        version: "1.0.0".into(),
        path: paths.at("installed").display().to_string(),
        architecture: "x64".into(),
        install_kind: "installer".into(),
        product_code: "old-product".into(),
        ..Default::default()
    };
    let mut updated = previous.clone();
    updated.version = "2.0.0".into();
    updated.product_code = "new-product".into();
    let package = paths.at("downloads/old.msi");
    files::write_bytes(&package, b"verified package fixture").unwrap();
    let asset = crate::model::Asset {
        name: "old.msi".into(),
        size: fs::metadata(&package).unwrap().len(),
        browser_download_url: format!(
            "https://github.com/{}/releases/download/1.0.0/old.msi",
            crate::model::repository("filmcraft")
        ),
        digest: Some(format!("sha256:{}", files::hash(&package).unwrap())),
    };
    let metadata = native::PackageIdentity {
        format: "msi".into(),
        identity: "upgrade-family".into(),
        product: "old-product".into(),
        version: "1.0.0".into(),
        architecture: "x64".into(),
    };
    let job = Job::new(paths.at("logs/test.log"), &Default::default());
    let prepared = native::Prepared {
        #[cfg(target_os = "windows")]
        windows_msi_scope: None,
        asset,
        tag: "1.0.0".into(),
        package,
        identity: metadata,
    };
    native::publish(&paths, &previous, &updated, &prepared, &job).unwrap();
    let id = native::key(&updated).unwrap();
    assert_eq!(id, native::key(&previous).unwrap());
    let point = read_point(&paths, &id).unwrap().unwrap();
    assert_eq!(point.version, "1.0.0");
    let retained = native::verify(&paths, &updated, &point).unwrap();
    assert_eq!(fs::read(&retained).unwrap(), b"verified package fixture");
    let mut another = updated.clone();
    another.path.push_str("-other");
    assert!(native::verify(&paths, &another, &point).is_err());
    another = updated.clone();
    another.version = "3.0.0".into();
    assert!(native::verify(&paths, &another, &point).is_err());
    fs::write(&retained, b"tampered").unwrap();
    assert!(native::verify(&paths, &updated, &point).is_err());
    fs::remove_dir_all(paths.root).unwrap();
}

#[test]
fn legacy_scope_migration_rollback_is_not_restored_as_same_scope() {
    let f = NativeFixture::new();
    let file = root(&f.paths, &native::key(&f.current).unwrap())
        .unwrap()
        .join("point.json");
    let mut value = serde_json::to_value(f.point()).unwrap();
    value["native"]["original_migration"] = serde_json::to_value(&f.previous).unwrap();
    files::write_json(&file, &value).unwrap();
    let point = f.point();
    let error = native::verify(&f.paths, &f.current, &point).unwrap_err();
    assert!(error.to_string().contains("scope"), "{error:#}");
    native::remove(&f.paths, &f.current, &point).unwrap();
    assert!(!file.exists());
    assert!(f.prepared.package.exists());
}

#[test]
#[cfg(target_os = "windows")]
fn native_retention_rejects_scope_changes_even_at_the_same_location() {
    let mut f = NativeFixture::new();
    f.previous.windows_msi_scope = Some("user".into());
    f.current.windows_msi_scope = Some("machine".into());
    assert!(native::publish(&f.paths, &f.previous, &f.current, &f.prepared, &f.job).is_err());
}
