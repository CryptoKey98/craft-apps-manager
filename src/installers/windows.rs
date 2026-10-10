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
        regex::Regex::new(r"^(?:v?\d+\.\d+\.\d+(?:\.\d+)?\s*)?(?:\((?:x64|x86|64-bit|32-bit)\))?$")
            .unwrap()
    });
    let display = display.trim().to_ascii_lowercase();
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
    detect_in_roots(app, roots)
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
fn detect_in_roots(
    app: &str,
    roots: impl IntoIterator<Item = (RegKey, u32)>,
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
            let mut location: String = key.get_value("InstallLocation").unwrap_or_default();
            location = location.trim().trim_matches('"').to_owned();
            if location.is_empty() {
                let icon: String = key.get_value("DisplayIcon").unwrap_or_default();
                let icon = icon.split(',').next().unwrap_or("").trim_matches('"');
                if let Some(parent) = Path::new(icon).parent() {
                    location = parent.display().to_string();
                }
            }
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
                            &crate::model::repository(app).to_string(),
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
            if best.as_ref().is_none_or(|current| {
                installation_version(&candidate.version) > installation_version(&current.version)
            }) {
                best = Some(candidate);
            }
        }
    }
    Ok(best)
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
fn installer_result(code: i32) -> Result<()> {
    match code {
        0 | 3010 | 1641 => Ok(()),
        1602 => Err(anyhow::Error::new(crate::jobs::Cancelled)
            .context("Windows installer cancelled and finished rollback")),
        _ => bail!("Installer failed ({code})"),
    }
}
fn run_exe(file: &Path, job: Option<&crate::jobs::Job>) -> Result<i32> {
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
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
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
    let current = detect(&app.name)?
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
        bail!("This installer must be removed through Windows Installed apps.");
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
