use craft_apps_manager::model;
use serde_json::json;

#[test]
fn persisted_model_records_preserve_legacy_fields_and_defaults() {
    let legacy = json!({"name":"filmcraft","version":"1.2.3","path":"saved/library"});
    let installed: model::Installed = serde_json::from_value(legacy.clone()).unwrap();
    let expected = json!({"name":"filmcraft","version":"1.2.3","path":"saved/library",
        "architecture":"","install_kind":"","product_code":"","installation_label":"","package_type":""});
    assert_eq!(serde_json::to_value(installed).unwrap(), expected);
    let config: model::Config =
        serde_json::from_value(json!({"appsRoot":"saved/library","apps":[legacy]})).unwrap();
    assert_eq!(
        serde_json::to_value(config).unwrap(),
        json!({"appsRoot":"saved/library",
        "apps":[expected],"installations":[],"portableLabelCounters":{}})
    );

    let prefs: model::Preferences =
        serde_json::from_value(json!({"checkUpdaterOnStartup":false})).unwrap();
    let saved = serde_json::to_value(prefs).unwrap();
    assert_eq!(saved["theme"], "system");
    assert_eq!(saved["checkManagerOnStartup"], false);
    assert!(saved.get("checkUpdaterOnStartup").is_none());
    let builder: model::BuilderPreferences = serde_json::from_value(json!({})).unwrap();
    assert_eq!(
        serde_json::to_value(builder).unwrap(),
        json!({"deleteCacheAfterSuccess":true,
        "deleteWorkspaceAfterSuccess":true,"logSizeMB":10,"logArchives":2})
    );

    let source = json!({"sha":"commit","branch":"main","repository":"owner/repository",
        "archiveSha256":"digest","downloadedAt":"timestamp"});
    let record: model::Source = serde_json::from_value(source.clone()).unwrap();
    assert_eq!(serde_json::to_value(record).unwrap(), source);
    let build = json!({"app":"filmcraft","commit":"commit","sourceBranch":"main",
        "builtAt":"timestamp","profile":"release","log":"saved/build.log"});
    let record: model::BuildInfo = serde_json::from_value(build.clone()).unwrap();
    assert_eq!(serde_json::to_value(record).unwrap(), build);
    let release: model::Release = serde_json::from_value(json!({"tag_name":"v1.2.3",
        "draft":false,"prerelease":false,"assets":[{"name":"package.zip","size":1,
        "browser_download_url":"https://example.invalid/package.zip"}]}))
    .unwrap();
    assert_eq!(
        serde_json::to_value(release).unwrap(),
        json!({"tag_name":"v1.2.3",
        "name":null,"draft":false,"prerelease":false,"assets":[{"name":"package.zip",
        "size":1,"browser_download_url":"https://example.invalid/package.zip","digest":null}]})
    );
}
