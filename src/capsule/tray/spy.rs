//! A fake `Shell_TrayWnd` on its own thread. It sits above explorer's in
//! z-order so `Shell_NotifyIcon` finds it first, records every message, and
//! forwards everything to the real tray so explorer's state stays complete.

use std::cell::OnceCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use capsule_core::tray_proto::{self, COPYDATA_ICON_RECT, COPYDATA_NOTIFYICON, NIF_ICON, NotifyIconMessage, handle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, CopyIcon, CreateIconIndirect, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, FindWindowExW, GetIconInfo, GetMessageW, HICON, HWND_MESSAGE, HWND_TOPMOST, ICONINFO, MSG,
    MSGFLT_ALLOW, PostMessageW, PostQuitMessage, RegisterClassExW, SMTO_ABORTIFHUNG, SWP_ASYNCWINDOWPOS,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SendMessageTimeoutW, SendNotifyMessageW, SetWindowPos, WM_ACTIVATEAPP,
    WM_APP, WM_COMMAND, WM_COPYDATA, WM_DESTROY, WM_USER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use super::TrayKey;
use crate::capsule::shell::real_tray;
use crate::capsule::util::{hwnd, make_long, own_window};

pub struct TrayEvent {
    pub msg: NotifyIconMessage,
    /// Our own copy of the icon, or 0.
    pub icon: isize,
}

pub struct Shared {
    pub queue: Mutex<VecDeque<TrayEvent>>,
    /// Our own `TaskbarCreated` broadcast reaches the spy too; explorer must
    /// not get it forwarded.
    pub taskbar_created_msg: u32,
    pub rects: Mutex<Rects>,
    /// Window that receives `WM_APP_TRAY` when the queue becomes non-empty.
    pub notify: AtomicIsize,
    pub notify_msg: u32,
}

#[derive(Default)]
pub struct Rects {
    pub fallback: RECT,
    pub icons: Vec<(TrayKey, RECT)>,
}

thread_local! {
    static SHARED: OnceCell<Arc<Shared>> = const { OnceCell::new() };
}

fn shared() -> Option<Arc<Shared>> {
    SHARED.with(|s| s.get().cloned())
}

const SPY_QUIT: u32 = WM_APP + 0x3F0;
const SPY_RECLAIM: u32 = WM_APP + 0x3F1;

pub struct Spy {
    hwnd: Arc<AtomicIsize>,
    thread: Option<JoinHandle<()>>,
}

impl Spy {
    pub fn start(shared: Arc<Shared>, rect: RECT) -> Spy {
        let slot = Arc::new(AtomicIsize::new(0));
        let ready = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let (slot2, ready2) = (slot.clone(), ready.clone());
        let thread = std::thread::spawn(move || {
            SHARED.with(|s| s.set(shared)).ok();
            let h = create(rect);
            slot2.store(h.map_or(0, |h| h.0 as isize), Ordering::Release);
            *ready2.0.lock().unwrap() = true;
            ready2.1.notify_all();
            if h.is_some() {
                let mut msg = MSG::default();
                while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                    unsafe { DispatchMessageW(&msg) };
                }
            }
        });
        let mut done = ready.0.lock().unwrap();
        while !*done {
            done = ready.1.wait(done).unwrap();
        }
        drop(done);
        Spy { hwnd: slot, thread: Some(thread) }
    }

    pub fn hwnd(&self) -> HWND {
        hwnd(self.hwnd.load(Ordering::Acquire))
    }

    pub fn raise(&self, rect: RECT) {
        unsafe {
            let _ = SetWindowPos(
                self.hwnd(),
                Some(HWND_TOPMOST),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
            );
        }
    }

    /// Retakes the top of the `Shell_TrayWnd` z-order and asks apps to
    /// re-add their icons, which went to explorer while it was above us.
    /// Owners in `known` already have their icons here and are not asked;
    /// many apps re-add by deleting their icon first, which flickers it.
    pub fn reclaim(&self, known: Vec<u32>) {
        let known = Box::into_raw(Box::new(known));
        if unsafe { PostMessageW(Some(self.hwnd()), SPY_RECLAIM, WPARAM(0), LPARAM(known as isize)) }.is_err() {
            drop(unsafe { Box::from_raw(known) });
        }
    }
}

