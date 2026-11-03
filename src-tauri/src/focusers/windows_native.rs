//! Focus a window belonging to the process instance observed by the scanner.

#[cfg(any(windows, test))]
fn decimal(value: &str, max_digits: usize) -> bool {
    !value.is_empty() && value.len() <= max_digits && value.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(any(windows, test))]
#[derive(Debug, PartialEq, Eq)]
struct Target {
    pid: u32,
    creation: u64,
    hwnd: usize,
    thread: u32,
}

#[cfg(any(windows, test))]
fn parse_target(focus_id: &str, outer_id: &str) -> Result<Target, String> {
    let pid = decimal(focus_id, 10)
        .then(|| focus_id.parse::<u32>().ok())
        .flatten()
        .filter(|&pid| pid > 0)
        .ok_or("Invalid Windows focus PID")?;
    // Legacy status files contain a PID without a process lifetime or window
    // identity. A recycled PID cannot safely be resolved from those records.
    let error = "Missing or invalid Windows window identity";
    let (creation, rest) = outer_id.split_once(':').ok_or(error)?;
    let (hwnd, thread) = rest.split_once(':').ok_or(error)?;
    if !decimal(creation, 19) || !decimal(hwnd, 20) || !decimal(thread, 10) {
        return Err(error.into());
    }
    let creation = creation.parse::<i64>().map_err(|_| error)?;
    let hwnd = hwnd.parse::<usize>().map_err(|_| error)?;
    let thread = thread.parse::<u32>().map_err(|_| error)?;
    if creation <= 0 || hwnd == 0 || thread == 0 {
        return Err(error.into());
    }
    Ok(Target {
        pid,
        creation: creation as u64,
        hwnd,
        thread,
    })
}

#[cfg(windows)]
pub fn focus(focus_id: &str, outer_id: &str) -> Result<(), String> {
    native::focus(parse_target(focus_id, outer_id)?)
}

#[cfg(not(windows))]
pub fn focus(_focus_id: &str, _outer_id: &str) -> Result<(), String> {
    Err("Windows focus is unavailable on this platform".into())
}

#[cfg(windows)]
pub(crate) use native::{observe_window, ObservedWindow};

