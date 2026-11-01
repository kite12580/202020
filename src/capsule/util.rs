use capsule_core::layout::PxRect;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, POINT, PROPERTYKEY, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetWindowThreadProcessId};
use windows::core::PWSTR;

pub const PKEY_APP_USER_MODEL_ID: PROPERTYKEY =
    PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

pub fn hwnd(v: isize) -> HWND {
    HWND(v as *mut _)
}

pub fn lparam_point(l: LPARAM) -> (i32, i32) {
    (l.0 as i16 as i32, (l.0 >> 16) as i16 as i32)
}

pub fn make_long(lo: i32, hi: i32) -> u32 {
    (lo as u16 as u32) | ((hi as u16 as u32) << 16)
}

pub fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// `(thread id, process id)` of a window.
pub fn window_ids(h: HWND) -> (u32, u32) {
    let mut pid = 0;
    let tid = unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    (tid, pid)
}

pub struct OwnedHandle(pub HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

pub fn process_path(pid: u32) -> Option<String> {
    let process = OwnedHandle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?);
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    unsafe { QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }.ok()?;
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

pub fn px_rect(r: RECT) -> PxRect {
    PxRect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
}

pub fn primary_monitor() -> PxRect {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let mon = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let _ = GetMonitorInfoW(mon, &mut info);
    }
    px_rect(info.rcMonitor)
}

pub fn scale(dip: f32, dpi: u32) -> i32 {
    (dip * dpi as f32 / 96.0).round() as i32
}
