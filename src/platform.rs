use anyhow::{bail, Result};
use std::{
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Output},
};
use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0},
        Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH},
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile,
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
        },
        UI::Shell::{IShellLinkW, ShellLink},
    },
};
pub fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn hidden(c: &mut Command) -> &mut Command {
    c.creation_flags(0x08000000)
}
pub fn output(c: &mut Command) -> Result<Output> {
    Ok(hidden(c).output()?)
}
pub fn atomic_replace(from: &Path, to: &Path) -> Result<()> {
    let a = wide(from);
    let b = wide(to);
    for i in 0..50 {
        let result = unsafe {
            MoveFileExW(
                PCWSTR(a.as_ptr()),
                PCWSTR(b.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if result.is_ok() {
            return Ok(());
        }
        if i == 49 {
            return Err(result.unwrap_err().into());
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    unreachable!()
}
pub struct Lock(HANDLE);
impl Lock {
    pub fn take(name: &str) -> Result<Self> {
        let name = wide(name);
        let h = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }?;
        let wait = unsafe { WaitForSingleObject(h, 0) };
        if wait != WAIT_OBJECT_0 && wait != WAIT_ABANDONED {
            unsafe {
                let _ = CloseHandle(h);
            }
            bail!("Another operation is already running");
        }
        Ok(Self(h))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}
pub struct ProcessGroup(HANDLE);
impl ProcessGroup {
    pub fn attach(child: &std::process::Child) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        let h = unsafe { CreateJobObjectW(None, None) }?;
        let group = Self(h);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32,
            )?;
            AssignProcessToJobObject(h, HANDLE(child.as_raw_handle()))?;
        }
        Ok(group)
    }
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub fn open(p: &Path) -> Result<()> {
    let absolute = std::fs::canonicalize(p)?;
    let path = absolute.to_string_lossy();
    let shell_path = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_string()
    };
    if absolute.is_dir() {
        Command::new("explorer.exe")
            .arg(shell_path.replace('/', r"\"))
            .spawn()?;
        return Ok(());
    }
    let path = wide(shell_path.replace('/', r"\"));
    let verb = wide("open");
    let result = unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        bail!(
            "Windows could not open {} (shell error {})",
            p.display(),
            result.0 as isize
        );
    }
    Ok(())
}
pub fn executable_version(path: &Path) -> Option<String> {
    #[link(name = "version")]
    extern "system" {
        fn GetFileVersionInfoSizeW(path: *const u16, handle: *mut u32) -> u32;
        fn GetFileVersionInfoW(
            path: *const u16,
            handle: u32,
            size: u32,
            data: *mut std::ffi::c_void,
        ) -> i32;
        fn VerQueryValueW(
            data: *const std::ffi::c_void,
            key: *const u16,
            value: *mut *mut std::ffi::c_void,
            length: *mut u32,
        ) -> i32;
    }
    let path = wide(path);
    let root = wide(r"\");
    unsafe {
        let mut unused = 0;
        let size = GetFileVersionInfoSizeW(path.as_ptr(), &mut unused);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return None;
        }
        let mut value = std::ptr::null_mut();
        let mut length = 0;
        if VerQueryValueW(data.as_ptr().cast(), root.as_ptr(), &mut value, &mut length) == 0
            || length < 52
            || value.is_null()
        {
            return None;
        }
        let value = value.cast::<u32>();
        if value.read_unaligned() != 0xfeef04bd {
            return None;
        }
        let high = value.add(2).read_unaligned();
        let low = value.add(3).read_unaligned();
        Some(format!("{}.{}.{}", high >> 16, high & 0xffff, low >> 16))
    }
}
pub fn running_app(name: &str) -> Result<bool> {
    let out = output(Command::new("tasklist.exe").args(["/FO", "CSV", "/NH"]))?;
    if !out.status.success() {
        bail!("Could not check running apps");
    }
    let s = String::from_utf8_lossy(&out.stdout).to_lowercase();
    Ok(s.lines().any(|l| {
        l.starts_with(&format!("\"{name}.exe\"")) || l.starts_with(&format!("\"{name}-cli.exe\""))
    }))
}
pub fn shortcut(path: &Path, target: &Path, args: &str, working: &Path) -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let result = (|| -> Result<()> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let t = wide(target);
            let a = wide(args);
            let w = wide(working);
            link.SetPath(PCWSTR(t.as_ptr()))?;
            link.SetArguments(PCWSTR(a.as_ptr()))?;
            link.SetWorkingDirectory(PCWSTR(w.as_ptr()))?;
            let file: IPersistFile = link.cast()?;
            let p = wide(path);
            file.Save(PCWSTR(p.as_ptr()), true)?;
            Ok(())
        })();
        CoUninitialize();
        result
    }
}
pub fn notify(exe: &Path, message: &str) -> Result<()> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) =
        hkcu.create_subkey("Software\\Classes\\AppUserModelId\\CraftApps.Updater.Rust")?;
    key.set_value("DisplayName", &"Craft Apps Updater")?;
    let (protocol, _) = hkcu.create_subkey("Software\\Classes\\craft-apps-updater-rust")?;
    protocol.set_value("", &"URL:Craft Apps Updater")?;
    protocol.set_value("URL Protocol", &"")?;
    let (command, _) = protocol.create_subkey("shell\\open\\command")?;
    command.set_value("", &format!("\"{}\"", exe.display()))?;
    use windows::{
        core::HSTRING,
        Data::Xml::Dom::XmlDocument,
        UI::Notifications::{ToastNotification, ToastNotificationManager},
    };
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(format!("<toast activationType='protocol' launch='craft-apps-updater-rust:'><visual><binding template='ToastGeneric'><text>Craft apps updated</text><text>{}</text></binding></visual></toast>",escape(message))))?;
    let toast = ToastNotification::CreateToastNotification(&xml)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from("CraftApps.Updater.Rust"))?
        .Show(&toast)?;
    Ok(())
}
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn verify_microsoft_signature(path: &Path) -> Result<()> {
    use windows::Win32::Security::{
        Cryptography::{CertGetNameStringW, CERT_NAME_SIMPLE_DISPLAY_TYPE},
        WinTrust::*,
    };
    let filename = wide(path);
    let mut info = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(filename.as_ptr()),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut info },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    unsafe {
        let code = WinVerifyTrust(None, &mut action, &mut data as *mut _ as _);
        let result = (|| -> Result<()> {
            if code != 0 {
                bail!("Microsoft installer signature is invalid ({code:#x})");
            }
            let provider = WTHelperProvDataFromStateData(data.hWVTStateData);
            if provider.is_null() {
                bail!("No signature provider")
            };
            let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
            if signer.is_null() || (*signer).csCertChain == 0 || (*signer).pasCertChain.is_null() {
                bail!("No signer certificate")
            };
            let cert = (*(*signer).pasCertChain).pCert;
            let len = CertGetNameStringW(cert, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, None);
            let mut name = vec![0u16; len as usize];
            CertGetNameStringW(
                cert,
                CERT_NAME_SIMPLE_DISPLAY_TYPE,
                0,
                None,
                Some(&mut name),
            );
            let name = String::from_utf16_lossy(name.strip_suffix(&[0]).unwrap_or(&name));
            if name != "Microsoft Corporation" {
                bail!("Unexpected installer publisher: {name}");
            }
            Ok(())
        })();
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrust(None, &mut action, &mut data as *mut _ as _);
        result
    }
}
pub fn run_elevated(exe: &Path, args: &str, job: &crate::jobs::Job) -> Result<()> {
    use windows::Win32::{
        System::Threading::{GetExitCodeProcess, TerminateProcess},
        UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
    };
    let file = wide(exe);
    let args = wide(args);
    let verb = wide("runas");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(args.as_ptr()),
        nShow: 0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info)?;
        let result = (|| -> Result<()> {
            loop {
                if job.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    if TerminateProcess(info.hProcess, 1).is_err() {
                        bail!("Cancelled; close the elevated Microsoft installer if it is still running.");
                    }
                    bail!("Cancelled");
                }
                if WaitForSingleObject(info.hProcess, 200) == WAIT_OBJECT_0 {
                    let mut code = 0;
                    GetExitCodeProcess(info.hProcess, &mut code)?;
                    if code != 0 && code != 3010 {
                        bail!("Microsoft installer failed ({code})");
                    }
                    return Ok(());
                }
            }
        })();
        let _ = CloseHandle(info.hProcess);
        result
    }
}
