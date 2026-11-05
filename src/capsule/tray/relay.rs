//! Receives the tray icon messages explorer's `Shell_TrayWnd` gets, relayed by
//! `capsule_hook.dll` from inside explorer, and the answers to the shell
//! lookups the bar asks the DLL for. Runs on its own thread because
//! explorer's tray thread waits for each relayed message.

use std::cell::{OnceCell, RefCell};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

use capsule_core::shell_proto::{self, Answer, Request};
use capsule_core::tray_proto::{
    self, COPYDATA_ICON_RECT, COPYDATA_NOTIFYICON, HOOK_ATTACH_MSG, HOOK_DETACH_MSG, NIF_ICON, NotifyIconMessage,
    handle,
};
use windows::Win32::Foundation::{FreeLibrary, HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, CopyIcon, CreateIconIndirect, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, FindWindowExW, GetIconInfo, GetMessageW, HHOOK, HICON, HOOKPROC, HWND_MESSAGE, ICONINFO,
    KillTimer, MSG, MSGFLT_ALLOW, PostMessageW, PostQuitMessage, PostThreadMessageW, RegisterWindowMessageW,
    SMTO_ABORTIFHUNG, SendMessageTimeoutW, SendNotifyMessageW, SetTimer, SetWindowsHookExW, UnhookWindowsHookEx,
    WH_GETMESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_COPYDATA, WM_DESTROY, WM_TIMER, WNDCLASSEXW,
};
use windows::core::{PCWSTR, s, w};

use super::TrayKey;
use crate::capsule::util::{hwnd, make_long, own_window, register_class, wide, window_ids};

const HOOK_DLL: &str = "capsule_hook.dll";

pub struct TrayEvent {
    pub msg: NotifyIconMessage,
    /// Our own copy of the icon, or 0.
    pub icon: isize,
}

pub enum HookEvent {
    /// The DLL attached to a new `Shell_TrayWnd` and takes requests.
    Attached,
    Answer(Answer),
}

pub struct Shared {
    pub queue: Mutex<VecDeque<TrayEvent>>,
    pub taskbar_created_msg: u32,
    pub rects: Mutex<Rects>,
    /// Window that receives `WM_APP_TRAY` when the queue becomes non-empty.
    pub notify: AtomicIsize,
    pub notify_msg: u32,
    pub hook_events: Mutex<VecDeque<HookEvent>>,
    /// Posted to `notify` when `hook_events` becomes non-empty.
    pub hook_msg: u32,
}

#[derive(Default)]
pub struct Rects {
    pub fallback: RECT,
    pub icons: Vec<(TrayKey, RECT)>,
}

/// Must stay installed until the DLL confirms it attached: removing the hook
/// earlier can unload the DLL before explorer's thread handles the attach.
struct Injection {
    hook: HHOOK,
    module: HMODULE,
}

impl Drop for Injection {
    fn drop(&mut self) {
        unsafe {
            if !self.hook.is_invalid() {
                let _ = UnhookWindowsHookEx(self.hook);
            }
            let _ = FreeLibrary(self.module);
        }
    }
}

#[derive(Default)]
struct Hook {
    tray: Option<HWND>,
    injection: Option<Injection>,
}

thread_local! {
    static SHARED: OnceCell<Arc<Shared>> = const { OnceCell::new() };
    static HOOK: RefCell<Hook> = RefCell::default();
}

fn shared() -> Option<Arc<Shared>> {
    SHARED.with(|s| s.get().cloned())
}

fn registered(cell: &OnceLock<u32>, name: &str) -> u32 {
    *cell.get_or_init(|| unsafe { RegisterWindowMessageW(PCWSTR(wide(name).as_ptr())) })
}

fn attach_msg() -> u32 {
    static ID: OnceLock<u32> = OnceLock::new();
    registered(&ID, HOOK_ATTACH_MSG)
}

fn detach_msg() -> u32 {
    static ID: OnceLock<u32> = OnceLock::new();
    registered(&ID, HOOK_DETACH_MSG)
}

