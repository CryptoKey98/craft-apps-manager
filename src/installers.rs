use crate::{model::Installed, platform};
use anyhow::{bail, Context, Result};
use std::{path::Path, process::Command};
use winreg::{enums::*, RegKey};
#[link(name = "msi")]
unsafe extern "system" {
    fn MsiQueryProductStateW(product: *const u16) -> i32;
}
fn product_installed(code: &str) -> bool {
    let code = platform::wide(code);
    // INSTALLSTATE_DEFAULT: registered as installed for the current user/machine.
    unsafe { MsiQueryProductStateW(code.as_ptr()) == 5 }
}
pub fn detect(app: &str) -> Result<Option<Installed>> {
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
                if ![crate::model::title(app).to_lowercase(), app.to_lowercase()]
                    .contains(&display.to_lowercase())
                {
                    continue;
                }
                let mut location: String = key.get_value("InstallLocation").unwrap_or_default();
                if location.is_empty() {
                    let icon: String = key.get_value("DisplayIcon").unwrap_or_default();
                    let icon = icon.split(',').next().unwrap_or("").trim_matches('"');
                    if let Some(parent) = Path::new(icon).parent() {
                        location = parent.display().to_string();
                    }
                }
                if !Path::new(&location).join(format!("{app}.exe")).is_file() {
                    let mut candidates = Vec::new();
                    for variable in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
                        if let Some(base) = std::env::var_os(variable) {
                            for folder in [&display, &crate::model::title(app), &app.to_string()] {
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
                        .find(|p| p.join(format!("{app}.exe")).is_file())
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
                return Ok(Some(Installed {
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
                }));
            }
        }
    }
    Ok(None)
}
pub fn run(file: &Path, app: &str) -> Result<Installed> {
    run_with_job(file, app, None)
}
pub fn run_with_job(file: &Path, app: &str, job: Option<&crate::jobs::Job>) -> Result<Installed> {
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
    let code = if file
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("msi"))
    {
        let mut child = platform::hidden(
            Command::new("msiexec.exe")
                .arg("/i")
                .arg(file)
                .arg("/norestart"),
        )
        .spawn()?;
        wait_installer(job, child.id(), || {
            Ok(child.try_wait()?.map(|status| status.code().unwrap_or(-1)))
        })?
    } else {
        run_exe(file, job)?
    };
    match code {
        0 | 3010 | 1641 => {}
        1602 => bail!("Installation was canceled"),
        _ => bail!("Installer failed ({code})"),
    }
    detect(app)?.context("Installer finished, but Windows has not registered a usable app installation. Check the installer or Windows Installed apps.")
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
    let status = platform::hidden(
        Command::new("msiexec.exe")
            .arg("/x")
            .arg(code)
            .arg("/norestart"),
    )
    .status()?;
    if !matches!(status.code(), Some(0 | 3010 | 1641)) {
        bail!("Uninstall did not complete: {status}");
    }
    if detect(&app.name)?.is_some() {
        bail!("Windows still reports the app installed");
    }
    Ok(())
}

#[cfg(test)]
mod cancellation_tests {
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
