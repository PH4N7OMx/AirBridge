//! iOS-inspired presentation. Inter is bundled under OFL; symbols are original geometry.
use crate::{i18n::tr, platform::wide};
use std::{
    ptr,
    sync::{Once, OnceLock},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Gdi::*, GdiPlus::*},
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::{GetFocus, SetFocus},
        WindowsAndMessaging::*,
    },
};

static DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
pub fn theme_mode() -> u8 {
    MODE.load(std::sync::atomic::Ordering::Relaxed)
}
pub fn set_theme(mode: u8) {
    MODE.store(mode, std::sync::atomic::Ordering::Relaxed);
    let dark = match mode {
        1 => false,
        2 => true,
        _ => system_is_dark(),
    };
    DARK.store(dark, std::sync::atomic::Ordering::Relaxed);
}
pub fn system_theme_changed() -> bool {
    theme_mode() == 0 && system_is_dark() != is_dark()
}
fn system_is_dark() -> bool {
    use windows_sys::Win32::System::Registry::*;
    let mut light = 1u32;
    let mut size = 4u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide("AppsUseLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&mut light as *mut u32).cast(),
            &mut size,
        );
    }
    light == 0
}
pub fn is_dark() -> bool {
    DARK.load(std::sync::atomic::Ordering::Relaxed)
}
pub fn bg() -> u32 {
    if is_dark() {
        rgb(20, 20, 22)
    } else {
        rgb(245, 245, 247)
    }
}
pub fn white() -> u32 {
    if is_dark() {
        rgb(34, 34, 37)
    } else {
        rgb(255, 255, 255)
    }
}
pub fn ink() -> u32 {
    if is_dark() {
        rgb(245, 245, 247)
    } else {
        rgb(29, 29, 31)
    }
}
pub fn secondary() -> u32 {
    if is_dark() {
        rgb(169, 169, 178)
    } else {
        rgb(110, 110, 115)
    }
}
pub fn blue() -> u32 {
    if is_dark() {
        rgb(80, 160, 255)
    } else {
        rgb(0, 113, 227)
    }
}
pub fn line_color() -> u32 {
    if is_dark() {
        rgb(61, 61, 66)
    } else {
        rgb(230, 230, 235)
    }
}
pub fn green() -> u32 {
    rgb(48, 186, 101)
}
fn tint() -> u32 {
    if is_dark() {
        rgb(29, 48, 72)
    } else {
        rgb(231, 241, 255)
    }
}
pub fn background_brush(color: u32) -> HBRUSH {
    static BRUSHES: OnceLock<std::sync::Mutex<std::collections::HashMap<u32, usize>>> =
        OnceLock::new();
    let mut brushes = BRUSHES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *brushes
        .entry(color)
        .or_insert_with(|| unsafe { CreateSolidBrush(color) as usize }) as HBRUSH
}
pub fn titlebar(hwnd: HWND) {
    unsafe {
        let dark = i32::from(is_dark());
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd,
            20,
            (&dark as *const i32).cast(),
            4,
        );
    }
}
pub const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

