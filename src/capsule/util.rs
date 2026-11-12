use std::ffi::c_void;

use capsule_core::launcher::folders::expand_env;
use capsule_core::layout::PxRect;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, POINT, PROPERTYKEY, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_ROUTINE_FLAGS, RegGetValueW};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOTOWNER, GWL_EXSTYLE, GetAncestor, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW,
    GetWindowThreadProcessId, HHOOK, HOOKPROC, IDC_ARROW, LoadCursorW, PostMessageW, RegisterClassExW,
    SetWindowsHookExW, WH_MOUSE_LL, WNDCLASSEXW, WS_EX_TOPMOST,
};
use windows::core::{PCWSTR, PWSTR};

pub const PKEY_APP_USER_MODEL_ID: PROPERTYKEY =
    PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn env(s: &str) -> String {
    expand_env(s, |n| std::env::var(n).ok())
}

pub fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// Converts and frees a CoTaskMem-allocated string.
pub fn take_string(s: PWSTR) -> Option<String> {
    let out = unsafe { s.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(s.0 as *const _)) };
    out
}

pub fn apps_folder(aumid: &str) -> String {
    format!(r"shell:AppsFolder\{aumid}")
}

pub fn hwnd(v: isize) -> HWND {
    HWND(v as *mut _)
}

pub fn root_owner(h: HWND) -> HWND {
    let r = unsafe { GetAncestor(h, GA_ROOTOWNER) };
    if r.is_invalid() { h } else { r }
}

pub fn post(owner: isize, msg: u32, wparam: usize) {
    unsafe {
        let _ = PostMessageW(Some(hwnd(owner)), msg, WPARAM(wparam), LPARAM(0));
    }
}

pub fn foreground() -> Option<HWND> {
    let h = unsafe { GetForegroundWindow() };
    (!h.is_invalid()).then_some(h)
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

pub fn own_window(h: HWND) -> bool {
    window_ids(h).1 == unsafe { GetCurrentProcessId() }
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

pub fn open_process(access: PROCESS_ACCESS_RIGHTS, pid: u32) -> Option<OwnedHandle> {
    unsafe { OpenProcess(access, false, pid) }.ok().map(OwnedHandle)
}

pub fn process_path(pid: u32) -> Option<String> {
    image_path(&open_process(PROCESS_QUERY_LIMITED_INFORMATION, pid)?)
}

pub fn image_path(process: &OwnedHandle) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    unsafe { QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }.ok()?;
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

pub fn px_rect(r: RECT) -> PxRect {
    PxRect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
}

pub fn primary_monitor_rect() -> RECT {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let mon = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let _ = GetMonitorInfoW(mon, &mut info);
    }
    info.rcMonitor
}

pub fn primary_monitor() -> PxRect {
    px_rect(primary_monitor_rect())
}

pub fn scale(dip: f32, dpi: u32) -> i32 {
    (dip * dpi as f32 / 96.0).round() as i32
}

pub fn dpi(h: HWND) -> u32 {
    unsafe { GetDpiForWindow(h) }.max(96)
}

pub fn topmost(h: HWND) -> bool {
    let ex = unsafe { GetWindowLongPtrW(h, GWL_EXSTYLE) };
    ex & WS_EX_TOPMOST.0 as isize != 0
}

pub fn mouse_hook(proc: HOOKPROC) -> Option<HHOOK> {
    let module = unsafe { GetModuleHandleW(None) }.ok().map(Into::into);
    unsafe { SetWindowsHookExW(WH_MOUSE_LL, proc, module, 0) }.ok()
}

/// Registers `class` for this module, with an arrow cursor.
pub fn register_class(class: WNDCLASSEXW) {
    unsafe {
        RegisterClassExW(&WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..class
        });
    }
}

pub fn reg_value(key: PCWSTR, name: PCWSTR, kind: REG_ROUTINE_FLAGS, buf: &mut [u8]) -> Option<usize> {
    let mut size = buf.len() as u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key, name, kind, None, Some(buf.as_mut_ptr() as *mut c_void), Some(&mut size))
            .ok()
            .ok()?;
    }
    Some(size as usize)
}

pub struct View(pub MEMORY_MAPPED_VIEW_ADDRESS);

impl Drop for View {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.0);
        }
    }
}
