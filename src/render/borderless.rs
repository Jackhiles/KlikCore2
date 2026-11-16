//! Borderless fullscreen (Windows): the game window without title bar or edges, covering
//! the monitor it is on (no display mode change, so switching away is instant). Leaving it
//! puts the window back exactly where and how big it was. (miniquad's own fullscreen
//! always covers the primary monitor and puts the window at the screen's corner after.)

/// Switch the game window in or out of borderless fullscreen. Returns false when this
/// isn't available (other systems, or no window found), so the caller can fall back.
pub fn set_fullscreen(on: bool) -> bool {
    #[cfg(windows)]
    return win::set_fullscreen(on);
    #[cfg(not(windows))]
    {
        let _ = on;
        false
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::sync::Mutex;

    type Hwnd = *mut c_void;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct MonitorInfo {
        size: u32,
        monitor: Rect,
        work: Rect,
        flags: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn EnumThreadWindows(thread: u32, f: extern "system" fn(Hwnd, isize) -> i32, data: isize) -> i32;
        fn GetClassNameW(wnd: Hwnd, name: *mut u16, len: i32) -> i32;
        fn GetWindowLongPtrW(wnd: Hwnd, index: i32) -> isize;
        fn SetWindowLongPtrW(wnd: Hwnd, index: i32, value: isize) -> isize;
        fn GetWindowRect(wnd: Hwnd, rect: *mut Rect) -> i32;
        fn SetWindowPos(wnd: Hwnd, after: Hwnd, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
        fn MonitorFromWindow(wnd: Hwnd, flags: u32) -> *mut c_void;
        fn GetMonitorInfoW(monitor: *mut c_void, info: *mut MonitorInfo) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    const GWL_STYLE: i32 = -16;
    const WS_POPUP: u32 = 0x8000_0000;
    const WS_VISIBLE: u32 = 0x1000_0000;
    const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
    const WS_CLIPCHILDREN: u32 = 0x0200_0000;
    const WS_SYSMENU: u32 = 0x0008_0000;
    const WS_MINIMIZEBOX: u32 = 0x0002_0000;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_FRAMECHANGED: u32 = 0x0020;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const MONITOR_DEFAULTTONEAREST: u32 = 2;

    /// The window's style and position before fullscreen.
    static SAVED: Mutex<Option<(isize, Rect)>> = Mutex::new(None);

    /// The miniquad window on this thread (the launcher, a separate process, has its own).
    fn game_window() -> Option<Hwnd> {
        extern "system" fn each(wnd: Hwnd, out: isize) -> i32 {
            let mut name = [0u16; 32];
            // SAFETY: `name` holds 32 UTF-16 units; `out` points at the caller's Option.
            let n = unsafe { GetClassNameW(wnd, name.as_mut_ptr(), name.len() as i32) };
            if String::from_utf16_lossy(&name[..n.max(0) as usize]) == "MINIQUADAPP" {
                unsafe { *(out as *mut Option<Hwnd>) = Some(wnd) };
                return 0;
            }
            1
        }
        let mut found: Option<Hwnd> = None;
        // SAFETY: the callback only writes through the pointer to `found`, which outlives the call.
        unsafe { EnumThreadWindows(GetCurrentThreadId(), each, &mut found as *mut _ as isize) };
        found
    }

    pub fn set_fullscreen(on: bool) -> bool {
        let Some(wnd) = game_window() else { return false };
        let mut saved = SAVED.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: plain Win32 calls on our own window with valid arguments.
        unsafe {
            if on {
                if saved.is_none() {
                    let mut r = Rect::default();
                    GetWindowRect(wnd, &mut r);
                    *saved = Some((GetWindowLongPtrW(wnd, GWL_STYLE), r));
                }
                let mut info = MonitorInfo { size: std::mem::size_of::<MonitorInfo>() as u32, ..Default::default() };
                if GetMonitorInfoW(MonitorFromWindow(wnd, MONITOR_DEFAULTTONEAREST), &mut info) == 0 {
                    return false;
                }
                let m = info.monitor;
                let style = WS_POPUP | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN | WS_SYSMENU | WS_MINIMIZEBOX;
                SetWindowLongPtrW(wnd, GWL_STYLE, style as isize);
                SetWindowPos(wnd, std::ptr::null_mut(), m.left, m.top, m.right - m.left, m.bottom - m.top, SWP_NOZORDER | SWP_FRAMECHANGED | SWP_SHOWWINDOW);
            } else {
                let Some((style, r)) = saved.take() else { return false };
                SetWindowLongPtrW(wnd, GWL_STYLE, style);
                SetWindowPos(wnd, std::ptr::null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_FRAMECHANGED | SWP_SHOWWINDOW);
            }
        }
        true
    }
}
