use anyhow::{Context, Result};
use eframe::egui;
use std::sync::mpsc::{self, Receiver};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

// A hidden Windows window does not receive winit's normal redraw requests.
// Post a paint message to its owning UI thread so queued tray actions and
// background-job completion are still processed without revealing the window.
#[derive(Clone, Copy)]
pub(super) struct Wake {
    #[cfg(target_os = "windows")]
    hwnd: isize,
}
impl Wake {
    pub(super) fn new(cc: &eframe::CreationContext<'_>) -> Self {
        #[cfg(target_os = "windows")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let hwnd = match cc.window_handle().map(|handle| handle.as_raw()) {
                Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
                _ => 0,
            };
            Self { hwnd }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = cc;
            Self {}
        }
    }
    fn repaint(self, ctx: &egui::Context) {
        ctx.request_repaint();
        #[cfg(target_os = "windows")]
        if self.hwnd != 0 {
            use windows::Win32::{
                Foundation::{HWND, LPARAM, WPARAM},
                UI::WindowsAndMessaging::{PostMessageW, WM_PAINT},
            };
            unsafe {
                let _ = PostMessageW(HWND(self.hwnd as *mut _), WM_PAINT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Open,
    Check,
    Exit,
}

pub(super) struct Tray {
    _icon: TrayIcon,
    check: MenuItem,
    events: Receiver<Action>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(target_os = "windows")]
    menu: Menu,
}

impl Tray {
    pub(super) fn new(ctx: &egui::Context, wake: Wake) -> Result<Self> {
        anyhow::ensure!(
            host_available(),
            "A system tray is not available on this desktop. Closing will quit normally."
        );
        let menu = Menu::new();
        let open = MenuItem::with_id("craft-tray-open", "Open Craft Apps Manager", true, None);
        let check = MenuItem::with_id("craft-tray-check", "Check for app updates", false, None);
        let exit = MenuItem::with_id("craft-tray-exit", "Exit", true, None);
        menu.append_items(&[&open, &check, &PredefinedMenuItem::separator(), &exit])?;
        let image = image::load_from_memory(include_bytes!("../../assets/icon.png"))?
            .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
            .into_rgba8();
        let icon = Icon::from_rgba(image.into_raw(), 32, 32)?;
        let icon = TrayIconBuilder::new()
            .with_id("craft-manager-tray")
            .with_tooltip("Craft Apps Manager")
            .with_menu(Box::new(menu.clone()))
            .with_icon(icon)
            .with_menu_on_left_click(false)
            .build()
            .context("Could not create the system tray icon; closing will quit normally")?;
        icon.set_visible(false)?;
        let (tx, events) = mpsc::channel();
        let menu_tx = tx.clone();
        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.0.as_str() {
                "craft-tray-open" => Action::Open,
                "craft-tray-check" => Action::Check,
                "craft-tray-exit" => Action::Exit,
                _ => return,
            };
            let _ = menu_tx.send(action);
            wake.repaint(&menu_ctx);
        }));
        let icon_ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if event.id().0 != "craft-manager-tray" {
                return;
            }
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                let _ = tx.send(Action::Open);
                wake.repaint(&icon_ctx);
            }
        }));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        #[cfg(target_os = "windows")]
        {
            let stop = stop.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Acquire) {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    if !stop.load(std::sync::atomic::Ordering::Acquire) {
                        wake.repaint(&ctx);
                    }
                }
            });
        }
        Ok(Self {
            _icon: icon,
            check,
            events,
            stop,
            #[cfg(target_os = "windows")]
            menu,
        })
    }

    pub(super) fn set_visible(&self, visible: bool) -> Result<()> {
        self._icon
            .set_visible(visible)
            .context("Could not change tray icon visibility")
    }

    pub(super) fn actions(&self) -> Vec<Action> {
        self.events.try_iter().collect()
    }

    pub(super) fn set_theme(&self, dark: bool) {
        #[cfg(target_os = "windows")]
        unsafe {
            use tray_icon::menu::MenuTheme;
            let _ = self.menu.set_theme_for_hwnd(
                self._icon.window_handle() as isize,
                if dark {
                    MenuTheme::Dark
                } else {
                    MenuTheme::Light
                },
            );
        }
        #[cfg(not(target_os = "windows"))]
        let _ = dark;
    }

    pub(super) fn set_check_enabled(&self, enabled: bool) {
        self.check.set_enabled(enabled);
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
        TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn host_available() -> bool {
    true
}

#[cfg(target_os = "linux")]
pub(super) fn host_available() -> bool {
    // The KSNI backend can register without a visible tray host. Never hide a
    // window in that case, or the user would have no way to reopen it.
    (|| -> Result<bool> {
        let connection = zbus::blocking::connection::Builder::session()?
            .method_timeout(std::time::Duration::from_secs(1))
            .build()?;
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            "org.kde.StatusNotifierWatcher",
            "/StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
        )?;
        Ok(proxy.get_property::<bool>("IsStatusNotifierHostRegistered")?)
    })()
    .unwrap_or(false)
}

