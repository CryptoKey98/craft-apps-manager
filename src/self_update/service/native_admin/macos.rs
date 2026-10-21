//! macOS bootstrap and filesystem authority. No downloaded executable runs here.
use super::*;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};

pub(crate) enum UpdateAccess {
    Writable,
    Administrator,
}

pub(crate) fn system_command(program: &str) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command
        .current_dir("/")
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", "/var/root")
        .env("LANG", "C");
    command
}

pub(crate) use crate::files::canonical_directory;

pub(crate) fn protected_parent(path: &Path) -> Result<()> {
    canonical_directory(path)?;
    let mut group = std::mem::MaybeUninit::<libc::group>::uninit();
    let mut buffer = vec![0u8; 16 * 1024];
    let mut result = std::ptr::null_mut();
    let code = unsafe {
        libc::getgrnam_r(
            c"admin".as_ptr(),
            group.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    ensure!(
        code == 0 && !result.is_null(),
        "Cannot resolve local administrator group"
    );
    let admin = unsafe { group.assume_init() }.gr_gid;
    for parent in path.ancestors() {
        let meta = fs::symlink_metadata(parent)?;
        ensure!(
            meta.uid() == 0
                && meta.mode() & 0o002 == 0
                && (meta.mode() & 0o020 == 0 || meta.gid() == admin),
            "Protected updates require an administrator-owned parent without shared write access"
        );
        deny_allow_acl(parent)?;
    }
    Ok(())
}

// Darwin ACL declarations match sys/acl.h. Fail closed for an extended allow ACL,
// whose effective writers cannot be determined safely from POSIX mode bits alone.
fn deny_allow_acl(path: &Path) -> Result<()> {
    use std::{ffi::c_void, os::unix::ffi::OsStrExt};
    unsafe extern "C" {
        fn acl_get_file(path: *const libc::c_char, kind: libc::c_int) -> *mut c_void;
        fn acl_valid(acl: *mut c_void) -> libc::c_int;
        fn acl_get_entry(
            acl: *mut c_void,
            index: libc::c_int,
            entry: *mut *mut c_void,
        ) -> libc::c_int;
        fn acl_get_tag_type(entry: *mut c_void, tag: *mut libc::c_int) -> libc::c_int;
        fn acl_free(acl: *mut c_void) -> libc::c_int;
    }
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let acl = unsafe { acl_get_file(path.as_ptr(), 0x100) };
    if acl.is_null() && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
        return Ok(()); // Darwin reports an absent extended ACL as ENOENT.
    }
    ensure!(!acl.is_null(), "Cannot inspect protected directory ACL");
    let check = (|| -> Result<()> {
        ensure!(unsafe { acl_valid(acl) } == 0, "Invalid directory ACL");
        let mut entry = std::ptr::null_mut();
        for index in 0..170 {
            if unsafe { acl_get_entry(acl, if index == 0 { 0 } else { -1 }, &mut entry) } != 0 {
                ensure!(
                    std::io::Error::last_os_error().raw_os_error() == Some(libc::EINVAL),
                    "Cannot read directory ACL"
                );
                return Ok(());
            }
            let mut tag = 0;
            ensure!(
                unsafe { acl_get_tag_type(entry, &mut tag) } == 0 && tag == 2,
                "Protected directory has an extended allow ACL; use a standard app location"
            );
        }
        anyhow::bail!("Too many directory ACL entries")
    })();
    unsafe {
        acl_free(acl);
    }
    check
}

pub(crate) fn update_access(bundle: &Path) -> Result<UpdateAccess> {
    let parent = bundle.parent().context("No bundle parent")?;
    canonical_directory(parent)?;
    ensure!(
        !parent.to_string_lossy().contains("AppTranslocation")
            && !parent.starts_with("/System")
            && !parent.starts_with("/Volumes"),
        "Move the manager to a writable local app folder before updating"
    );
    let probe = parent.join(format!(
        ".craft-update-access-{}",
        uuid::Uuid::new_v4().simple()
    ));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            fs::remove_file(probe)?;
            Ok(UpdateAccess::Writable)
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            protected_parent(parent)?;
            // Read-only volumes cannot become writable through administrator approval.
            use std::os::unix::ffi::OsStrExt;
            let c = std::ffi::CString::new(parent.as_os_str().as_bytes())?;
            let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
            ensure!(
                unsafe { libc::statfs(c.as_ptr(), stat.as_mut_ptr()) } == 0,
                "Cannot inspect update volume"
            );
            ensure!(
                unsafe { stat.assume_init() }.f_flags & libc::MNT_RDONLY as u32 == 0,
                "Update volume is read-only"
            );
            Ok(UpdateAccess::Administrator)
        }
        Err(e) => Err(e).context("Cannot inspect update directory"),
    }
}
pub(crate) fn account() -> Result<String> {
    Ok(unsafe { libc::getuid() }.to_string())
}
pub(crate) fn require_admin(account: &str) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "Administrator approval required"
    );
    let uid = account.parse::<u32>()?;
    ensure!(uid > 0 && uid < u32::MAX, "Invalid macOS caller UID");
    let args: Vec<_> = std::env::args().collect();
    let index = args
        .iter()
        .position(|s| s == "--macos-approval-uid")
        .context("Missing bootstrap identity")?;
    ensure!(
        args.get(index + 1) == Some(&uid.to_string()),
        "Bootstrap caller identity changed"
    );
    Ok(())
}
pub(crate) fn user_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path.parent().context("Missing approval parent")?)?;
    canonical_directory(path.parent().unwrap())?;
    fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}
