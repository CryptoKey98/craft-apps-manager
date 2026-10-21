#[test]
fn release_labels_cannot_replace_app_names() {
    use super::release_title;
    assert_eq!(
        release_title(Some("Release v0.101.2362.0"), "PowerToys"),
        "PowerToys"
    );
    assert_eq!(release_title(Some("Wordleaf"), "PowerToys"), "PowerToys");
    assert_eq!(
        release_title(Some("OBS Studio 32.2.2"), "obs-studio"),
        "OBS Studio"
    );
    assert_eq!(
        release_title(Some("MapCraft v1.2.0"), "mapcraft"),
        "MapCraft"
    );
    assert_eq!(
        release_title(Some("Autumn update"), "Example_App"),
        "Example App"
    );
    assert_eq!(release_title(None, "KeePassXC"), "KeePassXC");
}
