use crate::model::{Installed, Preferences};

pub const MANAGER_ARCH: &str = if cfg!(target_arch = "x86") {
    "x86"
} else if cfg!(target_arch = "aarch64") {
    "arm64"
} else {
    "x64"
};

/// Universal macOS bundles match all saved architecture selections.
pub fn installed_architecture_matches(installed: &Installed, prefs: &Preferences) -> bool {
    if installed.architecture.is_empty() {
        return false;
    }
    let arch = &installed.architecture;
    release_arch(arch) == release_arch(&prefs.architecture)
}

/// Native installer architectures are mutually exclusive in the manager.
pub fn installer_architecture_conflict(installed: &Installed, prefs: &Preferences) -> bool {
    prefs.release_format == "installer"
        && installed.install_kind == "installer"
        && !installed.path.is_empty()
        && !installed.version.is_empty()
        && !installed_architecture_matches(installed, prefs)
}

pub fn installer_architecture_conflict_message(
    installed: &Installed,
    prefs: &Preferences,
) -> String {
    let arch = if installed.architecture.is_empty() {
        MANAGER_ARCH
    } else {
        &installed.architecture
    };
    format!(
        "Uninstall the {} version before installing {}. Select {} to manage it.",
        architecture_label(arch),
        architecture_label(&prefs.architecture),
        architecture_label(arch)
    )
}

pub fn release_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

pub fn architecture_label(architecture: &str) -> &str {
    if cfg!(target_os = "macos") {
        "Universal (Intel + Apple silicon)"
    } else {
        architecture
    }
}

pub fn release_arch(architecture: &str) -> &str {
    if cfg!(target_os = "macos") {
        // Upstream publishes one universal DMG for Intel and Apple silicon.
        "universal"
    } else if cfg!(target_os = "linux") {
        match architecture {
            "x64" => "x86_64",
            "x86" => "i686",
            "arm64" => "aarch64",
            other => other,
        }
    } else {
        architecture
    }
}
