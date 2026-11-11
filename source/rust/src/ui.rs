use crate::{
    i18n::{self, tr},
    media::{Model, lock},
    platform::wide,
    settings::Settings,
    supervisor::{self, Action},
    ui_style as style,
};
use std::{
    io, ptr,
    sync::{Arc, Mutex, OnceLock, mpsc},
    thread,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::*, Shell::*, WindowsAndMessaging::*},
};

const SHOW: usize = 101;
const RECONNECT: usize = 102;
const LOGS: usize = 103;
const EXIT: usize = 104;
const FULLSCREEN: usize = 105;
const ROTATE: usize = 106;
const SETTINGS: usize = 107;
const SAVE: usize = 108;
const FIREWALL: usize = 110;
const SHARP_SCALING: usize = 111;
const SMOOTH_SCALING: usize = 112;
const UNFILTERED_SCALING: usize = 113;
const VIEW_MENU: usize = 114;
const MORE_MENU: usize = 115;
static CONTROL_FONT: OnceLock<usize> = OnceLock::new();
const THEME_SYSTEM: usize = 213;
const THEME_DARK: usize = 215;
const CLOSE_SETTINGS: usize = 216;
static RECEIVER_MENU: Mutex<usize> = Mutex::new(0);
const LANGUAGE: usize = 217;
const LANGUAGE_CHOICE: usize = 400;
const LANGUAGE_CHANGED: u32 = WM_APP + 4;
static RECEIVER_WINDOW: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(ptr::null_mut());

// Invalidate on frame arrival, rather than limiting presentation to a 16 ms timer.
pub fn notify_video_frame() {
    let hwnd = RECEIVER_WINDOW.load(std::sync::atomic::Ordering::Acquire);
    if !hwnd.is_null() {
        unsafe {
            InvalidateRect(hwnd, ptr::null(), 0);
        }
    }
}
const TRAY: u32 = WM_APP + 3;
const PROFILE: usize = 200;
const AUTO_RESOLUTION: usize = 212;
static RECEIVER: OnceLock<Arc<Mutex<Model>>> = OnceLock::new();
static CONFIG: OnceLock<Mutex<Settings>> = OnceLock::new();
static WINDOW_STATE: OnceLock<Mutex<Settings>> = OnceLock::new();
static ACTIONS: OnceLock<mpsc::Sender<Action>> = OnceLock::new();
static RECREATE_TRAY: OnceLock<u32> = OnceLock::new();
static FULL_RECT: Mutex<Option<RECT>> = Mutex::new(None);
type DisplaySignature = (u64, u64, u32, bool, bool, String);
static LAST_PAINT: Mutex<Option<(DisplaySignature, std::time::Instant)>> = Mutex::new(None);
type RotatedPicture = (u64, u64, u32, Arc<crate::media::Picture>);
static ROTATED: Mutex<Option<RotatedPicture>> = Mutex::new(None);