pub fn fonts() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        for data in [
            include_bytes!("../../../assets/fonts/Inter-Regular.ttf").as_slice(),
            include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf").as_slice(),
        ] {
            let mut count = 0;
            AddFontMemResourceEx(
                data.as_ptr().cast(),
                data.len() as u32,
                ptr::null(),
                &raw mut count,
            );
        }
    });
}
pub fn font(size: i32, bold: bool) -> HFONT {
    fonts();
    unsafe {
        CreateFontW(
            -size,
            0,
            0,
            0,
            if bold { 600 } else { 400 },
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32,
            wide("Inter").as_ptr(),
        )
    }
}
pub fn area(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}
pub fn fill(dc: HDC, r: RECT, color: u32) {
    unsafe {
        let b = CreateSolidBrush(color);
        FillRect(dc, &r, b);
        DeleteObject(b);
    }
}
// Render geometry at triple resolution, then filter back into the actual background.
// Text stays at native resolution so ClearType remains sharp.
fn supersampled(dc: HDC, r: RECT, draw: impl FnOnce(HDC)) -> bool {
    unsafe {
        let w = r.right - r.left;
        let h = r.bottom - r.top;
        if w <= 0 || h <= 0 {
            return false;
        }
        let buffer = CreateCompatibleDC(dc);
        let bitmap = CreateCompatibleBitmap(dc, w * 3, h * 3);
        if buffer.is_null() || bitmap.is_null() {
            if !buffer.is_null() {
                DeleteDC(buffer);
            }
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            return false;
        }
        let previous = SelectObject(buffer, bitmap);
        SetStretchBltMode(buffer, COLORONCOLOR);
        StretchBlt(buffer, 0, 0, w * 3, h * 3, dc, r.left, r.top, w, h, SRCCOPY);
        draw(buffer);
        let mode = SetStretchBltMode(dc, HALFTONE);
        let mut origin = POINT { x: 0, y: 0 };
        SetBrushOrgEx(dc, 0, 0, &mut origin);
        StretchBlt(dc, r.left, r.top, w, h, buffer, 0, 0, w * 3, h * 3, SRCCOPY);
        SetStretchBltMode(dc, mode);
        SetBrushOrgEx(dc, origin.x, origin.y, ptr::null_mut());
        SelectObject(buffer, previous);
        DeleteObject(bitmap);
        DeleteDC(buffer);
        true
    }
}
pub fn round(dc: HDC, r: RECT, radius: i32, color: u32) {
    if !supersampled(dc, r, |buffer| {
        round_native(
            buffer,
            area(0, 0, (r.right - r.left) * 3, (r.bottom - r.top) * 3),
            radius * 3,
            color,
        )
    }) {
        round_native(dc, r, radius, color);
    }
}
fn round_native(dc: HDC, r: RECT, radius: i32, color: u32) {
    unsafe {
        static GDI_PLUS: OnceLock<bool> = OnceLock::new();
        let ready = *GDI_PLUS.get_or_init(|| {
            let mut token = 0;
            let input = GdiplusStartupInput {
                GdiplusVersion: 1,
                ..std::mem::zeroed()
            };
            GdiplusStartup(&mut token, &input, ptr::null_mut()) == 0
        });
        if ready {
            let mut graphics = ptr::null_mut();
            let mut path = ptr::null_mut();
            let mut brush = ptr::null_mut();
            let argb =
                0xff000000 | ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255);
            let ok = GdipCreateFromHDC(dc, &mut graphics) == 0
                && GdipCreatePath(FillModeWinding, &mut path) == 0
                && GdipCreateSolidFill(argb, &mut brush) == 0;
            if ok {
                GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias8x8);
                let d = (radius * 2).min(r.right - r.left).min(r.bottom - r.top) as f32;
                for (x, y, angle) in [
                    (r.left as f32, r.top as f32, 180.0),
                    (r.right as f32 - d, r.top as f32, 270.0),
                    (r.right as f32 - d, r.bottom as f32 - d, 0.0),
                    (r.left as f32, r.bottom as f32 - d, 90.0),
                ] {
                    GdipAddPathArc(path, x, y, d, d, angle, 90.0);
                }
                GdipClosePathFigure(path);
                GdipFillPath(graphics, brush.cast(), path);
            }
            if !brush.is_null() {
                GdipDeleteBrush(brush.cast());
            }
            if !path.is_null() {
                GdipDeletePath(path);
            }
            if !graphics.is_null() {
                GdipDeleteGraphics(graphics);
            }
            if ok {
                return;
            }
        }
        let b = CreateSolidBrush(color);
        let old_b = SelectObject(dc, b);
        let old_p = SelectObject(dc, GetStockObject(NULL_PEN));
        RoundRect(dc, r.left, r.top, r.right, r.bottom, radius * 2, radius * 2);
        SelectObject(dc, old_b);
        SelectObject(dc, old_p);
        DeleteObject(b);
    }
}
#[allow(clippy::too_many_arguments)]
pub fn text(dc: HDC, value: &str, r: RECT, size: i32, bold: bool, color: u32, align: u32) {
    unsafe {
        let f = font(size, bold);
        let old = SelectObject(dc, f);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, color);
        let mut r = r;
        DrawTextW(
            dc,
            wide(value).as_ptr(),
            -1,
            &mut r,
            align | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        SelectObject(dc, old);
        DeleteObject(f);
    }
}
pub fn line(dc: HDC, points: &[(i32, i32)], color: u32, width: i32) {
    if points.is_empty() {
        return;
    }
    let pad = width + 2;
    let left = points.iter().map(|p| p.0).min().unwrap() - pad;
    let top = points.iter().map(|p| p.1).min().unwrap() - pad;
    let right = points.iter().map(|p| p.0).max().unwrap() + pad;
    let bottom = points.iter().map(|p| p.1).max().unwrap() + pad;
    let r = area(left, top, right - left, bottom - top);
    if !supersampled(dc, r, |buffer| {
        let scaled: Vec<_> = points
            .iter()
            .map(|(x, y)| ((x - left) * 3, (y - top) * 3))
            .collect();
        line_native(buffer, &scaled, color, width * 3);
    }) {
        line_native(dc, points, color, width);
    }
}
fn line_native(dc: HDC, points: &[(i32, i32)], color: u32, width: i32) {
    unsafe {
        let pen = CreatePen(PS_SOLID, width, color);
        let old = SelectObject(dc, pen);
        if let Some((x, y)) = points.first() {
            MoveToEx(dc, *x, *y, ptr::null_mut());
        }
        for (x, y) in points.iter().skip(1) {
            LineTo(dc, *x, *y);
        }
        SelectObject(dc, old);
        DeleteObject(pen);
    }
}
pub fn button(draw: &DRAWITEMSTRUCT) {
    let dc = draw.hDC;
    let r = draw.rcItem;
    let id = draw.CtlID;
    if id == 217 {
        fill(dc, r, white());
        round(dc, r, 10, bg());
        text(
            dc,
            crate::i18n::LANGUAGES[crate::i18n::index()].1,
            area(10, 0, r.right - r.left - 40, r.bottom - r.top),
            14,
            false,
            blue(),
            DT_CENTER,
        );
        let x = r.right - 21;
        let y = (r.bottom - r.top) / 2;
        line(
            dc,
            &[(x - 4, y - 2), (x, y + 2), (x + 4, y - 2)],
            secondary(),
            1,
        );
        return;
    }
    if id == 216 {
        fill(dc, r, bg());
        round(
            dc,
            r,
            18,
            if draw.itemState & ODS_SELECTED != 0 {
                tint()
            } else {
                white()
            },
        );
        line(dc, &[(13, 13), (23, 23)], secondary(), 2);
        line(dc, &[(23, 13), (13, 23)], secondary(), 2);
        return;
    }
    if (213..=215).contains(&id) {
        fill(dc, r, bg());
        let selected = (id - 213) as u8 == theme_mode();
        if selected {
            round(
                dc,
                area(1, 1, r.right - r.left - 2, r.bottom - r.top - 2),
                7,
                white(),
            );
        }
        let label = match id {
            213 => tr("system"),
            214 => tr("light"),
            _ => tr("dark"),
        };
        text(
            dc,
            label,
            r,
            12,
            selected,
            if selected { ink() } else { secondary() },
            DT_CENTER,
        );
        return;
    }
    let pressed = draw.itemState & ODS_SELECTED != 0;
    let primary = id == 108;
    let button_color = if primary {
        if pressed {
            rgb(0, 90, 190)
        } else {
            rgb(0, 113, 227)
        }
    } else if pressed {
        if is_dark() {
            rgb(42, 66, 96)
        } else {
            rgb(220, 232, 248)
        }
    } else {
        tint()
    };
    fill(dc, r, bg());
    round(dc, r, 12, button_color);
    let mut buffer = [0u16; 128];
    let len = unsafe { GetWindowTextW(draw.hwndItem, buffer.as_mut_ptr(), 128) };
    let label = String::from_utf16_lossy(&buffer[..len.max(0) as usize]);
    let mut label_rect = r;
    if [107, 114].contains(&id) {
        let x = r.left + 12;
        let y = r.top + (r.bottom - r.top) / 2;
        match id {
            114 => {
                line(dc, &[(x, y - 1), (x, y - 6), (x + 5, y - 6)], blue(), 1);
                line(
                    dc,
                    &[(x + 12, y + 1), (x + 12, y + 6), (x + 7, y + 6)],
                    blue(),
                    1,
                );
            }
            _ => {
                for (dy, cx) in [(-5, 4), (0, 9), (5, 5)] {
                    line(dc, &[(x, y + dy), (x + 13, y + dy)], blue(), 1);
                    round(dc, area(x + cx - 2, y + dy - 2, 4, 4), 2, blue());
                }
            }
        }
        label_rect.left += if id == 107 { 28 } else { 18 };
    }
    text(
        dc,
        &label,
        label_rect,
        13,
        true,
        if primary { rgb(255, 255, 255) } else { blue() },
        DT_CENTER,
    );
}

