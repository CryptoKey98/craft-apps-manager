use crate::{model::Installed, platform};
use anyhow::{bail, Context, Result};
use std::path::Path;
use winreg::{enums::*, RegKey};
#[link(name = "msi")]
unsafe extern "system" {
    fn MsiInstallProductW(package: *const u16, properties: *const u16) -> u32;
    fn MsiConfigureProductExW(
        product: *const u16,
        level: i32,
        state: i32,
        properties: *const u16,
    ) -> u32;
    fn MsiSetInternalUI(level: u32, owner: *mut *mut std::ffi::c_void) -> u32;
    fn MsiSetExternalUIW(
        handler: MsiUiHandler,
        filter: u32,
        context: *mut std::ffi::c_void,
    ) -> MsiUiHandler;
    fn MsiQueryProductStateW(product: *const u16) -> i32;
    fn MsiGetProductInfoW(
        product: *const u16,
        property: *const u16,
        value: *mut u16,
        size: *mut u32,
    ) -> u32;
    fn MsiOpenDatabaseW(path: *const u16, mode: *const u16, handle: *mut u32) -> u32;
    fn MsiDatabaseOpenViewW(db: u32, query: *const u16, view: *mut u32) -> u32;
    fn MsiViewExecute(view: u32, record: u32) -> u32;
    fn MsiViewFetch(view: u32, record: *mut u32) -> u32;
    fn MsiRecordGetStringW(record: u32, field: u32, value: *mut u16, size: *mut u32) -> u32;
    fn MsiGetComponentPathW(
        product: *const u16,
        component: *const u16,
        path: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn MsiCloseHandle(handle: u32) -> u32;
}
type MsiUiHandler =
    Option<unsafe extern "system" fn(*mut std::ffi::c_void, u32, *const u16) -> i32>;
static MSI_UI_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct MsiUiGuard {
    previous: MsiUiHandler,
    level: u32,
}
impl Drop for MsiUiGuard {
    fn drop(&mut self) {
        unsafe {
            MsiSetExternalUIW(self.previous, 0, std::ptr::null_mut());
            MsiSetInternalUI(self.level, std::ptr::null_mut());
        }
    }
}
unsafe extern "system" fn installer_ui(
    context: *mut std::ffi::c_void,
    message_type: u32,
    _message: *const u16,
) -> i32 {
    if context.is_null() {
        return 0;
    }
    // Only action/progress messages accept IDCANCEL; errors remain Windows' responsibility.
    if !matches!(
        message_type & 0xff00_0000,
        0x0800_0000 | 0x0900_0000 | 0x0a00_0000
    ) {
        return 0;
    }
    let job = &*(context as *const crate::jobs::Job);
    if job.cancel.load(std::sync::atomic::Ordering::Relaxed) {
        2
    } else {
        0
    }
}
fn msi_transaction(
    job: Option<&crate::jobs::Job>,
    basic: bool,
    run: impl FnOnce() -> u32,
) -> Result<i32> {
    let _lock = MSI_UI_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Windows Installer UI lock failed"))?;
    if let Some(job) = job {
        job.check()?;
    }
    let context = job.map_or(std::ptr::null_mut(), |j| {
        j as *const _ as *mut std::ffi::c_void
    });
    let guard = unsafe {
        MsiUiGuard {
            level: MsiSetInternalUI(if basic { 3 } else { 5 }, std::ptr::null_mut()),
            previous: MsiSetExternalUIW(
                Some(installer_ui),
                (1 << 8) | (1 << 9) | (1 << 10),
                context,
            ),
        }
    };
    let result = run();
    drop(guard);
    Ok(result as i32)
}
struct MsiHandle(u32);
impl Drop for MsiHandle {
    fn drop(&mut self) {
        unsafe {
            MsiCloseHandle(self.0);
        }
    }
}
// Read the installed component key path: many MSI packages leave InstallLocation blank.
fn msi_app_folder(product: &str, app: &str) -> Option<String> {
    let product = platform::wide(product);
    let property = platform::wide("LocalPackage");
    let mut value = vec![0u16; 32768];
    let mut size = (value.len() - 1) as u32;
    if unsafe {
        MsiGetProductInfoW(
            product.as_ptr(),
            property.as_ptr(),
            value.as_mut_ptr(),
            &mut size,
        )
    } != 0
    {
        return None;
    }
    let mut db = 0;
    if unsafe { MsiOpenDatabaseW(value.as_ptr(), std::ptr::null(), &mut db) } != 0 {
        return None;
    }
    let db = MsiHandle(db);
    let query = platform::wide("SELECT `File`.`FileName`, `Component`.`ComponentId` FROM `File`, `Component` WHERE `File`.`Component_` = `Component`.`Component`");
    let mut view = 0;
    if unsafe { MsiDatabaseOpenViewW(db.0, query.as_ptr(), &mut view) } != 0 {
        return None;
    }
    let view = MsiHandle(view);
    if unsafe { MsiViewExecute(view.0, 0) } != 0 {
        return None;
    }
    loop {
        let mut record = 0;
        if unsafe { MsiViewFetch(view.0, &mut record) } != 0 {
            break;
        }
        let record = MsiHandle(record);
        let mut size = (value.len() - 1) as u32;
        if unsafe { MsiRecordGetStringW(record.0, 1, value.as_mut_ptr(), &mut size) } != 0 {
            continue;
        }
        let name = String::from_utf16_lossy(&value[..size as usize]);
        let name = name.rsplit('|').next()?;
        if !crate::model::executable_names(app)
            .iter()
            .any(|n| n.eq_ignore_ascii_case(name))
        {
            continue;
        }
        let mut size = (value.len() - 1) as u32;
        if unsafe { MsiRecordGetStringW(record.0, 2, value.as_mut_ptr(), &mut size) } != 0 {
            continue;
        }
        let component = value.clone();
        let mut size = (value.len() - 1) as u32;
        if unsafe {
            MsiGetComponentPathW(
                product.as_ptr(),
                component.as_ptr(),
                value.as_mut_ptr(),
                &mut size,
            )
        } != 3
        {
            continue;
        }
        let path = std::path::PathBuf::from(String::from_utf16_lossy(&value[..size as usize]));
        let folder = path.parent()?;
        if crate::model::installed_executable(folder, app).is_some() {
            return Some(folder.display().to_string());
        }
    }
    None
}
pub fn custom_location_supported(app: &str) -> bool {
    matches!(
        app,
        "artcraft"
            | "designcraft"
            | "effectcraft"
            | "filmcraft"
            | "lightcraft"
            | "photocraft"
            | "printcraft"
            | "vectorcraft"
            | "wordcraft"
            | "gridcraft"
            | "deckcraft"
            | "cadcraft"
            | "soundcraft"
    )
}
pub fn package_location_supported(file: &Path) -> bool {
    install_directory_property(file).is_some()
}
fn install_directory_property(file: &Path) -> Option<&'static str> {
    let file = platform::wide(file);
    let mut db = 0;
    if unsafe { MsiOpenDatabaseW(file.as_ptr(), std::ptr::null(), &mut db) } != 0 {
        return None;
    }
    let db = MsiHandle(db);
    for property in ["INSTALLFOLDER", "INSTALLDIR"] {
        let query = platform::wide(format!(
            "SELECT `Directory` FROM `Directory` WHERE `Directory` = '{property}'"
        ));
        let mut view = 0;
        if unsafe { MsiDatabaseOpenViewW(db.0, query.as_ptr(), &mut view) } != 0 {
            continue;
        }
        let view = MsiHandle(view);
        if unsafe { MsiViewExecute(view.0, 0) } != 0 {
            continue;
        }
        let mut record = 0;
        if unsafe { MsiViewFetch(view.0, &mut record) } == 0 {
            let _record = MsiHandle(record);
            return Some(property);
        }
    }
    None
}

