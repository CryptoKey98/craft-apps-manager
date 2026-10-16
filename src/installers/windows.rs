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
    let known: Vec<_> = crate::app_identity::native(app)
        .into_iter()
        .filter(|identity| {
            identity.platform == "windows" && identity.package.eq_ignore_ascii_case(product)
        })
        .map(|identity| identity.executable)
        .collect();
    let explicit_identity = !known.is_empty();
    let names = if known.is_empty() {
        crate::model::executable_names(app)
    } else {
        known
    };
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
        if !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
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
        let verified = if explicit_identity {
            executable_architecture(&path).is_some_and(|arch| {
                crate::downloads::verify_desktop_executable(&path, arch).is_ok()
            })
        } else {
            crate::model::installed_executable(folder, app).is_some()
        };
        if verified {
            return Some(folder.display().to_string());
        }
    }
    None
}
// Read package metadata without executing the installer or trusting its filename.
fn msi_rows(file: &Path, query: &str) -> Result<Vec<String>> {
    let mut database = 0;
    anyhow::ensure!(
        unsafe {
            MsiOpenDatabaseW(
                platform::wide(file).as_ptr(),
                std::ptr::null(),
                &mut database,
            )
        } == 0,
        "Could not read MSI identity"
    );
    let database = MsiHandle(database);
    let mut view = 0;
    anyhow::ensure!(
        unsafe { MsiDatabaseOpenViewW(database.0, platform::wide(query).as_ptr(), &mut view) } == 0,
        "Could not query MSI identity"
    );
    let view = MsiHandle(view);
    anyhow::ensure!(
        unsafe { MsiViewExecute(view.0, 0) } == 0,
        "Could not read MSI identity table"
    );
    let mut rows = Vec::new();
    loop {
        let mut record = 0;
        let code = unsafe { MsiViewFetch(view.0, &mut record) };
        if code == 259 {
            break;
        }
        anyhow::ensure!(code == 0, "Could not read MSI identity row ({code})");
        let record = MsiHandle(record);
        let mut text = vec![0u16; 32768];
        let mut size = (text.len() - 1) as u32;
        anyhow::ensure!(
            unsafe { MsiRecordGetStringW(record.0, 1, text.as_mut_ptr(), &mut size) } == 0,
            "Could not read MSI identity value"
        );
        rows.push(String::from_utf16_lossy(&text[..size as usize]));
    }
    Ok(rows)
}
fn preferred_msi_executable(
    names: &[String],
    saved: &[String],
    guesses: &[String],
) -> Option<usize> {
    let matches = |known: &[String]| {
        names
            .iter()
            .enumerate()
            .filter(|(_, name)| known.iter().any(|known| known.eq_ignore_ascii_case(name)))
            .map(|(index, _)| index)
            .collect::<Vec<_>>()
    };
    let remembered = matches(saved);
    if remembered.len() == 1 {
        return Some(remembered[0]);
    }
    if !saved.is_empty() {
        return None;
    }
    let guessed = matches(guesses);
    (guessed.len() == 1).then(|| guessed[0])
}
pub fn batch_identity_ready(app: &str) -> bool {
    !crate::catalog::get(app).is_some_and(|entry| entry.repository.contains('/'))
        || exe_identity_ready(app).unwrap_or(false)
}
fn exe_identity_ready(app: &str) -> Result<bool> {
    for identity in crate::app_identity::native(app) {
        if identity.platform == "windows"
            && identity.package.starts_with("exe:")
            && detect_registered_product(app, None, None, Some(&identity.package))?.is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}
pub fn prepare_custom_identity(
    root: &Path,
    file: &Path,
    app: &str,
    job: &crate::jobs::Job,
) -> Result<()> {
    if !crate::catalog::get(app).is_some_and(|entry| entry.repository.contains('/'))
        || !file
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("msi"))
    {
        return Ok(());
    }
    let products = msi_rows(
        file,
        "SELECT `Value` FROM `Property` WHERE `Property` = 'ProductCode'",
    )?;
    let [product] = products.as_slice() else {
        bail!("MSI must identify one product");
    };
    anyhow::ensure!(
        product.starts_with('{')
            && product.ends_with('}')
            && uuid::Uuid::parse_str(product).is_ok(),
        "Invalid MSI product identity"
    );
    let families = msi_rows(
        file,
        "SELECT `Value` FROM `Property` WHERE `Property` = 'UpgradeCode'",
    )?;
    anyhow::ensure!(families.len() <= 1, "Invalid MSI upgrade family");
    let family = families.first().cloned().unwrap_or_default();
    anyhow::ensure!(
        family.is_empty() || uuid::Uuid::parse_str(&family).is_ok(),
        "Invalid MSI upgrade family"
    );
    let mut names: Vec<String> = msi_rows(file, "SELECT `FileName` FROM `File`")?
        .into_iter()
        .filter_map(|name| {
            let name = name.rsplit('|').next()?.to_owned();
            (name.to_ascii_lowercase().ends_with(".exe") && !name.contains(['/', '\\', ':']))
                .then_some(name)
        })
        .collect();
    names.sort();
    names.dedup();
    let remembered: Vec<_> = crate::app_identity::native(app)
        .into_iter()
        .filter(|identity| {
            identity.platform == "windows"
                && (identity.package.eq_ignore_ascii_case(product)
                    || (!family.is_empty() && identity.family.eq_ignore_ascii_case(&family)))
        })
        .map(|identity| identity.executable)
        .collect();
    let index = if let Some(index) =
        preferred_msi_executable(&names, &remembered, &crate::model::executable_names(app))
    {
        index
    } else {
        job.choose_identity("Choose this package's main application executable. The Windows Installer product ID will identify the installed copy.",names.clone())?
    };
    crate::app_identity::remember(
        root,
        app,
        crate::app_identity::NativeIdentity {
            launch: None,
            family,
            platform: "windows".into(),
            package: product.clone(),
            executable: names[index].clone(),
        },
    )
}
fn registry_scope(hive: winreg::HKEY, view: u32) -> &'static str {
    match (hive == HKEY_CURRENT_USER, view == KEY_WOW64_64KEY) {
        (true, true) => "HKCU/64",
        (true, false) => "HKCU/32",
        (false, true) => "HKLM/64",
        (false, false) => "HKLM/32",
    }
}
fn exe_registration_id(scope: &str, key: &str, folder: &Path) -> String {
    format!(
        "exe:{}",
        serde_json::to_string(&[
            scope.to_owned(),
            key.to_owned(),
            folder
                .to_string_lossy()
                .replace('/', "\\")
                .trim_end_matches('\\')
                .to_ascii_lowercase()
        ])
        .unwrap()
    )
}
fn identity_for_registration(
    app: &str,
    name: &str,
    key: &RegKey,
    scope: Option<&str>,
) -> Option<crate::app_identity::NativeIdentity> {
    let msi = key
        .get_value::<u32, _>("WindowsInstaller")
        .unwrap_or_default()
        == 1;
    crate::app_identity::native(app)
        .into_iter()
        .find(|identity| {
            if identity.platform != "windows" {
                return false;
            }
            if msi {
                return identity.package.eq_ignore_ascii_case(name);
            }
            let Some(raw) = identity.package.strip_prefix("exe:") else {
                return false;
            };
            let Ok([saved_scope, saved_key, saved_folder]) =
                serde_json::from_str::<[String; 3]>(raw)
            else {
                return false;
            };
            scope.is_none_or(|scope| scope == saved_scope)
                && name.eq_ignore_ascii_case(&saved_key)
                && registered_install_folder(key).is_some_and(|folder| {
                    folder
                        .to_string_lossy()
                        .replace('/', "\\")
                        .trim_end_matches('\\')
                        .eq_ignore_ascii_case(&saved_folder)
                })
        })
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Registration {
    identity: String,
    display: String,
    version: String,
    folder: std::path::PathBuf,
    icon: String,
    uninstall: String,
}
fn registrations() -> Vec<Registration> {
    let mut records = Vec::new();
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
                if key
                    .get_value::<u32, _>("SystemComponent")
                    .unwrap_or_default()
                    == 1
                {
                    continue;
                }
                let display: String = key.get_value("DisplayName").unwrap_or_default();
                let Some(folder) = registered_install_folder(&key) else {
                    continue;
                };
                if display.trim().is_empty() || !folder.is_dir() {
                    continue;
                }
                let msi = key
                    .get_value::<u32, _>("WindowsInstaller")
                    .unwrap_or_default()
                    == 1;
                let uninstall: String = key.get_value("UninstallString").unwrap_or_default();
                if msi {
                    if !product_installed(&name) {
                        continue;
                    }
                } else if uninstall_command(&uninstall).is_err() {
                    continue;
                }
                records.push(Registration {
                    identity: if msi {
                        name
                    } else {
                        exe_registration_id(registry_scope(hive, view), &name, &folder)
                    },
                    display,
                    version: key.get_value("DisplayVersion").unwrap_or_default(),
                    folder,
                    icon: key.get_value("DisplayIcon").unwrap_or_default(),
                    uninstall,
                });
            }
        }
    }
    records
}
fn changed_registrations<'a>(
    before: &[Registration],
    after: &'a [Registration],
) -> Vec<&'a Registration> {
    registration_candidates(before, after, false)
}
fn registration_candidates<'a>(
    before: &[Registration],
    after: &'a [Registration],
    allow_existing: bool,
) -> Vec<&'a Registration> {
    let changed: Vec<_> = after
        .iter()
        .filter(|record| !before.contains(record))
        .collect();
    if allow_existing && changed.is_empty() {
        after.iter().collect()
    } else {
        changed
    }
}
fn same_user_registration(left: &Registration, right: &Registration) -> bool {
    let decode =
        |identity: &str| serde_json::from_str::<[String; 3]>(identity.strip_prefix("exe:")?).ok();
    let (Some([left_scope, left_key, left_folder]), Some([right_scope, right_key, right_folder])) =
        (decode(&left.identity), decode(&right.identity))
    else {
        return false;
    };
    let normalized = |path: &str| {
        path.replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    // HKCU's uninstall namespace is shared across registry views. Do not merge
    // HKLM's separate 32/64-bit keys, different owners, uninstallers or products.
    ["HKCU/64", "HKCU/32"].contains(&left_scope.as_str())
        && ["HKCU/64", "HKCU/32"].contains(&right_scope.as_str())
        && left_key.eq_ignore_ascii_case(&right_key)
        && normalized(&left_folder) == normalized(&right_folder)
        && normalized(&left.folder.to_string_lossy()) == normalized(&right.folder.to_string_lossy())
        && left.display == right.display
        && left.version == right.version
        && left.icon == right.icon
        && left.uninstall == right.uninstall
}
fn deduplicate_identity_choices(candidates: &mut Vec<(&Registration, std::path::PathBuf)>) {
    let mut unique: Vec<(&Registration, std::path::PathBuf)> = Vec::new();
    for candidate in candidates.drain(..) {
        if !unique.iter().any(|(record, path)| {
            same_user_registration(record, candidate.0) && path == &candidate.1
        }) {
            unique.push(candidate);
        }
    }
    *candidates = unique;
}
fn learn_exe_installation(
    app: &str,
    before: &[Registration],
    job: Option<&crate::jobs::Job>,
    allow_existing: bool,
    expected_version: Option<&str>,
) -> Result<Installed> {
    let after = registrations();
    let changed = registration_candidates(before, &after, allow_existing);
    let mut candidates = Vec::new();
    for record in changed {
        if allow_existing
            && expected_version
                .is_some_and(|expected| !matches_expected_version(&record.version, expected))
        {
            continue;
        }
        let mut executables = Vec::new();
        if let Some(path) = registered_icon_executable(&record.folder, &record.icon) {
            executables.push(path);
        }
        if executables.is_empty() {
            let Ok(entries) = std::fs::read_dir(&record.folder) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_file()) {
                    if let Some(path) = registered_icon_executable(
                        &record.folder,
                        &entry.path().display().to_string(),
                    ) {
                        executables.push(path);
                    }
                }
            }
        }
        executables.sort();
        executables.dedup();
        for executable in executables {
            candidates.push((record, executable));
        }
    }
    deduplicate_identity_choices(&mut candidates);
    anyhow::ensure!(!candidates.is_empty(), "Installer finished, but no matching desktop application registration could be verified. Check Windows Installed apps; no unrelated program was adopted.");
    let job = job.context("Open the manager to confirm which program the installer registered")?;
    let title = if allow_existing {
        "The installer did not change Windows' application registrations. Explicitly choose the existing program to associate with this GitHub app, or cancel. Check its name, version, and location before selecting."
    } else {
        "Confirm the application registered by this installer. Only new or changed registrations are listed; select the program you installed."
    };
    let index = job.choose_identity(
        title,
        candidates
            .iter()
            .map(|(record, path)| {
                format!("{} {} · {}", record.display, record.version, path.display())
            })
            .collect(),
    )?;
    let (record, path) = &candidates[index];
    anyhow::ensure!(
        registrations().contains(record),
        "Selected Windows registration changed; refresh and try again"
    );
    anyhow::ensure!(
        registered_icon_executable(&record.folder, &path.display().to_string()).as_ref()
            == Some(path),
        "Selected executable changed or is no longer usable"
    );
    job.check()?;
    crate::app_identity::remember_current(
        app,
        crate::app_identity::NativeIdentity {
            launch: None,
            family: String::new(),
            platform: "windows".into(),
            package: record.identity.clone(),
            executable: path
                .file_name()
                .context("Missing executable name")?
                .to_string_lossy()
                .into_owned(),
        },
    )?;
    detect_registered_product(app, None, None, Some(&record.identity))?
        .context("Selected application registration could not be verified")
}
fn matches_registration(app: &str, key_name: &str, key: &RegKey, display: &str) -> bool {
    identity_for_registration(app, key_name, key, None).is_some()
        || matches_display_name(app, display)
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
        regex::Regex::new(r"^(?:(?:version\s+)?v?\d+\.\d+(?:\.\d+){0,2}\s*)?(?:\([0-9a-f]{8,40}\)\s*)?(?:\((?:x64|x86|64-bit|32-bit)\))?$")
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
        let root =
            std::env::temp_dir().join(format!("craft-exe-preflight-{}", uuid::Uuid::new_v4()));
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
        assert!(
            registration_choice(&folder, "photocraft", "chosen-key", &key, "HKLM/64").is_none()
        );
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
    detect_registered(app, architecture, None)
}
fn detect_registered(
    app: &str,
    architecture: Option<&str>,
    selected: Option<&Installed>,
) -> Result<Option<Installed>> {
    detect_registered_product(app, architecture, selected, None)
}
fn detect_registered_product(
    app: &str,
    architecture: Option<&str>,
    selected: Option<&Installed>,
    product: Option<&str>,
) -> Result<Option<Installed>> {
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
    detect_in_roots_for_product(app, roots, architecture, selected, product)
}
// MSI versions can have a fourth numeric component. Compare numbers, not text.
fn installation_version(value: &str) -> Option<[u64; 4]> {
    let parts = value
        .trim()
        .trim_start_matches('v')
        .split('.')
        .collect::<Vec<_>>();
    if !(2..=4).contains(&parts.len()) {
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
#[cfg(test)]
fn detect_in_roots_for_architecture(
    app: &str,
    roots: impl IntoIterator<Item = (RegKey, u32)>,
    architecture: Option<&str>,
) -> Result<Option<Installed>> {
    detect_in_roots_for_installation(app, roots, architecture, None)
}
#[cfg(test)]
fn detect_in_roots_for_installation(
    app: &str,
    roots: impl IntoIterator<Item = (RegKey, u32)>,
    architecture: Option<&str>,
    selected: Option<&Installed>,
) -> Result<Option<Installed>> {
    detect_in_roots_for_product(app, roots, architecture, selected, None)
}
fn detect_in_roots_for_product(
    app: &str,
    roots: impl IntoIterator<Item = (RegKey, u32)>,
    architecture: Option<&str>,
    selected: Option<&Installed>,
    product: Option<&str>,
) -> Result<Option<Installed>> {
    let mut best: Option<Installed> = None;
    for (root, view) in roots {
        for name in root.enum_keys().flatten() {
            let Ok(key) = root.open_subkey(&name) else {
                continue;
            };
            let display: String = key.get_value("DisplayName").unwrap_or_default();
            if !matches_registration(app, &name, &key, &display) {
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
                if identity_for_registration(app, &name, &key, None).is_some() {
                    continue;
                }
                // Conventional folders are discovery hints, not proof that this
                // registration owns the copy the user asked us to uninstall.
                if selected.is_some() {
                    continue;
                }
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
                installation_label: Default::default(),
                package_type: Default::default(),
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
                product_code: if msi == 1 {
                    name.clone()
                } else {
                    identity_for_registration(app, &name, &key, None)
                        .map(|identity| identity.package)
                        .unwrap_or_default()
                },
            };
            if product.is_some_and(|product| !candidate.product_code.eq_ignore_ascii_case(product))
            {
                continue;
            }
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
            if selected.is_some_and(|selected| {
                !candidate
                    .path
                    .replace('/', "\\")
                    .trim_end_matches('\\')
                    .eq_ignore_ascii_case(selected.path.replace('/', "\\").trim_end_matches('\\'))
                    || !candidate
                        .product_code
                        .eq_ignore_ascii_case(&selected.product_code)
            }) {
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
fn registration_choice(
    folder: &Path,
    app: &str,
    name: &str,
    key: &RegKey,
    scope: &str,
) -> Option<Option<std::path::PathBuf>> {
    let identity = identity_for_registration(app, name, key, Some(scope))?;
    let registered = registered_install_folder(key)
        .or_else(|| msi_app_folder(name, app).map(std::path::PathBuf::from))?;
    let expected = folder.canonicalize().ok()?;
    if registered.canonicalize().ok()? != expected {
        return None;
    }
    Some(registered_icon_executable(
        folder,
        &folder.join(identity.executable).display().to_string(),
    ))
}
/// An outer Some means a verified choice exists; a missing chosen file must not fall back to another executable.
pub fn registered_launch_choice(folder: &Path, app: &str) -> Option<Option<std::path::PathBuf>> {
    if crate::app_identity::native(app).is_empty() {
        return None;
    }
    for identity in crate::app_identity::native(app)
        .into_iter()
        .filter(|identity| identity.platform == "windows")
    {
        let exe = identity
            .package
            .strip_prefix("exe:")
            .and_then(|raw| serde_json::from_str::<[String; 3]>(raw).ok());
        let name = exe
            .as_ref()
            .map_or(identity.package.as_str(), |parts| parts[1].as_str());
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
                let scope = registry_scope(hive, view);
                if exe.as_ref().is_some_and(|parts| parts[0] != scope) {
                    continue;
                }
                let Ok(key) = RegKey::predef(hive).open_subkey_with_flags(
                    format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{name}"),
                    KEY_READ | view,
                ) else {
                    continue;
                };
                if let Some(choice) = registration_choice(folder, app, name, &key, scope) {
                    return Some(choice);
                }
            }
        }
    }
    None
}
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
                let Ok(key) = root.open_subkey(&name) else {
                    continue;
                };
                let display: String = key.get_value("DisplayName").unwrap_or_default();
                if !matches_registration(app, &name, &key, &display) {
                    continue;
                }
                if let Some(choice) =
                    registration_choice(folder, app, &name, &key, registry_scope(hive, view))
                {
                    return choice;
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
    run_with_destination_inner(file, app, job, destination, false, None)
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
        None,
    )
}
pub fn run_for_version(
    file: &Path,
    app: &str,
    job: Option<&crate::jobs::Job>,
    destination: Option<&Path>,
    expected: &str,
    selected: &Installed,
) -> Result<Installed> {
    run_with_destination_inner(
        file,
        app,
        job,
        destination,
        false,
        Some((expected, selected)),
    )
}
fn matches_expected_version(installed: &str, expected: &str) -> bool {
    match (
        crate::updates::version(installed),
        crate::updates::version(expected),
    ) {
        (Ok(installed), Ok(expected)) => installed == expected,
        _ => false,
    }
}
fn verify_selected_exe(
    selected: &Installed,
    expected: &str,
    current: Option<Installed>,
) -> Result<Installed> {
    let current = current.context("The intended installed copy could not be verified")?;
    let folder = |path: &str| {
        path.replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    anyhow::ensure!(folder(&current.path) == folder(&selected.path)
        && (selected.product_code.is_empty() || current.product_code.eq_ignore_ascii_case(&selected.product_code)),
        "Installer changed a different application copy; the selected installation was not replaced");
    anyhow::ensure!(
        matches_expected_version(&current.version, expected),
        "Installer did not register the requested version for the selected installation"
    );
    Ok(current)
}
fn run_with_destination_inner(
    file: &Path,
    app: &str,
    job: Option<&crate::jobs::Job>,
    destination: Option<&Path>,
    replacing: bool,
    intended: Option<(&str, &Installed)>,
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
    let custom_exe = file
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        && crate::catalog::get(app).is_some_and(|entry| entry.repository.contains('/'));
    if custom_exe
        && !exe_identity_ready(app)?
        && !job.is_some_and(|job| job.identity_choices_available())
    {
        bail!("This custom installer needs an initial identity confirmation. Install it individually from the manager first.");
    }
    let before = custom_exe.then(registrations);
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
    let installed = if file
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("msi"))
    {
        let product = msi_rows(
            file,
            "SELECT `Value` FROM `Property` WHERE `Property` = 'ProductCode'",
        )?;
        let [product] = product.as_slice() else {
            bail!("Installer package has no unique product identity");
        };
        detect_registered_product(app,None,None,Some(product))?.context("Installer finished, but its exact product registration and executable could not be verified")?
    } else if custom_exe && intended.is_some_and(|(_, selected)| !selected.path.is_empty()) {
        let (expected, selected) = intended.context("Missing selected installation")?;
        verify_selected_exe(
            selected,
            expected,
            detect_registered(app, None, Some(selected))?,
        )?
    } else if custom_exe {
        let after = registrations();
        let changed = changed_registrations(before.as_deref().unwrap_or_default(), &after);
        let known = crate::app_identity::native(app);
        let bound: Vec<_> = changed
            .iter()
            .filter(|record| {
                known.iter().any(|identity| {
                    identity.platform == "windows"
                        && identity.package.eq_ignore_ascii_case(&record.identity)
                })
            })
            .collect();
        if bound.len() == 1 {
            detect_registered_product(app, None, None, Some(&bound[0].identity))?
                .context("Updated program registration could not be verified")?
        } else if !changed.is_empty() {
            learn_exe_installation(app, before.as_deref().unwrap_or_default(), job, false, None)?
        } else {
            learn_exe_installation(
                app,
                before.as_deref().unwrap_or_default(),
                job,
                true,
                intended.map(|(expected, _)| expected),
            )?
        }
    } else {
        detect(app)?.context("Installer finished, but Windows has not registered a usable app installation. Check Windows Installed apps.")?
    };
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
    let current = detect_registered(&app.name, Some(&app.architecture), Some(app))?
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
    if product_installed(&app.product_code) {
        bail!("Windows still reports the selected app installation installed");
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
                if !matches_registration(&app.name, &name, &key, &display) {
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
                if app.product_code.starts_with("exe:")
                    && identity_for_registration(
                        &app.name,
                        &name,
                        &key,
                        Some(registry_scope(hive, view)),
                    )
                    .is_none_or(|identity| identity.package != app.product_code)
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
            && detect_registered(&app.name, Some(&app.architecture), Some(app))?.is_none()
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