pub fn detach(tray: HWND, bar_pid: u32) {
    unsafe {
        let _ = PostMessageW(Some(tray), detach_msg(), WPARAM(bar_pid as usize), LPARAM(0));
    }
}

const REQUEST_TIMEOUT_MS: u32 = 500;

/// Asks the DLL attached to `tray` for a shell lookup, answered through
/// `HookEvent::Answer`. False when it did not take the request.
pub fn request(tray: HWND, from: HWND, req: &Request) -> bool {
    let bytes = req.encode();
    let cds = COPYDATASTRUCT {
        dwData: shell_proto::COPYDATA_REQUEST,
        cbData: bytes.len() as u32,
        lpData: bytes.as_ptr() as *mut _,
    };
    let mut result = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            tray,
            WM_COPYDATA,
            WPARAM(from.0 as usize),
            LPARAM(&cds as *const COPYDATASTRUCT as isize),
            SMTO_ABORTIFHUNG,
            REQUEST_TIMEOUT_MS,
            Some(&mut result),
        )
    };
    ok.0 != 0 && result == 1
}

const RELAY_QUIT: u32 = WM_APP + 0x3F0;
const RELAY_ATTACH: u32 = WM_APP + 0x3F1;
const ATTACH_TIMER: usize = 1;
const ATTACH_TIMEOUT_MS: u32 = 5000;

pub struct Relay {
    hwnd: isize,
    thread: Option<JoinHandle<()>>,
}

impl Relay {
    pub fn start(shared: Arc<Shared>) -> Relay {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || {
            SHARED.with(|s| s.set(shared)).ok();
            let h = create();
            if h.is_none() {
                log::error!("tray relay window creation failed");
            }
            let _ = tx.send(h.map_or(0, |h| h.0 as isize));
            if h.is_some() {
                let mut msg = MSG::default();
                while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                    unsafe { DispatchMessageW(&msg) };
                }
            }
        });
        Relay { hwnd: rx.recv().unwrap_or(0), thread: Some(thread) }
    }

    fn hwnd(&self) -> HWND {
        hwnd(self.hwnd)
    }

    /// Hooks explorer's `Shell_TrayWnd`, then asks apps to re-add their icons.
    pub fn attach(&self, tray: HWND) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd()), RELAY_ATTACH, WPARAM(tray.0 as usize), LPARAM(0));
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd()), RELAY_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn create() -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        register_class(WNDCLASSEXW {
            lpfnWndProc: Some(wndproc),
            lpszClassName: w!("202020 tray relay"),
            ..Default::default()
        });
        let h = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("202020 tray relay"),
            PCWSTR::null(),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        for m in [WM_COPYDATA, attach_msg()] {
            let _ = ChangeWindowMessageFilterEx(h, m, MSGFLT_ALLOW, None);
        }
        Some(h)
    }
}

fn inject(relay: HWND, tray: HWND) -> Option<Injection> {
    let path = std::env::current_exe().ok()?.with_file_name(HOOK_DLL);
    let module = unsafe { LoadLibraryW(PCWSTR(wide(&path.to_string_lossy()).as_ptr())) }
        .inspect_err(|e| log::error!("tray hook: load {}: {e}", path.display()))
        .ok()?;
    let mut injection = Injection { hook: HHOOK::default(), module };
    let Some(proc) = (unsafe { GetProcAddress(module, s!("GetMsgProc")) }) else {
        log::error!("tray hook: {} has no GetMsgProc", path.display());
        return None;
    };
    let proc: HOOKPROC = Some(unsafe {
        std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT,
        >(proc)
    });
    let tid = window_ids(tray).0;
    if tid == 0 {
        log::error!("tray hook: Shell_TrayWnd {tray:?} is gone");
        return None;
    }
    injection.hook = unsafe { SetWindowsHookExW(WH_GETMESSAGE, proc, Some(module.into()), tid) }
        .inspect_err(|e| log::error!("tray hook: install: {e}"))
        .ok()?;
    unsafe { PostThreadMessageW(tid, attach_msg(), WPARAM(relay.0 as usize), LPARAM(tray.0 as isize)) }
        .inspect_err(|e| log::error!("tray hook: attach: {e}"))
        .ok()?;
    Some(injection)
}