/// Includes message-only windows, which some apps use as their notify-icon
/// owner.
fn announce_taskbar(msg: u32, known: &[u32]) {
    for parent in [None, Some(HWND_MESSAGE)] {
        let mut prev = None;
        while let Ok(h) = unsafe { FindWindowExW(parent, prev, PCWSTR::null(), PCWSTR::null()) } {
            if !(parent.is_some() && own_window(h)) && !known.contains(&(h.0 as usize as u32)) {
                unsafe {
                    let _ = SendNotifyMessageW(h, msg, WPARAM(0), LPARAM(0));
                }
            }
            prev = Some(h);
        }
    }
}

impl Drop for Spy {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd()), SPY_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn create(rect: RECT) -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("Shell_TrayWnd"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let h = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("Shell_TrayWnd"),
            PCWSTR::null(),
            WS_POPUP,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        let _ = ChangeWindowMessageFilterEx(h, WM_COPYDATA, MSGFLT_ALLOW, None);
        Some(h)
    }
}

fn forward(msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(real) = real_tray() else { return LRESULT(0) };
    let mut result = 0usize;
    unsafe { SendMessageTimeoutW(real, msg, wparam, lparam, SMTO_ABORTIFHUNG, 500, Some(&mut result)) };
    LRESULT(result as isize)
}

fn icon_rect(q: &tray_proto::IconRectQuery) -> LRESULT {
    let Some(shared) = shared() else { return LRESULT(0) };
    let key = TrayKey { hwnd: q.hwnd, uid: q.uid, guid: q.guid };
    let rects = shared.rects.lock().unwrap();
    let r = rects.icons.iter().find(|(k, _)| k.same(&key)).map_or(rects.fallback, |(_, r)| *r);
    let packed = if q.message == 1 { make_long(r.left, r.top) } else { make_long(r.right, r.bottom) };
    LRESULT(packed as isize)
}

/// `CopyIcon` can fail for icons of other processes when it tries to reload
/// them from their module, so fall back to copying the bitmaps.
fn copy_icon(h: isize) -> isize {
    let h = HICON(h as *mut _);
    if let Ok(c) = unsafe { CopyIcon(h) } {
        return c.0 as isize;
    }
    let mut info = ICONINFO::default();
    if unsafe { GetIconInfo(h, &mut info) }.is_err() {
        log::debug!("tray icon {h:?} unreadable");
        return 0;
    }
    let copy = unsafe { CreateIconIndirect(&info) }.map_or(0, |c| c.0 as isize);
    unsafe {
        let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
    }
    copy
}

fn record(m: NotifyIconMessage) {
    let Some(shared) = shared() else { return };
    let icon = if m.data.flags & NIF_ICON != 0 && m.data.icon != 0 { copy_icon(handle(m.data.icon)) } else { 0 };
    let was_empty = {
        let mut q = shared.queue.lock().unwrap();
        let was_empty = q.is_empty();
        q.push_back(TrayEvent { msg: m, icon });
        was_empty
    };
    if was_empty {
        unsafe {
            let _ = PostMessageW(
                Some(hwnd(shared.notify.load(Ordering::Relaxed))),
                shared.notify_msg,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COPYDATA => {
            let cds = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
            let bytes: &[u8] = if cds.lpData.is_null() {
                &[]
            } else {
                unsafe { std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize) }
            };
            match cds.dwData {
                COPYDATA_NOTIFYICON => {
                    if let Some(m) = tray_proto::parse_notify_icon(bytes) {
                        record(m);
                    }
                    forward(msg, wparam, lparam);
                    LRESULT(1)
                }
                COPYDATA_ICON_RECT => match tray_proto::parse_icon_rect_query(bytes) {
                    Some(q) => icon_rect(&q),
                    None => LRESULT(0),
                },
                _ => forward(msg, wparam, lparam),
            }
        }
        SPY_RECLAIM => {
            let known = unsafe { Box::from_raw(lparam.0 as *mut Vec<u32>) };
            unsafe {
                let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
            }
            if let Some(shared) = shared() {
                announce_taskbar(shared.taskbar_created_msg, &known);
            }
            LRESULT(0)
        }
        SPY_QUIT => {
            unsafe {
                let _ = DestroyWindow(h);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        m if shared().is_some_and(|s| s.taskbar_created_msg == m) => LRESULT(0),
        WM_ACTIVATEAPP | WM_COMMAND => forward(msg, wparam, lparam),
        m if m >= WM_USER => forward(msg, wparam, lparam),
        _ => unsafe { DefWindowProcW(h, msg, wparam, lparam) },
    }
}
