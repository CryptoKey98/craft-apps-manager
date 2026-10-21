#[test]
fn intended_version_verification_rejects_old_or_unknown_copies() {
    assert!(super::matches_expected_version("1.2.3.0", "v1.2.3"));
    assert!(!super::matches_expected_version("1.2.2", "1.2.3"));
    assert!(!super::matches_expected_version("unknown", "1.2.3"));
}

#[test]
fn identity_regressions_saved_choice_has_priority_over_guesses() {
    assert_eq!(
        super::preferred_msi_executable(
            &["photocraft.exe".into(), "Different.exe".into()],
            &["Different.exe".into()],
            &["photocraft.exe".into()]
        ),
        Some(1)
    );
}
#[test]
fn identity_regressions_other_platform_identity_never_authorizes_headless_exe() {
    let root = std::env::temp_dir().join(format!("craft-exe-preflight-{}", uuid::Uuid::new_v4()));
    crate::app_identity::remember(
        &root,
        "photocraft",
        crate::app_identity::NativeIdentity {
            launch: None,
            family: String::new(),
            platform: "linux".into(),
            package: "photocraft".into(),
            executable: "/usr/bin/photocraft".into(),
        },
    )
    .unwrap();
    assert!(!super::exe_identity_ready("photocraft").unwrap());
    crate::app_identity::load(&root.join("unused"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mirrored_registration_views_produce_one_choice_without_merging_distinct_apps() {
    use super::*;
    let folder = std::path::PathBuf::from(r"C:\Apps\WinMerge");
    let record = Registration {
        identity: exe_registration_id("HKCU/64", "same-key", &folder),
        display: "WinMerge".into(),
        version: "2.16.58.2".into(),
        folder: folder.clone(),
        icon: "WinMergeU.exe".into(),
        uninstall: "uninstall /user".into(),
    };
    let mirror = Registration {
        identity: exe_registration_id("HKCU/32", "same-key", &folder),
        ..record.clone()
    };
    let executable = folder.join("WinMergeU.exe");
    let mut choices = vec![(&record, executable.clone()), (&mirror, executable.clone())];
    deduplicate_identity_choices(&mut choices);
    assert_eq!(
        choices.len(),
        1,
        "shared per-user registration must appear once"
    );
    assert_eq!(choices[0].0.identity, record.identity);
    for different in [
        Registration {
            identity: exe_registration_id("HKLM/32", "same-key", &folder),
            ..mirror.clone()
        },
        Registration {
            identity: exe_registration_id("HKCU/32", "different-key", &folder),
            ..mirror.clone()
        },
        Registration {
            version: "3.0".into(),
            ..mirror.clone()
        },
        Registration {
            uninstall: "uninstall /other-profile".into(),
            ..mirror.clone()
        },
    ] {
        let mut choices = vec![
            (&record, executable.clone()),
            (&different, executable.clone()),
        ];
        deduplicate_identity_choices(&mut choices);
        assert_eq!(
            choices.len(),
            2,
            "distinct registration must remain selectable"
        );
    }
    let machine64 = Registration {
        identity: exe_registration_id("HKLM/64", "same-key", &folder),
        ..record.clone()
    };
    let machine32 = Registration {
        identity: exe_registration_id("HKLM/32", "same-key", &folder),
        ..record.clone()
    };
    let mut machine_choices = vec![
        (&machine64, executable.clone()),
        (&machine32, executable.clone()),
    ];
    deduplicate_identity_choices(&mut machine_choices);
    assert_eq!(
        machine_choices.len(),
        2,
        "machine registry views are separate registrations"
    );
    let mut choices = vec![
        (&record, executable.clone()),
        (&mirror, folder.join("Other.exe")),
    ];
    deduplicate_identity_choices(&mut choices);
    assert_eq!(
        choices.len(),
        2,
        "different executables must remain selectable"
    );
}

#[test]
fn custom_exe_updates_never_substitute_a_different_saved_copy() {
    use crate::model::Installed;
    let selected = Installed {
        name: "photocraft".into(),
        path: r"C:\Apps\CopyA".into(),
        product_code: "copy-a".into(),
        version: "1.0.0".into(),
        ..Default::default()
    };
    let other = Installed {
        path: r"C:\Apps\CopyB".into(),
        product_code: "copy-b".into(),
        version: "2.0.0".into(),
        ..selected.clone()
    };
    assert!(super::verify_selected_exe(&selected, "2.0.0", Some(other)).is_err());
    assert!(super::verify_selected_exe(&selected, "2.0.0", Some(selected.clone())).is_err());
    let updated = Installed {
        version: "2.0.0".into(),
        ..selected.clone()
    };
    assert_eq!(
        super::verify_selected_exe(&selected, "2.0.0", Some(updated.clone()))
            .unwrap()
            .path,
        updated.path
    );
}

#[test]
fn existing_registration_requires_explicit_adoption_and_never_displaces_changed_records() {
    let record = super::Registration {
        identity: "verified-id".into(),
        display: "Different name".into(),
        version: "1.0".into(),
        folder: r"C:\Apps\Different".into(),
        icon: String::new(),
        uninstall: "uninstall".into(),
    };
    let before = [record.clone()];
    let unchanged = [record.clone()];
    assert!(super::registration_candidates(&before, &unchanged, false).is_empty());
    assert_eq!(
        super::registration_candidates(&before, &unchanged, true),
        vec![&unchanged[0]]
    );
    let after = [
        record.clone(),
        super::Registration {
            identity: "new-id".into(),
            ..record
        },
    ];
    assert_eq!(
        super::registration_candidates(&before, &after, true),
        vec![&after[1]]
    );
}

#[test]
fn registration_changes_never_include_unchanged_programs() {
    let record = super::Registration {
        identity: "registered-id".into(),
        display: "Different Product Name".into(),
        version: "1.0".into(),
        folder: r"C:\Apps\Different".into(),
        icon: String::new(),
        uninstall: "uninstall".into(),
    };
    assert!(super::changed_registrations(
        std::slice::from_ref(&record),
        std::slice::from_ref(&record)
    )
    .is_empty());
    let changed = super::Registration {
        version: "2.0".into(),
        ..record.clone()
    };
    assert_eq!(
        super::changed_registrations(&[record], std::slice::from_ref(&changed)),
        vec![&changed]
    );
}
#[test]
fn real_msi_database_identity_is_read_without_executing_package() {
    use super::*;
    #[link(name = "msi")]
    extern "system" {
        fn MsiDatabaseCommit(database: u32) -> u32;
    }
    let path =
        std::env::temp_dir().join(format!("craft-identity-msi-{}.msi", uuid::Uuid::new_v4()));
    let mut database = 0;
    assert_eq!(
        unsafe {
            MsiOpenDatabaseW(
                platform::wide(&path).as_ptr(),
                3usize as *const u16,
                &mut database,
            )
        },
        0
    );
    let database = MsiHandle(database);
    for query in [
            "CREATE TABLE `Property` (`Property` CHAR(72) NOT NULL, `Value` CHAR(0) LOCALIZABLE PRIMARY KEY `Property`)",
            "INSERT INTO `Property` (`Property`,`Value`) VALUES ('ProductCode','{01234567-89AB-CDEF-0123-456789ABCDEF}')",
            "INSERT INTO `Property` (`Property`,`Value`) VALUES ('UpgradeCode','{11234567-89AB-CDEF-0123-456789ABCDEF}')",
            "CREATE TABLE `File` (`File` CHAR(72) NOT NULL, `FileName` CHAR(255) LOCALIZABLE PRIMARY KEY `File`)",
            "INSERT INTO `File` (`File`,`FileName`) VALUES ('MainFile','SHORT.EXE|DifferentName.exe')",
        ] {
            let mut view=0; assert_eq!(unsafe{MsiDatabaseOpenViewW(database.0,platform::wide(query).as_ptr(),&mut view)},0,"{query}");
            let view=MsiHandle(view);assert_eq!(unsafe{MsiViewExecute(view.0,0)},0,"{query}");
        }
    assert_eq!(unsafe { MsiDatabaseCommit(database.0) }, 0);
    drop(database);
    assert_eq!(
        msi_rows(
            &path,
            "SELECT `Value` FROM `Property` WHERE `Property` = 'ProductCode'"
        )
        .unwrap(),
        ["{01234567-89AB-CDEF-0123-456789ABCDEF}"]
    );
    assert_eq!(
        msi_rows(&path, "SELECT `FileName` FROM `File`").unwrap(),
        ["SHORT.EXE|DifferentName.exe"]
    );
    let root = path.with_extension("data");
    let mut entry = crate::catalog::builtin().remove(0);
    entry.key = "identityfixture".into();
    entry.repository = "someone/identityfixture".into();
    entry.title = "IdentityFixture".into();
    entry.asset_prefix = "identityfixture".into();
    entry.scheme = crate::catalog::Scheme::Generic;
    crate::files::write_json(&root.join("runtime/custom-apps.json"), &vec![entry]).unwrap();
    crate::catalog::load(&root);
    crate::app_identity::remember(
        &root,
        "identityfixture",
        crate::app_identity::NativeIdentity {
            launch: None,
            platform: "windows".into(),
            package: "{21234567-89AB-CDEF-0123-456789ABCDEF}".into(),
            family: "{11234567-89AB-CDEF-0123-456789ABCDEF}".into(),
            executable: "DifferentName.exe".into(),
        },
    )
    .unwrap();
    let job = crate::jobs::Job::new(root.join("logs/test.log"), &Default::default());
    prepare_custom_identity(&root, &path, "identityfixture", &job).unwrap();
    assert!(crate::app_identity::native("identityfixture")
        .iter()
        .any(
            |identity| identity.package == "{01234567-89AB-CDEF-0123-456789ABCDEF}"
                && identity.executable == "DifferentName.exe"
        ));
    crate::catalog::load(&root.join("unused"));
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_file(path).unwrap();
}
#[test]
fn explicit_registration_choice_overrides_guesses_and_missing_choice_does_not_fall_back() {
    use super::*;
    let id = uuid::Uuid::new_v4();
    let key_path = format!("Software\\CraftAppsManagerTests\\{id}");
    let folder = std::env::temp_dir().join(format!("craft-identity-choice-{id}"));
    std::fs::create_dir_all(&folder).unwrap();
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(&key_path)
        .unwrap();
    key.set_value("DisplayName", &"Completely Different Product")
        .unwrap();
    key.set_value("InstallLocation", &folder.display().to_string())
        .unwrap();
    let mut pe = vec![0u8; 158];
    pe[..2].copy_from_slice(b"MZ");
    pe[60..64].copy_from_slice(&64u32.to_le_bytes());
    pe[64..68].copy_from_slice(b"PE\0\0");
    pe[68..70].copy_from_slice(&0x014cu16.to_le_bytes());
    pe[84..86].copy_from_slice(&70u16.to_le_bytes());
    pe[88..90].copy_from_slice(&0x10bu16.to_le_bytes());
    pe[156..158].copy_from_slice(&2u16.to_le_bytes());
    std::fs::write(folder.join("photocraft.exe"), &pe).unwrap();
    std::fs::write(folder.join("Different.exe"), pe).unwrap();
    let identity = crate::app_identity::NativeIdentity {
        launch: None,
        family: String::new(),
        platform: "windows".into(),
        package: exe_registration_id("HKCU/64", "chosen-key", &folder),
        executable: "Different.exe".into(),
    };
    crate::app_identity::remember(&folder, "photocraft", identity).unwrap();
    assert!(matches_registration(
        "photocraft",
        "chosen-key",
        &key,
        "Completely Different Product"
    ));
    assert_eq!(
        registration_choice(&folder, "photocraft", "chosen-key", &key, "HKCU/64"),
        Some(Some(folder.join("Different.exe")))
    );
    assert!(registration_choice(&folder, "photocraft", "chosen-key", &key, "HKLM/64").is_none());
    std::fs::remove_file(folder.join("Different.exe")).unwrap();
    assert_eq!(
        registration_choice(&folder, "photocraft", "chosen-key", &key, "HKCU/64"),
        Some(None)
    );
    crate::app_identity::load(&folder.join("unused"));
    drop(key);
    RegKey::predef(HKEY_CURRENT_USER)
        .delete_subkey_all(key_path)
        .unwrap();
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn custom_display_versions_support_two_components() {
    assert!(super::matches_display_name("example", "Example 1.68 (x64)"));
    assert!(!super::matches_display_name(
        "example",
        "Example 1.68 Helper"
    ));
    assert_eq!(super::installation_version("1.68"), Some([1, 68, 0, 0]));
}
#[test]
fn registered_icon_requires_a_desktop_executable_in_the_app_folder() {
    let folder = std::env::temp_dir().join(format!("craft-icon-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&folder).unwrap();
    let mut pe = vec![0u8; 158];
    pe[..2].copy_from_slice(b"MZ");
    pe[60..64].copy_from_slice(&64u32.to_le_bytes());
    pe[64..68].copy_from_slice(b"PE\0\0");
    pe[68..70].copy_from_slice(&0x014cu16.to_le_bytes());
    pe[84..86].copy_from_slice(&70u16.to_le_bytes());
    pe[88..90].copy_from_slice(&0x10bu16.to_le_bytes());
    pe[156..158].copy_from_slice(&2u16.to_le_bytes());
    let executable = folder.join("UnexpectedName.exe");
    std::fs::write(&executable, &pe).unwrap();
    assert!(
        super::registered_icon_executable(&folder, &format!("\"{}\",0", executable.display()))
            .is_some()
    );
    assert!(super::registered_icon_executable(
        &folder.join("other"),
        &executable.display().to_string()
    )
    .is_none());
    assert!(super::registered_icon_executable(&folder, "relative.exe").is_none());
    let uninstall = folder.join("unins000.exe");
    std::fs::write(&uninstall, &pe).unwrap();
    assert!(super::registered_icon_executable(&folder, &uninstall.display().to_string()).is_none());
    pe[156..158].copy_from_slice(&3u16.to_le_bytes());
    std::fs::write(&executable, pe).unwrap();
    assert!(
        super::registered_icon_executable(&folder, &executable.display().to_string()).is_none()
    );
    std::fs::remove_file(executable).unwrap();
    std::fs::remove_file(uninstall).unwrap();
    std::fs::remove_dir(folder).unwrap();
}

#[test]
fn installer_display_names_accept_version_build_hash_and_architecture() {
    assert!(super::matches_display_name(
        "dbeaver",
        "DBeaver 26.2.2 (current user)"
    ));
    assert!(super::matches_display_name(
        "dbeaver",
        "DBeaver 26.2.2 (all users)"
    ));
    assert!(!super::matches_display_name(
        "dbeaver",
        "DBeaver Helper 26.2.2 (current user)"
    ));
    assert!(super::matches_display_name(
        "transmission",
        "Transmission version 4.1.3"
    ));
    assert!(super::matches_display_name(
        "transmission",
        "Transmission 4.1.3 (838877323f) (x64)"
    ));
    assert!(!super::matches_display_name(
        "transmission",
        "Transmission Helper 4.1.3"
    ));
    assert!(!super::matches_display_name(
        "transmission",
        "Transmission 4.1.3 (not-a-hash) (x64)"
    ));
}

#[test]
fn renamed_installations_select_newest_across_registry_roots() {
    use super::*;
    let id = uuid::Uuid::new_v4();
    let key_path = format!("Software\\CraftAppsManagerTests\\{id}");
    let folder = std::env::temp_dir().join(format!("craft-detection-{id}"));
    struct Cleanup(String, std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.0);
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    let _cleanup = Cleanup(key_path.clone(), folder.clone());
    let (root, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(&key_path)
        .unwrap();
    for (key, display, version, exe) in [
        ("old", "PrintCraft", "0.2.1", "printcraft.exe"),
        ("new", "PdfCraft", "0.4.1", "pdfcraft.exe"),
    ] {
        let dir = folder.join(key);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(exe), b"fixture").unwrap();
        let (entry, _) = root.create_subkey(format!("{key}\\product")).unwrap();
        entry.set_value("DisplayName", &display).unwrap();
        entry.set_value("DisplayVersion", &version).unwrap();
        entry
            .set_value("InstallLocation", &dir.to_string_lossy().as_ref())
            .unwrap();
    }
    for order in [["old", "new"], ["new", "old"]] {
        let roots = order.map(|name| (root.open_subkey(name).unwrap(), KEY_WOW64_64KEY));
        let detected = detect_in_roots("printcraft", roots).unwrap().unwrap();
        assert_eq!(detected.version, "0.4.1");
        assert_eq!(detected.path, folder.join("new").display().to_string());
    }
    let selected = Installed {
        name: "printcraft".into(),
        path: folder.join("old").display().to_string(),
        architecture: "x64".into(),
        install_kind: "installer".into(),
        ..Default::default()
    };
    let roots = ["old", "new"].map(|name| (root.open_subkey(name).unwrap(), KEY_WOW64_64KEY));
    assert_eq!(
        detect_in_roots_for_installation("printcraft", roots, Some("x64"), Some(&selected))
            .unwrap()
            .unwrap()
            .path,
        selected.path
    );
    let missing = Installed {
        path: folder.join("missing").display().to_string(),
        ..selected
    };
    let roots = ["old", "new"].map(|name| (root.open_subkey(name).unwrap(), KEY_WOW64_64KEY));
    assert!(
        detect_in_roots_for_installation("printcraft", roots, Some("x64"), Some(&missing))
            .unwrap()
            .is_none()
    );
    // A folder discovered through Program Files is not registration ownership.
    let guessed = folder.join("PrintCraft");
    std::fs::create_dir_all(&guessed).unwrap();
    std::fs::write(guessed.join("printcraft.exe"), b"fixture").unwrap();
    let old_key = root
        .open_subkey_with_flags(r"old\product", KEY_READ | KEY_WRITE)
        .unwrap();
    old_key.set_value("InstallLocation", &"").unwrap();
    let previous_program_files = std::env::var_os("ProgramFiles");
    std::env::set_var("ProgramFiles", &folder);
    let guessed_selected = Installed {
        name: "printcraft".into(),
        path: guessed.display().to_string(),
        architecture: "x64".into(),
        install_kind: "installer".into(),
        ..Default::default()
    };
    let guessed_result = detect_in_roots_for_installation(
        "printcraft",
        [(root.open_subkey("old").unwrap(), KEY_WOW64_64KEY)],
        Some("x64"),
        Some(&guessed_selected),
    );
    match previous_program_files {
        Some(value) => std::env::set_var("ProgramFiles", value),
        None => std::env::remove_var("ProgramFiles"),
    }
    old_key
        .set_value("InstallLocation", &folder.join("old").display().to_string())
        .unwrap();
    assert!(guessed_result.unwrap().is_none());
    // PE architecture overrides registry view and preferred architecture overrides newest version.
    let mut pe = vec![0u8; 158];
    pe[..2].copy_from_slice(b"MZ");
    pe[60..64].copy_from_slice(&64u32.to_le_bytes());
    pe[64..68].copy_from_slice(b"PE\0\0");
    pe[68..70].copy_from_slice(&0x014cu16.to_le_bytes());
    pe[84..86].copy_from_slice(&70u16.to_le_bytes());
    pe[88..90].copy_from_slice(&0x10bu16.to_le_bytes());
    pe[156..158].copy_from_slice(&2u16.to_le_bytes());
    std::fs::write(folder.join("new/pdfcraft.exe"), pe).unwrap();
    let roots = ["old", "new"].map(|name| (root.open_subkey(name).unwrap(), KEY_WOW64_64KEY));
    let x86 = detect_in_roots_for_architecture("printcraft", roots, Some("x86"))
        .unwrap()
        .unwrap();
    assert_eq!(x86.architecture, "x86");
    assert_eq!(x86.version, "0.4.1");
    let roots = ["old", "new"].map(|name| (root.open_subkey(name).unwrap(), KEY_WOW64_64KEY));
    let x64 = detect_in_roots_for_architecture("printcraft", roots, Some("x64"))
        .unwrap()
        .unwrap();
    assert_eq!(x64.architecture, "x64");
    assert_eq!(x64.version, "0.2.1");
    root.delete_subkey_all(r"new\product").unwrap();
    let roots = ["old", "new"].map(|name| (root.open_subkey(name).unwrap(), KEY_WOW64_64KEY));
    assert_eq!(
        detect_in_roots("printcraft", roots)
            .unwrap()
            .unwrap()
            .version,
        "0.2.1"
    );
}
#[test]
fn custom_install_location_rejects_existing_folders() {
    let root = std::env::temp_dir().join(format!("craft-msi-location-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    assert!(super::validate_custom_location(&root).is_err());
    assert!(super::validate_custom_location(&root.join("new-app")).is_ok());
    assert!(super::validate_custom_location(std::path::Path::new("relative")).is_err());
    assert!(super::validate_custom_location(&root.join("..").join("elsewhere")).is_err());
    std::fs::remove_dir(&root).unwrap();
}
#[test]
fn installed_versions_compare_numerically() {
    use super::installation_version as version;
    assert!(version("0.10.0") > version("0.9.0"));
    assert!(version("0.4.1.1") > version("0.4.1"));
    assert!(version("0.4.1") > version("unknown"));
    assert_eq!(version("v0.4.1"), version("0.4.1.0"));
}
#[test]
fn names_allow_version_and_architecture_but_reject_other_products() {
    for name in [
        "PhotoCraft",
        " PhotoCraft ",
        "PhotoCraft 0.3.0",
        "PhotoCraft 0.3.0 (x64)",
        "PhotoCraft (32-bit)",
    ] {
        assert!(super::matches_display_name("photocraft", name), "{name}");
    }
    for name in ["PrintCraft", "PDFCraft", "PrintCraft 0.2.1 (64-bit)"] {
        assert!(super::matches_display_name("printcraft", name), "{name}");
    }
    for name in [
        "PhotoCraft Helper",
        "PhotoCraftExtra",
        "Unrelated PhotoCraft",
        "PhotoCraft 0.3.0 Helper",
    ] {
        assert!(!super::matches_display_name("photocraft", name), "{name}");
    }
}