pub fn validate_custom_location(target: &Path) -> Result<()> {
    validate_location(target, true)
}
fn validate_location(target: &Path, must_be_new: bool) -> Result<()> {
    anyhow::ensure!(target.is_absolute(), "Choose an absolute install location");
    anyhow::ensure!(
        !must_be_new || !target.exists(),
        "Choose an empty, new app folder for this installation"
    );
    anyhow::ensure!(
        !target
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "The location cannot contain parent-directory segments"
    );
    let mut parent = Some(target);
    while let Some(path) = parent {
        anyhow::ensure!(
            !path.exists() || !crate::files::linked(path)?,
            "The install location cannot use linked folders"
        );
        parent = path.parent();
    }
    Ok(())
}
fn product_installed(code: &str) -> bool {
    let code = platform::wide(code);
    // INSTALLSTATE_DEFAULT: registered as installed for the current user/machine.
    unsafe { MsiQueryProductStateW(code.as_ptr()) == 5 }
}
fn matches_display_name(app: &str, display: &str) -> bool {
    static SUFFIX: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let suffix = SUFFIX.get_or_init(|| {
        regex::Regex::new(r"^(?:(?:version\s+)?v?\d+\.\d+\.\d+(?:\.\d+)?\s*)?(?:\([0-9a-f]{8,40}\)\s*)?(?:\((?:x64|x86|64-bit|32-bit)\))?$")
            .unwrap()
    });
    let mut display = display.trim().to_ascii_lowercase();
    for scope in [
        " (current user)",
        " (all users)",
        " (per-user)",
        " (per-machine)",
    ] {
        if let Some(base) = display.strip_suffix(scope) {
            display = base.trim_end().to_owned();
            break;
        }
    }
    [
        crate::model::title(app).to_ascii_lowercase(),
        app.to_ascii_lowercase(),
    ]
    .iter()
    .any(|alias| {
        display == *alias
            || display.strip_prefix(alias).is_some_and(|rest| {
                rest.starts_with(char::is_whitespace) && suffix.is_match(rest.trim())
            })
    })
}
#[cfg(test)]
mod detection_tests {
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
        assert!(super::registered_icon_executable(
            &folder,
            &format!("\"{}\",0", executable.display())
        )
        .is_some());
        assert!(super::registered_icon_executable(
            &folder.join("other"),
            &executable.display().to_string()
        )
        .is_none());
        assert!(super::registered_icon_executable(&folder, "relative.exe").is_none());
        let uninstall = folder.join("unins000.exe");
        std::fs::write(&uninstall, &pe).unwrap();
        assert!(
            super::registered_icon_executable(&folder, &uninstall.display().to_string()).is_none()
        );
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
        let root =
            std::env::temp_dir().join(format!("craft-msi-location-{}", uuid::Uuid::new_v4()));
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
}
pub fn detect(app: &str) -> Result<Option<Installed>> {
    detect_for_architecture(app, None)
}
pub fn detect_for_architecture(app: &str, architecture: Option<&str>) -> Result<Option<Installed>> {
    let mut roots = Vec::new();
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            if let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(
                "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
                KEY_READ | view,
            ) {
                roots.push((root, view));
            }
        }
    }
    detect_in_roots_for_architecture(app, roots, architecture)
}
// MSI versions can have a fourth numeric component. Compare numbers, not text.
fn installation_version(value: &str) -> Option<[u64; 4]> {
    let parts = value
        .trim()
        .trim_start_matches('v')
        .split('.')
        .collect::<Vec<_>>();
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let mut version = [0; 4];
    for (index, part) in parts.iter().enumerate() {
        version[index] = part.parse().ok()?;
    }
    Some(version)
}
#[cfg(test)]
fn detect_in_roots(
    app: &str,
    roots: impl IntoIterator<Item = (RegKey, u32)>,
) -> Result<Option<Installed>> {
    detect_in_roots_for_architecture(app, roots, None)
}
fn registered_install_folder(key: &RegKey) -> Option<std::path::PathBuf> {
    let location: String = key.get_value("InstallLocation").unwrap_or_default();
    if !location.trim().is_empty() {
        return Some(location.trim().trim_matches('"').into());
    }
    let icon: String = key.get_value("DisplayIcon").unwrap_or_default();
    let icon = icon
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"');
    if Path::new(icon).is_absolute() {
        return Path::new(icon).parent().map(Path::to_path_buf);
    }
    let command: String = key.get_value("UninstallString").unwrap_or_default();
    let (executable, _) = uninstall_command(&command).ok()?;
    // This is only a location hint. Detection still requires the app's executable.
    executable.parent().map(Path::to_path_buf)
}
fn detect_in_roots_for_architecture(
    app: &str,
    roots: impl IntoIterator<Item = (RegKey, u32)>,
    architecture: Option<&str>,
) -> Result<Option<Installed>> {
    let mut best: Option<Installed> = None;
    for (root, view) in roots {
        for name in root.enum_keys().flatten() {
            let Ok(key) = root.open_subkey(&name) else {
                continue;
            };
            let display: String = key.get_value("DisplayName").unwrap_or_default();
            if !matches_display_name(app, &display) {
                continue;
            }
            let mut location = registered_install_folder(&key)
                .map(|folder| folder.display().to_string())
                .unwrap_or_default();
            if crate::model::installed_executable(Path::new(&location), app).is_none()
                && key
                    .get_value::<u32, _>("WindowsInstaller")
                    .unwrap_or_default()
                    == 1
            {
                if let Some(folder) = msi_app_folder(&name, app) {
                    location = folder;
                }
            }
            if crate::model::installed_executable(Path::new(&location), app).is_none() {
                let mut candidates = Vec::new();
                for variable in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
                    if let Some(base) = std::env::var_os(variable) {
                        for folder in [
                            &display,
                            &crate::model::title(app),
                            &app.to_string(),
                            &crate::model::repository_name(app),
                        ] {
                            candidates.push(std::path::PathBuf::from(&base).join(folder));
                        }
                    }
                }
                if let Some(base) = std::env::var_os("LOCALAPPDATA") {
                    candidates.push(
                        std::path::PathBuf::from(base)
                            .join("Programs")
                            .join(&display),
                    );
                }
                if let Some(folder) = candidates
                    .into_iter()
                    .find(|p| crate::model::installed_executable(p, app).is_some())
                {
                    location = folder.display().to_string();
                } else {
                    continue;
                }
            }
            let msi: u32 = key.get_value("WindowsInstaller").unwrap_or_default();
            if msi == 1 && !product_installed(&name) {
                continue;
            }
            let candidate = Installed {
                name: app.into(),
                path: location,
                version: key.get_value("DisplayVersion").unwrap_or_default(),
                architecture: if view == KEY_WOW64_32KEY {
                    "x86"
                } else {
                    "x64"
                }
                .into(),
                install_kind: "installer".into(),
                product_code: if msi == 1 { name } else { String::new() },
            };
            let mut candidate = candidate;
            if let Some(executable) =
                crate::model::installed_executable(Path::new(&candidate.path), app)
            {
                if let Some(actual) = executable_architecture(&executable) {
                    candidate.architecture = actual.into();
                }
            }
            if architecture.is_some_and(|wanted| candidate.architecture != wanted) {
                continue;
            }
            if best.as_ref().is_none_or(|current| {
                installation_version(&candidate.version) > installation_version(&current.version)
            }) {
                best = Some(candidate);
            }
        }
    }
    Ok(best)
}
// Use the matching app's registered executable, rather than guessing its filename.
// DisplayIcon may reference a DLL or uninstaller, so validate the target before use.
pub fn registered_launch_executable(folder: &Path, app: &str) -> Option<std::path::PathBuf> {
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(
                "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
                KEY_READ | view,
            ) else {
                continue;
            };
            for name in root.enum_keys().flatten() {
                let Ok(key) = root.open_subkey(name) else {
                    continue;
                };
                let display: String = key.get_value("DisplayName").unwrap_or_default();
                if !matches_display_name(app, &display) {
                    continue;
                }
                let icon: String = key.get_value("DisplayIcon").unwrap_or_default();
                if let Some(path) = registered_icon_executable(folder, &icon) {
                    return Some(path);
                }
            }
        }
    }
    None
}
fn registered_icon_executable(folder: &Path, icon: &str) -> Option<std::path::PathBuf> {
    let raw = icon.trim();
    let raw = if let Some(quoted) = raw.strip_prefix('"') {
        let (path, suffix) = quoted.split_once('"')?;
        if !suffix.trim().is_empty() && !suffix.trim().starts_with(',') {
            return None;
        }
        path
    } else {
        raw.rsplit_once(',')
            .filter(|(_, index)| index.trim().parse::<i32>().is_ok())
            .map_or(raw, |(path, _)| path)
    };
    let path = std::path::PathBuf::from(raw.trim());
    if !path.is_absolute() || !path.extension()?.eq_ignore_ascii_case("exe") {
        return None;
    }
    let canonical = path.canonicalize().ok()?;
    let root = folder.canonicalize().ok()?;
    // A registry entry must not cause another application's executable to be chosen.
    if canonical.parent()? != root {
        return None;
    }
    let filename = canonical
        .file_stem()?
        .to_string_lossy()
        .to_ascii_lowercase();
    if filename.starts_with("unins") || filename.contains("uninstall") || filename.contains("setup")
    {
        return None;
    }
    let architecture = executable_architecture(&canonical)?;
    crate::downloads::verify_desktop_executable(&canonical, architecture).ok()?;
    Some(path)
}

