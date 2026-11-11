use std::{
    collections::HashMap,
    ptr,
    sync::{Mutex, OnceLock},
};
use windows_sys::Win32::{
    Graphics::Gdi::HDC, System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*,
};
// MAKEINTRESOURCE(1): an integer resource identifier, never dereferenced as a pointer.
const ICON_RESOURCE: *const u16 = ptr::without_provenance(1);
pub fn handle() -> HICON {
    static ICON: OnceLock<usize> = OnceLock::new();
    *ICON.get_or_init(|| unsafe {
        let icon = LoadIconW(GetModuleHandleW(ptr::null()), ICON_RESOURCE);
        if icon.is_null() {
            LoadIconW(ptr::null_mut(), IDI_APPLICATION) as usize
        } else {
            icon as usize
        }
    }) as HICON
}
pub fn draw(dc: HDC, x: i32, y: i32, size: i32) {
    static SIZES: OnceLock<Mutex<HashMap<i32, usize>>> = OnceLock::new();
    let mut sizes = SIZES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let icon = *sizes.entry(size).or_insert_with(|| unsafe {
        let icon = LoadImageW(
            GetModuleHandleW(ptr::null()),
            ICON_RESOURCE,
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        );
        if icon.is_null() {
            handle() as usize
        } else {
            icon as usize
        }
    }) as HICON;
    unsafe {
        DrawIconEx(dc, x, y, icon, size, size, 0, ptr::null_mut(), DI_NORMAL);
    }
}
