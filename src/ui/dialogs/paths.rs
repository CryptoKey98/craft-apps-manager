use super::*;

#[derive(Default)]
pub(in crate::ui) struct DestinationPreview {
    cached: Option<DestinationPreviewEntry>,
}

struct DestinationPreviewEntry {
    root: std::path::PathBuf,
    parent: String,
    app: String,
    result: Result<std::path::PathBuf, String>,
}

impl DestinationPreview {
    pub(super) fn get(
        &mut self,
        paths: &model::Paths,
        parent: &str,
        app: &str,
        resolve: impl FnOnce() -> anyhow::Result<std::path::PathBuf>,
    ) -> anyhow::Result<std::path::PathBuf> {
        if self.cached.as_ref().is_none_or(|entry| {
            entry.root != paths.root || entry.parent != parent || entry.app != app
        }) {
            self.cached = Some(DestinationPreviewEntry {
                root: paths.root.clone(),
                parent: parent.into(),
                app: app.into(),
                result: resolve().map_err(|error| format!("{error:#}")),
            });
        }
        self.cached
            .as_ref()
            .unwrap()
            .result
            .clone()
            .map_err(anyhow::Error::msg)
    }
}

pub(super) fn windows_installer_parent(
    architecture: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> String {
    let preferred = if architecture == "x86" {
        "ProgramFiles(x86)"
    } else {
        "ProgramW6432"
    };
    [preferred, "ProgramFiles"]
        .into_iter()
        .filter_map(lookup)
        .find(|path| std::path::Path::new(path).is_absolute())
        .unwrap_or_default()
}

pub(super) fn default_install_parent(
    paths: &model::Paths,
    current: Option<&model::Installed>,
    prefs: &model::Preferences,
    additional: bool,
) -> String {
    if prefs.release_format == "installer" && cfg!(target_os = "windows") {
        return windows_installer_parent(&prefs.architecture, |name| std::env::var(name).ok());
    }
    if !additional && prefs.release_format == "portable" {
        if let Some(parent) = current
            .filter(|copy| !copy.path.is_empty() && copy.install_kind != "installer")
            .and_then(|copy| std::path::Path::new(&copy.path).parent())
        {
            return parent.display().to_string();
        }
    }
    paths.at("releases").display().to_string()
}

pub(super) fn desktop_shortcut_option(ui: &mut egui::Ui, checked: &mut bool) {
    ui.horizontal(|ui| {
        ui.set_min_height(24.0);
        theme::checkbox(
            ui,
            checked,
            16.0,
            theme::palette().accent,
            "Create desktop shortcut",
            true,
        );
        if ui
            .add(egui::Label::new("Create desktop shortcut").sense(Sense::click()))
            .clicked()
        {
            *checked = !*checked;
        }
    });
}

pub(super) fn open_folder_button(ui: &mut egui::Ui) -> egui::Response {
    btn("Open folder")
        .kind(Kind::Ghost)
        .icon(Icon::Folder)
        .icon_weight(1.5)
        .text_color(theme::palette().link)
        .show(ui)
}

pub(super) fn compact_path(display: &str) -> String {
    if display.chars().count() <= 64 {
        return display.to_owned();
    }
    let beginning: String = display.chars().take(10).collect();
    let ending: String = display
        .chars()
        .rev()
        .take(32)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{beginning}…{ending}")
}

pub(super) fn folder_path_row(
    ui: &mut egui::Ui,
    path: &std::path::Path,
    mut open: impl FnMut(&std::path::Path),
) {
    ui.horizontal(|ui| {
        let path_width = (ui.available_width() - 112.0 - ui.spacing().item_spacing.x).max(20.0);
        ui.allocate_ui_with_layout(
            egui::vec2(path_width, 32.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(path_width);
                let display = path.display().to_string();
                let compact = compact_path(&display);
                ui.add(egui::Label::new(RichText::new(compact).size(14.0)).truncate())
                    .on_hover_text(display);
            },
        );
        if btn("Open folder")
            .kind(Kind::Ghost)
            .icon(Icon::Folder)
            .icon_weight(1.5)
            .text_color(theme::palette().link)
            .min_width(112.0)
            .show(ui)
            .clicked()
        {
            let mut existing = path;
            while !existing.is_dir() {
                if let Some(parent) = existing.parent() {
                    existing = parent;
                } else {
                    return;
                }
            }
            open(existing);
        }
    });
}

#[cfg(test)]
mod install_location_tests {
    use super::*;

    #[test]
    fn destination_preview_caches_success_and_errors_until_inputs_change() {
        let paths = model::Paths::new(std::env::temp_dir().join("preview-test"), None);
        let mut preview = DestinationPreview::default();
        let target = paths.at("releases/filmcraft-2");
        assert_eq!(
            preview
                .get(&paths, "parent", "filmcraft", || Ok(target.clone()))
                .unwrap(),
            target
        );
        assert_eq!(
            preview
                .get(&paths, "parent", "filmcraft", || panic!(
                    "must not rescan on repaint"
                ))
                .unwrap(),
            target
        );
        assert!(preview
            .get(&paths, "changed", "filmcraft", || anyhow::bail!(
                "invalid folder"
            ))
            .is_err());
        assert!(preview
            .get(&paths, "changed", "filmcraft", || panic!(
                "must cache errors too"
            ))
            .is_err());
        assert!(preview
            .get(&paths, "changed", "photocraft", || Ok(target.clone()))
            .is_ok());
        preview = DestinationPreview::default();
        let next = paths.at("releases/filmcraft-3");
        assert_eq!(
            preview
                .get(&paths, "parent", "filmcraft", || Ok(next.clone()))
                .unwrap(),
            next
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn installer_defaults_follow_architecture_and_never_fall_back_to_releases() {
        let lookup = |name: &str| match name {
            "ProgramW6432" => Some(r"C:\Program Files".into()),
            "ProgramFiles(x86)" | "ProgramFiles" => Some(r"C:\Program Files (x86)".into()),
            _ => None,
        };
        assert_eq!(windows_installer_parent("x64", lookup), r"C:\Program Files");
        assert_eq!(
            windows_installer_parent("arm64", lookup),
            r"C:\Program Files"
        );
        assert_eq!(
            windows_installer_parent("x86", lookup),
            r"C:\Program Files (x86)"
        );
        assert!(windows_installer_parent("x64", |_| None).is_empty());
        assert!(windows_installer_parent("x64", |_| Some("relative".into())).is_empty());
    }
}
