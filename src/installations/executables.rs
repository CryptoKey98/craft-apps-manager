use std::path::{Path, PathBuf};

/// The launchable item a release installs. On macOS this is an `.app` bundle
/// directory such as `PhotoCraft.app`; elsewhere it is a single executable file.
pub fn executable_name(app: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{app}.exe")
    } else if cfg!(target_os = "macos") {
        if app == "cadcraft" {
            return "CADCraft.app".into();
        }
        format!(
            "{}{}.app",
            app[..1].to_uppercase(),
            app[1..].replace("craft", "Craft")
        )
    } else {
        app.into()
    }
}

/// The bare executable produced by a source build.
pub fn build_executable_name(app: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{app}.exe")
    } else {
        app.into()
    }
}

/// Whether a release item exists. macOS app bundles are directories.
pub fn is_executable(path: &Path) -> bool {
    if cfg!(target_os = "macos") {
        path.exists()
    } else {
        path.is_file()
    }
}

pub fn executable_names(app: &str) -> Vec<String> {
    let mut names: Vec<String> = crate::catalog::names(app)
        .iter()
        .map(|name| executable_name(name))
        .collect();
    if crate::catalog::get(app).is_some_and(|entry| entry.scheme == crate::catalog::Scheme::Generic)
    {
        names = desktop_executable_names(names);
    }
    #[cfg(target_os = "macos")]
    names.extend(
        crate::app_identity::native(app)
            .into_iter()
            .filter(|identity| identity.platform == "macos")
            .map(|identity| identity.executable),
    );
    names.dedup();
    // Prefer the current release name on Unix; keep older names as fallbacks.
    // Windows installer resolution is handled separately.
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        names.reverse();
    }
    names
}

// Generic desktop projects often suffix the GUI binary while shipping separate CLI tools.
// Keep the accepted suffixes explicit so daemon/helper binaries are never selected.
fn desktop_executable_names(mut names: Vec<String>) -> Vec<String> {
    let originals = names.clone();
    for name in originals {
        let (stem, extension) = name
            .strip_suffix(".exe")
            .map(|stem| (stem, ".exe"))
            .unwrap_or((&name, ""));
        for suffix in [
            "-qt",
            "-qt5",
            "-qt6",
            "-gui",
            "-desktop",
            "-client",
            ".Launcher",
            "-launcher",
        ] {
            names.push(format!("{stem}{suffix}{extension}"));
        }
    }
    names
}

pub fn installed_executable(folder: &Path, app: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    if let Some(executable) = crate::app_identity::portable(folder, app) {
        return Some(executable);
    }
    #[cfg(target_os = "macos")]
    {
        crate::installers::resolve_bundle(folder, app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(target_os = "windows")]
        if !crate::app_identity::native(app).is_empty() {
            if let Some(choice) = crate::installers::registered_launch_choice(folder, app) {
                return choice;
            }
        }
        #[cfg(target_os = "linux")]
        if let Some(executable) = crate::installers::registered_launch_executable(folder, app) {
            return Some(executable);
        }
        let executable = executable_names(app)
            .into_iter()
            .map(|name| folder.join(name))
            .find(|path| is_executable(path));
        #[cfg(target_os = "windows")]
        let executable =
            executable.or_else(|| crate::installers::registered_launch_executable(folder, app));
        executable.or_else(|| {
            if crate::catalog::get(app)?.scheme != crate::catalog::Scheme::Generic {
                return None;
            }
            let names = executable_names(app);
            let matches: Vec<_> = walkdir::WalkDir::new(folder)
                .max_depth(4)
                .into_iter()
                .filter_map(std::result::Result::ok)
                .filter(|entry| {
                    entry.file_type().is_file()
                        && names.iter().any(|name| {
                            entry
                                .file_name()
                                .to_string_lossy()
                                .eq_ignore_ascii_case(name)
                        })
                })
                .map(|entry| entry.into_path())
                .collect();
            (matches.len() == 1).then(|| matches[0].clone())
        })
    }
}

#[cfg(test)]
mod desktop_executable_tests {
    #[test]
    fn desktop_variants_keep_exact_names_and_exclude_background_tools() {
        let names = super::desktop_executable_names(vec!["transmission.exe".into()]);
        assert_eq!(names[0], "transmission.exe");
        assert!(names.contains(&"transmission-qt.exe".into()));
        assert!(!names.contains(&"transmission-daemon.exe".into()));
        assert!(!names.contains(&"transmission-remote.exe".into()));
        let launchers = super::desktop_executable_names(vec!["everythingtoolbar.exe".into()]);
        assert!(launchers.contains(&"everythingtoolbar.Launcher.exe".into()));
        assert!(launchers.contains(&"everythingtoolbar-launcher.exe".into()));
        assert!(!launchers.contains(&"unins000.exe".into()));
        assert!(!launchers.contains(&"everythingtoolbar-helper.exe".into()));
        let unix = super::desktop_executable_names(vec!["example".into()]);
        assert!(unix.contains(&"example-gui".into()));
    }
}
