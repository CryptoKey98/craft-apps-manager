use craft_apps_manager::manager_updates::api::{CheckTrigger, UpdateStatus};
#[cfg(not(feature = "service-updater"))]
use craft_apps_manager::self_update::Plan;
#[test]
fn host_identity_json_contract() {
    let identity = craft_apps_manager::manager_updates::api::ExpectedIdentity {
        version: "1.2.3".into(),
        revision: 2,
        architecture: "x64".into(),
        platform: "windows".into(),
    };
    let value = serde_json::to_value(identity).unwrap();
    assert_eq!(value["revision"], 2);
    assert_eq!(value.as_object().unwrap().len(), 4);
}
#[test]
fn host_api_is_available_without_service() {
    let _ = CheckTrigger::Periodic(60);
    assert!(matches!(UpdateStatus::Current, UpdateStatus::Current));
}
#[cfg(not(feature = "service-updater"))]
#[test]
fn absent_service_rejects_authority_before_writes() {
    let root = std::env::temp_dir().join(format!("no-service-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("plan.json");
    let plan = Plan {
        expected_identity: Some(craft_apps_manager::manager_updates::api::ExpectedIdentity {
            version: "1.2.3".into(),
            revision: 2,
            architecture: "x64".into(),
            platform: "windows".into(),
        }),
        layout: 0,
        msi: false,
        linux_package: false,
        parent_pid: 0,
        target: root.join("never-touch"),
        staged: root.join("missing"),
        hash: String::new(),
        root: root.clone(),
        tools: root.clone(),
    };
    std::fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let error = craft_apps_manager::self_update::apply(&path).unwrap_err();
    assert!(format!("{error:#}")
        .contains("Update service authorization requires a service-enabled manager"));
    assert!(!plan.target.exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(not(feature = "service-updater"))]
#[test]
fn v2_discriminator_cannot_be_deserialized_as_legacy_plan() {
    let root = std::env::temp_dir().join(format!("no-service-schema-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("plan.json");
    let value = serde_json::json!({"schema":2,"expected_identity":null,"layout":0,"msi":false,"linux_package":false,"parent_pid":0,"target":root.join("untouched"),"staged":root.join("missing"),"hash":"","root":root,"tools":root});
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let error = craft_apps_manager::self_update::apply(&path).unwrap_err();
    assert!(format!("{error:#}")
        .contains("Update service authorization requires a service-enabled manager"));
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn embedded_settings_match_requested_build() {
    let identity = craft_apps_manager::update_revisions::build_identity();
    if let Ok(value) = std::env::var("CRAFT_EXPECT_FINGERPRINT") {
        assert_eq!(
            identity["update_config_fingerprint"].as_str().unwrap(),
            value
        );
    }
    assert_eq!(identity["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(identity["update_source"], env!("CRAFT_UPDATE_SOURCE"));

    for (expected, actual) in [
        ("CRAFT_EXPECT_SOURCE", env!("CRAFT_UPDATE_SOURCE")),
        ("CRAFT_EXPECT_REPOSITORY", env!("CRAFT_MANAGER_REPOSITORY")),
        ("CRAFT_EXPECT_REVISION", env!("CRAFT_BUILD_REVISION")),
        (
            "CRAFT_EXPECT_KEY",
            env!("CRAFT_UPDATE_PUBLIC_KEY_FINGERPRINT"),
        ),
    ] {
        if let Ok(value) = std::env::var(expected) {
            assert_eq!(value, actual, "{expected}");
        }
    }
}
