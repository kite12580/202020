use std::collections::BTreeMap;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Globalization::{
    GetLocaleInfoEx, LCIDToLocaleName, LOCALE_SISO639LANGNAME, LOCALE_SLOCALIZEDDISPLAYNAME,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, GetKeyboardLayoutList, HKL};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GUITHREADINFO, GetGUIThreadInfo, PostMessageW, WM_INPUTLANGCHANGEREQUEST,
};
use windows::core::{PCWSTR, w};

use crate::util::{from_wide, window_ids};

const LOCALE_NAME_MAX_LENGTH: usize = 85;

/// The thread whose input language the user sees for `fg`. UWP frames host
/// the app's input thread in a child CoreWindow.
pub fn input_thread(fg: HWND) -> u32 {
    let core = unsafe { FindWindowExW(Some(fg), None, w!("Windows.UI.Core.CoreWindow"), PCWSTR::null()) };
    window_ids(core.unwrap_or(fg)).0
}

pub fn current(fg: HWND) -> isize {
    unsafe { GetKeyboardLayout(input_thread(fg)) }.0 as isize
}

fn locale_name(hkl: isize) -> String {
    let lcid = (hkl as usize & 0xFFFF) as u32;
    let mut buf = [0u16; LOCALE_NAME_MAX_LENGTH];
    let n = unsafe { LCIDToLocaleName(lcid, Some(&mut buf), 0) };
    if n == 0 { String::new() } else { from_wide(&buf) }
}

fn locale_info(name: &str, kind: u32) -> String {
    let name = crate::util::wide(name);
    let mut buf = [0u16; 128];
    unsafe { GetLocaleInfoEx(PCWSTR(name.as_ptr()), kind, Some(&mut buf)) };
    from_wide(&buf)
}

pub fn label(hkl: isize, overrides: &BTreeMap<String, String>) -> String {
    let iso = locale_info(&locale_name(hkl), LOCALE_SISO639LANGNAME);
    capsule_core::kbd::label(&iso, overrides)
}

pub fn display_name(hkl: isize) -> String {
    locale_info(&locale_name(hkl), LOCALE_SLOCALIZEDDISPLAYNAME)
}

pub fn installed() -> Vec<isize> {
    let n = unsafe { GetKeyboardLayoutList(None) } as usize;
    let mut list = vec![HKL::default(); n];
    let n = unsafe { GetKeyboardLayoutList(Some(&mut list)) } as usize;
    list.truncate(n);
    list.into_iter().map(|h| h.0 as isize).collect()
}

pub fn switch(fg: HWND, hkl: isize) {
    let mut info = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    let target = if unsafe { GetGUIThreadInfo(input_thread(fg), &mut info) }.is_ok() && !info.hwndFocus.is_invalid() {
        info.hwndFocus
    } else {
        fg
    };
    unsafe {
        let _ = PostMessageW(Some(target), WM_INPUTLANGCHANGEREQUEST, WPARAM(0), LPARAM(hkl));
    }
}
