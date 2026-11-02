use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, MF_CHECKED, MF_STRING, PostMessageW, SetForegroundWindow,
    TPM_BOTTOMALIGN, TPM_CENTERALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_NULL,
};
use windows::core::{PCSTR, PCWSTR, w};

use crate::capsule::util::wide;

pub struct Item<'a> {
    pub id: u32,
    pub text: &'a str,
    pub checked: bool,
}

pub fn command(id: u32, text: &str) -> Item<'_> {
    Item { id, text, checked: false }
}

/// Shows a popup menu anchored above `(x, y)` and returns the chosen id, or
/// 0. Pumps messages until the menu closes.
pub fn show(owner: HWND, items: &[Item], x: i32, y: i32) -> u32 {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        for item in items {
            let flags = if item.checked { MF_STRING | MF_CHECKED } else { MF_STRING };
            let _ = AppendMenuW(menu, flags, item.id as usize, PCWSTR(wide(item.text).as_ptr()));
        }
        let _ = SetForegroundWindow(owner);
        let flags = TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_CENTERALIGN;
        let id = TrackPopupMenuEx(menu, flags.0, x, y, owner, None).0 as u32;
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        id
    }
}

/// Opts menus into dark mode through uxtheme's undocumented
/// `SetPreferredAppMode` (#135) and `FlushMenuThemes` (#136).
pub fn allow_dark_menus() {
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        if let Some(f) = GetProcAddress(uxtheme, PCSTR(135 as *const u8)) {
            let set: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            set(1);
        }
        if let Some(f) = GetProcAddress(uxtheme, PCSTR(136 as *const u8)) {
            let flush: unsafe extern "system" fn() = std::mem::transmute(f);
            flush();
        }
    }
}
