use super::*;
#[test]
fn managed_anycpu_is_not_confused_with_required_or_preferred_x86() {
    let path = std::env::temp_dir().join(format!("craft-clr-{}.exe", uuid::Uuid::new_v4()));
    let mut bytes = vec![0; 512];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
    bytes[64..68].copy_from_slice(b"PE\0\0");
    bytes[68..70].copy_from_slice(&0x14cu16.to_le_bytes());
    bytes[70..72].copy_from_slice(&1u16.to_le_bytes());
    bytes[84..86].copy_from_slice(&224u16.to_le_bytes());
    bytes[88..90].copy_from_slice(&0x10bu16.to_le_bytes());
    bytes[156..158].copy_from_slice(&2u16.to_le_bytes());
    bytes[180..184].copy_from_slice(&16u32.to_le_bytes());
    bytes[296..300].copy_from_slice(&0x2000u32.to_le_bytes());
    bytes[324..328].copy_from_slice(&0x2000u32.to_le_bytes());
    bytes[328..332].copy_from_slice(&160u32.to_le_bytes());
    bytes[332..336].copy_from_slice(&352u32.to_le_bytes());
    for flags in [3u32, 0x20001, 0] {
        bytes[368..372].copy_from_slice(&flags.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(windows_executable_architecture(&path).unwrap(), "x86");
    }
    bytes[368..372].copy_from_slice(&1u32.to_le_bytes());
    std::fs::write(&path, &bytes).unwrap();
    let actual = windows_executable_architecture(&path).unwrap();
    if cfg!(target_arch = "x86_64") {
        assert_eq!(actual, "x64");
    }
    assert!(verify_desktop_executable(&path, actual).is_ok());
    bytes[296..300].copy_from_slice(&0xfffffff0u32.to_le_bytes());
    std::fs::write(&path, &bytes).unwrap();
    assert!(windows_executable_architecture(&path).is_err());
    std::fs::remove_file(path).unwrap();
}
fn fixture(machine: u16, subsystem: u16, extra: &[u8]) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("craft-exe-{}.exe", uuid::Uuid::new_v4()));
    let mut bytes = vec![0; 158];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
    bytes[64..68].copy_from_slice(b"PE\0\0");
    bytes[68..70].copy_from_slice(&machine.to_le_bytes());
    bytes[84..86].copy_from_slice(&70u16.to_le_bytes());
    bytes[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[156..158].copy_from_slice(&subsystem.to_le_bytes());
    bytes.extend_from_slice(extra);
    std::fs::write(&path, bytes).unwrap();
    path
}
#[test]
fn actual_binary_type_blocks_console_apps_and_wrong_architecture() {
    let console = fixture(0x8664, 3, b"ordinary program");
    assert!(verify_installer_executable(&console).is_err());
    assert!(verify_desktop_executable(&console, "x64").is_err());
    let gui = fixture(0x8664, 2, b"ordinary desktop program");
    assert!(verify_desktop_executable(&gui, "x64").is_ok());
    assert!(verify_desktop_executable(&gui, "arm64").is_err());
    assert!(verify_installer_executable(&gui).is_err());
    std::fs::remove_file(console).unwrap();
    std::fs::remove_file(gui).unwrap();
}
#[test]
fn installer_markers_cross_scan_boundaries_and_do_not_execute_files() {
    let mut extra = vec![0; 65530 - 158];
    extra.extend_from_slice(b"Inno Setup Setup Data (6.4.2)");
    let installer = fixture(0x014c, 2, &extra);
    assert!(verify_installer_executable(&installer).is_ok());
    std::fs::remove_file(installer).unwrap();
}