pub fn landing(dc: HDC, w: i32, h: i32, name: &str, audio: bool) {
    let cw = (w - 80).clamp(360, 660);
    let x = (w - cw) / 2;
    let y = 100.max((h - 482) / 2);
    // Dark surfaces use tonal separation. Light surfaces have a shallow soft shadow.
    if !is_dark() {
        for spread in (1..=6).rev() {
            let shade = 245 - (7 - spread);
            round(
                dc,
                area(
                    x - spread,
                    y + 3 - spread,
                    cw + spread * 2,
                    406 + spread * 2,
                ),
                28 + spread,
                rgb(shade as u32, shade as u32, shade as u32 + 2),
            );
        }
    }
    round(dc, area(x, y, cw, 406), 28, white());
    let cx = w / 2;
    crate::icon::draw(dc, cx - 38, y + 32, 76);
    text(
        dc,
        if audio {
            tr("audio_playing")
        } else {
            tr("mirror_title")
        },
        area(x + 24, y + 124, cw - 48, 48),
        34,
        true,
        ink(),
        DT_CENTER,
    );
    text(
        dc,
        if audio {
            tr("audio_description")
        } else {
            tr("mirror_description")
        },
        area(x + 20, y + 176, cw - 40, 28),
        15,
        false,
        secondary(),
        DT_CENTER,
    );
    let pill_w = (name.chars().count() as i32 * 9 + 58).clamp(140, cw - 80);
    round(dc, area(cx - pill_w / 2, y + 224, pill_w, 38), 19, bg());
    round(dc, area(cx - pill_w / 2 + 16, y + 239, 8, 8), 4, green());
    text(
        dc,
        name,
        area(cx - pill_w / 2 + 34, y + 224, pill_w - 48, 38),
        15,
        true,
        ink(),
        DT_CENTER,
    );
    fill(dc, area(x + 32, y + 286, cw - 64, 1), line_color());
    for (index, label) in [
        tr("control_center"),
        tr("screen_mirroring"),
        tr("choose_receiver"),
    ]
    .iter()
    .enumerate()
    {
        let col = (cw - 48) / 3;
        let sx = x + 24 + index as i32 * col;
        round(dc, area(sx + col / 2 - 14, y + 310, 28, 28), 14, tint());
        text(
            dc,
            &(index + 1).to_string(),
            area(sx + col / 2 - 14, y + 310, 28, 28),
            13,
            true,
            blue(),
            DT_CENTER,
        );
        text(
            dc,
            label,
            area(sx, y + 350, col, 26),
            12,
            false,
            ink(),
            DT_CENTER,
        );
    }
    text(
        dc,
        tr("same_network"),
        area(x, y + 425, cw, 26),
        13,
        false,
        secondary(),
        DT_CENTER,
    );
}