pub(super) fn hide_on_close(enabled: bool, available: bool, exiting: bool) -> bool {
    enabled && available && !exiting
}

impl super::App {
    fn show_from_tray(&mut self, ctx: &egui::Context) {
        self.tray_hidden = false;
        if let Some(tray) = &self.tray {
            let _ = tray.set_visible(false);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    pub(super) fn poll_tray(&mut self, ctx: &egui::Context) {
        if self.builder {
            return;
        }
        if !self.preferences.close_to_tray {
            if self.tray_hidden {
                self.show_from_tray(ctx);
            }
            self.tray = None;
            self.tray_attempted = false;
            self.tray_error = None;
            return;
        }
        if !self.tray_attempted {
            self.tray_attempted = true;
            match Tray::new(ctx, self.tray_wake) {
                Ok(tray) => self.tray = Some(tray),
                Err(error) => {
                    self.tray_error = Some(format!("{error:#}"));
                    self.job.log(&format!("System tray unavailable: {error:#}"));
                }
            }
        }
        // If a Linux tray host disappears, restore the hidden window rather
        // than leaving the app running without a reachable Open/Exit menu.
        if cfg!(target_os = "linux") && self.tray.is_some() {
            if self.tray_probe.is_none()
                && self.tray_probe_at.elapsed() >= std::time::Duration::from_secs(10)
            {
                self.tray_probe_at = std::time::Instant::now();
                let (tx, rx) = mpsc::channel();
                self.tray_probe = Some(rx);
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(host_available());
                    ctx.request_repaint();
                });
            }
            if let Some(probe) = &self.tray_probe {
                if let Ok(available) = probe.try_recv() {
                    self.tray_probe = None;
                    if !available {
                        self.tray = None;
                        self.tray_error = Some(
                            "The system tray is no longer available. Closing will quit normally."
                                .into(),
                        );
                        if self.tray_hidden {
                            self.show_from_tray(ctx);
                        }
                    }
                }
            }
        }
        let busy = self.job.state.lock().unwrap().busy
            || self.build_job.state.lock().unwrap().busy
            || self.release_pending()
            || self.manager_plan.is_some();
        let can_check = !busy
            && self.periodic_receiver.is_none()
            && self.checking_apps.is_empty()
            && self.display_config.as_ref().is_some_and(|config| {
                config.apps.iter().any(|app| {
                    self.preferences.selected_apps.contains(&app.name)
                        && craft_apps_manager::updates::installed_for_updates(
                            app,
                            &self.preferences,
                        )
                })
            });
        let actions = if let Some(tray) = &self.tray {
            tray.set_theme(ctx.style().visuals.dark_mode);
            tray.set_check_enabled(can_check);
            tray.actions()
        } else {
            Vec::new()
        };
        for action in actions {
            match action {
                Action::Open => self.show_from_tray(ctx),
                Action::Check if can_check => {
                    let (tx, rx) = mpsc::channel();
                    self.periodic_receiver = Some(rx);
                    let paths = self.paths.clone();
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let job = craft_apps_manager::jobs::Job::new(
                            paths.at("runtime/tray-app-check.log"),
                            &Default::default(),
                        );
                        let result = craft_apps_manager::hourly::run(&paths, &job)
                            .and_then(|_| craft_apps_manager::hourly::read(&paths));
                        let _ = tx.send(result.map_err(|error| format!("{error:#}")));
                        ctx.request_repaint();
                    });
                }
                Action::Exit => {
                    self.closing = true;
                    self.release_plan = None;
                    self.job
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    self.build_job
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    if busy {
                        self.show_from_tray(ctx);
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn closing_requires_opt_in_and_a_tray_but_exit_always_quits() {
        assert!(super::hide_on_close(true, true, false));
        assert!(!super::hide_on_close(false, true, false));
        assert!(!super::hide_on_close(true, false, false));
        assert!(!super::hide_on_close(true, true, true));
        let mut prefs: craft_apps_manager::model::Preferences = serde_json::from_str("{}").unwrap();
        assert!(!prefs.close_to_tray);
        prefs.close_to_tray = true;
        let saved = serde_json::to_vec(&prefs).unwrap();
        assert!(
            serde_json::from_slice::<craft_apps_manager::model::Preferences>(&saved)
                .unwrap()
                .close_to_tray
        );
    }
}