fn executable_architecture(path: &Path) -> Option<&'static str> {
    crate::downloads::windows_executable_architecture(path).ok()
}
pub fn run(file: &Path, app: &str) -> Result<Installed> {
    run_with_job(file, app, None)
}
pub fn run_with_job(file: &Path, app: &str, job: Option<&crate::jobs::Job>) -> Result<Installed> {
    run_with_destination(file, app, job, None)
}
pub fn run_with_destination(
    file: &Path,
    app: &str,
    job: Option<&crate::jobs::Job>,
    destination: Option<&Path>,
) -> Result<Installed> {
    run_with_destination_inner(file, app, job, destination, false)
}
/// Replace an already verified native installation without relocating its files.
pub fn run_replacement(
    file: &Path,
    previous: &Installed,
    job: Option<&crate::jobs::Job>,
) -> Result<Installed> {
    run_with_destination_inner(
        file,
        &previous.name,
        job,
        Some(Path::new(&previous.path)),
        true,
    )
}
fn run_with_destination_inner(
    file: &Path,
    app: &str,
    job: Option<&crate::jobs::Job>,
    destination: Option<&Path>,
    replacing: bool,
) -> Result<Installed> {
    validate_installer_package(file)?;
    if let Some(target) = destination {
        anyhow::ensure!(
            (replacing || custom_location_supported(app))
                && file
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("msi")),
            "This installer does not support custom locations"
        );
        anyhow::ensure!(
            detect(app)?.is_none(),
            "Existing installer updates keep their current location"
        );
        validate_location(target, !replacing)?;
    }
    // Pass the registered component directory again during upgrades so a major
    // upgrade cannot silently move a custom installation back to Program Files.
    let existing = if destination.is_none()
        && custom_location_supported(app)
        && file
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("msi"))
    {
        detect(app)?.map(|a| std::path::PathBuf::from(a.path))
    } else {
        None
    };
    let destination = destination.or(existing.as_deref());
    if let Some(job) = job {
        job.check()?;
    }
    let absolute = std::fs::canonicalize(file)?;
    let text = absolute.to_string_lossy();
    let normalized = if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
    }
    .replace('/', r"\");
    let file = Path::new(&normalized);
    let directory_property = install_directory_property(file);
    let managed_location = directory_property.is_some();
    let destination = destination.filter(|_| managed_location);
    if !managed_location {
        if let Some(job) = job {
            job.log("This package controls its installation location. The manager will use the registered installed path.");
        }
    }
    let code = if file
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("msi"))
    {
        let mut properties = "REBOOT=ReallySuppress".to_string();
        if let Some(target) = destination {
            let path = target
                .to_str()
                .context("Install location must be valid Unicode")?;
            anyhow::ensure!(
                !path.contains(['"', '\0', '\r', '\n']),
                "Invalid install location"
            );
            properties.push_str(&format!(
                " {}=\"{}\"",
                directory_property.unwrap(),
                path.trim_end_matches('\\')
            ));
        }
        let package = platform::wide(file);
        let properties = platform::wide(properties);
        msi_transaction(job, managed_location, || unsafe {
            MsiInstallProductW(package.as_ptr(), properties.as_ptr())
        })?
    } else {
        run_exe(file, job)?
    };
    installer_result(code)?;
    let installed = detect(app)?.context("Installer finished, but Windows has not registered a usable app installation. Check the installer or Windows Installed apps.")?;
    if let Some(target) = destination {
        if std::fs::canonicalize(&installed.path).ok() != std::fs::canonicalize(target).ok() {
            if let Some(job) = job {
                job.log(&format!(
                    "Installer selected a different folder. Using the actual installation: {}",
                    installed.path
                ));
            }
        }
    }
    Ok(installed)
}
fn validate_installer_package(file: &Path) -> Result<()> {
    match file.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("exe") => {
            crate::downloads::verify_installer_executable(file)
        }
        Some(extension) if extension.eq_ignore_ascii_case("msi") => {
            let path = platform::wide(file);
            let mut database = 0;
            let code = unsafe { MsiOpenDatabaseW(path.as_ptr(), std::ptr::null(), &mut database) };
            if code != 0 {
                bail!("Download is not a readable Windows Installer package ({code})");
            }
            unsafe {
                MsiCloseHandle(database);
            }
            Ok(())
        }
        _ => bail!(
            "Unsupported Windows installer package. Choose an MSI or supported EXE installer."
        ),
    }
}
fn installer_result(code: i32) -> Result<()> {
    match code {
        0 | 3010 | 1641 => Ok(()),
        1602 => Err(anyhow::Error::new(crate::jobs::Cancelled)
            .context("Windows installer cancelled and finished rollback")),
        _ => bail!("Installer failed ({code})"),
    }
}
fn run_exe(file: &Path, job: Option<&crate::jobs::Job>) -> Result<i32> {
    run_exe_with_arguments(file, "", job)
}
fn run_exe_with_arguments(
    file: &Path,
    arguments: &str,
    job: Option<&crate::jobs::Job>,
) -> Result<i32> {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{GetExitCodeProcess, GetProcessId, WaitForSingleObject},
            UI::{
                Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
                WindowsAndMessaging::SW_SHOWNORMAL,
            },
        },
    };
    let file = platform::wide(file);
    let verb = platform::wide("open");
    let arguments = platform::wide(arguments);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(arguments.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info)?;
        let process = info.hProcess;
        if process.is_invalid() {
            bail!("Installer did not return a process handle");
        }
        let result = wait_installer(job, GetProcessId(process), || {
            let status = WaitForSingleObject(process, 0);
            if status == windows::Win32::Foundation::WAIT_TIMEOUT {
                return Ok(None);
            }
            if status != WAIT_OBJECT_0 {
                bail!("Could not wait for the installer");
            }
            let mut code = 0;
            GetExitCodeProcess(process, &mut code)?;
            Ok(Some(code as i32))
        });
        let _ = CloseHandle(process);
        result
    }
}
fn request_installer_close(pid: u32) -> Result<bool> {
    use windows::Win32::{
        Foundation::{BOOL, HWND, LPARAM, WPARAM},
        UI::WindowsAndMessaging::*,
    };
    struct Request {
        pid: u32,
        sent: bool,
    }
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
        let request = &mut *(data.0 as *mut Request);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == request.pid && IsWindowVisible(hwnd).as_bool() {
            request.sent |= PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok();
        }
        BOOL(1)
    }
    let mut request = Request { pid, sent: false };
    unsafe {
        EnumWindows(Some(visit), LPARAM(&mut request as *mut Request as isize))?;
    }
    Ok(request.sent)
}
fn wait_installer(
    job: Option<&crate::jobs::Job>,
    pid: u32,
    mut poll: impl FnMut() -> Result<Option<i32>>,
) -> Result<i32> {
    let mut requested = false;
    loop {
        if let Some(code) = poll()? {
            return Ok(code);
        }
        if let Some(job) = job {
            if !requested && job.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                requested = true;
                let sent = request_installer_close(pid).unwrap_or(false);
                let message = if sent {
                    "Cancellation requested. Confirm in the installer if prompted; waiting for Windows to finish rollback."
                } else {
                    "Windows cannot forward cancellation to this installer. Use Cancel in the installer window; waiting for it to finish safely."
                };
                job.stage("Cancel requested", None, message);
                job.log(message);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
pub fn uninstall(app: &Installed) -> Result<()> {
    let current = detect_for_architecture(&app.name, Some(&app.architecture))?
        .context("This app is no longer installed. Refresh its status before uninstalling.")?;
    let app = &current;
    let code = &app.product_code;
    if code.len() != 38
        || !code.starts_with('{')
        || !code.ends_with('}')
        || !code[1..37]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b'-')
    {
        return uninstall_exe(app);
    }
    let product = platform::wide(code);
    let properties = platform::wide("REBOOT=ReallySuppress");
    let code = msi_transaction(None, true, || unsafe {
        MsiConfigureProductExW(product.as_ptr(), 0, 2, properties.as_ptr())
    })?;
    installer_result(code)?;
    if detect(&app.name)?.is_some() {
        bail!("Windows still reports the app installed");
    }
    Ok(())
}

// Use the registered interactive command, without a shell or guessed silent switches.
fn uninstall_command(command: &str) -> Result<(std::path::PathBuf, String)> {
    let command = command.trim();
    if command.contains(['\0', '\r', '\n']) {
        bail!("Invalid registered uninstall command");
    }
    let (file, arguments) = if let Some(rest) = command.strip_prefix('"') {
        let end = rest
            .find('"')
            .context("Uninstall command has an unclosed quote")?;
        let arguments = &rest[end + 1..];
        if !arguments.is_empty() && !arguments.starts_with(char::is_whitespace) {
            bail!("Invalid registered uninstall command");
        }
        (&rest[..end], arguments.trim())
    } else {
        let lower = command.to_ascii_lowercase();
        let end = lower
            .match_indices(".exe")
            .find_map(|(offset, _)| {
                let end = offset + 4;
                (command[end..].is_empty() || command[end..].starts_with(char::is_whitespace))
                    .then_some(end)
            })
            .context("Uninstall command must name an executable")?;
        (&command[..end], command[end..].trim())
    };
    let path = std::path::PathBuf::from(file);
    if !path.is_absolute() || !file.to_ascii_lowercase().ends_with(".exe") {
        bail!("Uninstall command must name an absolute EXE path");
    }
    Ok((path, arguments.to_owned()))
}

fn registered_uninstaller(app: &Installed) -> Result<Option<(RegKey, String, String)>> {
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(
                "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
                KEY_READ | view,
            ) else {
                continue;
            };
            for name in root.enum_keys().flatten() {
                let Ok(key) = root.open_subkey(&name) else {
                    continue;
                };
                let display: String = key.get_value("DisplayName").unwrap_or_default();
                if !matches_display_name(&app.name, &display) {
                    continue;
                }
                let Some(folder) = registered_install_folder(&key) else {
                    continue;
                };
                if !folder
                    .to_string_lossy()
                    .trim_end_matches(['\\', '/'])
                    .eq_ignore_ascii_case(app.path.trim_end_matches(['\\', '/']))
                {
                    continue;
                }
                let command: String = key.get_value("UninstallString").unwrap_or_default();
                if !command.trim().is_empty() {
                    return Ok(Some((root, name, command)));
                }
            }
        }
    }
    Ok(None)
}

