use super::*;

struct Fixture {
    root: PathBuf,
    paths: Paths,
    folder: PathBuf,
    desktop: PathBuf,
    marker: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("craft-installer-links-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.join("library"), None);
        let folder = root.join("installed");
        let desktop = root.join("desktop");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(&desktop).unwrap();
        std::fs::write(folder.join("pdfcraft.exe"), b"isolated fixture").unwrap();
        files::write_json(&paths.at("installations.json"), &serde_json::json!({
            "appsRoot":paths.root,"apps":[{"name":"printcraft","path":folder,"version":"1.0.0","architecture":"x64","install_kind":"installer"}]
        })).unwrap();
        let marker = paths.at("runtime/desktop-shortcuts/printcraft.json");
        Self {
            root,
            paths,
            folder,
            desktop,
            marker,
        }
    }
    fn owner(&self) -> Ownership {
        if self.marker.exists() {
            files::read_json(&self.marker).unwrap()
        } else {
            Ownership {
                installation_id: None,
                app: "printcraft".into(),
                library: self.paths.root.clone(),
                shortcut: None,
            }
        }
    }
    fn create(&self) {
        create_installation_shortcut(
            &self.paths,
            &self.marker,
            self.owner(),
            &self.folder,
            &self.desktop,
            "Installer",
        )
        .unwrap();
    }
    fn publisher_link(&self, args: &str, target: &Path) -> PathBuf {
        let link = self.desktop.join("PDFCraft.lnk");
        crate::platform::shortcut(&link, target, args, &self.folder).unwrap();
        link
    }
    fn count(&self) -> usize {
        std::fs::read_dir(&self.desktop).unwrap().count()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn installer_reuses_existing_pdfcraft_shortcut_without_creating_duplicate() {
    let f = Fixture::new();
    let link = f.publisher_link("", &f.folder.join("pdfcraft.exe"));
    let original = std::fs::read(&link).unwrap();
    f.create();
    assert_eq!(f.count(), 1);
    assert_eq!(std::fs::read(&link).unwrap(), original);
    assert!(
        !f.marker.exists(),
        "An installer/user-owned shortcut must not be adopted for deletion"
    );
}

#[test]
fn installer_does_not_reuse_shortcuts_with_arguments_or_other_targets() {
    for other_target in [false, true] {
        let f = Fixture::new();
        let target = f.folder.join(if other_target {
            "other.exe"
        } else {
            "pdfcraft.exe"
        });
        std::fs::write(&target, b"isolated executable").unwrap();
        f.publisher_link(if other_target { "" } else { "/uninstall" }, &target);
        f.create();
        assert_eq!(
            f.count(),
            2,
            "A different action/copy does not replace the requested launch shortcut"
        );
    }
}

#[test]
fn installer_removes_only_verified_manager_duplicate_and_preserves_user_edits() {
    for edited in [false, true] {
        let f = Fixture::new();
        f.create();
        let owned = f.owner().shortcut.unwrap().0;
        if edited {
            std::fs::write(&owned, b"user edited link").unwrap();
        }
        let publisher = f.publisher_link("", &f.folder.join("pdfcraft.exe"));
        let original = std::fs::read(&publisher).unwrap();
        f.create();
        assert_eq!(owned.exists(), edited);
        assert_eq!(std::fs::read(&publisher).unwrap(), original);
        assert_eq!(f.count(), if edited { 2 } else { 1 });
    }
}

#[test]
fn installer_reuses_public_desktop_link_without_owning_or_modifying_it() {
    let f = Fixture::new();
    f.create();
    let owned = f.owner().shortcut.unwrap().0;
    let shared = f.root.join("public-desktop");
    std::fs::create_dir_all(&shared).unwrap();
    let publisher = shared.join("PDFCraft.lnk");
    crate::platform::shortcut(&publisher, &f.folder.join("pdfcraft.exe"), "", &f.folder).unwrap();
    let original = std::fs::read(&publisher).unwrap();
    assert!(
        reuse_installer_shortcut(&f.marker, &mut f.owner(), &f.folder, &f.desktop, &[&shared])
            .unwrap()
    );
    assert!(!owned.exists());
    assert_eq!(std::fs::read(&publisher).unwrap(), original);
    assert!(f.owner().shortcut.is_none());
}
