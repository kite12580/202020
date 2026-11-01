use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WAIT_OBJECT_0};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, SetWindowRgn};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcessId, INFINITE, OpenEventW, OpenProcess, PROCESS_SYNCHRONIZE,
    SYNCHRONIZATION_SYNCHRONIZE, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Shell::{ABM_GETSTATE, ABM_SETSTATE, APPBARDATA, SHAppBarMessage};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_SHOW, FindWindowExW, IsWindowVisible, OBJID_WINDOW, SPI_GETWORKAREA, SPI_SETWORKAREA, SPIF_SENDCHANGE,
    SW_HIDE, SW_SHOWNA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, ShowWindow, SystemParametersInfoW, WINEVENT_OUTOFCONTEXT,
};
use windows::core::{PCWSTR, w};

use crate::capsule::util::{OwnedHandle, hwnd, primary_monitor, wide, window_ids};

const ABS_AUTOHIDE: u32 = 1;
const ABS_ALWAYSONTOP: u32 = 2;

/// Explorer's `Shell_TrayWnd`, read by the tray spy thread for forwarding.
pub static REAL_TRAY: AtomicIsize = AtomicIsize::new(0);

/// Set while the taskbar is deliberately shown so explorer can handle a
/// replayed shortcut.
static REVEALED: AtomicBool = AtomicBool::new(false);

pub fn real_tray() -> Option<HWND> {
    let v = REAL_TRAY.load(Ordering::Relaxed);
    (v != 0).then(|| hwnd(v))
}

/// Finds the `Shell_TrayWnd` that isn't our spy.
pub fn find_real_tray() -> Option<HWND> {
    let own = unsafe { GetCurrentProcessId() };
    let mut after = None;
    loop {
        let h = unsafe { FindWindowExW(None, after, w!("Shell_TrayWnd"), PCWSTR::null()) }.ok()?;
        if window_ids(h).1 != own {
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

fn monitor_work_area() -> RECT {
    let m = primary_monitor();
    RECT { left: m.left, top: m.top, right: m.right, bottom: m.bottom }
}

#[derive(Clone, Copy, Debug)]
pub struct Saved {
    pub ab_state: u32,
    pub work_area: RECT,
}

/// Puts the taskbar back. The tray spy must be gone so `SHAppBarMessage`
/// reaches explorer.
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
    exit_event: OwnedHandle,
    show_hook: HWINEVENTHOOK,
}

/// Numbers each takeover so a restarted bar never reuses the previous
/// watchdog's already-signaled exit event.
static TAKEOVERS: AtomicU32 = AtomicU32::new(0);

fn exit_event_name(pid: u32, seq: u32) -> Vec<u16> {
    wide(&format!("Local\\202020-exit-{pid}-{seq}"))
}

impl Takeover {
    pub fn start(tray: HWND) -> Option<Takeover> {
        let saved = Saved { ab_state: ab_state(tray), work_area: work_area() };
        let pid = unsafe { GetCurrentProcessId() };
        let seq = TAKEOVERS.fetch_add(1, Ordering::Relaxed);
        let name = exit_event_name(pid, seq);
        let exit_event = OwnedHandle(unsafe { CreateEventW(None, true, false, PCWSTR(name.as_ptr())) }.ok()?);
        let r = saved.work_area;
        let exe = std::env::current_exe().ok()?;
        let spawned = std::process::Command::new(exe)
            .arg("--watchdog")
            .args([pid, seq, saved.ab_state].map(|v| v.to_string()))
            .args([r.left, r.top, r.right, r.bottom].map(|v| v.to_string()))
            .args((log::max_level() != log::LevelFilter::Off).then_some("--verbose"))
            .spawn();
        if let Err(e) = spawned {
            log::error!("watchdog: {e}");
            return None;
        }
        let mut t = Takeover { saved, exit_event, show_hook: HWINEVENTHOOK::default() };
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
        let full = monitor_work_area();
        if work_area() != full {
            set_work_area(&full);
            log::debug!("work area reset to monitor");
        }
    }

    pub fn stop(self) {
        REVEALED.store(false, Ordering::Relaxed);
        unsafe {
            let _ = UnhookWinEvent(self.show_hook);
            let _ = SetEvent(self.exit_event.0);
        }
        restore(&self.saved);
    }
}

/// `--watchdog <pid> <seq> <ab_state> <left> <top> <right> <bottom>`
pub fn run_watchdog(args: &[String]) -> i32 {
    let nums: Vec<i64> = args.iter().filter_map(|a| a.parse().ok()).collect();
    let [pid, seq, ab_state, left, top, right, bottom] = nums[..] else {
        return 2;
    };
    let saved = Saved {
        ab_state: ab_state as u32,
        work_area: RECT { left: left as i32, top: top as i32, right: right as i32, bottom: bottom as i32 },
    };
    let pid = pid as u32;
    let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else {
        return 1;
    };
    let process = OwnedHandle(process);
    let name = exit_event_name(pid, seq as u32);
    let Ok(event) = (unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(name.as_ptr())) }) else {
        return 1;
    };
    let event = OwnedHandle(event);
    let r = unsafe { WaitForMultipleObjects(&[process.0, event.0], false, INFINITE) };
    if r == WAIT_OBJECT_0 && unsafe { WaitForSingleObject(event.0, 0) } != WAIT_OBJECT_0 {
        log::warn!("202020 {pid} exited without cleanup");
        restore(&saved);
    }
    0
}
