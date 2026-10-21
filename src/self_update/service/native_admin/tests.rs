use super::*;
#[test]
fn final_acknowledgement_is_observable_but_cannot_authorize_another_install() {
    let mut gate = Gate {
        token: "approved".into(),
        expires: 200,
        count: 1,
        next: 0,
        pending: None,
    };
    assert!(gate.close("approved", 100).is_err());
    gate.claim("approved", 0, 100).unwrap();
    assert!(gate.close("approved", 100).is_err());
    assert!(gate.acknowledge("approved", 0, 100).unwrap());
    assert_eq!(gate.next, 1);
    assert_eq!(gate.pending, None);
    assert!(gate.claim("approved", 1, 100).is_err());
    assert!(gate.claim("approved", 0, 100).is_err());
    assert!(gate.close("wrong", 100).is_err());
    assert!(gate.close("approved", 200).is_err());
    gate.close("approved", 100).unwrap();
}
#[test]
#[cfg(target_os = "macos")]
fn macos_missing_session_is_not_a_verified_acknowledgement() {
    let root = std::env::temp_dir().join(format!("craft-missing-ack-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let (manifest, trust) = mac_fixture(super::super::model::ZIP, super::super::model::DMG);
    let catalog = verify_catalog(&manifest.envelope, &trust, 1001, None).unwrap();
    assert!(acknowledge(&root, &catalog.catalog().packages[0]).is_err());
    fs::remove_dir(root).unwrap();
}
#[test]
#[cfg(target_os = "macos")]
fn frozen_bundle_survives_a_signed_main_executable_rename() {
    assert!(session_target_matches(
        Path::new("/Applications/Manager.app/Contents/MacOS/old"),
        Path::new("/Applications/Manager.app/Contents/MacOS/new")
    ));
    assert!(!session_target_matches(
        Path::new("/Applications/Manager.app/Contents/MacOS/old"),
        Path::new("/Applications/Other.app/Contents/MacOS/new")
    ));
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("craft-renamed-binding-{}", uuid::Uuid::new_v4()));
    let bundle = root.join("Manager.app");
    fs::create_dir_all(bundle.join("Contents")).unwrap();
    fs::write(bundle.join("Contents/Info.plist"), b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleExecutable</key><string>new</string></dict></plist>").unwrap();
    assert!(
        macos_plan_target_matches(&bundle.join("Contents/MacOS/old"), &bundle),
        "The approved bundle must remain bound after its signed entrypoint changes"
    );
    assert!(!macos_plan_target_matches(
        &bundle.join("Contents/MacOS/old"),
        &root.join("Other.app")
    ));
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn failed_preparation_removes_secrets_but_keeps_downloads_and_diagnostics() {
    let home = std::env::temp_dir().join(format!("craft-preparation-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&home).unwrap();
    for name in [
        "approved.json",
        "session.json",
        "package-0.package",
        "authorization.log",
    ] {
        fs::write(home.join(name), b"owned").unwrap();
    }
    drop(PreparationGuard {
        home: home.clone(),
        keep: false,
    });
    assert!(!home.join("approved.json").exists());
    assert!(!home.join("session.json").exists());
    assert_eq!(fs::read(home.join("package-0.package")).unwrap(), b"owned");
    assert!(home.join("authorization.log").exists());
    fs::write(home.join("session.json"), b"active").unwrap();
    drop(PreparationGuard {
        home: home.clone(),
        keep: true,
    });
    assert_eq!(fs::read(home.join("session.json")).unwrap(), b"active");
    fs::remove_dir_all(home).unwrap();
}
fn mac_fixture(first: &str, second: &str) -> (Manifest, TrustContext) {
    let (mut manifest, trust) = fixture();
    let envelope: serde_json::Value = serde_json::from_slice(&manifest.envelope).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_str(envelope["payload"].as_str().unwrap()).unwrap();
    let caps = serde_json::json!([
        first,
        second,
        "layout.manager.v1",
        "admin.bundle-replace.v1"
    ]);
    for (index, p) in value["packages"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        let handler = if index == 0 { first } else { second };
        p["platform"] = "macos".into();
        p["installation"] = "bundle".into();
        p["handler"] = handler.into();
        p["required_capabilities"] = serde_json::json!([handler, "layout.manager.v1"]);
        p.as_object_mut().unwrap().remove("native_version");
    }
    for r in value["routes"].as_array_mut().unwrap() {
        r["platform"] = "macos".into();
        r["installation"] = "bundle".into();
        r["required_capabilities"] =
            serde_json::json!([if r["id"] == "to-b" { first } else { second }]);
        r["destination_capabilities"] = caps.clone();
    }
    let payload = serde_json::to_string(&value).unwrap();
    let signer = SigningKey::from_bytes(&[7; 32]);
    let signature = signer
        .sign(payload.as_bytes())
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    manifest.envelope =
        serde_json::to_vec(&serde_json::json!({"payload":payload,"signature":signature})).unwrap();
    manifest.source.platform = "macos".into();
    manifest.source.installation = InstallationKind::Bundle;
    manifest.source.capabilities = caps
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    let catalog = verify_catalog(&manifest.envelope, &trust, 1001, None).unwrap();
    let super::super::model::CheckOutcome::BridgeUpdate(offer) =
        super::super::routing::select_update(&catalog, &manifest.source).unwrap()
    else {
        panic!("Expected macOS bridge")
    };
    manifest.offer = offer;
    (manifest, trust)
}
#[test]
fn macos_scope_accepts_supported_bundle_transports() {
    use super::super::model::{DMG, TAR, ZIP};
    for (first, second) in [(ZIP, DMG), (ZIP, TAR), (DMG, ZIP)] {
        let (manifest, trust) = mac_fixture(first, second);
        let catalog = verify_catalog(&manifest.envelope, &trust, 1001, None).unwrap();
        let packages = native_packages(&catalog, &manifest.source, &manifest.offer)
            .expect("Signed macOS bundles must be eligible for exact scoped approval");
        assert_eq!(
            packages
                .iter()
                .map(|p| p.handler.as_str())
                .collect::<Vec<_>>(),
            vec![first, second]
        );
    }
}
#[test]
fn macos_direct_grant_does_not_require_a_bridge_journey() {
    let (mut manifest, trust) = mac_fixture(super::super::model::ZIP, super::super::model::DMG);
    manifest.source.version = "1.1.0".into();
    let catalog = verify_catalog(&manifest.envelope, &trust, 1001, None).unwrap();
    let super::super::model::CheckOutcome::DirectUpdate(offer) =
        super::super::routing::select_update(&catalog, &manifest.source).unwrap()
    else {
        panic!("Expected direct update")
    };
    manifest.offer = offer;
    manifest.packages = vec![PathBuf::from("two")];
    assert!(
        approved(&manifest, &trust, 1001).is_ok(),
        "A signed direct bundle update needs one scoped approval"
    );
}
#[test]
fn macos_step_scope_limits_packages_without_changing_signed_offer() {
    let (mut manifest, trust) = mac_fixture(super::super::model::ZIP, super::super::model::DMG);
    let mut record = serde_json::to_value(&manifest).unwrap();
    record["grant_span"] = "step".into();
    manifest = serde_json::from_value(record).expect("Explicit single-step grant must deserialize");
    manifest.packages.truncate(1);
    assert_eq!(manifest.offer.route_ids, vec!["to-b", "to-c"]);
    assert!(approved(&manifest, &trust, 1001).is_ok());
    manifest
        .packages
        .push(PathBuf::from("unapproved-second-step"));
    assert!(approved(&manifest, &trust, 1001).is_err());
}
#[test]
fn native_manifest_default_encoding_and_bounded_bridge_are_preserved() {
    let (manifest, trust) = fixture();
    let value = serde_json::to_value(&manifest).unwrap();
    assert!(value.get("grant_span").is_none());
    assert!(approved(
        &serde_json::from_value(value.clone()).unwrap(),
        &trust,
        1001
    )
    .is_ok());
    let mut step = value;
    step["grant_span"] = "step".into();
    if let Ok(manifest) = serde_json::from_value::<Manifest>(step) {
        assert!(
            approved(&manifest, &trust, 1001).is_err(),
            "Native bridge authorization cannot be truncated"
        );
    }
    let (mut manifest, trust) = fixture();
    manifest.offer.route_ids.truncate(1);
    manifest.packages.truncate(1);
    assert!(approved(&manifest, &trust, 1001).is_err());
}
#[test]
fn macos_scope_rejects_missing_capability_and_changed_source() {
    let (manifest, trust) = mac_fixture(super::super::model::ZIP, super::super::model::DMG);
    for change in ["admin", "handler", "architecture", "platform", "repository"] {
        let mut changed = manifest.clone();
        match change {
            "admin" => {
                changed
                    .source
                    .capabilities
                    .remove("admin.bundle-replace.v1");
            }
            "handler" => {
                changed.source.capabilities.remove(super::super::model::DMG);
            }
            "architecture" => changed.source.architecture = "arm64".into(),
            "platform" => changed.source.platform = "linux".into(),
            _ => changed.source.repository = "different/manager".into(),
        }
        assert!(
            approved(&changed, &trust, 1001).is_err(),
            "Changed {change} must reject the scope"
        );
    }
}
#[test]
fn native_admin_approval_is_ordered_single_use_and_expires() {
    let mut gate = Gate {
        token: "approved".into(),
        expires: 200,
        count: 2,
        next: 0,
        pending: None,
    };
    assert!(gate.claim("wrong", 0, 100).is_err());
    assert!(gate.claim("approved", 1, 100).is_err());
    gate.claim("approved", 0, 100)
        .expect("First approved native package must run");
    assert!(gate.claim("approved", 0, 100).is_err());
    assert!(gate.acknowledge("wrong", 0, 100).is_err());
    assert!(gate.acknowledge("approved", 1, 100).is_err());
    assert!(!gate.acknowledge("approved", 0, 100).unwrap());
    assert!(gate.claim("approved", 1, 200).is_err());
    gate.claim("approved", 1, 199)
        .expect("Second package after restart must reuse approval");
    assert!(gate.acknowledge("approved", 1, 199).unwrap());
    assert!(gate.claim("approved", 2, 199).is_err());
}

fn fixture() -> (Manifest, TrustContext) {
    let mut c: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/updater-v2/catalog.json"
    )))
    .unwrap();
    for p in c["packages"].as_array_mut().unwrap() {
        p["installation"] = "deb".into();
        p["handler"] = "native.deb.v1".into();
        p["required_capabilities"] = serde_json::json!(["native.deb.v1", "layout.manager.v1"]);
        p["native_version"] = if p["id"] == "b-linux" {
            "1.1.0".into()
        } else {
            "1.2.0".into()
        };
    }
    for r in c["routes"].as_array_mut().unwrap() {
        r["installation"] = "deb".into();
        r["required_capabilities"] = serde_json::json!(["native.deb.v1"]);
        r["destination_capabilities"] = serde_json::json!(["native.deb.v1", "layout.manager.v1"]);
        if r["id"] == "to-c" {
            r["min_source"]["version"] = "1.1.0".into();
        }
    }
    let signer = SigningKey::from_bytes(&[7; 32]);
    let payload = serde_json::to_string(&c).unwrap();
    let signature = signer
        .sign(payload.as_bytes())
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let envelope =
        serde_json::to_vec(&serde_json::json!({"payload":payload,"signature":signature})).unwrap();
    let trust = TrustContext {
        repository: "owner/manager".into(),
        channel: "stable".into(),
        keys: vec![signer.verifying_key().to_bytes()],
    };
    let catalog = verify_catalog(&envelope, &trust, 1001, None).unwrap();
    let source = ClientProfile {
        repository: trust.repository.clone(),
        channel: "stable".into(),
        version: "1.0.0".into(),
        revision: 1,
        platform: "linux".into(),
        architecture: "x64".into(),
        installation: InstallationKind::Deb,
        capabilities: ["native.deb.v1".into(), "layout.manager.v1".into()]
            .into_iter()
            .collect(),
    };
    let super::super::model::CheckOutcome::BridgeUpdate(offer) =
        super::super::routing::select_update(&catalog, &source).unwrap()
    else {
        panic!("Expected a forced native bridge")
    };
    (
        Manifest {
            schema: 1,
            grant_span: GrantSpan::Journey,
            envelope,
            source,
            offer,
            expires: 2000,
            target: PathBuf::from("manager"),
            root: PathBuf::from("data"),
            source_hash: "0".repeat(64),
            account: "test".into(),
            callback: 1,
            secret: [9; 32],
            packages: vec![PathBuf::from("one"), PathBuf::from("two")],
        },
        trust,
    )
}
#[test]
fn native_admin_manifest_requires_signed_bounded_native_journey() {
    let (mut manifest, trust) = fixture();
    assert!(approved(&manifest, &trust, 1001).is_ok());
    assert!(approved(&manifest, &trust, 2000).is_err());
    manifest.packages.pop();
    assert!(approved(&manifest, &trust, 1001).is_err());
    let (mut manifest, trust) = fixture();
    manifest.envelope[0] = b'!';
    assert!(approved(&manifest, &trust, 1001).is_err());
    let (mut manifest, trust) = fixture();
    manifest.source.installation = InstallationKind::Portable;
    assert!(approved(&manifest, &trust, 1001).is_err());
    let (mut manifest, trust) = fixture();
    manifest.expires = 1001 + 3601;
    assert!(approved(&manifest, &trust, 1001).is_err());
}
#[test]
fn native_admin_nonblocking_listener_reads_delayed_authenticated_frame() {
    for role in ["native-worker", "native-client"] {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut client = connection(listener.local_addr().unwrap().port()).unwrap();
        let mut accepted = accepted_connection(&listener).unwrap();
        let writer = std::thread::spawn(move || {
            // A peer need not have its response ready when accept completes.
            std::thread::sleep(Duration::from_millis(50));
            wire_send(
                &mut client,
                &[9; 32],
                &Hello {
                    role: role.into(),
                    port: 42,
                },
            )
        });
        let result = wire_read::<Hello>(&mut accepted, &[9; 32]);
        let sent = writer.join().unwrap();
        let hello = result.expect("Accepted connection must wait for the authenticated frame");
        sent.unwrap();
        assert_eq!(hello.role, role);
        assert_eq!(hello.port, 42);
    }
}
#[test]
fn native_admin_frames_reject_wrong_credentials_and_oversized_input() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let writer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        wire_send(
            &mut stream,
            &[1; 32],
            &Hello {
                role: "native-worker".into(),
                port: 4,
            },
        )
        .unwrap();
    });
    let mut reader = TcpStream::connect(address).unwrap();
    assert!(wire_read::<Hello>(&mut reader, &[2; 32]).is_err());
    writer.join().unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let writer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .write_all(&((MAX_MESSAGE + 1) as u32).to_be_bytes())
            .unwrap();
    });
    let mut reader = TcpStream::connect(address).unwrap();
    assert!(wire_read::<Hello>(&mut reader, &[1; 32]).is_err());
    writer.join().unwrap();
}
#[test]
fn native_admin_new_connections_reuse_only_the_remaining_approved_index() {
    let (manifest, _) = fixture();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let session = Session {
        port: listener.local_addr().unwrap().port(),
        manifest,
    };
    let key = session.manifest.secret;
    let worker = std::thread::spawn(move || {
        let mut gate = Gate {
            token: "test".into(),
            expires: 2000,
            count: 2,
            next: 0,
            pending: None,
        };
        for expected in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let req: Request = wire_read(&mut stream, &key).unwrap();
            assert_eq!(req.role, "native-client");
            assert_eq!(req.index, expected);
            gate.claim("test", req.index, 1001).unwrap();
            wire_send(
                &mut stream,
                &key,
                &Reply {
                    role: "native-result".into(),
                    id: req.id,
                    code: Some(0),
                    error: None,
                    log: String::new(),
                },
            )
            .unwrap();
            gate.acknowledge("test", expected, 1001).unwrap();
        }
        assert!(gate.claim("test", 1, 1001).is_err());
    });
    for index in 0..2 {
        assert_eq!(
            exchange(&session, request("install", index), false, None)
                .unwrap()
                .code,
            Some(0)
        );
    }
    worker.join().unwrap();
}
#[test]
fn native_admin_snapshot_copy_is_size_and_checksum_bounded() {
    let root = std::env::temp_dir().join(format!("craft-native-copy-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let (manifest, trust) = fixture();
    let (catalog, _) = approved(&manifest, &trust, 1001).unwrap();
    let mut p = catalog.catalog().packages[0].clone();
    let source = root.join("source");
    fs::write(&source, b"approved").unwrap();
    p.size = 8;
    p.sha256 = files::hash(&source).unwrap();
    copy_package(&source, &root.join("valid"), &p).unwrap();
    p.size = 7;
    assert!(copy_package(&source, &root.join("oversized"), &p).is_err());
    p.size = 8;
    p.sha256 = "0".repeat(64);
    assert!(copy_package(&source, &root.join("tampered"), &p).is_err());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn native_admin_cannot_advance_before_installed_identity_acknowledgement() {
    let mut gate = Gate {
        token: "approved".into(),
        expires: 200,
        count: 2,
        next: 0,
        pending: None,
    };
    gate.claim("approved", 0, 100).unwrap();
    assert!(
        gate.claim("approved", 1, 100).is_err(),
        "The next installer must wait for verified completion acknowledgement"
    );
}

#[test]
#[cfg(not(target_os = "macos"))]
fn native_admin_expired_completed_install_revokes_continuation_without_failing_restart() {
    let root = std::env::temp_dir().join(format!("craft-native-expiry-{}", uuid::Uuid::new_v4()));
    let home = root.join("runtime/native-admin/test");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(root.join("runtime/manager-updates")).unwrap();
    let (mut manifest, trust) = fixture();
    let (catalog, _) = approved(&manifest, &trust, 1001).unwrap();
    let package = catalog.catalog().packages[0].clone();
    manifest.root = root.canonicalize().unwrap();
    manifest.expires = now() - 1;
    manifest.packages = vec![home.join("one"), home.join("two")];
    let descriptor = home.join("session.json");
    files::write_json(&descriptor, &Session { manifest, port: 1 }).unwrap();
    files::write_json(&marker(&root), &descriptor).unwrap();
    let proof = root.join("runtime/manager-updates/bridge-approval.json");
    fs::write(&proof, b"previous bridge consent").unwrap();
    let result = acknowledge(&root, &package);
    assert!(
        result.is_ok(),
        "A verified completed install must still restart: {result:?}"
    );
    assert!(!marker(&root).exists());
    assert!(!descriptor.exists());
    assert!(!proof.exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn msi_install_folder_quotes_only_the_property_value() {
    use std::process::Command;
    let capture =
        std::env::temp_dir().join(format!("craft-msi-command-{}.txt", uuid::Uuid::new_v4()));
    let test = concat!(module_path!(), "::capture_msi_property_command_line");
    let test = test.split_once("::").unwrap().1;
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", test, "--ignored", "--skip"]);
    command.env("CRAFT_MSI_COMMAND_CAPTURE", &capture);
    platform::msi_install_folder(&mut command, Path::new(r"\\?\C:\User Data\Manager")).unwrap();
    assert!(crate::platform::hidden(&mut command)
        .status()
        .unwrap()
        .success());
    let raw = fs::read_to_string(&capture).unwrap();
    fs::remove_file(capture).unwrap();
    assert!(
        raw.contains(r#"INSTALLFOLDER="C:\User Data\Manager""#),
        "Windows Installer needs quotes around the property value, not the whole argument: {raw}"
    );
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "Subprocess probe for Windows MSI command-line serialization; run by its parent test"]
fn capture_msi_property_command_line() {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCommandLineW() -> *const u16;
    }
    let raw = unsafe {
        windows::core::PCWSTR(GetCommandLineW())
            .to_string()
            .unwrap()
    };
    fs::write(std::env::var_os("CRAFT_MSI_COMMAND_CAPTURE").unwrap(), raw).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn msi_install_folder_rejects_command_line_delimiters() {
    for folder in [
        "C:\\bad\" OTHER=1",
        "C:\\bad\rOTHER=1",
        "C:\\bad\nOTHER=1",
        "C:\\bad\0OTHER=1",
    ] {
        assert!(platform::msi_install_folder(
            &mut std::process::Command::new("unused.exe"),
            Path::new(folder)
        )
        .is_err());
    }
}
