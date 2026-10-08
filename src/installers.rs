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
        platform::hidden(
            Command::new("msiexec.exe")
                .arg("/i")
                .arg(file)
                .arg("/norestart"),
        )
        .status()?
        .code()
        .unwrap_or(-1)
    } else {
        run_exe(file)?
    };
    match code {
        0 | 3010 | 1641 => {}
        1602 => bail!("Installation was canceled"),
        _ => bail!("Installer failed ({code})"),
    }
    detect(app)?.context("Installer finished, but Windows has not registered a usable app installation. Check the installer or Windows Installed apps.")
}
fn run_exe(file: &Path) -> Result<i32> {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{GetExitCodeProcess, WaitForSingleObject},
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
        let result = (|| -> Result<i32> {
            if WaitForSingleObject(process, u32::MAX) != WAIT_OBJECT_0 {
                bail!("Could not wait for the installer");
            }
            let mut code = 0;
            GetExitCodeProcess(process, &mut code)?;
            Ok(code as i32)
        })();
        let _ = CloseHandle(process);
        result
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
