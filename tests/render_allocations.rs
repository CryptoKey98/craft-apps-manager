//! Deterministic allocation budgets for metadata used on every UI frame.
use craft_apps_manager::{catalog, files};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountingAllocator;
thread_local! {
    static BYTES: Cell<Option<usize>> = const { Cell::new(None) };
}
fn count(bytes: usize) {
    let _ = BYTES.try_with(|total| {
        if let Some(value) = total.get() {
            total.set(Some(value + bytes));
        }
    });
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        count(layout.size());
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        count(layout.size());
        ptr
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let ptr = System.realloc(ptr, layout, size);
        count(size);
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn allocated<T>(read: impl FnOnce() -> T) -> (T, usize) {
    BYTES.with(|total| total.set(Some(0)));
    let value = read();
    let bytes = BYTES.with(|total| total.replace(None).unwrap());
    (value, bytes)
}

struct Library(std::path::PathBuf);
impl Drop for Library {
    fn drop(&mut self) {
        catalog::load(&self.0.join("empty"));
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn single_catalog_lookup_does_not_copy_unrelated_apps() {
    let root = Library(std::env::temp_dir().join(format!("craft-lookup-{}", uuid::Uuid::new_v4())));
    let fixture = catalog::get("filmcraft").unwrap();
    let mut entries = Vec::new();
    for i in 0..256 {
        let mut entry = fixture.clone();
        entry.key = format!("fixturecraft{i}");
        entry.repository = entry.key.clone();
        entry.description = "Unrelated app metadata. ".repeat(100);
        entries.push(entry);
    }
    files::write_json(
        &root.0.join("runtime/catalog.json"),
        &serde_json::json!({
            "checkedAt": 0, "entries": entries, "learned": {}
        }),
    )
    .unwrap();
    catalog::load(&root.0);
    assert!(catalog::all().len() >= 256);
    let (keys, key_bytes) = allocated(catalog::release_keys);
    let (ordered, order_bytes) =
        allocated(|| catalog::metadata::ordered_release_apps(&keys, "portable"));
    let (_, title_bytes) = allocated(|| catalog::metadata::title("fixturecraft42"));
    println!("List keys: {key_bytes} bytes; ordered list: {order_bytes}; title: {title_bytes}");
    assert_eq!(ordered, keys);
    assert!(
        key_bytes < 64 * 1024,
        "App keys must not copy release metadata"
    );
    assert!(
        order_bytes < 64 * 1024,
        "Ordering must not copy release metadata"
    );
    let (_, expected_title_bytes) = allocated(|| entries[42].title.clone());
    assert_eq!(
        title_bytes, expected_title_bytes,
        "A label must only copy its text"
    );
    let (entry, bytes) = allocated(|| catalog::get("filmcraft").unwrap());
    let (_, old_bytes) = allocated(|| catalog::all().into_iter().find(|e| e.key == "filmcraft"));
    println!("Single lookup: {bytes} bytes; whole-catalog lookup: {old_bytes} bytes");
    assert_eq!(entry, fixture);
    let (_, selected_bytes) = allocated(|| fixture.clone());
    assert_eq!(
        bytes, selected_bytes,
        "Lookup must only copy the matching entry"
    );
    let (missing, bytes) = allocated(|| catalog::get("not-an-app"));
    assert!(missing.is_none());
    assert_eq!(bytes, 0, "A missing lookup must not copy the catalog");
    let mut detached = entry;
    detached.title = "Edited returned copy".into();
    assert_ne!(detached.title, fixture.title);
    assert_eq!(catalog::get("filmcraft").unwrap(), fixture);
    assert_eq!(
        catalog::get("fixturecraft42").unwrap().description,
        entries[42].description
    );
    // Reloads must be observed immediately; no stale lookup cache is introduced.
    entries[42].description = "Changed publisher metadata".into();
    files::write_json(
        &root.0.join("runtime/catalog.json"),
        &serde_json::json!({
            "checkedAt": 0, "entries": entries, "learned": {}
        }),
    )
    .unwrap();
    catalog::load(&root.0);
    assert_eq!(
        catalog::get("fixturecraft42").unwrap().description,
        "Changed publisher metadata"
    );
    entries[42].title = "Renamed app".into();
    entries[42].category = "Changed category".into();
    entries[42].release = false;
    entries[42].source = true;
    files::write_json(
        &root.0.join("runtime/catalog.json"),
        &serde_json::json!({"checkedAt": 0, "entries": entries, "learned": {}}),
    )
    .unwrap();
    catalog::load(&root.0);
    assert_eq!(catalog::metadata::title("fixturecraft42"), "Renamed app");
    assert_eq!(
        catalog::metadata::category("fixturecraft42"),
        "Changed category"
    );
    assert!(!catalog::release_keys()
        .iter()
        .any(|key| key == "fixturecraft42"));
    assert!(catalog::source_keys()
        .iter()
        .any(|key| key == "fixturecraft42"));
    assert_eq!(
        catalog::metadata::ordered_release_apps(
            &[
                "missing".into(),
                "fixturecraft42".into(),
                "filmcraft".into()
            ],
            "installer"
        ),
        vec!["filmcraft"]
    );
}

#[test]
fn display_options_do_not_copy_unrelated_settings_and_keep_selection_rules() {
    use craft_apps_manager::{
        install_options,
        model::{Installed, Preferences},
    };
    let mut base = Preferences {
        selected_apps: vec!["Large unrelated preference".repeat(40_000)],
        ..Default::default()
    };
    let installed = Installed {
        name: "filmcraft".into(),
        path: "fixture/filmcraft".into(),
        version: "1.0.0".into(),
        architecture: "x86".into(),
        install_kind: "installer".into(),
        ..Default::default()
    };
    let saved = install_options::InstallOptions {
        release_format: "portable".into(),
        architecture: "x64".into(),
    };
    base.app_install_options
        .insert(install_options::identity("filmcraft"), saved.clone());
    let (options, bytes) =
        allocated(|| install_options::for_selection(&base, "filmcraft", Some(&installed), false));
    let (_, old_bytes) = allocated(|| install_options::for_installed(&base, &installed).unwrap());
    println!("Display options: {bytes} bytes; full settings: {old_bytes} bytes");
    let (_, selected_bytes) =
        allocated(|| install_options::InstallOptions::installed(&installed).unwrap());
    assert_eq!(
        bytes, selected_bytes,
        "Display options must only copy format and architecture"
    );
    assert_eq!(
        options,
        install_options::InstallOptions::installed(&installed).unwrap()
    );
    assert_eq!(
        install_options::for_selection(&base, "filmcraft", Some(&installed), true),
        saved
    );
    base.app_install_options.clear();
    // An existing verified installation remains the fallback when no explicit intent exists.
    assert_eq!(
        install_options::for_selection(&base, "filmcraft", Some(&installed), true),
        options
    );
    let defaults = install_options::preferred(&base, "filmcraft");
    let options = install_options::for_selection(&base, "filmcraft", None, false);
    assert_eq!(options.release_format, defaults.release_format);
    assert_eq!(options.architecture, defaults.architecture);
    let invalid = Installed {
        architecture: "unknown".into(),
        ..installed.clone()
    };
    assert_eq!(
        install_options::for_selection(&base, "filmcraft", Some(&invalid), false),
        options
    );
    let other_app = Installed {
        name: "designcraft".into(),
        ..installed.clone()
    };
    assert_eq!(
        install_options::for_selection(&base, "filmcraft", Some(&other_app), false),
        options
    );
    let universal = Installed {
        architecture: "universal".into(),
        ..installed
    };
    assert_eq!(
        install_options::for_selection(&base, "filmcraft", Some(&universal), false).architecture,
        install_options::host_architecture()
    );
    // No stale memoized choices after a preference edit or selected-copy change.
    base.app_install_options
        .insert(install_options::identity("filmcraft"), saved.clone());
    assert_eq!(
        install_options::for_selection(&base, "filmcraft", None, false),
        saved
    );
    let operation = install_options::apply(&base, &options);
    assert_eq!(operation.selected_apps, base.selected_apps);
    assert_eq!(operation.app_install_options, base.app_install_options);
}
