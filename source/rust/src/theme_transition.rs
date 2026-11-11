//! A short composited crossfade hides sequential Win32 child-control repaints.
use crate::platform::wide;
use std::{ptr, time::Instant};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::DwmFlush, Gdi::*},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};
struct Frame {
    bitmap: HBITMAP,
    width: i32,
    height: i32,
    started: Instant,
}
const CLASS: &str = "AirBridge.ThemeTransition";
pub fn capture() -> HWND {
    unsafe {
        let mut bounds: Option<RECT> = None;
        let mut owner = ptr::null_mut();
        for class in ["AirBridge.Rust.Receiver", "AirBridge.Rust.Settings"] {
            let hwnd = FindWindowW(wide(class).as_ptr(), ptr::null());
            if !hwnd.is_null() && IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 {
                let mut r: RECT = std::mem::zeroed();
                GetWindowRect(hwnd, &mut r);
                owner = hwnd;
                bounds = Some(match bounds {
                    None => r,
                    Some(b) => RECT {
                        left: b.left.min(r.left),
                        top: b.top.min(r.top),
                        right: b.right.max(r.right),
                        bottom: b.bottom.max(r.bottom),
                    },
                });
            }
        }
        let Some(mut r) = bounds else {
            return ptr::null_mut();
        };
        r.left -= 12;
        r.top -= 12;
        r.right += 12;
        r.bottom += 12;
        let w = r.right - r.left;
        let h = r.bottom - r.top;
        if w <= 0 || h <= 0 || i64::from(w) * i64::from(h) > 32_000_000 {
            return ptr::null_mut();
        }
        let screen = GetDC(ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        if dc.is_null() || bitmap.is_null() {
            if !dc.is_null() {
                DeleteDC(dc);
            }
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            ReleaseDC(ptr::null_mut(), screen);
            return ptr::null_mut();
        }
        let old = SelectObject(dc, bitmap);
        let copied = BitBlt(dc, 0, 0, w, h, screen, r.left, r.top, SRCCOPY | CAPTUREBLT);
        SelectObject(dc, old);
        DeleteDC(dc);
        ReleaseDC(ptr::null_mut(), screen);
        if copied == 0 {
            DeleteObject(bitmap);
            return ptr::null_mut();
        }
        let previous = FindWindowW(wide(CLASS).as_ptr(), ptr::null());
        if !previous.is_null() {
            DestroyWindow(previous);
        }
        let class = wide(CLASS);
        let instance = GetModuleHandleW(ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let frame = Box::into_raw(Box::new(Frame {
            bitmap,
            width: w,
            height: h,
            started: Instant::now(),
        }));
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class.as_ptr(),
            wide("").as_ptr(),
            WS_POPUP,
            r.left,
            r.top,
            w,
            h,
            owner,
            ptr::null_mut(),
            instance,
            frame.cast(),
        );
        if hwnd.is_null() {
            let frame = Box::from_raw(frame);
            DeleteObject(frame.bitmap);
            return hwnd;
        }
        SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        UpdateWindow(hwnd);
        DwmFlush();
        hwnd
    }
}
pub fn reveal(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    unsafe {
        DwmFlush();
        let frame = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Frame;
        if !frame.is_null() {
            (*frame).started = Instant::now();
            SetTimer(hwnd, 1, 16, None);
        }
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lp as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            return 1;
        }
        let frame = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Frame;
        match msg {
            WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
            WM_NCHITTEST => HTTRANSPARENT as isize,
            WM_ERASEBKGND => 1,
            WM_PAINT if !frame.is_null() => {
                let mut ps = std::mem::zeroed();
                let target = BeginPaint(hwnd, &mut ps);
                let dc = CreateCompatibleDC(target);
                let old = SelectObject(dc, (*frame).bitmap);
                BitBlt(
                    target,
                    0,
                    0,
                    (*frame).width,
                    (*frame).height,
                    dc,
                    0,
                    0,
                    SRCCOPY,
                );
                SelectObject(dc, old);
                DeleteDC(dc);
                EndPaint(hwnd, &ps);
                0
            }
            WM_TIMER if !frame.is_null() => {
                let t = ((*frame).started.elapsed().as_secs_f32() / 0.18).min(1.0);
                if t >= 1.0 {
                    DestroyWindow(hwnd);
                } else {
                    let ease = t * t * (3.0 - 2.0 * t);
                    SetLayeredWindowAttributes(
                        hwnd,
                        0,
                        ((1.0 - ease) * 255.0).round() as u8,
                        LWA_ALPHA,
                    );
                }
                0
            }
            WM_NCDESTROY => {
                if !frame.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let frame = Box::from_raw(frame);
                    DeleteObject(frame.bitmap);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