/// Downloaded MSI sources are disposable after verified installation; Windows
/// owns its maintenance cache. Never remove an EXE used as the uninstaller.
pub(crate) fn download_retention_reason(package: &Path, app: &Installed) -> Result<Option<String>> {
    if package
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("msi"))
    {
        return Ok(None);
    }
    if !package
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return Ok(Some("Package maintenance requirements are unknown.".into()));
    }
    let Some((root, name, command)) = registered_uninstaller(app)? else {
        return Ok(Some(
            "No independent registered uninstaller was verified.".into(),
        ));
    };
    let (uninstaller, _) = uninstall_command(&command)?;
    let directory = package.parent().context("Installer has no parent folder")?;
    if !uninstaller.is_file() || uninstaller.starts_with(directory) {
        return Ok(Some(
            "The uninstall command may depend on this download.".into(),
        ));
    }
    let key = root.open_subkey(name)?;
    let directory = directory
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    for field in [
        "UninstallString",
        "QuietUninstallString",
        "ModifyPath",
        "InstallSource",
        "BundleCachePath",
    ] {
        let value: String = match key.get_value(field) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if value.contains('%')
            || value
                .replace('/', "\\")
                .to_ascii_lowercase()
                .contains(&directory)
        {
            return Ok(Some(
                "Windows uninstall or repair registration references this download folder.".into(),
            ));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod download_cleanup_tests {
    use super::*;

    #[test]
    fn downloaded_msi_is_disposable_after_verified_installation() {
        let app = Installed::default();
        for name in ["filmcraft.msi", "custom-setup.MSI"] {
            assert!(download_retention_reason(Path::new(name), &app)
                .unwrap()
                .is_none());
        }
    }
}

fn uninstall_exe(app: &Installed) -> Result<()> {
    let (root, key, command) = registered_uninstaller(app)?
        .context("No matching Windows uninstall command was found. Use Windows Installed apps.")?;
    let (file, arguments) = uninstall_command(&command)?;
    if !file.is_file() {
        bail!("The registered uninstaller is missing. Use Windows Installed apps.");
    }
    let code = run_exe_with_arguments(&file, &arguments, None)?;
    installer_result(code)?;
    // A cancelled wizard can return success. Never discard the installation in that case.
    for _ in 0..20 {
        if root.open_subkey(&key).is_err()
            && detect_for_architecture(&app.name, Some(&app.architecture))?.is_none()
        {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    bail!("Uninstall did not complete or was cancelled. Windows still reports this app installed.")
}

#[cfg(test)]
mod uninstall_command_tests {
    #[test]
    fn shared_installer_entry_rejects_unknown_and_invalid_packages() {
        assert!(super::validate_installer_package(std::path::Path::new("unknown.zip")).is_err());
        let path = std::env::temp_dir().join(format!("craft-invalid-{}.MSI", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"not an MSI database").unwrap();
        assert!(super::validate_installer_package(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn preserves_registered_arguments_without_shell_interpretation() {
        let (file, arguments) = super::uninstall_command(
            r#""C:\Program Files\Example\unins000.exe" /REMOVE "some value""#,
        )
        .unwrap();
        assert_eq!(
            file,
            std::path::PathBuf::from(r"C:\Program Files\Example\unins000.exe")
        );
        assert_eq!(arguments, r#"/REMOVE "some value""#);
        let (file, arguments) =
            super::uninstall_command(r"C:\Program Files\Example\uninstall.exe /uninstall").unwrap();
        assert_eq!(
            file,
            std::path::PathBuf::from(r"C:\Program Files\Example\uninstall.exe")
        );
        assert_eq!(arguments, "/uninstall");
        for invalid in [
            "uninstall.exe",
            "cmd /c uninstall.exe",
            "https://example.com/uninstall.exe",
            "\"C:\\app.exe\"/bad",
            "C:\\app.exe\n/bad",
        ] {
            assert!(super::uninstall_command(invalid).is_err(), "{invalid}");
        }
    }
}

pub fn installer_label() -> &'static str {
    "Windows"
}
pub fn installer_extension() -> Result<&'static str> {
    Ok(".msi")
}
#[cfg(test)]
mod cancellation_tests {
    #[test]
    #[ignore = "Requires a per-user test MSI specified by CRAFT_TEST_MSI (product CF7C536E-50DA-4A9F-B315-ABF9C07C707D)"]
    fn native_msi_transaction_honors_folder_and_completes_cancel_rollback() {
        use super::*;
        let package = std::path::PathBuf::from(
            std::env::var_os("CRAFT_TEST_MSI")
                .expect("Set CRAFT_TEST_MSI to the temporary per-user fixture"),
        );
        assert_eq!(install_directory_property(&package), Some("INSTALLFOLDER"));
        let folder = std::env::temp_dir().join(format!("craft MSI test {}", uuid::Uuid::new_v4()));
        let product = platform::wide("{CF7C536E-50DA-4A9F-B315-ABF9C07C707D}");
        let package = platform::wide(package.to_string_lossy().replace('/', r"\"));
        let properties = platform::wide(format!(
            "REBOOT=ReallySuppress INSTALLFOLDER=\"{}\"",
            folder.display()
        ));
        let remove_properties = platform::wide("REBOOT=ReallySuppress");
        struct Cleanup(Vec<u16>, Vec<u16>, std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = msi_transaction(None, true, || unsafe {
                    MsiConfigureProductExW(self.0.as_ptr(), 0, 2, self.1.as_ptr())
                });
                let _ = std::fs::remove_dir_all(&self.2);
            }
        }
        let _cleanup = Cleanup(product.clone(), remove_properties.clone(), folder.clone());
        let code = msi_transaction(None, true, || unsafe {
            MsiInstallProductW(package.as_ptr(), properties.as_ptr())
        })
        .unwrap();
        installer_result(code).unwrap();
        assert!(folder.join("payload.txt").is_file());
        assert_eq!(unsafe { MsiQueryProductStateW(product.as_ptr()) }, 5);
        // Exercise the relocation strategy with a per-user fixture, preserving
        // local user settings while Windows updates component registration.
        std::fs::write(folder.join("settings.json"), b"user settings").unwrap();
        let backup = folder.with_extension("recovery");
        crate::files::copy_directory_verified(&folder, &backup).unwrap();
        let code = msi_transaction(None, true, || unsafe {
            MsiConfigureProductExW(product.as_ptr(), 0, 2, remove_properties.as_ptr())
        })
        .unwrap();
        installer_result(code).unwrap();
        assert!(!folder.join("payload.txt").exists());
        let relocated = folder.with_extension("relocated");
        let _relocated_cleanup = Cleanup(
            product.clone(),
            remove_properties.clone(),
            relocated.clone(),
        );
        let relocated_properties = platform::wide(format!(
            "REBOOT=ReallySuppress INSTALLFOLDER=\"{}\"",
            relocated.display()
        ));
        let code = msi_transaction(None, true, || unsafe {
            MsiInstallProductW(package.as_ptr(), relocated_properties.as_ptr())
        })
        .unwrap();
        installer_result(code).unwrap();
        crate::relocation::restore_extra_files(&backup, &relocated).unwrap();
        assert!(relocated.join("payload.txt").is_file());
        assert_eq!(
            std::fs::read(relocated.join("settings.json")).unwrap(),
            b"user settings"
        );
        let code = msi_transaction(None, true, || unsafe {
            MsiConfigureProductExW(product.as_ptr(), 0, 2, remove_properties.as_ptr())
        })
        .unwrap();
        installer_result(code).unwrap();
        std::fs::remove_dir_all(backup).unwrap();
        let job = crate::jobs::Job::new(folder.with_extension("log"), &Default::default());
        let code = msi_transaction(Some(&job), true, || {
            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            unsafe { MsiInstallProductW(package.as_ptr(), properties.as_ptr()) }
        })
        .unwrap();
        assert_eq!(code, 1602);
        assert!(crate::jobs::is_cancelled(
            &installer_result(code).unwrap_err()
        ));
        assert!(!folder.join("payload.txt").exists());
        assert_ne!(unsafe { MsiQueryProductStateW(product.as_ptr()) }, 5);
    }
    #[test]
    fn msi_callback_only_cancels_progress_and_preserves_native_errors() {
        let job = crate::jobs::Job::new(
            std::env::temp_dir().join("craft-msi-callback.log"),
            &Default::default(),
        );
        let context = &job as *const _ as *mut std::ffi::c_void;
        unsafe {
            assert_eq!(
                super::installer_ui(context, 0x0a00_0000, std::ptr::null()),
                0
            );
        }
        job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        for message in [0x0800_0000, 0x0900_0000, 0x0a00_0000] {
            unsafe {
                assert_eq!(super::installer_ui(context, message, std::ptr::null()), 2);
            }
        }
        unsafe {
            assert_eq!(
                super::installer_ui(context, 0x0100_0000, std::ptr::null()),
                0
            );
            assert_eq!(
                super::installer_ui(std::ptr::null_mut(), 0x0a00_0000, std::ptr::null()),
                0
            );
        }
    }
    #[test]
    fn explicit_windows_cancel_is_typed_but_failures_and_success_stay_distinct() {
        assert!(crate::jobs::is_cancelled(
            &super::installer_result(1602).unwrap_err()
        ));
        assert!(!crate::jobs::is_cancelled(
            &super::installer_result(5).unwrap_err()
        ));
        for code in [0, 3010, 1641] {
            super::installer_result(code).unwrap();
        }
    }
    #[test]
    fn cancellation_waits_for_the_installer_result() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let job = crate::jobs::Job::new(root.join("installer.log"), &Default::default());
        job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut polls = 0;
        let code = super::wait_installer(Some(&job), u32::MAX, || {
            polls += 1;
            Ok((polls == 3).then_some(1602))
        })
        .unwrap();
        assert_eq!(polls, 3);
        assert_eq!(code, 1602);
        assert_eq!(job.state.lock().unwrap().stage, "Cancel requested");
        std::fs::remove_dir_all(root).unwrap();
    }
}
