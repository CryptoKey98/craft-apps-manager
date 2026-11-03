//! Immediate MSI custom action: retain the related per-user product's install directory.
//! Kept inside the incoming package so unmodified older updaters benefit too.

fn choose_location(locations: &[String]) -> Result<Option<&str>, &'static str> {
    let mut selected: Option<&str> = None;
    for location in locations {
        if location.trim().is_empty() {
            return Err("The previous Manager installation has no registered folder.");
        }
        if let Some(previous) = selected {
            let normalize = |s: &str| s.replace('/', "\\").trim_end_matches('\\').to_lowercase();
            if normalize(previous) != normalize(location) {
                return Err("Related Manager installations have different folders.");
            }
        } else {
            selected = Some(location);
        }
    }
    Ok(selected)
}

fn registered_folder_usable(
    absolute: bool,
    exists: bool,
    directory: bool,
    maintenance: bool,
) -> bool {
    absolute && (directory || (maintenance && !exists))
}

#[cfg(windows)]
mod action {
    use super::choose_location;
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::{Path, PathBuf},
        ptr,
    };

    const SUCCESS: u32 = 0;
    const MORE_DATA: u32 = 234;
    const UNKNOWN_PRODUCT: u32 = 1605;
    const INSTALL_FAILURE: u32 = 1603;

    #[link(name = "msi")]
    extern "system" {
        fn MsiGetPropertyW(session: u32, name: *const u16, value: *mut u16, size: *mut u32) -> u32;
        fn MsiSetPropertyW(session: u32, name: *const u16, value: *const u16) -> u32;
        fn MsiGetProductInfoExW(
            product: *const u16,
            sid: *const u16,
            context: u32,
            property: *const u16,
            value: *mut u16,
            size: *mut u32,
        ) -> u32;
        fn MsiCreateRecord(fields: u32) -> u32;
        fn MsiRecordSetStringW(record: u32, field: u32, value: *const u16) -> u32;
        fn MsiProcessMessage(session: u32, kind: u32, record: u32) -> i32;
        fn MsiCloseHandle(handle: u32) -> u32;
    }

    #[link(name = "version")]
    extern "system" {
        fn GetFileVersionInfoSizeW(file: *const u16, handle: *mut u32) -> u32;
        fn GetFileVersionInfoW(file: *const u16, handle: u32, size: u32, data: *mut u8) -> i32;
        fn VerQueryValueW(
            data: *const u8,
            key: *const u16,
            value: *mut *const u32,
            size: *mut u32,
        ) -> i32;
    }

    fn executable_version(file: &Path) -> Option<String> {
        let name: Vec<u16> = file.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let mut ignored = 0;
            let size = GetFileVersionInfoSizeW(name.as_ptr(), &mut ignored);
            if size == 0 || size > 1024 * 1024 {
                return None;
            }
            let mut data = vec![0; size as usize];
            if GetFileVersionInfoW(name.as_ptr(), 0, size, data.as_mut_ptr()) == 0 {
                return None;
            }
            let mut value = ptr::null();
            let mut length = 0;
            if VerQueryValueW(data.as_ptr(), wide("\\").as_ptr(), &mut value, &mut length) == 0
                || value.is_null()
                || length < 52
            {
                return None;
            }
            if ptr::read_unaligned(value) != 0xfeef04bd {
                return None;
            }
            let ms = ptr::read_unaligned(value.add(4));
            let ls = ptr::read_unaligned(value.add(5));
            Some(format!("{}.{}.{}", ms >> 16, ms & 0xffff, ls >> 16))
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn read_string(mut read: impl FnMut(*mut u16, *mut u32) -> u32) -> Result<String, u32> {
        let mut size = 0;
        // MsiGetProperty requires a non-null empty buffer even for a size query.
        let mut empty = 0;
        let first = read(&mut empty, &mut size);
        if first != SUCCESS && first != MORE_DATA {
            return Err(first);
        }
        if size > 32767 {
            return Err(INSTALL_FAILURE);
        }
        let mut value = vec![0; size as usize + 1];
        size += 1;
        let code = read(value.as_mut_ptr(), &mut size);
        if code != SUCCESS {
            return Err(code);
        }
        String::from_utf16(&value[..size as usize]).map_err(|_| INSTALL_FAILURE)
    }

    fn property(session: u32, name: &str) -> Result<String, u32> {
        let name = wide(name);
        read_string(|value, size| unsafe { MsiGetPropertyW(session, name.as_ptr(), value, size) })
    }

    fn registered_property(product: &str, property: &str) -> Result<String, u32> {
        let product = wide(product);
        let name = wide(property);
        // This package is per-user. Never borrow another user's or a machine-wide path.
        for context in [2, 1] {
            // MSIINSTALLCONTEXT_USERUNMANAGED, USERMANAGED
            let value = read_string(|value, size| unsafe {
                MsiGetProductInfoExW(
                    product.as_ptr(),
                    ptr::null(),
                    context,
                    name.as_ptr(),
                    value,
                    size,
                )
            });
            if !matches!(value, Err(UNKNOWN_PRODUCT)) {
                return value;
            }
        }
        Err(UNKNOWN_PRODUCT)
    }

    fn registered_location(session: u32, product: &str) -> Result<String, u32> {
        let location = registered_property(product, "InstallLocation")?;
        if !location.is_empty() {
            return Ok(location);
        }
        // Pre-1.0 packages did not persist ARPINSTALLLOCATION. Only accept their
        // default directory when its MSI marker and executable version match the
        // registered related product. Never guess an unknown custom directory.
        let base = property(session, "LocalAppDataFolder")?;
        let name = property(session, "MANAGERINSTALLDIRNAME")?;
        if base.is_empty() || name.is_empty() || name.contains(['/', '\\']) {
            return Err(INSTALL_FAILURE);
        }
        let folder = legacy_default_folder(&base, &name)?;
        let version = registered_property(product, "VersionString")?;
        if !folder.join("installation-msi.json").is_file()
            || executable_version(&folder.join("CraftApps-Manager.exe")).as_deref()
                != Some(&version)
        {
            return Err(INSTALL_FAILURE);
        }
        log(session, "Verified the default folder of a legacy Manager installation without folder registration.", false);
        Ok(folder.to_string_lossy().into_owned())
    }

    fn legacy_default_folder(base: &str, name: &str) -> Result<PathBuf, u32> {
        if base.is_empty() || name.is_empty() || name.contains(['/', '\\']) {
            return Err(INSTALL_FAILURE);
        }
        Ok(PathBuf::from(base).join("Programs").join(name))
    }

    fn log(session: u32, text: &str, error: bool) {
        let text = wide(text);
        unsafe {
            let record = MsiCreateRecord(0);
            if record != 0 {
                MsiRecordSetStringW(record, 0, text.as_ptr());
                MsiProcessMessage(session, if error { 0x01000000 } else { 0x04000000 }, record);
                MsiCloseHandle(record);
            }
        }
    }

    fn preserve(session: u32) -> Result<(), u32> {
        let related = property(session, "WIX_UPGRADE_DETECTED")?;
        let installed = property(session, "Installed")?;
        let maintenance = related.is_empty() && !installed.is_empty();
        let products = if maintenance {
            property(session, "ProductCode")?
        } else {
            related
        };
        let locations = products
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|product| registered_location(session, product))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(location) = choose_location(&locations).map_err(|message| {
            log(session, message, true);
            INSTALL_FAILURE
        })? {
            let path = OsString::from_wide(&location.encode_utf16().collect::<Vec<_>>());
            let path = Path::new(&path);
            if !super::registered_folder_usable(
                path.is_absolute(),
                path.exists(),
                path.is_dir(),
                maintenance,
            ) {
                log(session, "The previous Manager installation folder is unavailable; upgrade stopped before removal.", true);
                return Err(INSTALL_FAILURE);
            }
            let code = unsafe {
                MsiSetPropertyW(
                    session,
                    wide("INSTALLFOLDER").as_ptr(),
                    wide(location).as_ptr(),
                )
            };
            if code != SUCCESS {
                return Err(code);
            }
            log(
                session,
                "Preserving the registered Manager installation folder.",
                false,
            );
        }
        Ok(())
    }

    #[no_mangle]
    pub extern "system" fn PreserveManagerInstallLocation(session: u32) -> u32 {
        match std::panic::catch_unwind(|| preserve(session)) {
            Ok(Ok(())) => SUCCESS,
            Ok(Err(code)) => {
                log(session, &format!("Could not preserve the existing Manager installation folder (Windows Installer status {code}); no files were removed."), true);
                INSTALL_FAILURE
            }
            Err(_) => INSTALL_FAILURE,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn legacy_folder_matches_the_msi_programs_subdirectory() {
            assert_eq!(
                legacy_default_folder(r"C:\Users\Test\AppData\Local", "Manager x64"),
                Ok(PathBuf::from(
                    r"C:\Users\Test\AppData\Local\Programs\Manager x64"
                ))
            );
            assert!(legacy_default_folder("", "Manager").is_err());
            assert!(legacy_default_folder(r"C:\Users\Test\AppData\Local", r"..\Other").is_err());
        }
        #[test]
        fn property_size_query_uses_nonnull_buffer_and_reads_unicode() {
            let expected = "C:\\User data\\Mánager 测试";
            let encoded: Vec<u16> = expected.encode_utf16().collect();
            let result = read_string(|buffer, count| unsafe {
                assert!(!buffer.is_null());
                if *count == 0 {
                    *count = encoded.len() as u32;
                    MORE_DATA
                } else {
                    assert!(*count as usize > encoded.len());
                    ptr::copy_nonoverlapping(encoded.as_ptr(), buffer, encoded.len());
                    *buffer.add(encoded.len()) = 0;
                    *count = encoded.len() as u32;
                    SUCCESS
                }
            });
            assert_eq!(result, Ok(expected.to_string()));
        }
        #[test]
        fn missing_property_is_empty_without_an_error() {
            assert_eq!(
                read_string(|_, size| {
                    unsafe { *size = 0 };
                    SUCCESS
                }),
                Ok(String::new())
            );
        }
        #[test]
        fn oversized_property_is_rejected_before_allocation() {
            assert_eq!(
                read_string(|_, size| {
                    unsafe { *size = u32::MAX };
                    MORE_DATA
                }),
                Err(INSTALL_FAILURE)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repair_may_recreate_a_missing_known_folder_but_upgrade_must_not_guess() {
        assert!(registered_folder_usable(true, false, false, true));
        assert!(!registered_folder_usable(true, false, false, false));
        assert!(!registered_folder_usable(true, true, false, true));
        assert!(!registered_folder_usable(false, false, false, true));
    }
    #[test]
    fn fresh_install_keeps_package_default() {
        assert_eq!(choose_location(&[]), Ok(None));
    }
    #[test]
    fn upgrade_keeps_custom_folder_with_spaces_and_unicode() {
        let locations = vec!["C:\\Test data\\Mánager 测试\\".into()];
        assert_eq!(choose_location(&locations), Ok(Some(locations[0].as_str())));
    }
    #[test]
    fn conflicting_related_products_stop_before_removal() {
        assert!(choose_location(&["C:\\One".into(), "D:\\Two".into()]).is_err());
    }
    #[test]
    fn missing_registration_does_not_silently_move_to_default() {
        assert!(choose_location(&[String::new()]).is_err());
    }
    #[test]
    fn equivalent_locations_are_unambiguous() {
        let locations = vec!["C:\\Test Folder\\".into(), "c:/test folder".into()];
        assert_eq!(choose_location(&locations), Ok(Some(locations[0].as_str())));
    }
}
