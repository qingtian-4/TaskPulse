//! Windows observations for TaskPulse's own tray window, never a global hook.
use tauri::AppHandle;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId,
    IsWindowVisible, WindowFromPoint, GA_ROOT, WM_ENTERMENULOOP, WM_EXITMENULOOP, WM_NCDESTROY,
};

pub fn tray_host_focused() -> bool {
    let mut point = POINT::default();
    // SAFETY: only current OS focus/hit-test metadata is read. The caller also
    // requires the point to fall in this app's Shell notification-icon rect.
    unsafe {
        if GetCursorPos(&mut point) == 0 {
            return false;
        }
        let hit = GetAncestor(WindowFromPoint(point), GA_ROOT);
        if hit.is_null() || GetAncestor(GetForegroundWindow(), GA_ROOT) != hit {
            return false;
        }
        let mut class = [0u16; 128];
        let len = GetClassNameW(hit, class.as_mut_ptr(), class.len() as i32);
        matches!(
            String::from_utf16_lossy(&class[..len.max(0) as usize]).as_str(),
            "Shell_TrayWnd"
                | "Shell_SecondaryTrayWnd"
                | "NotifyIconOverflowWindow"
                | "TopLevelWindowForOverflowXamlIsland"
        )
    }
}

pub fn window_state(window: &tauri::WebviewWindow) -> (bool, bool) {
    let Ok(handle) = window.hwnd() else {
        return (false, false);
    };
    let window = handle.0 as HWND;
    // SAFETY: the window supplies its live HWND. Unlike Tao's cached focus,
    // these observations describe the state when this queued callback runs.
    unsafe {
        (
            GetForegroundWindow() == window,
            IsWindowVisible(window) != 0,
        )
    }
}

pub fn tray_button_down() -> bool {
    // SAFETY: read-only observations of the current physical mouse gesture.
    unsafe {
        GetAsyncKeyState(i32::from(VK_LBUTTON)) < 0 || GetAsyncKeyState(i32::from(VK_RBUTTON)) < 0
    }
}

pub fn foreground_in_process() -> bool {
    let mut pid = 0;
    // SAFETY: HWND is an OS value; pid is a live initialized output buffer.
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut pid) };
    pid == std::process::id()
}

const SUBCLASS_ID: usize = 0x5450_4d45;

unsafe extern "system" fn menu_events(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        // SAFETY: our unique subclass owns this Box until its window is destroyed.
        unsafe {
            RemoveWindowSubclass(window, Some(menu_events), id);
            drop(Box::from_raw(data as *mut AppHandle));
            return DefSubclassProc(window, message, wparam, lparam);
        }
    }
    // SAFETY: SetWindowSubclass keeps our Box alive until WM_NCDESTROY above.
    let app = unsafe { (&*(data as *const AppHandle)).clone() };
    if message == WM_ENTERMENULOOP {
        crate::tray::menu_opened(&app);
    }
    // SAFETY: retain the original native menu/window behavior and return value.
    let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };
    if message == WM_EXITMENULOOP {
        // Reconcile after the native menu returns and dispatches its selected
        // command. No timeout: the next main-loop turn observes actual focus.
        std::thread::spawn(move || crate::tray::menu_closed(&app));
    }
    result
}

pub fn observe_menu(app: &AppHandle, tray: &tauri::tray::TrayIcon) -> Result<(), String> {
    let app = app.clone();
    tray.with_inner_tray_icon(move |inner| {
        let window = inner.window_handle() as HWND;
        let mut pid = 0;
        // SAFETY: the tray supplies its own live HWND; pid is initialized.
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        if pid != std::process::id() {
            return Err("Unexpected tray window owner".into());
        }
        let context = Box::into_raw(Box::new(app));
        // SAFETY: only this app's tray is subclassed on its owning main thread.
        if unsafe { SetWindowSubclass(window, Some(menu_events), SUBCLASS_ID, context as usize) }
            == 0
        {
            // SAFETY: installation failed, so the native window never owned it.
            unsafe {
                drop(Box::from_raw(context));
            }
            return Err("Cannot observe the native tray menu lifecycle".into());
        }
        Ok(())
    })
    .map_err(|error| error.to_string())?
}