const SWITCH_DURATION: f32 = 0.18;
struct SwitchState {
    checked: bool,
    position: f32,
    from: f32,
    started: std::time::Instant,
    pressed: bool,
    keyboard_focus: bool,
}
impl SwitchState {
    fn update(&mut self) -> bool {
        let t = (self.started.elapsed().as_secs_f32() / SWITCH_DURATION).min(1.0);
        let ease = 1.0 - (1.0 - t).powi(3);
        self.position = self.from + (f32::from(self.checked) - self.from) * ease;
        t < 1.0
    }
}
// Keyboard-focusable switch; only the thumb moves. Track color changes immediately.
pub unsafe extern "system" fn switch_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_CREATE {
            let state = Box::new(SwitchState {
                checked: false,
                position: 0.0,
                from: 0.0,
                started: std::time::Instant::now(),
                pressed: false,
                keyboard_focus: false,
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
            return 0;
        }
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SwitchState;
        if state.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        match msg {
            WM_NCDESTROY => {
                KillTimer(hwnd, 1);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(state));
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            BM_GETCHECK => isize::from((*state).checked),
            BM_SETCHECK => {
                (*state).checked = wp != 0;
                (*state).position = f32::from(wp != 0);
                (*state).from = (*state).position;
                KillTimer(hwnd, 1);
                InvalidateRect(hwnd, ptr::null(), 0);
                0
            }
            WM_LBUTTONDOWN => {
                (*state).pressed = true;
                (*state).keyboard_focus = false;
                windows_sys::Win32::UI::Input::KeyboardAndMouse::SetCapture(hwnd);
                SetFocus(hwnd);
                InvalidateRect(hwnd, ptr::null(), 0);
                0
            }
            WM_LBUTTONUP => {
                let pressed = (*state).pressed;
                (*state).pressed = false;
                windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
                let x = lp as i16 as i32;
                let y = (lp >> 16) as i16 as i32;
                if pressed && (0..54).contains(&x) && (0..35).contains(&y) {
                    SendMessageW(hwnd, BM_CLICK, 0, 0);
                }
                0
            }
            BM_CLICK => {
                if windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(hwnd) != 0 {
                    (*state).update();
                    (*state).from = (*state).position;
                    (*state).checked = !(*state).checked;
                    (*state).started = std::time::Instant::now();
                    SetTimer(hwnd, 1, 16, None);
                    InvalidateRect(hwnd, ptr::null(), 0);
                    SendMessageW(
                        GetParent(hwnd),
                        WM_COMMAND,
                        GetDlgCtrlID(hwnd) as usize,
                        hwnd as isize,
                    );
                }
                0
            }
            WM_TIMER => {
                if !(*state).update() {
                    KillTimer(hwnd, 1);
                }
                InvalidateRect(hwnd, ptr::null(), 0);
                0
            }
            WM_KEYUP if wp == 32 => {
                (*state).keyboard_focus = true;
                SendMessageW(hwnd, BM_CLICK, 0, 0);
                0
            }
            WM_CHAR if wp == 32 => 0,
            WM_SETFOCUS => {
                (*state).keyboard_focus = !(*state).pressed;
                InvalidateRect(hwnd, ptr::null(), 0);
                0
            }
            WM_KILLFOCUS | WM_ENABLE => {
                InvalidateRect(hwnd, ptr::null(), 0);
                0
            }
            WM_GETDLGCODE => DLGC_BUTTON as isize,
            WM_ERASEBKGND => 1,
            WM_PAINT | WM_PRINTCLIENT => {
                let mut ps = std::mem::zeroed();
                let target = if msg == WM_PAINT {
                    BeginPaint(hwnd, &mut ps)
                } else {
                    wp as HDC
                };
                let mut r = std::mem::zeroed();
                GetClientRect(hwnd, &mut r);
                let dc = CreateCompatibleDC(target);
                let bitmap = CreateCompatibleBitmap(target, r.right, r.bottom);
                if dc.is_null() || bitmap.is_null() {
                    if !bitmap.is_null() {
                        DeleteObject(bitmap);
                    }
                    if !dc.is_null() {
                        DeleteDC(dc);
                    }
                    if msg == WM_PAINT {
                        EndPaint(hwnd, &ps);
                    }
                    return 0;
                }
                let old = SelectObject(dc, bitmap);
                fill(dc, r, white());
                if GetFocus() == hwnd && (*state).keyboard_focus {
                    round(dc, area(0, 1, 53, 33), 17, blue());
                }
                let track = if (*state).checked {
                    green()
                } else if is_dark() {
                    rgb(80, 80, 88)
                } else {
                    rgb(215, 216, 222)
                };
                round(dc, area(2, 3, 48, 28), 14, track);
                let x = 4.0 + 20.0 * (*state).position;
                round(
                    dc,
                    area(x.round() as i32, 5, 24, 24),
                    12,
                    rgb(255, 255, 255),
                );
                BitBlt(target, 0, 0, r.right, r.bottom, dc, 0, 0, SRCCOPY);
                SelectObject(dc, old);
                DeleteObject(bitmap);
                DeleteDC(dc);
                if msg == WM_PAINT {
                    EndPaint(hwnd, &ps);
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
