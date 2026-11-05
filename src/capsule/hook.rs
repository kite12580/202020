//! The connection to `capsule_hook.dll` inside explorer, shared by the bar and
//! the launcher: the DLL stays attached while a `Hook` exists. A relay thread
//! injects it and receives what it sends back: tray icon messages for the bar,
//! and answers to shell work. The relay runs on its own thread because
//! explorer's tray thread waits for each relayed message.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;

use capsule_core::shell_proto::{self, Answer, Request};
use capsule_core::tray_proto::{
    self, COPYDATA_ICON_RECT, COPYDATA_NOTIFYICON, HOOK_ATTACH_MSG, HOOK_DETACH_MSG, HOOK_TRAY_MSG, NIF_ICON,
    NotifyIconMessage, handle,
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
    SMTO_ABORTIFHUNG, SendMessageTimeoutW, SendMessageW, SendNotifyMessageW, SetTimer, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_GETMESSAGE, WM_APP, WM_COPYDATA, WM_DESTROY, WM_TIMER, WNDCLASSEXW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, s, w};

use crate::capsule::shell::find_real_tray;
use crate::capsule::tray::TrayKey;
use crate::capsule::util::{hwnd, make_long, own_window, register_class, wide, window_ids};

const HOOK_DLL: &str = "capsule_hook.dll";

pub struct TrayEvent {
    pub msg: NotifyIconMessage,
    /// Our own copy of the icon, or 0.
    pub icon: isize,
}

/// What the bar shares with the relay to show explorer's tray icons.
pub struct TrayShared {
    pub queue: Mutex<VecDeque<TrayEvent>>,
    pub rects: Mutex<Rects>,
    /// Window that receives `notify_msg` when the queue becomes non-empty.
    pub notify: AtomicIsize,
    pub notify_msg: u32,
}

#[derive(Default)]
pub struct Rects {
    pub fallback: RECT,
    pub icons: Vec<(TrayKey, RECT)>,
}

pub enum HookEvent {
    /// The DLL attached to a `Shell_TrayWnd` and takes requests; requests
    /// made before are not answered.
    Attached,
    Answer(Answer),
}

struct Client {
    window: isize,
    msg: u32,
    events: VecDeque<HookEvent>,
}

impl Client {
    fn push(&mut self, ev: HookEvent) {
        self.events.push_back(ev);
        if self.events.len() == 1 {
            unsafe {
                let _ = PostMessageW(Some(hwnd(self.window)), self.msg, WPARAM(0), LPARAM(0));
            }
        }
    }
}

struct State {
    relay: isize,
    thread: Option<JoinHandle<()>>,
    clients: Vec<Client>,
    tray: Option<Arc<TrayShared>>,
    /// The `Shell_TrayWnd` the DLL is attached to.
    attached: Option<isize>,
}

static STATE: Mutex<State> =
    Mutex::new(State { relay: 0, thread: None, clients: Vec::new(), tray: None, attached: None });
/// Serializes starting and stopping the relay, which `STATE` cannot: the
/// relay thread needs `STATE` until it has stopped.
static LIFECYCLE: Mutex<()> = Mutex::new(());

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap()
}

