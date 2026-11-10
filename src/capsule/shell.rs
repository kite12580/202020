use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, SetWindowRgn};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Shell::{ABM_GETSTATE, ABM_SETSTATE, APPBARDATA, SHAppBarMessage};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_SHOW, FindWindowExW, IsWindowVisible, OBJID_WINDOW, SPI_GETWORKAREA, SPI_SETWORKAREA, SPIF_SENDCHANGE,
    SW_HIDE, SW_SHOWNA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, ShowWindow, SystemParametersInfoW, WINEVENT_OUTOFCONTEXT,
};
use windows::core::{PCWSTR, w};

use crate::capsule::util::{hwnd, primary_monitor_rect, process_path, window_ids};
use crate::capsule::watchdog;

const ABS_AUTOHIDE: u32 = 1;
const ABS_ALWAYSONTOP: u32 = 2;

pub static REAL_TRAY: AtomicIsize = AtomicIsize::new(0);

/// Set while the taskbar is deliberately shown so explorer can handle a
/// replayed shortcut.
static REVEALED: AtomicBool = AtomicBool::new(false);

pub fn real_tray() -> Option<HWND> {
    let v = REAL_TRAY.load(Ordering::Relaxed);
    (v != 0).then(|| hwnd(v))
}

/// Skips the `Shell_TrayWnd`s other programs create to intercept tray icons.
pub fn find_real_tray() -> Option<HWND> {
    let mut after = None;
    loop {
        let h = unsafe { FindWindowExW(None, after, w!("Shell_TrayWnd"), PCWSTR::null()) }.ok()?;
        if process_path(window_ids(h).1).is_some_and(|p| p.to_ascii_lowercase().ends_with("\\explorer.exe")) {
            return Some(h);
        }
        after = Some(h);
    }
}

fn appbar_data(tray: HWND) -> APPBARDATA {
    APPBARDATA { cbSize: size_of::<APPBARDATA>() as u32, hWnd: tray, ..Default::default() }
}

fn ab_state(tray: HWND) -> u32 {
    let mut d = appbar_data(tray);
    unsafe { SHAppBarMessage(ABM_GETSTATE, &mut d) as u32 }
}

fn set_ab_state(tray: HWND, state: u32) {
    let mut d = appbar_data(tray);
    d.lParam = LPARAM(state as isize);
    unsafe { SHAppBarMessage(ABM_SETSTATE, &mut d) };
}

pub fn work_area() -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut r as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    r
}

fn set_work_area(r: &RECT) {
    unsafe {
        let _ = SystemParametersInfoW(SPI_SETWORKAREA, 0, Some(r as *const RECT as *mut c_void), SPIF_SENDCHANGE);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Saved {
    pub ab_state: u32,
    pub work_area: RECT,
}

pub fn restore(saved: &Saved) {
    let Some(tray) = find_real_tray() else { return };
    set_ab_state(tray, saved.ab_state);
    unsafe {
        SetWindowRgn(tray, None, true);
        let _ = ShowWindow(tray, SW_SHOWNA);
    }
    if saved.work_area != RECT::default() {
        set_work_area(&saved.work_area);
    }
    log::info!("taskbar restored (state {})", saved.ab_state);
}

/// `--restore`: recovery when both processes were killed.
pub fn force_restore() {
    restore(&Saved { ab_state: ABS_ALWAYSONTOP, work_area: RECT::default() });
}

unsafe extern "system" fn on_tray_shown(
    _hook: HWINEVENTHOOK,
    _event: u32,
    h: HWND,
    object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if object == OBJID_WINDOW.0 && Some(h) == real_tray() && !REVEALED.load(Ordering::Relaxed) {
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
    }
}

/// An empty window region keeps the taskbar invisible even while it is
/// shown, which explorer requires to handle Win+number.
unsafe fn erase(tray: HWND) {
    unsafe { SetWindowRgn(tray, Some(CreateRectRgn(0, 0, 0, 0)), false) };
}

pub struct Takeover {
    saved: Saved,
    show_hook: HWINEVENTHOOK,
}

impl Takeover {
    pub fn start(tray: HWND) -> Option<Takeover> {
        let saved = Saved { ab_state: ab_state(tray), work_area: work_area() };
        if !watchdog::set_takeover(Some(saved)) {
            return None;
        }
        let mut t = Takeover { saved, show_hook: HWINEVENTHOOK::default() };
        t.apply(tray);
        Some(t)
    }

    pub fn apply(&mut self, tray: HWND) {
        set_ab_state(tray, ABS_AUTOHIDE);
        unsafe {
            erase(tray);
            let _ = ShowWindow(tray, SW_HIDE);
            if !self.show_hook.is_invalid() {
                let _ = UnhookWinEvent(self.show_hook);
            }
            let (tid, pid) = window_ids(tray);
            self.show_hook = SetWinEventHook(
                EVENT_OBJECT_SHOW,
                EVENT_OBJECT_SHOW,
                None,
                Some(on_tray_shown),
                pid,
                tid,
                WINEVENT_OUTOFCONTEXT,
            );
        }
        self.ensure_work_area();
        log::info!("taskbar hidden");
    }

    /// Shows the erased taskbar so explorer handles its Win+number shortcuts.
    pub fn reveal(&self) {
        REVEALED.store(true, Ordering::Relaxed);
        if let Some(tray) = real_tray() {
            unsafe {
                erase(tray);
                let _ = ShowWindow(tray, SW_SHOWNA);
            }
        }
    }

    pub fn revealed(&self) -> bool {
        REVEALED.load(Ordering::Relaxed)
    }

    pub fn conceal(&self) {
        REVEALED.store(false, Ordering::Relaxed);
        self.ensure_hidden();
    }

    /// Re-hides the taskbar if explorer showed it without a SHOW event.
    pub fn ensure_hidden(&self) {
        if !self.revealed()
            && let Some(tray) = real_tray()
            && unsafe { IsWindowVisible(tray) }.as_bool()
        {
            unsafe {
                let _ = ShowWindow(tray, SW_HIDE);
            }
        }
    }

    pub fn ensure_work_area(&self) {
        let full = primary_monitor_rect();
        if work_area() != full {
            set_work_area(&full);
            log::debug!("work area reset to monitor");
        }
    }

    pub fn stop(self) {
        REVEALED.store(false, Ordering::Relaxed);
        unsafe {
            let _ = UnhookWinEvent(self.show_hook);
        }
        restore(&self.saved);
        watchdog::set_takeover(None);
    }
}
