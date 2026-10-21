#[cfg(target_os = "macos")]
pub(super) use super::macos::{
    account, elevate, private_directory, require_admin, snapshot_file, target, user_directory,
    validate,
};
use super::*;
#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::process::Command;
#[cfg(target_os = "linux")]
use std::process::Stdio;
pub(super) fn lock_snapshot(path: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    Ok(options.open(path)?)
}
#[cfg(target_os = "linux")]
pub(super) fn snapshot_file(path: &Path) -> Result<fs::File> {
    Ok(fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?)
}

pub(super) struct Elevated {
    #[cfg(target_os = "macos")]
    pub(super) pid: i32,
    #[cfg(target_os = "linux")]
    child: std::process::Child,
    #[cfg(target_os = "windows")]
    handle: windows::Win32::Foundation::HANDLE,
}
impl Elevated {
    pub(super) fn running(&mut self) -> Result<bool> {
        #[cfg(target_os = "macos")]
        {
            if unsafe { libc::kill(self.pid, 0) } == 0 {
                return Ok(true);
            }
            match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::EPERM) => Ok(true),
                Some(libc::ESRCH) => Ok(false),
                _ => Err(std::io::Error::last_os_error().into()),
            }
        }
        #[cfg(target_os = "linux")]
        {
            Ok(self.child.try_wait()?.is_none())
        }
        #[cfg(target_os = "windows")]
        {
            Ok(
                unsafe { windows::Win32::System::Threading::WaitForSingleObject(self.handle, 0) }
                    == windows::Win32::Foundation::WAIT_TIMEOUT,
            )
        }
    }
}
#[cfg(target_os = "windows")]
impl Drop for Elevated {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

pub(super) struct PrivateDirectory(pub PathBuf);
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("craft-native-approved-"))
            && !files::linked(&self.0).unwrap_or(true)
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
pub(super) fn extension(kind: &InstallationKind) -> Result<&'static str> {
    Ok(match kind {
        InstallationKind::Msi => ".msi",
        InstallationKind::Deb => ".deb",
        InstallationKind::Rpm => ".rpm",
        InstallationKind::Arch => ".pkg.tar.zst",
        #[cfg(target_os = "macos")]
        InstallationKind::Bundle => ".package",
        _ => anyhow::bail!("Not a native package"),
    })
}
#[cfg(target_os = "linux")]
pub(super) fn account() -> Result<String> {
    Ok(unsafe { libc::getuid() }.to_string())
}
#[cfg(target_os = "linux")]
pub(super) fn require_admin(account: &str) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "Native bridge helper requires administrator approval"
    );
    ensure!(
        std::env::var("PKEXEC_UID").ok().as_deref() == Some(account) && account != "0",
        "Native bridge caller identity changed"
    );
    Ok(())
}
#[cfg(target_os = "linux")]
pub(super) fn user_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    fs::create_dir_all(path.parent().context("Missing native approval parent")?)?;
    fs::DirBuilder::new().mode(0o700).create(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
#[cfg(target_os = "linux")]
pub(super) fn private_directory() -> Result<PrivateDirectory> {
    use std::os::unix::fs::DirBuilderExt;
    let path = Path::new("/tmp").join(format!(
        "craft-native-approved-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::DirBuilder::new().mode(0o700).create(&path)?;
    Ok(PrivateDirectory(path))
}
#[cfg(target_os = "linux")]
pub(super) fn elevate(helper: &Path, manifest: &Path, checksum: &str) -> Result<Elevated> {
    let log = fs::File::create(
        manifest
            .parent()
            .context("Missing native approval directory")?
            .join("authorization.log"),
    )?;
    let child = Command::new("pkexec")
        .arg("--disable-internal-agent")
        .arg(helper)
        .arg("--native-bridge-admin")
        .arg(manifest)
        .arg(checksum)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    Ok(Elevated { child })
}
#[cfg(target_os = "linux")]
pub(super) fn target(path: &Path, profile: &ClientProfile) -> Result<()> {
    ensure!(
        path == Path::new("/usr/bin/craft-apps-manager")
            && profile.installation != InstallationKind::Msi,
        "Invalid native manager target"
    );
    Ok(())
}
#[cfg(target_os = "linux")]
pub(super) fn validate(file: &Path, p: &PackageRecord, _catalog: &VerifiedCatalog) -> Result<()> {
    crate::self_update::validate_native_record(file, p)
}
pub(super) fn batch_supported(file: &Path) -> Result<bool> {
    #[cfg(target_os = "windows")]
    {
        crate::installers::manager_msi_batch_ready(file)
    }
    #[cfg(target_os = "linux")]
    {
        let _ = file;
        Ok(true)
    }
    #[cfg(target_os = "macos")]
    {
        super::macos::batch_supported(file)
    }
}
#[cfg(target_os = "linux")]
pub(super) fn verify_registered(
    target: &Path,
    _file: &Path,
    p: &PackageRecord,
    _catalog: &VerifiedCatalog,
) -> Result<()> {
    crate::self_update::verify_native_registered(target, p)
}
#[cfg(target_os = "linux")]
pub(super) fn install(
    file: &Path,
    _target: &Path,
    kind: &InstallationKind,
    log: &Path,
) -> Result<i32> {
    let args: &[&str] = match kind {
        InstallationKind::Deb => &["/usr/bin/apt-get", "install", "--reinstall", "-y"],
        InstallationKind::Rpm => &["/usr/bin/dnf", "install", "-y"],
        InstallationKind::Arch => &["/usr/bin/pacman", "-U", "--noconfirm"],
        _ => anyhow::bail!("Wrong native package manager"),
    };
    let log = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(log)?;
    let status = Command::new(args[0])
        .args(&args[1..])
        .arg(file)
        .current_dir("/")
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("HOME", "/root")
        .env("LANG", "C.UTF-8")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .status()?;
    status
        .code()
        .context("Package manager terminated without an exit code")
}

#[cfg(target_os = "windows")]
pub(super) fn account() -> Result<String> {
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL},
            Security::{
                Authorization::ConvertSidToStringSidW, GetTokenInformation, TokenUser, TOKEN_QUERY,
                TOKEN_USER,
            },
            System::Threading::{GetCurrentProcess, OpenProcessToken},
        },
    };
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let result = (|| -> Result<String> {
            let mut size = 0;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
            let mut data = vec![0u8; size as usize];
            GetTokenInformation(
                token,
                TokenUser,
                Some(data.as_mut_ptr().cast()),
                size,
                &mut size,
            )?;
            let user = std::ptr::read_unaligned(data.as_ptr().cast::<TOKEN_USER>());
            let mut text = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut text)?;
            let value = text.to_string();
            let _ = LocalFree(HLOCAL(text.0.cast()));
            Ok(value?)
        })();
        let _ = CloseHandle(token);
        result
    }
}
#[cfg(target_os = "windows")]
pub(super) fn require_admin(expected: &str) -> Result<()> {
    ensure!(
        unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() },
        "Native bridge helper requires administrator approval"
    );
    ensure!(
        account()? == expected,
        "Approve the bridge using the same Windows account to preserve per-user MSI ownership"
    );
    Ok(())
}
#[cfg(target_os = "windows")]
fn known_folder(id: &windows::core::GUID) -> Result<PathBuf> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DEFAULT},
    };
    unsafe {
        let value = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)?;
        let path = value.to_string();
        CoTaskMemFree(Some(value.0.cast()));
        Ok(PathBuf::from(path?))
    }
}
#[cfg(target_os = "windows")]
fn system_command(name: &str) -> Result<Command> {
    use windows::Win32::UI::Shell::{FOLDERID_System, FOLDERID_Windows};
    let system = known_folder(&FOLDERID_System)?;
    let windows = known_folder(&FOLDERID_Windows)?;
    let mut command = Command::new(system.join(name));
    command
        .current_dir(&system)
        .env_clear()
        .env("SystemRoot", &windows)
        .env("WINDIR", &windows)
        .env("PATH", system);
    Ok(command)
}
#[cfg(target_os = "windows")]
fn acl(path: &Path, owner: Option<&str>, grants: &[String]) -> Result<()> {
    if let Some(owner) = owner {
        ensure!(
            crate::platform::hidden(
                system_command("icacls.exe")?
                    .arg(path)
                    .arg("/setowner")
                    .arg(owner)
            )
            .output()?
            .status
            .success(),
            "Could not secure native approval owner"
        );
    }
    let mut command = system_command("icacls.exe")?;
    command
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .args(grants);
    ensure!(
        crate::platform::hidden(&mut command)
            .output()?
            .status
            .success(),
        "Could not secure native approval directory"
    );
    Ok(())
}
#[cfg(target_os = "windows")]
pub(super) fn user_directory(path: &Path) -> Result<()> {
    ensure!(!path.exists(), "Native approval directory already exists");
    fs::create_dir_all(path)?;
    acl(
        path,
        None,
        &[
            format!("*{}:(OI)(CI)F", account()?),
            "*S-1-5-18:(OI)(CI)F".into(),
            "*S-1-5-32-544:(OI)(CI)F".into(),
        ],
    )
}
#[cfg(target_os = "windows")]
pub(super) fn private_directory() -> Result<PrivateDirectory> {
    use windows::Win32::{Storage::FileSystem::CreateDirectoryW, UI::Shell::FOLDERID_Windows};
    let windows = known_folder(&FOLDERID_Windows)?;
    let path = windows.join("Temp").join(format!(
        "craft-native-approved-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let security = ProtectedSecurity::new()?;
    let attributes = security.attributes();
    let wide = crate::platform::wide(&path);
    unsafe {
        CreateDirectoryW(windows::core::PCWSTR(wide.as_ptr()), Some(&attributes))?;
    }
    Ok(PrivateDirectory(path))
}
#[cfg(target_os = "windows")]
struct ProtectedSecurity(windows::Win32::Security::PSECURITY_DESCRIPTOR);
#[cfg(target_os = "windows")]
impl ProtectedSecurity {
    fn new() -> Result<Self> {
        use windows::{
            core::PCWSTR,
            Win32::Security::{
                Authorization::{
                    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
                },
                PSECURITY_DESCRIPTOR,
            },
        };
        let text = crate::platform::wide("O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)");
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(text.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )?;
        }
        Ok(Self(descriptor))
    }
    fn attributes(&self) -> windows::Win32::Security::SECURITY_ATTRIBUTES {
        windows::Win32::Security::SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<windows::Win32::Security::SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0 .0,
            bInheritHandle: false.into(),
        }
    }
}
#[cfg(target_os = "windows")]
impl Drop for ProtectedSecurity {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(
                self.0 .0,
            ));
        }
    }
}
#[cfg(target_os = "windows")]
pub(super) fn snapshot_file(path: &Path) -> Result<fs::File> {
    use std::os::windows::io::FromRawHandle;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{GENERIC_READ, GENERIC_WRITE},
            Storage::FileSystem::{
                CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ,
            },
        },
    };
    let security = ProtectedSecurity::new()?;
    let attributes = security.attributes();
    let wide = crate::platform::wide(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ,
            Some(&attributes),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )?
    };
    Ok(unsafe { fs::File::from_raw_handle(handle.0) })
}
#[cfg(target_os = "windows")]
pub(super) fn elevate(helper: &Path, manifest: &Path, checksum: &str) -> Result<Elevated> {
    use windows::{
        core::PCWSTR,
        Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
    };
    let file = crate::platform::wide(helper);
    let verb = crate::platform::wide("runas");
    let text = manifest.to_str().context("Invalid native approval path")?;
    ensure!(
        !text.contains(['"', '\r', '\n']),
        "Invalid native approval path"
    );
    let args = crate::platform::wide(format!("--native-bridge-admin \"{text}\" {checksum}"));
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
        ShellExecuteExW(&mut info).context("Administrator approval was declined")?;
    }
    let handle = info.hProcess;
    ensure!(
        !handle.is_invalid(),
        "Native administrator worker returned no process handle"
    );
    Ok(Elevated { handle })
}
#[cfg(target_os = "windows")]
pub(super) fn target(path: &Path, profile: &ClientProfile) -> Result<()> {
    ensure!(
        profile.installation == InstallationKind::Msi
            && path
                .parent()
                .is_some_and(|p| p.join("installation-msi.json").is_file()),
        "Not the selected MSI manager installation"
    );
    Ok(())
}
#[cfg(target_os = "windows")]
pub(super) fn validate(file: &Path, p: &PackageRecord, catalog: &VerifiedCatalog) -> Result<()> {
    ensure!(
        p.installation == InstallationKind::Msi,
        "Wrong native package type"
    );
    super::super::handlers::native::validate_msi(
        file,
        &catalog.catalog().release(p).version,
        &p.architecture,
    )
}
#[cfg(target_os = "windows")]
pub(super) fn verify_registered(
    target: &Path,
    file: &Path,
    _p: &PackageRecord,
    _catalog: &VerifiedCatalog,
) -> Result<()> {
    ensure!(
        target.is_file()
            && target
                .parent()
                .is_some_and(|p| p.join("installation-msi.json").is_file())
            && crate::installers::manager_msi_registered(file, target, &account()?)?,
        "Native MSI registration, user context or installation target could not be verified"
    );
    Ok(())
}
#[cfg(target_os = "windows")]
pub(super) fn install(
    file: &Path,
    target: &Path,
    kind: &InstallationKind,
    log: &Path,
) -> Result<i32> {
    ensure!(*kind == InstallationKind::Msi, "Wrong native package type");
    let folder = target.parent().context("Missing MSI installation folder")?;
    drop(snapshot_file(log)?);
    let mut command = system_command("msiexec.exe")?;
    command
        .env(
            "TEMP",
            log.parent().context("Missing installer log directory")?,
        )
        .env("TMP", log.parent().unwrap())
        .arg("/i")
        .arg(crate::self_update::installer_path(file))
        .args(["/qn", "/norestart", "MSIINSTALLPERUSER=1"]);
    msi_install_folder(&mut command, folder)?;
    command
        .arg("/L*v")
        .arg(crate::self_update::installer_path(log));
    let status = crate::platform::hidden(&mut command).status()?;
    status.code().context("MSI terminated without an exit code")
}

#[cfg(target_os = "windows")]
pub(super) fn msi_install_folder(command: &mut Command, folder: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    let folder = crate::self_update::installer_path(folder);
    let folder = folder
        .to_str()
        .context("Invalid MSI installation folder encoding")?;
    ensure!(
        !folder.contains(['"', '\r', '\n', '\0']),
        "Invalid MSI installation folder"
    );
    // msiexec parses property assignments itself. Command::arg would quote the
    // whole assignment for spaces, but MSI requires quotes around its value.
    command.raw_arg(format!("INSTALLFOLDER=\"{folder}\""));
    Ok(())
}