unsafe fn add_menu(menu: HMENU, id: usize, name: &str) {
    unsafe {
        AppendMenuW(menu, MF_STRING, id, wide(name).as_ptr());
    }
}
fn tray(hwnd: HWND, op: u32, message: Option<&str>) {
    unsafe {
        let mut data: NOTIFYICONDATAW = std::mem::zeroed();
        data.cbSize = std::mem::size_of_val(&data) as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = TRAY;
        data.hIcon = crate::icon::handle();
        let tip = wide("AirBridge");
        data.szTip[..tip.len()].copy_from_slice(&tip);
        if let Some(message) = message {
            data.uFlags |= NIF_INFO;
            data.dwInfoFlags = NIIF_WARNING;
            let title = wide("AirBridge");
            data.szInfoTitle[..title.len()].copy_from_slice(&title);
            let text = wide(message);
            let n = text.len().min(data.szInfo.len() - 1);
            data.szInfo[..n].copy_from_slice(&text[..n]);
        }
        Shell_NotifyIconW(op, &data);
    }
}
pub fn shell(root: std::path::PathBuf) -> io::Result<()> {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let mut settings =
            Settings::load(std::path::PathBuf::from(appdata).join("AirPlay-Windows/settings.ini"))?;
        i18n::initialize(&mut settings);
    }
    let (tx, rx) = mpsc::channel();
    ACTIONS.set(tx).ok();
    let hwnd = create(
        "AirBridge.Rust.Supervisor",
        "AirBridge",
        false,
        Some(shell_proc),
    )?;
    RECREATE_TRAY
        .set(unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) })
        .ok();
    tray(hwnd, NIM_ADD, None);
    let raw = hwnd as usize;
    let worker = thread::spawn(move || supervisor::worker(root, rx, raw));
    message_loop();
    if let Some(tx) = ACTIONS.get() {
        let _ = tx.send(Action::Exit);
    }
    let _ = worker.join();
    tray(hwnd, NIM_DELETE, None);
    Ok(())
}
unsafe extern "system" fn shell_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if RECREATE_TRAY.get().is_some_and(|id| *id == msg) {
        tray(hwnd, NIM_ADD, None);
        return 0;
    }
    match msg {
        LANGUAGE_CHANGED => {
            i18n::set(wp);
            0
        }
        TRAY if lp as u32 == WM_LBUTTONDBLCLK => {
            crate::platform::focus();
            0
        }
        TRAY if lp as u32 == WM_RBUTTONUP || lp as u32 == WM_CONTEXTMENU => {
            unsafe {
                let menu = CreatePopupMenu();
                add_menu(menu, SHOW, tr("open_receiver"));
                add_menu(menu, RECONNECT, tr("restart_connection"));
                add_menu(menu, LOGS, tr("open_logs"));
                add_menu(menu, EXIT, tr("exit"));
                let mut p = POINT { x: 0, y: 0 };
                GetCursorPos(&mut p);
                SetForegroundWindow(hwnd);
                let choice = TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    p.x,
                    p.y,
                    0,
                    hwnd,
                    ptr::null(),
                ) as usize;
                DestroyMenu(menu);
                PostMessageW(hwnd, WM_COMMAND, choice, 0);
            }
            0
        }
        WM_COMMAND => {
            match wp & 0xffff {
                SHOW => crate::platform::focus(),
                RECONNECT => {
                    if let Some(tx) = ACTIONS.get() {
                        let _ = tx.send(Action::Restart);
                    }
                }
                LOGS => open_logs(),
                EXIT => {
                    if let Some(tx) = ACTIONS.get() {
                        let _ = tx.send(Action::Exit);
                    }
                }
                _ => {}
            }
            0
        }
        supervisor::FAILED => {
            tray(hwnd, NIM_MODIFY, Some(tr("receiver_stopped")));
            0
        }
        supervisor::STOPPED => {
            unsafe {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_CLOSE | WM_QUERYENDSESSION => {
            if let Some(tx) = ACTIONS.get() {
                let _ = tx.send(Action::Exit);
            }
            if msg == WM_QUERYENDSESSION { 1 } else { 0 }
        }
        WM_DESTROY => {
            tray(hwnd, NIM_DELETE, None);
            unsafe {
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

pub fn receiver(model: Arc<Mutex<Model>>, settings: Settings) -> io::Result<i32> {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        match Settings::load(std::path::PathBuf::from(local).join("AirBridge/window-state.ini")) {
            Ok(state) => {
                WINDOW_STATE.set(Mutex::new(state)).ok();
            }
            Err(error) => log::warn!("cannot load window placement: {error}"),
        }
    }
    style::set_theme(settings.theme_mode());
    RECEIVER.set(model).ok();
    CONFIG.set(Mutex::new(settings)).ok();
    let hwnd = create(
        "AirBridge.Rust.Receiver",
        "AirBridge",
        false,
        Some(receiver_proc),
    )?;
    unsafe {
        style::titlebar(hwnd);
        rebuild_receiver_menu();
        for (id, label) in [
            (VIEW_MENU, tr("view")),
            (SETTINGS, tr("settings")),
            (MORE_MENU, "•••"),
        ] {
            control(hwnd, "BUTTON", label, BS_OWNERDRAW as u32, 0, 0, 80, 38, id);
        }
        let maximized = restore_window(hwnd, "main", true);
        layout_toolbar(hwnd);
        RECEIVER_WINDOW.store(hwnd, std::sync::atomic::Ordering::Release);
        SetTimer(hwnd, 1, 16, None);
        ShowWindow(hwnd, if maximized { SW_SHOWMAXIMIZED } else { SW_SHOW });
    }
    Ok(message_loop())
}
fn rebuild_receiver_menu() {
    unsafe {
        let bar = CreateMenu();
        let session = CreatePopupMenu();
        let display = CreatePopupMenu();
        add_menu(session, RECONNECT, tr("reconnect"));
        add_menu(session, FIREWALL, tr("allow_network"));
        add_menu(session, LOGS, tr("open_logs"));
        add_menu(session, EXIT, tr("exit"));
        add_menu(display, FULLSCREEN, tr("fullscreen"));
        add_menu(display, ROTATE, tr("rotate"));
        add_menu(display, SHARP_SCALING, tr("scaling_sharp"));
        add_menu(display, SMOOTH_SCALING, tr("scaling_smooth"));
        add_menu(display, UNFILTERED_SCALING, tr("scaling_unfiltered"));
        let unfiltered = RECEIVER.get().is_some_and(|m| lock(m).unfiltered_scaling);
        let smooth = RECEIVER.get().is_some_and(|m| lock(m).smooth_scaling);
        CheckMenuRadioItem(
            display,
            SHARP_SCALING as u32,
            UNFILTERED_SCALING as u32,
            if unfiltered {
                UNFILTERED_SCALING
            } else if smooth {
                SMOOTH_SCALING
            } else {
                SHARP_SCALING
            } as u32,
            MF_BYCOMMAND,
        );
        AppendMenuW(
            bar,
            MF_POPUP,
            session as usize,
            wide(tr("open_receiver")).as_ptr(),
        );
        AppendMenuW(bar, MF_POPUP, display as usize, wide(tr("view")).as_ptr());
        let old = std::mem::replace(&mut *lock(&RECEIVER_MENU), bar as usize);
        if old != 0 {
            DestroyMenu(old as HMENU);
        }
    }
}
unsafe extern "system" fn receiver_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_EXITSIZEMOVE => {
            remember_window(hwnd, "main");
            0
        }
        WM_SETTINGCHANGE => {
            if style::system_theme_changed() {
                apply_theme(0);
            }
            0
        }
        WM_SIZE => {
            layout_toolbar(hwnd);
            unsafe {
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_GETMINMAXINFO => {
            unsafe {
                let info = &mut *(lp as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT { x: 680, y: 700 };
            }
            0
        }
        WM_DRAWITEM => {
            unsafe {
                style::button(&*(lp as *const DRAWITEMSTRUCT));
            }
            1
        }
        WM_COMMAND => {
            match wp & 0xffff {
                VIEW_MENU | MORE_MENU => unsafe {
                    let bar = *lock(&RECEIVER_MENU) as HMENU;
                    let menu = GetSubMenu(bar, if wp & 0xffff == VIEW_MENU { 1 } else { 0 });
                    let button = GetDlgItem(hwnd, (wp & 0xffff) as i32);
                    let mut r = std::mem::zeroed();
                    GetWindowRect(button, &mut r);
                    let choice = TrackPopupMenu(
                        menu,
                        TPM_RETURNCMD | TPM_RIGHTALIGN,
                        r.right,
                        r.bottom + 6,
                        0,
                        hwnd,
                        ptr::null(),
                    ) as usize;
                    if choice != 0 {
                        PostMessageW(hwnd, WM_COMMAND, choice, 0);
                    }
                },
                RECONNECT => unsafe {
                    DestroyWindow(hwnd);
                    PostQuitMessage(2);
                },
                EXIT => unsafe {
                    DestroyWindow(hwnd);
                },
                LOGS => open_logs(),
                FULLSCREEN => toggle_fullscreen(hwnd),
                SHARP_SCALING | SMOOTH_SCALING | UNFILTERED_SCALING => {
                    let unfiltered = wp & 0xffff == UNFILTERED_SCALING;
                    let smooth = wp & 0xffff == SMOOTH_SCALING;
                    if let Some(model) = RECEIVER.get() {
                        let mut model = lock(model);
                        model.smooth_scaling = smooth;
                        model.unfiltered_scaling = unfiltered;
                    }
                    if let Some(config) = CONFIG.get() {
                        let mut settings = lock(config);
                        settings.set("mirror_smooth_scaling", u8::from(smooth));
                        settings.set("mirror_unfiltered_scaling", u8::from(unfiltered));
                        if let Err(error) = settings.save() {
                            log::warn!("cannot save display scaling: {error}");
                        }
                    }
                    unsafe {
                        CheckMenuRadioItem(
                            GetSubMenu(*lock(&RECEIVER_MENU) as HMENU, 1),
                            SHARP_SCALING as u32,
                            UNFILTERED_SCALING as u32,
                            if unfiltered {
                                UNFILTERED_SCALING
                            } else if smooth {
                                SMOOTH_SCALING
                            } else {
                                SHARP_SCALING
                            } as u32,
                            MF_BYCOMMAND,
                        );
                    }
                }
                ROTATE => {
                    if let Some(m) = RECEIVER.get() {
                        let mut m = lock(m);
                        m.rotation = (m.rotation + 90) % 360;
                    }
                }
                SETTINGS => {
                    let _ = settings_window(hwnd);
                }
                FIREWALL => allow_firewall(hwnd),
                _ => {}
            }
            0
        }
        WM_KEYDOWN => {
            if wp == 0x7a {
                toggle_fullscreen(hwnd);
            }
            if wp == 0x1b && lock(&FULL_RECT).is_some() {
                toggle_fullscreen(hwnd);
            }
            if wp == b'R' as usize
                && unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x11) }
                    >= 0
                && let Some(m) = RECEIVER.get()
            {
                let mut m = lock(m);
                m.rotation = (m.rotation + 90) % 360;
            }
            0
        }
        WM_TIMER => {
            if crate::RECEIVER_FAILED.load(std::sync::atomic::Ordering::Acquire) {
                unsafe {
                    DestroyWindow(hwnd);
                    PostQuitMessage(1);
                }
            } else {
                if unsafe { IsIconic(hwnd) } == 0
                    && let Some(model) = RECEIVER.get()
                {
                    let mut m = lock(model);
                    let mut area: RECT = unsafe { std::mem::zeroed() };
                    unsafe {
                        GetClientRect(hwnd, &mut area);
                    }
                    let size = (
                        area.right.max(1) as u32,
                        (area.bottom - chrome_height()).max(1) as u32,
                    );
                    m.display_size = if m.rotation.is_multiple_of(180) {
                        size
                    } else {
                        (size.1, size.0)
                    };
                    let signature = (
                        m.frames,
                        m.generation,
                        m.rotation,
                        m.picture.is_some(),
                        m.recorder.as_ref().is_some_and(|r| r.active()),
                        m.metadata.clone(),
                    );
                    let mut last = lock(&LAST_PAINT);
                    if last.as_ref().is_none_or(|(old, time)| {
                        *old != signature || time.elapsed() >= std::time::Duration::from_secs(1)
                    }) {
                        *last = Some((signature, std::time::Instant::now()));
                        unsafe {
                            InvalidateRect(hwnd, ptr::null(), 0);
                        }
                    }
                }
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(hwnd, None);
            0
        }
        WM_PRINTCLIENT => {
            paint(hwnd, Some(wp as HDC));
            0
        }
        WM_CLOSE => {
            unsafe {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
            remember_window(hwnd, "main");
            RECEIVER_WINDOW.store(ptr::null_mut(), std::sync::atomic::Ordering::Release);
            unsafe {
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}
fn paint(hwnd: HWND, provided: Option<HDC>) {
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let target = provided.unwrap_or_else(|| BeginPaint(hwnd, &mut ps));
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let dc = CreateCompatibleDC(target);
        let bitmap = CreateCompatibleBitmap(target, rect.right.max(1), rect.bottom.max(1));
        if dc.is_null() || bitmap.is_null() {
            if !dc.is_null() {
                DeleteDC(dc);
            }
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if provided.is_none() {
                EndPaint(hwnd, &ps);
            }
            return;
        }
        let previous = SelectObject(dc, bitmap);
        SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT));
        let brush = CreateSolidBrush(style::bg());
        FillRect(dc, &rect, brush);
        DeleteObject(brush);
        let width = rect.right.max(1);
        let height = rect.bottom.max(1);
        let fullscreen = lock(&FULL_RECT).is_some();
        let top = if fullscreen { 0 } else { 78 };
        let bottom = 0;
        if let Some(model) = RECEIVER.get() {
            // Copy only the Arc while locked; decoding and GDI never hold each other's locks.
            let (picture, frames, audio, metadata, rotation) = {
                let m = lock(model);
                (
                    m.picture.clone().or_else(|| m.artwork.clone()),
                    m.frames,
                    m.audio_frames,
                    m.metadata.clone(),
                    m.rotation,
                )
            };
            if let Some(p) = picture {
                style::fill(
                    dc,
                    style::area(0, top, width, height - top - bottom),
                    style::rgb(12, 12, 14),
                );
                let p = if rotation == 0 {
                    p
                } else {
                    let mut cache = lock(&ROTATED);
                    let generation = lock(model).generation;
                    if cache.as_ref().is_none_or(|(f, g, r, _)| {
                        *f != frames || *g != generation || *r != rotation
                    }) {
                        *cache =
                            Some((frames, generation, rotation, Arc::new(rotate(&p, rotation))));
                    }
                    cache.as_ref().unwrap().3.clone()
                };
                let scale = (width as f64 / p.width as f64)
                    .min((height - top - bottom).max(1) as f64 / p.height as f64);
                let dw = (p.width as f64 * scale).round() as i32;
                let dh = (p.height as f64 * scale).round() as i32;
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = p.width as i32;
                bmi.bmiHeader.biHeight = -(p.height as i32);
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 32;
                bmi.bmiHeader.biCompression = BI_RGB;
                SetStretchBltMode(
                    dc,
                    if RECEIVER.get().is_some_and(|m| lock(m).unfiltered_scaling) {
                        COLORONCOLOR
                    } else {
                        HALFTONE
                    },
                );
                SetBrushOrgEx(dc, 0, 0, ptr::null_mut());
                StretchDIBits(
                    dc,
                    (width - dw) / 2,
                    top + ((height - top - bottom - dh) / 2).max(0),
                    dw,
                    dh,
                    0,
                    0,
                    p.width as i32,
                    p.height as i32,
                    p.pixels.as_ptr().cast(),
                    &bmi,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
            } else {
                let name = CONFIG
                    .get()
                    .map(|c| lock(c).name())
                    .unwrap_or_else(|| "AirBridge".into());
                style::landing(dc, width, height, &name, audio > 0);
            }
            if !fullscreen {
                style::fill(dc, style::area(0, 0, width, 78), style::bg());
                crate::icon::draw(dc, 24, 21, 36);
                style::text(
                    dc,
                    "AirBridge",
                    style::area(69, 14, 250, 30),
                    21,
                    true,
                    style::ink(),
                    DT_LEFT,
                );
                style::round(dc, style::area(70, 49, 7, 7), 4, style::green());
                let label = if frames > 0 {
                    tr("status_video")
                } else if audio > 0 {
                    tr("status_audio")
                } else {
                    tr("status_ready")
                };
                style::text(
                    dc,
                    label,
                    style::area(83, 42, width - 420, 22),
                    12,
                    false,
                    style::secondary(),
                    DT_LEFT,
                );
                let _ = metadata;
            }
        }
        BitBlt(target, 0, 0, rect.right, rect.bottom, dc, 0, 0, SRCCOPY);
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        if provided.is_none() {
            EndPaint(hwnd, &ps);
        }
    }
}
fn rotate(p: &crate::media::Picture, degrees: u32) -> crate::media::Picture {
    let (w, h) = if degrees == 180 {
        (p.width, p.height)
    } else {
        (p.height, p.width)
    };
    let mut pixels = vec![0; p.pixels.len()];
    for y in 0..p.height {
        for x in 0..p.width {
            let (dx, dy) = match degrees {
                90 => (p.height - 1 - y, x),
                180 => (p.width - 1 - x, p.height - 1 - y),
                _ => (y, p.width - 1 - x),
            };
            let src = ((y * p.width + x) * 4) as usize;
            let dst = ((dy * w + dx) * 4) as usize;
            pixels[dst..dst + 4].copy_from_slice(&p.pixels[src..src + 4]);
        }
    }
    crate::media::Picture {
        width: w,
        height: h,
        pixels,
    }
}
fn toggle_fullscreen(hwnd: HWND) {
    if lock(&FULL_RECT).is_none() {
        remember_window(hwnd, "main");
    }
    unsafe {
        let saved = lock(&FULL_RECT).take();
        if let Some(rect) = saved {
            SetWindowLongPtrW(
                hwnd,
                GWL_STYLE,
                (WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_VISIBLE) as isize,
            );
            SetWindowPos(
                hwnd,
                ptr::null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_FRAMECHANGED | SWP_NOZORDER,
            );
        } else {
            let mut rect = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            *lock(&FULL_RECT) = Some(rect);
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of_val(&info) as u32;
            GetMonitorInfoW(monitor, &mut info);
            let r = info.rcMonitor;
            SetWindowLongPtrW(
                hwnd,
                GWL_STYLE,
                (WS_POPUP | WS_CLIPCHILDREN | WS_VISIBLE) as isize,
            );
            SetWindowPos(
                hwnd,
                ptr::null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_FRAMECHANGED | SWP_NOZORDER,
            );
        }
    }
    layout_toolbar(hwnd);
}
fn chrome_height() -> i32 {
    if lock(&FULL_RECT).is_some() { 0 } else { 78 }
}
fn layout_toolbar(hwnd: HWND) {
    unsafe {
        let mut r = std::mem::zeroed();
        GetClientRect(hwnd, &mut r);
        let full = lock(&FULL_RECT).is_some();
        for (id, offset, width) in [
            (VIEW_MENU, 318, 100),
            (SETTINGS, 208, 144),
            (MORE_MENU, 53, 38),
        ] {
            let child = GetDlgItem(hwnd, id as i32);
            ShowWindow(child, if full { SW_HIDE } else { SW_SHOW });
            SetWindowPos(
                child,
                ptr::null_mut(),
                r.right - offset,
                20,
                width,
                38,
                SWP_NOZORDER,
            );
        }
        InvalidateRect(hwnd, ptr::null(), 0);
    }
}
fn settings_window(owner: HWND) -> io::Result<()> {
    let existing = unsafe { FindWindowW(wide("AirBridge.Rust.Settings").as_ptr(), ptr::null()) };
    if !existing.is_null() {
        unsafe {
            SetForegroundWindow(existing);
        }
        return Ok(());
    }
    let hwnd = create("AirBridge.Rust.Settings", "", false, Some(settings_proc))?;
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner as isize);
        SetWindowLongPtrW(
            hwnd,
            GWL_STYLE,
            (WS_POPUP | WS_THICKFRAME | WS_CLIPCHILDREN) as isize,
        );
        let mut owner_rect = std::mem::zeroed();
        GetWindowRect(owner, &mut owner_rect);
        SetWindowPos(
            hwnd,
            ptr::null_mut(),
            owner_rect.left + 100,
            owner_rect.top + 40,
            544,
            794,
            SWP_NOZORDER | SWP_FRAMECHANGED,
        );
        style::titlebar(hwnd);
        restore_window(hwnd, "settings", false);
        let preference = 2u32;
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd,
            33,
            (&preference as *const u32).cast(),
            4,
        );
        let border_color = 0xffff_fffeu32; // DWMWA_COLOR_NONE; keep DWM corners/shadow only.
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd,
            34,
            (&border_color as *const u32).cast(),
            4,
        );
        windows_sys::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea(
            hwnd,
            &MARGINS {
                cxLeftWidth: 1,
                cxRightWidth: 1,
                cyTopHeight: 1,
                cyBottomHeight: 1,
            },
        );
        control(
            hwnd,
            "BUTTON",
            tr("close_settings"),
            BS_OWNERDRAW as u32,
            478,
            24,
            36,
            36,
            CLOSE_SETTINGS,
        );
        SendMessageW(
            hwnd,
            WM_CHANGEUISTATE,
            (UIS_SET | (UISF_HIDEFOCUS << 16)) as usize,
            0,
        );
        let instance = GetModuleHandleW(ptr::null());
        let switch_class = wide("AirBridge.Switch");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(style::switch_proc),
            hInstance: instance,
            lpszClassName: switch_class.as_ptr(),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_HAND),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let cfg = lock(CONFIG.get().unwrap());
        let (manual_width, manual_height) = cfg.manual_resolution();
        for (id, y, checked) in [
            (AUTO_RESOLUTION, 330, cfg.auto_resolution()),
            (210, 542, cfg.flag("mirror_hwaccel", true)),
            (211, 594, cfg.flag("hevc_enabled", true)),
        ] {
            control(hwnd, "AirBridge.Switch", "", 0, 460, y, 54, 35, id);
            SendMessageW(
                GetDlgItem(hwnd, id as i32),
                BM_SETCHECK,
                usize::from(checked),
                0,
            );
        }
        for (index, (value, y)) in [
            (cfg.name(), 130),
            (manual_width.to_string(), 399),
            (manual_height.to_string(), 399),
            (cfg.number("max_fps", 60, 1, 60).to_string(), 452),
        ]
        .iter()
        .enumerate()
        {
            let (x, w) = match index {
                0 => (260, 248),
                1 => (381, 40),
                2 => (457, 40),
                _ => (446, 22),
            };
            control(
                hwnd,
                "EDIT",
                value,
                ES_AUTOHSCROLL as u32
                    | if index > 0 {
                        ES_CENTER as u32
                    } else {
                        ES_RIGHT as u32
                    }
                    | if index > 0 { ES_NUMBER as u32 } else { 0 },
                x,
                *y,
                w,
                22,
                PROFILE + index,
            );
            SetWindowSubclass(
                GetDlgItem(hwnd, (PROFILE + index) as i32),
                Some(edit_proc),
                1,
                PROFILE + index,
            );
            if index > 0 {
                SendMessageW(
                    GetDlgItem(hwnd, (PROFILE + index) as i32),
                    EM_SETMARGINS,
                    (EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize,
                    0,
                );
                SendMessageW(
                    GetDlgItem(hwnd, (PROFILE + index) as i32),
                    EM_SETLIMITTEXT,
                    if index == 3 { 3 } else { 5 },
                    0,
                );
            }
        }
        for (id, x, w) in [(213, 260, 82), (214, 342, 82), (215, 424, 82)] {
            control(hwnd, "BUTTON", "", BS_OWNERDRAW as u32, x, 177, w, 28, id);
        }
        control(
            hwnd,
            "BUTTON",
            i18n::LANGUAGES[i18n::index()].1,
            BS_OWNERDRAW as u32,
            260,
            227,
            248,
            32,
            LANGUAGE,
        );
        update_resolution_controls(hwnd);
        control(
            hwnd,
            "BUTTON",
            tr("save_reconnect"),
            BS_OWNERDRAW as u32,
            24,
            702,
            496,
            46,
            SAVE,
        );
        ShowWindow(hwnd, SW_SHOW);
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)] // Mirrors the Win32 control creation geometry.
unsafe fn control(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: usize,
) {
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | style,
            x,
            y,
            w,
            h,
            parent,
            id as HMENU,
            GetModuleHandleW(ptr::null()),
            ptr::null(),
        );
        if !hwnd.is_null() {
            SendMessageW(
                hwnd,
                WM_SETFONT,
                *CONTROL_FONT.get_or_init(|| style::font(15, false) as usize),
                1,
            );
        }
    }
}
unsafe extern "system" fn settings_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCCALCSIZE => return 0,
        WM_NCPAINT => return 0,
        WM_NCACTIVATE => unsafe {
            // Keep activation bookkeeping without painting the stock resize frame.
            return DefWindowProcW(hwnd, msg, wp, -1);
        },
        WM_EXITSIZEMOVE | WM_DESTROY => {
            remember_window(hwnd, "settings");
            return 0;
        }
        WM_NCHITTEST => unsafe {
            let mut point = POINT {
                x: lp as i16 as i32,
                y: (lp >> 16) as i16 as i32,
            };
            ScreenToClient(hwnd, &mut point);
            return if point.y < 100 && point.x < 470 {
                HTCAPTION as isize
            } else {
                HTCLIENT as isize
            };
        },
        WM_SETTINGCHANGE => {
            if style::system_theme_changed() {
                apply_theme(0);
            }
            return 0;
        }
        WM_ERASEBKGND => return 1,
        WM_PAINT | WM_PRINTCLIENT => {
            paint_settings(
                hwnd,
                if msg == WM_PRINTCLIENT {
                    Some(wp as HDC)
                } else {
                    None
                },
            );
            return 0;
        }
        WM_DRAWITEM => {
            unsafe {
                style::button(&*(lp as *const DRAWITEMSTRUCT));
            }
            return 1;
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => unsafe {
            SetTextColor(
                wp as HDC,
                if windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(lp as HWND) != 0
                {
                    style::blue()
                } else {
                    style::secondary()
                },
            );
            let numeric = GetDlgCtrlID(lp as HWND) != PROFILE as i32;
            let color = if numeric { style::bg() } else { style::white() };
            SetBkColor(wp as HDC, color);
            return style::background_brush(color) as isize;
        },
        WM_KEYDOWN if wp == 27 => {
            unsafe {
                DestroyWindow(hwnd);
            }
            return 0;
        }
        WM_COMMAND if [IDCANCEL as usize, CLOSE_SETTINGS].contains(&(wp & 0xffff)) => {
            unsafe {
                DestroyWindow(hwnd);
            }
            return 0;
        }
        _ => {}
    }
    if msg == WM_COMMAND {
        let id = wp & 0xffff;
        let event = (wp >> 16) as u32;
        if (PROFILE + 1..=PROFILE + 3).contains(&id) && [EN_CHANGE, EN_KILLFOCUS].contains(&event) {
            normalize_numeric(hwnd, id, event == EN_KILLFOCUS);
            return 0;
        }
        if id == LANGUAGE {
            unsafe {
                let menu = CreatePopupMenu();
                for (index, (_, name)) in i18n::LANGUAGES.iter().enumerate() {
                    add_menu(menu, LANGUAGE_CHOICE + index, name);
                }
                CheckMenuRadioItem(
                    menu,
                    LANGUAGE_CHOICE as u32,
                    (LANGUAGE_CHOICE + i18n::LANGUAGES.len() - 1) as u32,
                    (LANGUAGE_CHOICE + i18n::index()) as u32,
                    MF_BYCOMMAND,
                );
                let mut r = std::mem::zeroed();
                GetWindowRect(GetDlgItem(hwnd, LANGUAGE as i32), &mut r);
                let choice = TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTALIGN,
                    r.right,
                    r.bottom + 4,
                    0,
                    hwnd,
                    ptr::null(),
                ) as usize;
                DestroyMenu(menu);
                if choice != 0 {
                    PostMessageW(hwnd, WM_COMMAND, choice, 0);
                }
            }
            return 0;
        }
        if (LANGUAGE_CHOICE..LANGUAGE_CHOICE + i18n::LANGUAGES.len()).contains(&id) {
            let index = id - LANGUAGE_CHOICE;
            let mut config = lock(CONFIG.get().unwrap());
            let mut next = config.clone();
            next.set("language", i18n::LANGUAGES[index].0);
            if let Err(error) = next.save() {
                crate::platform::error(&format!("{}: {error}", tr("error_language")));
                return 0;
            }
            *config = next;
            drop(config);
            i18n::set(index);
            unsafe {
                let main = GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) as HWND;
                for (id, key) in [(VIEW_MENU, "view"), (SETTINGS, "settings")] {
                    SetWindowTextW(GetDlgItem(main, id as i32), wide(tr(key)).as_ptr());
                }
                SetWindowTextW(
                    GetDlgItem(hwnd, SAVE as i32),
                    wide(tr("save_reconnect")).as_ptr(),
                );
                SetWindowTextW(
                    GetDlgItem(hwnd, CLOSE_SETTINGS as i32),
                    wide(tr("close_settings")).as_ptr(),
                );
                SetWindowTextW(
                    GetDlgItem(hwnd, LANGUAGE as i32),
                    wide(i18n::LANGUAGES[index].1).as_ptr(),
                );
                let shell = FindWindowW(wide("AirBridge.Rust.Supervisor").as_ptr(), ptr::null());
                if !shell.is_null() {
                    PostMessageW(shell, LANGUAGE_CHANGED, index, 0);
                }
            }
            rebuild_receiver_menu();
            redraw_theme();
            return 0;
        }
        if (THEME_SYSTEM..=THEME_DARK).contains(&id) {
            let mode = (id - THEME_SYSTEM) as u8;
            let mut config = lock(CONFIG.get().unwrap());
            let mut next = config.clone();
            next.set(
                "theme_mode",
                match mode {
                    1 => "light",
                    2 => "dark",
                    _ => "system",
                },
            );
            if let Err(error) = next.save() {
                crate::platform::error(&format!("{}: {error}", tr("error_theme")));
                return 0;
            }
            *config = next;
            drop(config);
            apply_theme(mode);
            return 0;
        }
    }
    if msg == WM_COMMAND && wp & 0xffff == AUTO_RESOLUTION {
        update_resolution_controls(hwnd);
        return 0;
    }
    if msg == WM_COMMAND && wp & 0xffff == SAVE {
        for id in PROFILE + 1..=PROFILE + 3 {
            normalize_numeric(hwnd, id, true);
        }
        let mut values = Vec::new();
        for id in PROFILE..PROFILE + 4 {
            let mut buf = [0u16; 256];
            let n = unsafe { GetWindowTextW(GetDlgItem(hwnd, id as i32), buf.as_mut_ptr(), 256) };
            values.push(String::from_utf16_lossy(&buf[..n.max(0) as usize]));
        }
        let name = values[0].trim();
        let automatic =
            unsafe { SendMessageW(GetDlgItem(hwnd, AUTO_RESOLUTION as i32), BM_GETCHECK, 0, 0) }
                == 1;
        let dimensions_valid = values[1..3]
            .iter()
            .all(|v| v.parse::<u32>().is_ok_and(|n| (320..=8192).contains(&n)));
        let fps_valid = values[3]
            .parse::<u32>()
            .is_ok_and(|n| (1..=60).contains(&n));
        if name.is_empty()
            || name.len() > 50
            || name.contains(['.', '\\', '\r', '\n'])
            || (!automatic && !dimensions_valid)
            || !fps_valid
        {
            crate::platform::error(tr("invalid_settings"));
            return 0;
        }
        let mut cfg = lock(CONFIG.get().unwrap());
        let mut next = cfg.clone();
        next.set("mirror_auto_resolution", if automatic { "1" } else { "0" });
        for (k, v) in ["receiver_name", "mirror_width", "mirror_height", "max_fps"]
            .into_iter()
            .zip(values)
        {
            if automatic && !dimensions_valid && matches!(k, "mirror_width" | "mirror_height") {
                continue;
            }
            next.set(k, v);
        }
        for (id, key) in [(210, "mirror_hwaccel"), (211, "hevc_enabled")] {
            let checked = unsafe { SendMessageW(GetDlgItem(hwnd, id), BM_GETCHECK, 0, 0) } == 1;
            next.set(key, if checked { "1" } else { "0" });
        }
        match next.save() {
            Ok(()) => {
                *cfg = next;
                unsafe {
                    remember_window(GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) as HWND, "main");
                    DestroyWindow(hwnd);
                    PostQuitMessage(2);
                }
            }
            Err(e) => crate::platform::error(&format!("{}: {e}", tr("error_save"))),
        }
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}
fn remember_window(hwnd: HWND, key: &str) {
    if key == "main" && lock(&FULL_RECT).is_some() {
        return;
    }
    let Some(state) = WINDOW_STATE.get() else {
        return;
    };
    unsafe {
        let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
        placement.length = std::mem::size_of_val(&placement) as u32;
        if GetWindowPlacement(hwnd, &mut placement) == 0 {
            return;
        }
        let mut r: RECT = std::mem::zeroed();
        GetWindowRect(hwnd, &mut r);
        let maximized = IsZoomed(hwnd) != 0 || placement.flags & WPF_RESTORETOMAXIMIZED != 0;
        if IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 {
            r = placement.rcNormalPosition;
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of_val(&info) as u32;
            if GetMonitorInfoW(MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST), &mut info) != 0 {
                let dx = info.rcWork.left - info.rcMonitor.left;
                let dy = info.rcWork.top - info.rcMonitor.top;
                r.left += dx;
                r.right += dx;
                r.top += dy;
                r.bottom += dy;
            }
        }
        let mut state = lock(state);
        for (suffix, value) in [
            ("x", r.left),
            ("y", r.top),
            ("width", r.right - r.left),
            ("height", r.bottom - r.top),
            ("maximized", i32::from(maximized)),
        ] {
            state.set(&format!("{key}_{suffix}"), value);
        }
        if let Err(error) = state.save() {
            log::warn!("cannot save window placement: {error}");
        }
    }
}
fn restore_window(hwnd: HWND, key: &str, resizable: bool) -> bool {
    unsafe {
        let mut r: RECT = std::mem::zeroed();
        GetWindowRect(hwnd, &mut r);
        let mut w = r.right - r.left;
        let mut h = r.bottom - r.top;
        let mut maximized = false;
        if let Some(state) = WINDOW_STATE.get() {
            let state = lock(state);
            if let (Some(x), Some(y)) = (
                state.signed(&format!("{key}_x")),
                state.signed(&format!("{key}_y")),
            ) {
                r.left = x.clamp(-100000, 100000);
                r.top = y.clamp(-100000, 100000);
            }
            if resizable {
                w = state
                    .signed(&format!("{key}_width"))
                    .unwrap_or(w)
                    .clamp(680, 16384);
                h = state
                    .signed(&format!("{key}_height"))
                    .unwrap_or(h)
                    .clamp(700, 16384);
                maximized = state.flag(&format!("{key}_maximized"), false);
            }
        }
        r.right = r.left + w;
        r.bottom = r.top + h;
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of_val(&info) as u32;
        if GetMonitorInfoW(MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST), &mut info) != 0 {
            let work = info.rcWork;
            if resizable {
                w = w.min((work.right - work.left).max(680));
                h = h.min((work.bottom - work.top).max(700));
            }
            r.left = r.left.clamp(work.left, (work.right - w).max(work.left));
            r.top = r.top.clamp(work.top, (work.bottom - h).max(work.top));
        }
        SetWindowPos(
            hwnd,
            ptr::null_mut(),
            r.left,
            r.top,
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        maximized
    }
}
fn apply_theme(mode: u8) {
    if mode == style::theme_mode() && !style::system_theme_changed() {
        redraw_theme();
        return;
    }
    let overlay = crate::theme_transition::capture();
    style::set_theme(mode);
    redraw_theme();
    crate::theme_transition::reveal(overlay);
}
fn redraw_theme() {
    unsafe {
        for class in ["AirBridge.Rust.Receiver", "AirBridge.Rust.Settings"] {
            let hwnd = FindWindowW(wide(class).as_ptr(), ptr::null());
            if !hwnd.is_null() {
                style::titlebar(hwnd);
                RedrawWindow(
                    hwnd,
                    ptr::null(),
                    ptr::null_mut(),
                    RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_ERASE | RDW_FRAME | RDW_UPDATENOW,
                );
            }
        }
    }
}
unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _subclass: usize,
    id: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_SETTEXT && id > PROFILE {
            let value = if lp == 0 {
                String::new()
            } else {
                let ptr = lp as *const u16;
                let mut len = 0;
                while len < 1024 && *ptr.add(len) != 0 {
                    len += 1;
                }
                String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
            };
            let filtered = numeric_text(&value, id, false);
            let result = DefSubclassProc(hwnd, msg, wp, wide(&filtered).as_ptr() as isize);
            align_edit_caret(hwnd, false, id == PROFILE);
            return result;
        }
        if msg == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(edit_proc), 1);
        }
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        if id > PROFILE && (msg == WM_PASTE || msg == WM_KILLFOCUS) {
            normalize_numeric(GetParent(hwnd), id, msg == WM_KILLFOCUS);
        }
        if [
            WM_SETFOCUS,
            WM_SETFONT,
            WM_LBUTTONDOWN,
            WM_LBUTTONUP,
            WM_MOUSEMOVE,
            WM_KEYDOWN,
            WM_KEYUP,
            WM_CHAR,
            WM_PASTE,
            WM_CUT,
            WM_CLEAR,
            WM_PAINT,
            EM_SETSEL,
        ]
        .contains(&msg)
        {
            align_edit_caret(hwnd, msg == WM_SETFOCUS || msg == WM_SETFONT, id == PROFILE);
        }
        result
    }
}
fn align_edit_caret(hwnd: HWND, recreate: bool, name: bool) {
    unsafe {
        if windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() != hwnd {
            return;
        }
        let mut point = POINT { x: 0, y: 0 };
        if GetCaretPos(&mut point) == 0 {
            return;
        }
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return;
        }
        let font = SendMessageW(hwnd, WM_GETFONT, 0, 0) as HFONT;
        let old = SelectObject(dc, font);
        let mut metrics: TEXTMETRICW = std::mem::zeroed();
        GetTextMetricsW(dc, &mut metrics);
        SelectObject(dc, old);
        ReleaseDC(hwnd, dc);
        let inset = if name { 1 } else { 2 };
        let height = (metrics.tmHeight - inset * 2).max(1);
        let mut format: RECT = std::mem::zeroed();
        SendMessageW(hwnd, EM_GETRECT, 0, (&mut format as *mut RECT) as isize);
        if recreate {
            CreateCaret(hwnd, ptr::null_mut(), 1, height);
        }
        SetCaretPos(point.x, format.top + inset);
        if recreate {
            ShowCaret(hwnd);
        }
    }
}
fn numeric_text(old: &str, id: usize, finalize: bool) -> String {
    let digits: String = old.chars().filter(char::is_ascii_digit).collect();
    let fps = id == PROFILE + 3;
    let max = if fps { 60 } else { 8192 };
    let min = if fps { 1 } else { 320 };
    if digits.is_empty() {
        if finalize {
            if fps {
                "60".into()
            } else if id == PROFILE + 1 {
                "1920".into()
            } else {
                "1080".into()
            }
        } else {
            String::new()
        }
    } else {
        let n = digits.parse::<u64>().unwrap_or(u64::MAX).min(max);
        if finalize {
            n.max(min).to_string()
        } else {
            n.to_string()
        }
    }
}
fn normalize_numeric(parent: HWND, id: usize, finalize: bool) {
    unsafe {
        let edit = GetDlgItem(parent, id as i32);
        let mut buf = [0u16; 256];
        let len = GetWindowTextW(edit, buf.as_mut_ptr(), 256);
        let old = String::from_utf16_lossy(&buf[..len.max(0) as usize]);
        let next = numeric_text(&old, id, finalize);
        if old != next {
            let selection = SendMessageW(edit, EM_GETSEL, 0, 0) as u32;
            SetWindowTextW(edit, wide(&next).as_ptr());
            let caret = ((selection & 0xffff) as usize).min(next.len());
            SendMessageW(edit, EM_SETSEL, caret, caret as isize);
        }
    }
}
fn paint_settings(hwnd: HWND, provided: Option<HDC>) {
    unsafe {
        let mut ps = std::mem::zeroed();
        let dc = provided.unwrap_or_else(|| BeginPaint(hwnd, &mut ps));
        let mut r = std::mem::zeroed();
        GetClientRect(hwnd, &mut r);
        style::fill(dc, r, style::bg());
        style::text(
            dc,
            tr("settings"),
            style::area(24, 18, 460, 42),
            30,
            true,
            style::ink(),
            DT_LEFT,
        );
        style::text(
            dc,
            tr("settings_subtitle"),
            style::area(25, 63, 470, 24),
            14,
            false,
            style::secondary(),
            DT_LEFT,
        );
        for (y, h) in [(111, 160), (310, 181), (530, 108)] {
            style::round(dc, style::area(24, y, 496, h), 16, style::white());
        }
        for (label, y) in [
            (tr("receiver_name"), 124),
            (tr("theme"), 176),
            (tr("language"), 228),
            (tr("resolution"), 394),
            (tr("frame_rate"), 447),
            (tr("gpu_decoding"), 542),
            (tr("hevc_video"), 594),
        ] {
            style::text(
                dc,
                label,
                style::area(42, y, 220, 30),
                15,
                false,
                style::ink(),
                DT_LEFT,
            );
        }
        for (label, y) in [(tr("streaming"), 286), (tr("video_processing"), 506)] {
            style::text(
                dc,
                label,
                style::area(40, y, 460, 20),
                11,
                true,
                style::secondary(),
                DT_LEFT,
            );
        }
        style::text(
            dc,
            tr("auto_resolution"),
            style::area(42, 322, 360, 26),
            15,
            false,
            style::ink(),
            DT_LEFT,
        );
        style::text(
            dc,
            tr("auto_hint"),
            style::area(42, 352, 390, 22),
            13,
            false,
            style::secondary(),
            DT_LEFT,
        );
        style::round(dc, style::area(256, 173, 254, 36), 10, style::bg());
        let automatic =
            SendMessageW(GetDlgItem(hwnd, AUTO_RESOLUTION as i32), BM_GETCHECK, 0, 0) == 1;
        if automatic {
            style::text(
                dc,
                tr("automatic"),
                style::area(280, 394, 224, 30),
                15,
                false,
                style::secondary(),
                DT_RIGHT,
            );
        } else {
            for x in [374, 450] {
                style::round(dc, style::area(x, 391, 54, 36), 9, style::bg());
            }
            style::text(
                dc,
                "×",
                style::area(428, 394, 22, 30),
                15,
                false,
                style::secondary(),
                DT_CENTER,
            );
        }
        style::round(dc, style::area(440, 444, 64, 36), 9, style::bg());
        style::text(
            dc,
            "FPS",
            style::area(472, 447, 25, 30),
            12,
            false,
            style::secondary(),
            DT_CENTER,
        );
        for y in [165, 219, 383, 437, 584] {
            style::fill(dc, style::area(42, y, 460, 1), style::line_color());
        }
        style::text(
            dc,
            tr("apply_hint"),
            style::area(24, 662, 496, 26),
            13,
            false,
            style::secondary(),
            DT_CENTER,
        );
        if provided.is_none() {
            EndPaint(hwnd, &ps);
        }
    }
}
fn update_resolution_controls(hwnd: HWND) {
    unsafe {
        let automatic =
            SendMessageW(GetDlgItem(hwnd, AUTO_RESOLUTION as i32), BM_GETCHECK, 0, 0) == 1;
        for id in [PROFILE + 1, PROFILE + 2] {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
                GetDlgItem(hwnd, id as i32),
                i32::from(!automatic),
            );
            ShowWindow(
                GetDlgItem(hwnd, id as i32),
                if automatic { SW_HIDE } else { SW_SHOW },
            );
        }
        InvalidateRect(hwnd, ptr::null(), 0);
    }
}
fn open_logs() {
    if let Ok(root) = crate::root() {
        let _ = std::process::Command::new("explorer.exe")
            .arg(root.join("logs"))
            .spawn();
    }
}
fn allow_firewall(hwnd: HWND) {
    if let Ok(exe) = std::env::current_exe() {
        let params = format!(
            "advfirewall firewall add rule name=\"AirBridge Rust local network\" dir=in action=allow program=\"{}\" enable=yes profile=private,domain remoteip=localsubnet",
            exe.display()
        );
        let result = unsafe {
            ShellExecuteW(
                hwnd,
                wide("runas").as_ptr(),
                wide("netsh.exe").as_ptr(),
                wide(params).as_ptr(),
                ptr::null(),
                SW_HIDE,
            )
        };
        if result as isize <= 32 {
            crate::platform::error(tr("firewall_error"));
        }
    }
}
fn create(class: &str, title: &str, visible: bool, proc: WNDPROC) -> io::Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(ptr::null());
        let settings = class == "AirBridge.Rust.Settings";
        let class = wide(class);
        let wc = WNDCLASSW {
            style: if settings { CS_DROPSHADOW } else { 0 },
            lpfnWndProc: proc,
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hIcon: if settings {
                ptr::null_mut()
            } else {
                crate::icon::handle()
            },
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | if visible { WS_VISIBLE } else { 0 };
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide(title).as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1000,
            760,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if hwnd.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(hwnd)
        }
    }
}
fn message_loop() -> i32 {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let result = GetMessageW(&mut msg, ptr::null_mut(), 0, 0);
            if result <= 0 {
                return if result == -1 { 1 } else { msg.wParam as i32 };
            }
            let settings = FindWindowW(wide("AirBridge.Rust.Settings").as_ptr(), ptr::null());
            if !settings.is_null() && IsDialogMessageW(settings, &msg) != 0 {
                continue;
            }
            let receiver = FindWindowW(wide("AirBridge.Rust.Receiver").as_ptr(), ptr::null());
            if !receiver.is_null() && (msg.hwnd == receiver || IsChild(receiver, msg.hwnd) != 0) {
                if msg.message == WM_KEYDOWN && [0x7a, 0x1b, b'R' as usize].contains(&msg.wParam) {
                    SendMessageW(receiver, WM_KEYDOWN, msg.wParam, msg.lParam);
                    continue;
                }
                if IsDialogMessageW(receiver, &msg) != 0 {
                    continue;
                }
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotations_preserve_all_pixels() {
        let p = crate::media::Picture {
            width: 2,
            height: 3,
            pixels: (0..24).collect(),
        };
        let p = rotate(&rotate(&rotate(&rotate(&p, 90), 90), 90), 90);
        assert_eq!((p.width, p.height), (2, 3));
        assert_eq!(p.pixels, (0..24).collect::<Vec<_>>());
    }
}
