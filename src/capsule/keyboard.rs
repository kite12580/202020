use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Globalization::{
    GetLocaleInfoEx, LCIDToLocaleName, LOCALE_SISO639LANGNAME, LOCALE_SLOCALIZEDDISPLAYNAME,
};
use windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, GetKeyboardLayoutList, HKL};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GUITHREADINFO, GetGUIThreadInfo, PostMessageW, SendMessageCallbackW, WM_IME_CONTROL,
    WM_INPUTLANGCHANGEREQUEST,
};
use windows::core::{PCWSTR, w};

use crate::capsule::util::{from_wide, wide, window_ids};

const LOCALE_NAME_MAX_LENGTH: usize = 85;
const IMC_GETCONVERSIONMODE: usize = 1;
const IMC_GETOPENSTATUS: usize = 5;
const IME_CMODE_NATIVE: usize = 1;

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
    let name = wide(name);
    let mut buf = [0u16; 128];
    unsafe { GetLocaleInfoEx(PCWSTR(name.as_ptr()), kind, Some(&mut buf)) };
    from_wide(&buf)
}

pub fn iso(hkl: isize) -> String {
    locale_info(&locale_name(hkl), LOCALE_SISO639LANGNAME)
}

fn focus_window(fg: HWND) -> HWND {
    let mut info = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    if unsafe { GetGUIThreadInfo(input_thread(fg), &mut info) }.is_ok() && !info.hwndFocus.is_invalid() {
        info.hwndFocus
    } else {
        fg
    }
}

pub type ImeCallback = unsafe extern "system" fn(HWND, u32, usize, LRESULT);

fn query_ime(ime: HWND, cmd: usize, data: usize, callback: ImeCallback) -> bool {
    unsafe { SendMessageCallbackW(ime, WM_IME_CONTROL, WPARAM(cmd), LPARAM(0), Some(callback), data) }.is_ok()
}

/// Asks the IME of `fg`'s focused thread whether it is open. `callback`
/// gets the IME window, `data` and the open status.
pub fn query_ime_open(fg: HWND, data: usize, callback: ImeCallback) -> bool {
    let ime = unsafe { ImmGetDefaultIMEWnd(focus_window(fg)) };
    !ime.is_invalid() && query_ime(ime, IMC_GETOPENSTATUS, data, callback)
}

/// Asks an open IME for its conversion mode; see `native_mode`.
pub fn query_ime_mode(ime: HWND, data: usize, callback: ImeCallback) -> bool {
    query_ime(ime, IMC_GETCONVERSIONMODE, data, callback)
}

pub fn native_mode(mode: usize) -> bool {
    mode & IME_CMODE_NATIVE != 0
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
    unsafe {
        let _ = PostMessageW(Some(focus_window(fg)), WM_INPUTLANGCHANGEREQUEST, WPARAM(0), LPARAM(hkl));
    }
}
