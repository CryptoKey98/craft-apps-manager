//! Keep eframe drawing during the Windows move/resize message loop.
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            KillTimer, PostMessageW, SetTimer, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_NCDESTROY,
            WM_PAINT, WM_TIMER,
        },
    },
};

pub(super) fn install(cc: &eframe::CreationContext<'_>) {
    if let Ok(RawWindowHandle::Win32(handle)) = cc.window_handle().map(|handle| handle.as_raw()) {
        // Creation runs on the window's UI thread. The callback removes itself at destruction.
        unsafe {
            let _ = SetWindowSubclass(HWND(handle.hwnd.get() as *mut _), Some(on_message), 1, 0);
        }
    }
}

unsafe extern "system" fn on_message(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    _: usize,
) -> LRESULT {
    let timer_id = on_message as *const () as usize;
    match message {
        WM_ENTERSIZEMOVE => {
            SetTimer(hwnd, timer_id, super::theme::BUILD_FRAME_INTERVAL_MS, None);
        }
        WM_EXITSIZEMOVE | WM_NCDESTROY => {
            let _ = KillTimer(hwnd, timer_id);
            if message == WM_NCDESTROY {
                let _ = RemoveWindowSubclass(hwnd, Some(on_message), subclass_id);
            }
        }
        WM_TIMER if wparam.0 == timer_id => {
            // Dispatch through winit's normal paint path, after this callback returns.
            let _ = PostMessageW(hwnd, WM_PAINT, WPARAM(0), LPARAM(0));
            return LRESULT(0);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}
