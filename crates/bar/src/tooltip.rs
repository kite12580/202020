use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Controls::{
    SetWindowTheme, TOOLTIPS_CLASSW, TTF_ABSOLUTE, TTF_TRACK, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH, TTM_TRACKACTIVATE,
    TTM_TRACKPOSITION, TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetWindowRect, SendMessageW, WINDOW_STYLE, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, PWSTR, w};

use crate::util::{make_long, wide};

pub struct Tooltip {
    hwnd: HWND,
    owner: HWND,
    text: Vec<u16>,
    shown: bool,
}

impl Tooltip {
    pub fn new(owner: HWND) -> Self {
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST,
                TOOLTIPS_CLASSW,
                PCWSTR::null(),
                WS_POPUP | WINDOW_STYLE(TTS_NOPREFIX | TTS_ALWAYSTIP),
                0,
                0,
                0,
                0,
                Some(owner),
                None,
                None,
                None,
            )
        }
        .unwrap_or_default();
        let mut t = Tooltip { hwnd, owner, text: wide(""), shown: false };
        let mut info = t.info();
        unsafe {
            SendMessageW(hwnd, TTM_ADDTOOLW, None, Some(LPARAM(&mut info as *mut _ as isize)));
            SendMessageW(hwnd, TTM_SETMAXTIPWIDTH, None, Some(LPARAM(480)));
        }
        t.set_dark(true);
        t
    }

    fn info(&mut self) -> TTTOOLINFOW {
        TTTOOLINFOW {
            cbSize: size_of::<TTTOOLINFOW>() as u32,
            uFlags: TTF_TRACK | TTF_ABSOLUTE,
            hwnd: self.owner,
            uId: 1,
            lpszText: PWSTR(self.text.as_mut_ptr()),
            ..Default::default()
        }
    }

    pub fn set_dark(&self, dark: bool) {
        let theme = if dark { w!("DarkMode_Explorer") } else { w!("Explorer") };
        unsafe {
            let _ = SetWindowTheme(self.hwnd, theme, PCWSTR::null());
        }
    }

    /// Shows `text` centered horizontally on `x`, just above `top` (screen px).
    pub fn show(&mut self, text: &str, x: i32, top: i32) {
        if text.is_empty() {
            self.hide();
            return;
        }
        self.text = wide(text);
        let mut info = self.info();
        let lp = Some(LPARAM(&mut info as *mut _ as isize));
        unsafe {
            SendMessageW(self.hwnd, TTM_UPDATETIPTEXTW, None, lp);
            SendMessageW(self.hwnd, TTM_TRACKPOSITION, None, Some(LPARAM(make_long(x, top) as isize)));
            SendMessageW(self.hwnd, TTM_TRACKACTIVATE, Some(WPARAM(1)), lp);
            let mut r = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut r);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let pos = make_long(x - w / 2, top - h - 6);
            SendMessageW(self.hwnd, TTM_TRACKPOSITION, None, Some(LPARAM(pos as isize)));
        }
        self.shown = true;
    }

    pub fn hide(&mut self) {
        if !self.shown {
            return;
        }
        self.shown = false;
        let mut info = self.info();
        unsafe {
            SendMessageW(self.hwnd, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(LPARAM(&mut info as *mut _ as isize)));
        }
    }
}

impl Drop for Tooltip {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