/// Asks the attached DLL for shell work; `from` gets the answer as a
/// `HookEvent::Answer` if it holds a `Hook`. False when the DLL did not take
/// the request.
pub fn request(from: HWND, req: &Request) -> bool {
    let Some(tray) = state().attached else { return false };
    let bytes = req.encode();
    let cds = COPYDATASTRUCT {
        dwData: shell_proto::COPYDATA_REQUEST,
        cbData: bytes.len() as u32,
        lpData: bytes.as_ptr() as *mut _,
    };
    let mut result = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd(tray),
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

pub struct Hook {
    window: isize,
    tray: bool,
}

impl Hook {
    /// `window` gets `msg` when it has `HookEvent`s to take. Tray icon
    /// messages are relayed to `tray`, which only the bar passes.
    pub fn acquire(window: HWND, msg: u32, tray: Option<Arc<TrayShared>>) -> Hook {
        let _lifecycle = LIFECYCLE.lock().unwrap();
        let mut s = state();
        if s.thread.is_none() {
            let (relay, thread) = start_relay();
            s.relay = relay;
            s.thread = Some(thread);
            unsafe {
                let _ = PostMessageW(Some(hwnd(relay)), RELAY_ATTACH, WPARAM(0), LPARAM(0));
            }
        }
        let mut client = Client { window: window.0 as isize, msg, events: VecDeque::new() };
        if s.attached.is_some() {
            client.push(HookEvent::Attached);
        }
        s.clients.push(client);
        let hook = Hook { window: window.0 as isize, tray: tray.is_some() };
        if tray.is_some() {
            s.tray = tray;
            if s.attached.is_some() {
                unsafe {
                    let _ = PostMessageW(Some(hwnd(s.relay)), RELAY_TRAY, WPARAM(1), LPARAM(0));
                }
            }
        }
        hook
    }

    pub fn take_events(&self) -> Vec<HookEvent> {
        let mut s = state();
        let client = s.clients.iter_mut().find(|c| c.window == self.window);
        client.map(|c| c.events.drain(..).collect()).unwrap_or_default()
    }
}

/// Once the bar's `Hook` is gone, nothing more reaches its `TrayShared`.
impl Drop for Hook {
    fn drop(&mut self) {
        let _lifecycle = LIFECYCLE.lock().unwrap();
        let mut s = state();
        s.clients.retain(|c| c.window != self.window);
        if self.tray {
            s.tray = None;
        }
        if !s.clients.is_empty() {
            let relay = hwnd(s.relay);
            drop(s);
            if self.tray {
                unsafe { SendMessageW(relay, RELAY_TRAY, Some(WPARAM(0)), None) };
            }
            return;
        }
        let relay = hwnd(s.relay);
        let thread = s.thread.take();
        drop(s);
        unsafe {
            let _ = PostMessageW(Some(relay), RELAY_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(t) = thread {
            let _ = t.join();
        }
    }
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
struct Target {
    tray: Option<HWND>,
    injection: Option<Injection>,
}

thread_local! {
    static TARGET: RefCell<Target> = RefCell::default();
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

fn tray_msg() -> u32 {
    static ID: OnceLock<u32> = OnceLock::new();
    registered(&ID, HOOK_TRAY_MSG)
}

fn taskbar_created_msg() -> u32 {
    static ID: OnceLock<u32> = OnceLock::new();
    registered(&ID, "TaskbarCreated")
}

pub fn detach(tray: HWND, pid: u32) {
    unsafe {
        let _ = PostMessageW(Some(tray), detach_msg(), WPARAM(pid as usize), LPARAM(0));
    }
}

const REQUEST_TIMEOUT_MS: u32 = 500;
/// Attaches to the current `Shell_TrayWnd`.
const RELAY_ATTACH: u32 = WM_APP + 0x3F1;
/// Starts (wParam 1) or stops relaying tray icon messages.
const RELAY_TRAY: u32 = WM_APP + 0x3F2;
const RELAY_QUIT: u32 = WM_APP + 0x3F0;
const ATTACH_TIMER: usize = 1;
const ATTACH_TIMEOUT_MS: u32 = 5000;

fn start_relay() -> (isize, JoinHandle<()>) {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let thread = std::thread::spawn(move || {
        let h = create();
        if h.is_none() {
            log::error!("hook relay window creation failed");
        }
        let _ = tx.send(h.map_or(0, |h| h.0 as isize));
        if h.is_some() {
            let mut msg = MSG::default();
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                unsafe { DispatchMessageW(&msg) };
            }
        }
    });
    (rx.recv().unwrap_or(0), thread)
}

/// Top-level, unlike a message-only window, so it hears explorer's
/// `TaskbarCreated` broadcast after a restart.
fn create() -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        register_class(WNDCLASSEXW {
            lpfnWndProc: Some(wndproc),
            lpszClassName: w!("202020 hook relay"),
            ..Default::default()
        });
        let h = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("202020 hook relay"),
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        for m in [WM_COPYDATA, attach_msg(), taskbar_created_msg()] {
            let _ = ChangeWindowMessageFilterEx(h, m, MSGFLT_ALLOW, None);
        }
        Some(h)
    }
}

fn inject(relay: HWND, tray: HWND) -> Option<Injection> {
    let path = std::env::current_exe().ok()?.with_file_name(HOOK_DLL);
    let module = unsafe { LoadLibraryW(PCWSTR(wide(&path.to_string_lossy()).as_ptr())) }
        .inspect_err(|e| log::error!("hook: load {}: {e}", path.display()))
        .ok()?;
    let mut injection = Injection { hook: HHOOK::default(), module };
    let Some(proc) = (unsafe { GetProcAddress(module, s!("GetMsgProc")) }) else {
        log::error!("hook: {} has no GetMsgProc", path.display());
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
        log::error!("hook: Shell_TrayWnd {tray:?} is gone");
        return None;
    }
    injection.hook = unsafe { SetWindowsHookExW(WH_GETMESSAGE, proc, Some(module.into()), tid) }
        .inspect_err(|e| log::error!("hook: install: {e}"))
        .ok()?;
    unsafe { PostThreadMessageW(tid, attach_msg(), WPARAM(relay.0 as usize), LPARAM(tray.0 as isize)) }
        .inspect_err(|e| log::error!("hook: attach: {e}"))
        .ok()?;
    Some(injection)
}

/// Hooks the current `Shell_TrayWnd` unless already hooked.
fn attach(relay: HWND) {
    let Some(tray) = find_real_tray() else {
        log::error!("hook: no Shell_TrayWnd");
        return;
    };
    TARGET.with_borrow_mut(|t| {
        if t.tray == Some(tray) {
            return;
        }
        log::debug!("hook: attaching to {tray:?}");
        state().attached = None;
        if let Some(old) = t.tray.replace(tray) {
            detach(old, unsafe { GetCurrentProcessId() });
        }
        t.injection = inject(relay, tray);
        unsafe {
            SetTimer(Some(relay), ATTACH_TIMER, ATTACH_TIMEOUT_MS, None);
        }
    });
}

fn on_attached() {
    let Some(tray) = TARGET.with_borrow_mut(|t| t.injection.take().and(t.tray)) else { return };
    log::info!("hook attached");
    let mut s = state();
    s.attached = Some(tray.0 as isize);
    for c in &mut s.clients {
        c.push(HookEvent::Attached);
    }
    let tray_wanted = s.tray.is_some();
    drop(s);
    if tray_wanted {
        set_tray(true);
    }
}

/// Starting asks apps to re-add their icons, which the DLL then relays.
fn set_tray(on: bool) {
    let Some(tray) = state().attached else { return };
    unsafe {
        let pid = WPARAM(GetCurrentProcessId() as usize);
        SendMessageTimeoutW(hwnd(tray), tray_msg(), pid, LPARAM(on as isize), SMTO_ABORTIFHUNG, 500, None);
    }
    if on {
        announce_taskbar(taskbar_created_msg());
    }
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

fn tray_shared() -> Option<Arc<TrayShared>> {
    state().tray.clone()
}

fn icon_rect(q: &tray_proto::IconRectQuery) -> LRESULT {
    let Some(shared) = tray_shared() else { return LRESULT(0) };
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
    let Some(shared) = tray_shared() else { return };
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

fn answer(to: isize, a: Answer) {
    if let Some(c) = state().clients.iter_mut().find(|c| c.window == to) {
        c.push(HookEvent::Answer(a));
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
                        Some(a) => answer(wparam.0 as isize, a),
                        None => log::debug!("hook answer unparsed ({} bytes)", bytes.len()),
                    }
                    LRESULT(1)
                }
                _ => LRESULT(0),
            }
        }
        RELAY_ATTACH => {
            attach(h);
            LRESULT(0)
        }
        RELAY_TRAY => {
            set_tray(wparam.0 != 0);
            LRESULT(0)
        }
        m if m == attach_msg() => {
            unsafe {
                let _ = KillTimer(Some(h), ATTACH_TIMER);
            }
            on_attached();
            LRESULT(0)
        }
        m if m == taskbar_created_msg() => {
            attach(h);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == ATTACH_TIMER => {
            unsafe {
                let _ = KillTimer(Some(h), ATTACH_TIMER);
            }
            if TARGET.with_borrow_mut(|t| t.injection.take()).is_some() {
                log::error!("hook did not attach to explorer");
            }
            LRESULT(0)
        }
        RELAY_QUIT => {
            TARGET.with_borrow_mut(|t| {
                t.injection = None;
                if let Some(tray) = t.tray.take() {
                    detach(tray, unsafe { GetCurrentProcessId() });
                }
            });
            state().attached = None;
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
