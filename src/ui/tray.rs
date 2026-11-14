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
    #[cfg(target_os = "linux")]
    can_hide: bool,
}
impl Wake {
    pub(super) fn can_hide(self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.can_hide
        }
        #[cfg(not(target_os = "linux"))]
        {
            true
        }
    }
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
        #[cfg(target_os = "linux")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            Self {
                can_hide: matches!(
                    cc.window_handle().map(|handle| handle.as_raw()),
                    Ok(RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_))
                ),
            }
        }
        #[cfg(target_os = "macos")]
        {
            let _ = cc;
            Self {}
        }
    }
    pub(super) fn repaint(self, ctx: &egui::Context) {
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

#[derive(Clone)]
struct EventTarget {
    generation: u64,
    sender: mpsc::Sender<Action>,
    context: egui::Context,
    wake: Wake,
}

// The native libraries only accept their global callbacks once. Keep those
// callbacks stable and replace their destination when the tray is recreated.
static EVENT_TARGET: std::sync::Mutex<Option<EventTarget>> = std::sync::Mutex::new(None);
static EVENT_HANDLERS: std::sync::Once = std::sync::Once::new();
static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn dispatch(action: Action) {
    let target = EVENT_TARGET.lock().unwrap().clone();
    if let Some(target) = target {
        let _ = target.sender.send(action);
        target.wake.repaint(&target.context);
    }
}

fn attach_events(ctx: &egui::Context, wake: Wake) -> (u64, Receiver<Action>) {
    EVENT_HANDLERS.call_once(|| {
        MenuEvent::set_event_handler(Some(|event: MenuEvent| match event.id.0.as_str() {
            "craft-tray-open" => dispatch(Action::Open),
            "craft-tray-check" => dispatch(Action::Check),
            "craft-tray-exit" => dispatch(Action::Exit),
            _ => {}
        }));
        TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
            if event.id().0 == "craft-manager-tray"
                && matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                )
            {
                dispatch(Action::Open);
            }
        }));
    });
    let (sender, events) = mpsc::channel();
    let generation = NEXT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    *EVENT_TARGET.lock().unwrap() = Some(EventTarget {
        generation,
        sender,
        context: ctx.clone(),
        wake,
    });
    (generation, events)
}

pub(super) struct Tray {
    _icon: TrayIcon,
    check: MenuItem,
    events: Receiver<Action>,
    generation: u64,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(target_os = "windows")]
    menu: Menu,
}

impl Tray {
    pub(super) fn new(ctx: &egui::Context, wake: Wake) -> Result<Self> {
        #[cfg(target_os = "linux")]
        anyhow::ensure!(
            wake.can_hide,
            "Close-to-tray needs X11 or XWayland on this desktop. Closing will quit normally."
        );
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
        let (generation, events) = attach_events(ctx, wake);
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
            generation,
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

    #[cfg(all(test, target_os = "windows"))]
    pub(in crate::ui) fn send_native_menu_command(&self, index: i32) {
        use tray_icon::menu::ContextMenu;
        use windows::Win32::{
            Foundation::{HWND, LPARAM, WPARAM},
            UI::WindowsAndMessaging::{GetMenuItemID, SendMessageW, HMENU, WM_COMMAND},
        };
        let command = unsafe { GetMenuItemID(HMENU(self.menu.hpopupmenu() as *mut _), index) };
        assert_ne!(command, u32::MAX);
        unsafe {
            SendMessageW(
                HWND(self._icon.window_handle() as *mut _),
                WM_COMMAND,
                WPARAM(command as usize),
                LPARAM(0),
            );
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        let mut target = EVENT_TARGET.lock().unwrap();
        if target
            .as_ref()
            .is_some_and(|target| target.generation == self.generation)
        {
            *target = None;
        }
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
    pub(crate) fn attach_instance(
        &mut self,
        instance: crate::app::instance::Instance,
        cc: &eframe::CreationContext<'_>,
    ) {
        let wake = self.tray_wake;
        let ctx = cc.egui_ctx.clone();
        instance.set_wake(move || wake.repaint(&ctx));
        self.instance = Some(instance);
    }

    pub(super) fn show_from_tray(&mut self, ctx: &egui::Context) {
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
        let busy = self.job.snapshot().busy
            || self.build_job.snapshot().busy
            || self.release_pending()
            || self.manager_plan.is_some();
        let can_check = !busy
            && self.periodic_receiver.is_none()
            && self.checking_apps.is_empty()
            && self.display_config.as_ref().is_some_and(|config| {
                !craft_apps_manager::updates::installed_check_targets(config).is_empty()
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
                Action::Check if can_check => self.start_background_app_checks(ctx, false),
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
    #[cfg(target_os = "windows")]
    #[test]
    fn exit_event_reaches_recreated_tray() {
        use super::*;
        use tray_icon::menu::ContextMenu;
        use windows::Win32::{
            Foundation::{HWND, LPARAM, WPARAM},
            UI::WindowsAndMessaging::{GetMenuItemID, SendMessageW, HMENU, WM_COMMAND},
        };
        let ctx = egui::Context::default();
        for generation in 0..3 {
            let tray = Tray::new(&ctx, Wake { hwnd: 0 }).unwrap();
            let command = unsafe { GetMenuItemID(HMENU(tray.menu.hpopupmenu() as *mut _), 3) };
            assert_ne!(command, u32::MAX);
            unsafe {
                SendMessageW(
                    HWND(tray._icon.window_handle() as *mut _),
                    WM_COMMAND,
                    WPARAM(command as usize),
                    LPARAM(0),
                );
            }
            assert_eq!(
                tray.actions(),
                vec![Action::Exit],
                "Exit must work in tray generation {generation}"
            );
        }
    }

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
