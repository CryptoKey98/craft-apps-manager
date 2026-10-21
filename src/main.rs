#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod app;
mod ui;
#[cfg(target_os = "windows")]
use craft_apps_manager::platform;
use std::fs;
fn main() {
    if let Err(e) = app::cli::run() {
        let message = format!("{e:#}");
        #[cfg(unix)]
        if std::env::args()
            .any(|arg| arg == "--approved-admin-batch" || arg == "--native-bridge-admin")
        {
            // The root helper reports through its captured stderr, never user settings/log paths.
            eprintln!("{message}");
            std::process::exit(1);
        }
        let path = craft_apps_manager::self_update::startup_log_path();
        if let Some(path) = path {
            let _ = fs::create_dir_all(path.parent().unwrap());
            let _ = fs::write(path, &message);
        }
        if std::env::args().any(|a| {
            a == "--update"
                || a == "--approved-admin-batch"
                || a == "--native-bridge-admin"
                || a == "--update-source"
                || a == "--background"
                || a == "--check-app-updates"
                || a == "--check-source-updates"
                || a == "--apply-self-update"
                || a == "--list-apps"
                || a == "--install-app"
                || a == "--uninstall-app"
                || a == "--restore-relocation-files"
        }) {
            eprintln!("{message}");
            std::process::exit(1);
        }
        #[cfg(unix)]
        eprintln!("{message}");
        #[cfg(target_os = "windows")]
        unsafe {
            let text = platform::wide(&message);
            let title = platform::wide("Craft Apps Manager");
            windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                None,
                windows::core::PCWSTR(text.as_ptr()),
                windows::core::PCWSTR(title.as_ptr()),
                windows::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
    }
}