/// Includes message-only windows, which some apps use as their notify-icon
/// owner.
fn announce_taskbar(msg: u32) {
    let mut count = 0;
    for parent in [None, Some(HWND_MESSAGE)] {
        let mut prev = None;
        while let Ok(h) = unsafe { FindWindowExW(parent, prev, PCWSTR::null(), PCWSTR::null()) } {
            if !(parent.is_some() && own_window(h)) {
                unsafe {
                    let _ = SendNotifyMessageW(h, msg, WPARAM(0), LPARAM(0));
                }
                count += 1;
            }
            prev = Some(h);
        }
    }
    log::debug!("TaskbarCreated sent to {count} windows");
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

/// Posts `msg` to the bar when `queue` was empty.
fn push<T>(shared: &Shared, queue: &Mutex<VecDeque<T>>, item: T, msg: u32) {
    let was_empty = {
        let mut q = queue.lock().unwrap();
        let was_empty = q.is_empty();
        q.push_back(item);
        was_empty
    };
    if was_empty {
        unsafe {
            let _ = PostMessageW(Some(hwnd(shared.notify.load(Ordering::Relaxed))), msg, WPARAM(0), LPARAM(0));
        }
    }
}

fn record(m: NotifyIconMessage) {
    let Some(shared) = shared() else { return };
    let icon = if m.data.flags & NIF_ICON != 0 && m.data.icon != 0 { copy_icon(handle(m.data.icon)) } else { 0 };
    push(&shared, &shared.queue, TrayEvent { msg: m, icon }, shared.notify_msg);
}

fn hook_event(ev: HookEvent) {
    if let Some(shared) = shared() {
        push(&shared, &shared.hook_events, ev, shared.hook_msg);
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
                    match tray_proto::parse_notify_icon(bytes) {
                        Some(m) => record(m),
                        None => log::debug!("tray msg unparsed ({} bytes)", bytes.len()),
                    }
                    LRESULT(1)
                }
                COPYDATA_ICON_RECT => match tray_proto::parse_icon_rect_query(bytes) {
                    Some(q) => icon_rect(&q),
                    None => LRESULT(0),
                },
                shell_proto::COPYDATA_ANSWER => {
                    match Answer::decode(bytes) {
                        Some(a) => hook_event(HookEvent::Answer(a)),
                        None => log::debug!("hook answer unparsed ({} bytes)", bytes.len()),
                    }
                    LRESULT(1)
                }
                _ => LRESULT(0),
            }
        }
        RELAY_ATTACH => {
            let tray = hwnd(wparam.0 as isize);
            log::debug!("tray hook: attaching to {tray:?}");
            HOOK.with_borrow_mut(|hook| {
                if let Some(old) = hook.tray.replace(tray)
                    && old != tray
                {
                    detach(old, unsafe { GetCurrentProcessId() });
                }
                hook.injection = inject(h, tray);
            });
            unsafe {
                SetTimer(Some(h), ATTACH_TIMER, ATTACH_TIMEOUT_MS, None);
            }
            LRESULT(0)
        }
        m if m == attach_msg() => {
            unsafe {
                let _ = KillTimer(Some(h), ATTACH_TIMER);
            }
            if HOOK.with_borrow_mut(|hook| hook.injection.take()).is_some() {
                log::info!("tray hook attached");
                hook_event(HookEvent::Attached);
                if let Some(shared) = shared() {
                    announce_taskbar(shared.taskbar_created_msg);
                }
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == ATTACH_TIMER => {
            unsafe {
                let _ = KillTimer(Some(h), ATTACH_TIMER);
            }
            if HOOK.with_borrow_mut(|hook| hook.injection.take()).is_some() {
                log::error!("tray hook did not attach to explorer");
            }
            LRESULT(0)
        }
        RELAY_QUIT => {
            HOOK.with_borrow_mut(|hook| {
                hook.injection = None;
                if let Some(tray) = hook.tray.take() {
                    detach(tray, unsafe { GetCurrentProcessId() });
                }
            });
            unsafe {
                let _ = DestroyWindow(h);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(h, msg, wparam, lparam) },
    }
}