#[cfg(windows)]
mod native {
    use super::Target;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HWND, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetForegroundWindow, GetWindow, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindowAsync,
        GA_ROOT, GW_OWNER, SW_RESTORE,
    };

    struct Process(HANDLE);

    impl Process {
        fn open(pid: u32) -> Result<Self, String> {
            // SAFETY: the handle is non-inheritable and uniquely owned here.
            let handle = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    pid,
                )
            };
            if handle.is_null() {
                return Err("Windows focus process is unavailable".into());
            }
            Ok(Self(handle))
        }

        fn creation(&self) -> Result<u64, String> {
            let [mut creation, mut exit, mut kernel, mut user] = [FILETIME::default(); 4];
            // SAFETY: the process handle and the four output buffers are live.
            if unsafe { GetProcessTimes(self.0, &mut creation, &mut exit, &mut kernel, &mut user) }
                == 0
            {
                return Err("Cannot verify Windows focus process identity".into());
            }
            Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
        }

        fn alive(&self) -> bool {
            // SAFETY: the process handle stays open throughout window selection.
            unsafe { WaitForSingleObject(self.0, 0) == WAIT_TIMEOUT }
        }
    }

    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: the process owns this kernel handle.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn owner_thread(pid: u32, window: HWND) -> Option<u32> {
        let mut owner = 0;
        // SAFETY: HWND is an opaque OS value; owner is an initialized output.
        unsafe {
            let thread = GetWindowThreadProcessId(window, &mut owner);
            (thread != 0
                && owner == pid
                && GetAncestor(window, GA_ROOT) == window
                && GetWindow(window, GW_OWNER).is_null()
                && IsWindowVisible(window) != 0)
                .then_some(thread)
        }
    }

    pub(crate) struct ObservedWindow {
        pub identity: String,
        pub title: Option<String>,
    }

    pub(crate) fn observe_window(pid: u32, sampled_creation: u64) -> Option<ObservedWindow> {
        let process = Process::open(pid).ok()?;
        let creation = process.creation().ok()?;
        // CIM timestamps carry microseconds; FILETIME carries 100ns ticks.
        // Capture the exact FILETIME for later comparisons.
        if creation / 10 != sampled_creation / 10 || !process.alive() {
            return None;
        }
        struct Search {
            pid: u32,
            window: HWND,
            thread: u32,
        }
        unsafe extern "system" fn visit(window: HWND, data: isize) -> i32 {
            // SAFETY: EnumWindows calls synchronously with our live Search pointer.
            let search = unsafe { &mut *(data as *mut Search) };
            if let Some(thread) = owner_thread(search.pid, window) {
                if !search.window.is_null() {
                    return 0;
                }
                search.window = window;
                search.thread = thread;
            }
            1
        }
        let mut search = Search {
            pid,
            window: null_mut(),
            thread: 0,
        };
        // SAFETY: the callback does not escape the call or retain its context.
        let enumerated = unsafe { EnumWindows(Some(visit), (&mut search as *mut Search) as isize) };
        if enumerated == 0
            || !process.alive()
            || owner_thread(pid, search.window) != Some(search.thread)
        {
            return None;
        }
        let mut text = [0u16; 512];
        // SAFETY: this is a bounded writable buffer and a verified HWND.
        let len = unsafe { GetWindowTextW(search.window, text.as_mut_ptr(), text.len() as i32) };
        Some(ObservedWindow {
            identity: format!("{creation}:{}:{}", search.window as usize, search.thread),
            title: (len > 0).then(|| String::from_utf16_lossy(&text[..len as usize])),
        })
    }

    pub(super) fn focus(target: Target) -> Result<(), String> {
        let process = Process::open(target.pid)?;
        if process.creation()? != target.creation || !process.alive() {
            return Err("Windows focus process has expired".into());
        }
        let window = target.hwnd as HWND;
        let still_owned =
            || process.alive() && owner_thread(target.pid, window) == Some(target.thread);
        if !still_owned() {
            return Err("Windows focus window has expired".into());
        }
        // SAFETY: the process and observed window identity were rechecked.
        if unsafe { IsIconic(window) != 0 && ShowWindowAsync(window, SW_RESTORE) == 0 } {
            return Err("Could not restore Windows focus window".into());
        }
        if !still_owned() {
            return Err("Windows focus window has expired".into());
        }
        // SAFETY: ownership was rechecked immediately before activation.
        let focused =
            unsafe { GetForegroundWindow() == window || SetForegroundWindow(window) != 0 };
        if !focused || !still_owned() {
            return Err("Could not activate Windows focus window".into());
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WS_EX_NOACTIVATE, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
        };

        struct Window(HWND);
        impl Window {
            fn new() -> Self {
                let name: Vec<u16> = "Static\0".encode_utf16().collect();
                // SAFETY: a test-owned top-level static window, with no activation.
                let window = unsafe {
                    CreateWindowExW(
                        WS_EX_NOACTIVATE,
                        name.as_ptr(),
                        name.as_ptr(),
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                        0,
                        0,
                        20,
                        20,
                        null_mut(),
                        null_mut(),
                        null_mut(),
                        null_mut(),
                    )
                };
                assert!(!window.is_null());
                Self(window)
            }
        }
        impl Drop for Window {
            fn drop(&mut self) {
                // SAFETY: this test destroys its own window on the creating thread.
                unsafe { DestroyWindow(self.0) };
            }
        }

        #[test]
        fn rejects_stale_process_and_window_even_after_same_process_replaces_it() {
            let pid = std::process::id();
            let creation = Process::open(pid).unwrap().creation().unwrap();
            let first = Window::new();
            let observation = observe_window(pid, creation).expect("first owned window");
            let mut target =
                super::super::parse_target(&pid.to_string(), &observation.identity).unwrap();
            assert_eq!(target.hwnd, first.0 as usize);
            assert!(owner_thread(pid.wrapping_add(1), first.0).is_none());
            target.creation += 1;
            assert_eq!(
                focus(target).unwrap_err(),
                "Windows focus process has expired"
            );
            let second = Window::new();
            assert!(observe_window(pid, creation).is_none(), "ambiguous windows");
            drop(first);
            assert!(
                observe_window(pid, creation).is_some(),
                "replacement window exists"
            );
            assert_eq!(
                super::super::focus(&pid.to_string(), &observation.identity).unwrap_err(),
                "Windows focus window has expired",
            );
            drop(second);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_decimal_zero_overflow_and_legacy_targets() {
        for value in [
            "",
            "0",
            " 42",
            "+42",
            "42\n",
            "42;fixture",
            "１",
            "4294967296",
        ] {
            assert!(parse_target(value, "10000:12:34").is_err(), "{value:?}");
        }
        for value in [
            "",
            "0",
            "1:2",
            "+1:12:34",
            "9223372036854775808:12:34",
            "1:0:34",
            "1:12:0",
            "1:12:34:56",
        ] {
            assert!(parse_target("42", value).is_err(), "{value:?}");
        }
        assert_eq!(
            parse_target("42", "10000:12:34"),
            Ok(Target {
                pid: 42,
                creation: 10000,
                hwnd: 12,
                thread: 34
            })
        );
    }
}