pub(super) fn private_directory() -> Result<platform::PrivateDirectory> {
    let base = Path::new("/private/tmp");
    let path = base.join(format!(
        "craft-native-approved-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::DirBuilder::new().mode(0o700).create(&path)?;
    let directory = platform::PrivateDirectory(path);
    deny_allow_acl(&directory.0)?;
    Ok(directory)
}
pub(crate) fn snapshot_file(path: &Path) -> Result<fs::File> {
    Ok(fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?)
}
pub(crate) fn target(path: &Path, profile: &ClientProfile) -> Result<()> {
    ensure!(
        profile.platform == "macos" && profile.installation == InstallationKind::Bundle,
        "Wrong macOS installation kind"
    );
    let bundle = crate::self_update::app_bundle(path)?;
    ensure!(
        bundle.canonicalize()? == bundle && path.canonicalize()? == path,
        "Linked manager target"
    );
    files::no_links(&bundle)?;
    protected_parent(bundle.parent().context("No bundle parent")?)?;
    crate::self_update::verify_bundle(&bundle)
}
pub(crate) fn validate(file: &Path, p: &PackageRecord, _: &VerifiedCatalog) -> Result<()> {
    ensure!(
        p.platform == "macos"
            && p.installation == InstallationKind::Bundle
            && matches!(
                p.handler.as_str(),
                super::super::model::ZIP | super::super::model::TAR | super::super::model::DMG
            ),
        "Unsupported bundle transport"
    );
    ensure!(
        fs::metadata(file)?.len() == p.size && files::hash(file)? == p.sha256,
        "Bundle archive changed"
    );
    Ok(())
}
pub(crate) fn batch_supported(_: &Path) -> Result<bool> {
    Ok(true)
}

pub(super) fn elevate(
    helper: &Path,
    manifest: &Path,
    checksum: &str,
) -> Result<platform::Elevated> {
    let helper = helper.canonicalize()?;
    let manifest = manifest.canonicalize()?;
    let home = manifest.parent().context("Missing approval home")?;
    canonical_directory(home)?;
    ensure!(
        fs::metadata(home)?.uid() == unsafe { libc::geteuid() }
            && fs::metadata(home)?.mode() & 0o777 == 0o700,
        "Approval folder is not private"
    );
    crate::self_update::clear_quarantine(&helper)?;
    let approved: Manifest = super::read_bounded(&manifest)?;
    ensure!(
        files::hash(&manifest)? == checksum && files::hash(&helper)? == approved.source_hash,
        "Approval helper differs from the frozen running manager"
    );
    let script = super::macos_bootstrap::script(
        &helper,
        &manifest,
        checksum,
        &approved.source_hash,
        unsafe { libc::getuid() },
    )?;
    let out = system_command("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .output()?;
    ensure!(
        out.status.success(),
        "macOS administrator approval was declined or failed"
    );
    let pid = std::str::from_utf8(&out.stdout)?.trim().parse::<i32>()?;
    ensure!(pid > 1, "Invalid administrator worker PID");
    Ok(platform::Elevated { pid })
}

pub(super) fn bootstrap_guard() -> Result<platform::PrivateDirectory> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "Administrator worker required"
    );
    let executable = std::env::current_exe()?.canonicalize()?;
    let home = executable
        .parent()
        .context("Missing private helper directory")?;
    ensure!(
        home.parent() == Some(Path::new("/private/tmp"))
            && home
                .file_name()
                .is_some_and(|s| s.to_string_lossy().starts_with("craft-native-approved-")),
        "Invalid bootstrap directory"
    );
    canonical_directory(home)?;
    let meta = fs::metadata(home)?;
    ensure!(
        meta.uid() == 0 && meta.mode() & 0o777 == 0o700,
        "Untrusted bootstrap directory"
    );
    deny_allow_acl(home)?;
    Ok(platform::PrivateDirectory(home.to_path_buf()))
}

pub(super) fn validate_downloads(manifest: &Manifest) -> Result<()> {
    let uid = manifest.account.parse::<u32>()?;
    canonical_directory(&manifest.root)?;
    ensure!(
        fs::metadata(&manifest.root)?.uid() == uid,
        "Library owner differs from caller"
    );
    let first = manifest.packages.first().context("Empty approval")?;
    let home = first.parent().context("Missing approval directory")?;
    canonical_directory(home)?;
    files::inside(home, &manifest.root.join("runtime/native-admin"))?;
    let meta = fs::metadata(home)?;
    ensure!(
        meta.uid() == uid && meta.mode() & 0o777 == 0o700,
        "Approval directory ownership changed"
    );
    for (index, file) in manifest.packages.iter().enumerate() {
        ensure!(
            file == &home.join(format!("package-{index}.package")),
            "Unbound package path"
        );
        let meta = fs::symlink_metadata(file)?;
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink() && meta.uid() == uid,
            "Package ownership changed"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extended_allow_acl_is_not_mistaken_for_a_private_directory() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("craft-acl-{}", uuid::Uuid::new_v4()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let status = std::process::Command::new("/bin/chmod")
            .arg("+a")
            .arg("everyone allow list,search,add_file,add_subdirectory")
            .arg(&root)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(deny_allow_acl(&root).is_err());
        assert!(std::process::Command::new("/bin/chmod")
            .arg("-N")
            .arg(&root)
            .status()
            .unwrap()
            .success());
        deny_allow_acl(&root).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn standard_application_parent_is_inspected_without_elevation() {
        // Read-only characterization of Darwin's absent-ACL result and admin group.
        protected_parent(Path::new("/Applications")).unwrap();
    }
    #[test]
    fn linked_and_user_owned_protected_parents_are_rejected() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("craft-parent-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let link = root.join("link");
        symlink("/Applications", &link).unwrap();
        assert!(canonical_directory(&link).is_err());
        assert!(protected_parent(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
