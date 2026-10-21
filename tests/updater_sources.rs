use craft_apps_manager::manager_updates::coordinator::{selected_source, UpdateSource};
#[cfg(feature = "service-updater")]
mod common_updater;

#[test]
fn source_build_choice_is_explicit_and_rejects_unknown_modes() {
    assert_eq!(selected_source("github").unwrap(), UpdateSource::Github);
    assert_eq!(selected_source("service").unwrap(), UpdateSource::Service);
    for invalid in ["", "automatic", "SERVICE", "github,service"] {
        assert!(selected_source(invalid).is_err());
    }
}

#[test]
fn normal_source_build_defaults_to_github_and_release_activation_is_explicit() {
    let build = include_str!("../build.rs");
    assert!(build.contains("CRAFT_UPDATE_SOURCE"));
    assert!(include_str!("../updater.toml").contains("source = \"github\""));
    let workflow = include_str!("../.github/workflows/release.yml");
    assert!(workflow.contains("CRAFT_UPDATE_SOURCE: ${{ vars.CRAFT_UPDATE_V2_ENABLED == 'true' && 'service' || 'github' }}"));
}

#[cfg(feature = "service-updater")]
#[test]
#[ignore = "run explicitly once in github mode and once in service mode"]
fn compiled_source_dispatch_preserves_service_trust_and_never_falls_back() {
    use craft_apps_manager::{
        files,
        manager_updates::{
            check_state::CheckTrigger, coordinator::check_status, model::CheckOutcome,
        },
        model::Paths,
        self_update::REPOSITORY_NAME,
    };
    use sha2::{Digest, Sha256};
    let temp = common_updater::Temp::new();
    let paths = Paths::new(temp.0.clone(), None);
    let url = format!("https://api.github.com/repos/{REPOSITORY_NAME}/releases/latest");
    files::write_json(&temp.0.join("runtime/api-cache").join(format!("{:x}.json", Sha256::digest(url.as_bytes()))),
        &serde_json::json!({"at":chrono::Utc::now().timestamp(),"etag":null,"value":{"tag_name":"v0.0.1","draft":false,"prerelease":false,"assets":[]}})).unwrap();
    let scope = format!("{REPOSITORY_NAME}\nstable");
    let trust = temp
        .0
        .join("runtime/manager-updates")
        .join(format!("trust-{:x}.json", Sha256::digest(scope.as_bytes())));
    files::write_bytes(&trust, b"deliberately invalid service trust").unwrap();
    let result = check_status(&paths, CheckTrigger::Manual);
    match selected_source(env!("CRAFT_UPDATE_SOURCE")).unwrap() {
        UpdateSource::Github => assert!(matches!(result.unwrap().outcome, CheckOutcome::Current)),
        UpdateSource::Service => assert!(
            result.is_err(),
            "Service trust failure must not return the cached GitHub result"
        ),
    }
    assert_eq!(
        std::fs::read(&trust).unwrap(),
        b"deliberately invalid service trust"
    );
}
