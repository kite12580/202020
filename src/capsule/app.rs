use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use capsule_core::config::CapsuleBar;
use capsule_core::layout::{self, Hit, Input, Layout, METRICS, Rect};
use capsule_core::taskband::{Pin, Task};
use capsule_core::{clock, kbd};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Globalization::{DATE_LONGDATE, GetDateFormatEx, GetTimeFormatEx, TIME_FORMAT_FLAGS};
use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP, HGDIOBJ, InvalidateRect, ScreenToClient, ValidateRect};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    HPOWERNOTIFY, POWERBROADCAST_SETTING, RegisterPowerSettingNotification, UnregisterPowerSettingNotification,
};
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Controls::{
    HOVER_DEFAULT, ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, WM_MOUSEHOVER, WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_HOVER, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE,
};
use windows::Win32::UI::Shell::{
    ABM_NEW, ABM_REMOVE, APPBARDATA, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHAppBarMessage,
    SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, ChangeWindowMessageFilterEx, CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW,
    DeregisterShellHookWindow, DestroyWindow, DispatchMessageW, EndMenu, GA_ROOTOWNER, GWL_STYLE, GetAncestor,
    GetCursorPos, GetForegroundWindow, GetSystemMetrics, GetWindowLongW, GetWindowRect, HICON, HWND_TOPMOST, IsZoomed,
    KillTimer, MA_NOACTIVATE, MSG, MSGFLT_ALLOW, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE,
    PeekMessageW, PostMessageW, PostQuitMessage, QS_ALLINPUT, RegisterShellHookWindow, RegisterWindowMessageW,
    SM_CXDRAG, SM_CYDRAG, SPI_SETHIGHCONTRAST, SPI_SETWORKAREA, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetCoalescableTimer, SetForegroundWindow, SetTimer,
    SetWindowPos, ShowWindow, TranslateMessage, WA_INACTIVE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    WM_ACTIVATE, WM_APP, WM_CAPTURECHANGED, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_ERASEBKGND,
    WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEACTIVATE,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCACTIVATE, WM_PAINT, WM_POWERBROADCAST, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SETTINGCHANGE, WM_TIMECHANGE, WM_TIMER, WM_WTSSESSION_CHANGE, WNDCLASSEXW, WS_CAPTION, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{GUID, PCWSTR, w};

use crate::capsule::config_store::ConfigStore;
use crate::capsule::gfx::{Color, Font, Gfx, Image, Surface, Text};
use crate::capsule::icons::IconWorker;
use crate::capsule::keyboard;
use crate::capsule::keyhook::{self, KeyHook};
use crate::capsule::launcher;
use crate::capsule::menu::{self, Item};
use crate::capsule::pinned::{self, PinnedApp};
use crate::capsule::popup::Popup;
use crate::capsule::shell::{self, REAL_TRAY, Takeover};
use crate::capsule::theme::{self, Theme};
use crate::capsule::tooltip::Tooltip;
use crate::capsule::tray::relay::{Rects, Relay, Shared};
use crate::capsule::tray::{self, Change, NIN_POPUPCLOSE, NIN_POPUPOPEN, Tray, TrayKey};
use crate::capsule::util::{
    apps_folder, class_name, from_wide, hwnd, lparam_point, own_window, primary_monitor, register_class, scale,
};
use crate::capsule::volume::Volume;
use crate::capsule::windows_list::{self as winlist, IconRank, WindowList};

#[derive(Clone, Copy)]
pub struct Options {
    pub safe: bool,
    /// Window that receives the `*_msg` messages.
    pub owner: isize,
    pub exited_msg: u32,
    pub disable_msg: u32,
    pub restart_msg: u32,
}

impl Options {
    pub fn post_owner(&self, msg: u32) {
        unsafe {
            let _ = PostMessageW(Some(hwnd(self.owner)), msg, WPARAM(0), LPARAM(0));
        }
    }
}

const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_VOLUME: u32 = WM_APP + 2;
const WM_APP_AUDIO_DEVICE: u32 = WM_APP + 3;
const WM_APP_ICON: u32 = WM_APP + 4;
const WM_APP_APPBAR: u32 = WM_APP + 5;
const WM_APP_WINEVENT: u32 = WM_APP + 6;
const WM_APP_WIN_DIGIT: u32 = WM_APP + 7;
const WM_APP_INPUT: u32 = WM_APP + 8;
pub const WM_APP_STOP: u32 = WM_APP + 9;
pub const WM_APP_BREAK: u32 = WM_APP + 10;
const WM_APP_REPOSITION: u32 = WM_APP + 11;
pub const WM_APP_RELOAD_CONFIG: u32 = WM_APP + 12;

pub static STOP: AtomicBool = AtomicBool::new(false);
pub static BREAK: AtomicBool = AtomicBool::new(false);

const TIMER_CLOCK: usize = 1;
const TIMER_SLOW: usize = 2;
const TIMER_CONFIG: usize = 3;
const TIMER_KEYBOARD: usize = 4;
const TIMER_WORK_AREA: usize = 5;
const TIMER_AUDIO: usize = 6;
const TIMER_WIN_DIGIT: usize = 7;
const TIMER_REVEAL: usize = 8;
const TIMER_PINS: usize = 9;
const TIMER_ICONS: usize = 10;

const PRUNE_EVERY: u32 = 5;

const WINDOW_ICON: f32 = 20.0;
const TASK_ICON_LIFT: f32 = 1.5;
const TRAY_ICON: f32 = 16.0;
const POPUP_GAP: f32 = 6.0;
const START_LOGO: f32 = 16.0;
const START_LOGO_GAP: f32 = 1.0;
const SNAP: f32 = 12.0;
const START_LOGO_FROM: Color = Color(0.29, 0.79, 1.0, 1.0);
const START_LOGO_TO: Color = Color(0.0, 0.40, 0.80, 1.0);
const CLOCK_LINE_OVERLAP: f32 = 2.0;
const ELEVATED_HINT: &str = "This program runs as administrator. Run 202020 as administrator to use its icon.";

const ABN_FULLSCREENAPP: usize = 2;
const WTS_SESSION_LOCK: usize = 7;
const WTS_SESSION_UNLOCK: usize = 8;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
const PBT_POWERSETTINGCHANGE: usize = 0x8013;
const GUID_CONSOLE_DISPLAY_STATE: GUID = GUID::from_u128(0x6fe69556_704a_47a0_8f24_c28d936fda47);

const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const EVENT_OBJECT_FOCUS: u32 = 0x8005;
const EVENT_OBJECT_CLOAKED: u32 = 0x8017;
const EVENT_OBJECT_UNCLOAKED: u32 = 0x8018;

const FRAME_CLASS: &str = "ApplicationFrameWindow";
const CORE_WINDOW_CLASS: &str = "Windows.UI.Core.CoreWindow";

const MENU_RESTART: u32 = 1;
const MENU_QUIT: u32 = 2;

const GLYPH_CHEVRON: &str = "\u{E70E}";
const GLYPH_MUTE: &str = "\u{E74F}";
const GLYPH_VOLUME: [&str; 4] = ["\u{E992}", "\u{E993}", "\u{E994}", "\u{E995}"];

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app unless it is already borrowed further up the stack,
/// which happens when a handler makes a call that dispatches messages.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| cell.try_borrow_mut().ok().and_then(|mut a| a.as_mut().map(f)))
}

/// Menus pump messages and so must run with the app unborrowed.
enum DeferredMenu {
    App,
    Keyboard,
    Tray(usize),
}

type IconKey = (String, u32);

enum IconTarget {
    Window(HWND, IconRank),
    Tray(TrayKey),
    Pin(String),
}

struct Drag {
    cursor: POINT,
    origin: POINT,
    moved: bool,
    hit: Option<Hit>,
    double: bool,
}

#[derive(Default)]
struct Texts {
    keyboard: Option<Text>,
    chevron: Option<Text>,
    volume_glyph: Option<Text>,
    clock_time: Option<Text>,
    clock_date: Option<Text>,
    keyboard_slot: f32,
    volume_slot: f32,
    clock_slot: f32,
}

pub struct App {
    hwnd: HWND,
    opts: Options,
    cfg: ConfigStore,
    gfx: Gfx,
    surface: Option<Surface>,
    theme: Theme,
    dpi: u32,
    layout: Layout,
    bounds: RECT,
    bar_tray: Vec<usize>,
    windows: WindowList,
    pins: Vec<PinnedApp>,
    tasks: Vec<Task>,
    tray: Tray,
    tray_shared: Arc<Shared>,
    relay: Option<Relay>,
    takeover: Option<Takeover>,
    volume: Volume,
    keyhook: Option<KeyHook>,
    pin_watch: Option<pinned::Watch>,
    keyboard_layout: isize,
    keyboard_iso: String,
    keyboard_label: String,
    keyboard_retries: u8,
    ime_native: Option<bool>,
    ime_query: usize,
    volume_glyph: &'static str,
    pending_win_digits: Vec<u16>,
    last_win_digit: Option<Instant>,
    clock: (String, String),
    texts: Texts,
    icon_worker: IconWorker,
    icon_requests: HashMap<u64, IconKey>,
    icon_waiters: HashMap<IconKey, Vec<IconTarget>>,
    icon_cache: HashMap<IconKey, Image>,
    next_icon_request: u64,
    icon_refresh: Vec<HWND>,
    foreground: HWND,
    focus: HWND,
    hover: Option<Hit>,
    drag: Option<Drag>,
    wheel: i32,
    double_click: bool,
    tray_popup_open: Option<TrayKey>,
    tooltip: Tooltip,
    popup: Popup,
    appbar_fullscreen: bool,
    fullscreen: bool,
    hidden: bool,
    locked: bool,
    display_off: bool,
    hooks: Vec<HWINEVENTHOOK>,
    power_notify: Option<HPOWERNOTIFY>,
    reposition_pending: bool,
    shell_msg: u32,
    taskbar_created_msg: u32,
    last_work_area_fix: Option<Instant>,
    deferred: VecDeque<DeferredMenu>,
    slow_ticks: u32,
    stopped: bool,
}

/// Runs the bar on the calling thread until `STOP` is set. `ready` gets the
/// bar window, or `None` if it could not be created.
pub fn run(opts: Options, ready: impl FnOnce(Option<HWND>)) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES,
        });
    }
    menu::allow_dark_menus();
    let windows = create_windows();
    ready(windows.map(|(bar, _)| bar));
    let Some((bar, popup)) = windows else {
        log::error!("window creation failed");
        unsafe { CoUninitialize() };
        return;
    };
    APP.with(|c| *c.borrow_mut() = Some(App::new(bar, popup, opts)));
    with_app(App::start);
    message_loop();
    with_app(App::stop);
    APP.with(|c| c.borrow_mut().take());
    unsafe {
        let _ = DestroyWindow(bar);
        CoUninitialize();
    }
}

fn create_windows() -> Option<(HWND, HWND)> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        for (name, proc_) in [
            (w!("CapsuleBar"), bar_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT),
            (w!("CapsuleBarPopup"), popup_proc),
        ] {
            register_class(WNDCLASSEXW {
                style: CS_DBLCLKS,
                lpfnWndProc: Some(proc_),
                lpszClassName: name,
                ..Default::default()
            });
        }
        let bar = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            w!("CapsuleBar"),
            w!("capsule-bar"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        let popup = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("CapsuleBarPopup"),
            w!("capsule-bar tray"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            Some(bar),
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        Some((bar, popup))
    }
}

fn message_loop() {
    let mut msg = MSG::default();
    while !STOP.load(Ordering::Relaxed) {
        let watches =
            with_app(|a| [a.cfg.watch_handle(), a.pin_watch.as_ref().map_or_else(HANDLE::default, |w| w.handle())])
                .unwrap_or_default();
        let handles: Vec<HANDLE> = watches.into_iter().filter(|h| !h.is_invalid()).collect();
        let r = unsafe { MsgWaitForMultipleObjectsEx(Some(&handles), u32::MAX, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
        if let Some(&signaled) = handles.get(r.0.wrapping_sub(WAIT_OBJECT_0.0) as usize) {
            if signaled == watches[0] {
                with_app(App::on_config_signal);
            } else {
                with_app(App::on_pins_signal);
            }
            continue;
        }
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                return;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

fn run_deferred() {
    while let Some(d) = with_app(|a| a.deferred.pop_front()).flatten() {
        match d {
            DeferredMenu::App => app_menu(),
            DeferredMenu::Keyboard => keyboard_menu(),
            DeferredMenu::Tray(i) => tray_menu(i),
        }
    }
}

unsafe extern "system" fn bar_proc(h: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCACTIVATE => return unsafe { DefWindowProcW(h, msg, WPARAM(1), lparam) },
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => return LRESULT(1),
        _ => {}
    }
    let r = with_app(|a| a.bar_message(msg, wparam, lparam)).flatten();
    run_deferred();
    r.unwrap_or_else(|| unsafe { DefWindowProcW(h, msg, wparam, lparam) })
}

unsafe extern "system" fn popup_proc(h: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCACTIVATE => return unsafe { DefWindowProcW(h, msg, WPARAM(1), lparam) },
        WM_ERASEBKGND => return LRESULT(1),
        _ => {}
    }
    let r = with_app(|a| a.popup_message(msg, wparam, lparam)).flatten();
    run_deferred();
    r.unwrap_or_else(|| unsafe { DefWindowProcW(h, msg, wparam, lparam) })
}

unsafe extern "system" fn on_winevent(
    _hook: HWINEVENTHOOK,
    event: u32,
    h: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    if h.is_invalid() || (event != EVENT_OBJECT_FOCUS && (object != 0 || child != 0)) {
        return;
    }
    if with_app(|a| a.on_winevent(event, h)).is_none() {
        let bar = BAR.load(Ordering::Relaxed);
        unsafe {
            let _ = PostMessageW(Some(hwnd(bar)), WM_APP_WINEVENT, WPARAM(event as usize), LPARAM(h.0 as isize));
        }
    }
}

unsafe extern "system" fn on_ime_open(ime: HWND, _msg: u32, query: usize, open: LRESULT) {
    with_app(|a| a.on_ime_open(ime, query, open.0 != 0));
}

unsafe extern "system" fn on_ime_mode(_ime: HWND, _msg: u32, query: usize, mode: LRESULT) {
    with_app(|a| a.on_ime_mode(query, keyboard::native_mode(mode.0 as usize)));
}

unsafe extern "system" fn on_window_icon(h: HWND, _msg: u32, small: usize, result: LRESULT) {
    with_app(|a| a.on_window_icon(h, small != 0, result.0));
}

static BAR: AtomicIsize = AtomicIsize::new(0);

fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

/// Shell flyouts such as the input switcher and the IME mode indicator are
/// top-level CoreWindows that take the foreground and can cover the whole
/// monitor without being fullscreen apps.
fn shell_flyout(h: HWND) -> bool {
    class_name(h) == CORE_WINDOW_CLASS
}

/// `None` while a shell flyout is in front, which says nothing about the app
/// behind it.
fn fullscreen_foreground() -> Option<bool> {
    if let Ok(state) = unsafe { SHQueryUserNotificationState() }
        && matches!(state, QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)
    {
        return Some(true);
    }
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_invalid() || own_window(fg) || unsafe { IsZoomed(fg) }.as_bool() {
        return Some(false);
    }
    match class_name(fg).as_str() {
        "Progman" | "WorkerW" => return Some(false),
        CORE_WINDOW_CLASS => return None,
        _ => {}
    }
    let style = unsafe { GetWindowLongW(fg, GWL_STYLE) } as u32;
    if style & WS_CAPTION.0 == WS_CAPTION.0 {
        return Some(false);
    }
    let mut r = RECT::default();
    if unsafe { GetWindowRect(fg, &mut r) }.is_err() {
        return Some(false);
    }
    let m = primary_monitor();
    Some(r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom)
}

fn long_date() -> String {
    let st = unsafe { GetLocalTime() };
    let mut buf = [0u16; 128];
    let fmt = |flags, pattern: PCWSTR, buf: &mut [u16; 128]| {
        unsafe { GetDateFormatEx(PCWSTR::null(), flags, Some(&st), pattern, Some(buf), PCWSTR::null()) };
        from_wide(buf)
    };
    let date = fmt(DATE_LONGDATE, PCWSTR::null(), &mut buf);
    let weekday = fmt(Default::default(), w!("dddd"), &mut buf);
    unsafe { GetTimeFormatEx(PCWSTR::null(), TIME_FORMAT_FLAGS(0), Some(&st), PCWSTR::null(), Some(&mut buf)) };
    let time = from_wide(&buf);
    if date.contains(&weekday) { format!("{date} {time}") } else { format!("{date} {weekday} {time}") }
}

impl App {
    fn new(hwnd: HWND, popup: HWND, opts: Options) -> App {
        BAR.store(hwnd.0 as isize, Ordering::Relaxed);
        let cfg = ConfigStore::open();
        let theme = Theme::read();
        let taskbar_created_msg = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        let tray_shared = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            taskbar_created_msg,
            rects: Mutex::new(Rects::default()),
            notify: AtomicIsize::new(hwnd.0 as isize),
            notify_msg: WM_APP_TRAY,
        });
        App {
            hwnd,
            gfx: Gfx::new(),
            surface: None,
            dpi: unsafe { GetDpiForWindow(hwnd) }.max(96),
            layout: Layout::default(),
            bounds: RECT::default(),
            bar_tray: Vec::new(),
            windows: WindowList::default(),
            pins: Vec::new(),
            tasks: Vec::new(),
            tray: Tray::default(),
            tray_shared,
            relay: None,
            takeover: None,
            volume: Volume::new(hwnd.0 as isize, WM_APP_VOLUME, WM_APP_AUDIO_DEVICE),
            keyhook: None,
            pin_watch: pinned::Watch::new(),
            keyboard_layout: 0,
            keyboard_iso: String::new(),
            keyboard_label: String::new(),
            keyboard_retries: 0,
            ime_native: None,
            ime_query: 0,
            volume_glyph: "",
            pending_win_digits: Vec::new(),
            last_win_digit: None,
            clock: Default::default(),
            texts: Texts::default(),
            icon_worker: IconWorker::new(hwnd.0 as isize, WM_APP_ICON),
            icon_requests: HashMap::new(),
            icon_waiters: HashMap::new(),
            icon_cache: HashMap::new(),
            next_icon_request: 1,
            icon_refresh: Vec::new(),
            foreground: HWND::default(),
            focus: HWND::default(),
            hover: None,
            drag: None,
            wheel: 0,
            double_click: false,
            tray_popup_open: None,
            tooltip: Tooltip::new(hwnd),
            popup: Popup::new(popup),
            appbar_fullscreen: false,
            fullscreen: false,
            hidden: false,
            locked: false,
            display_off: false,
            hooks: Vec::new(),
            power_notify: None,
            reposition_pending: false,
            shell_msg: unsafe { RegisterWindowMessageW(w!("SHELLHOOK")) },
            taskbar_created_msg,
            last_work_area_fix: None,
            deferred: VecDeque::new(),
            slow_ticks: 0,
            stopped: false,
            theme,
            cfg,
            opts,
        }
    }

    fn start(&mut self) {
        let h = self.hwnd;
        unsafe {
            for m in [self.taskbar_created_msg, self.shell_msg, WM_APP_APPBAR] {
                let _ = ChangeWindowMessageFilterEx(h, m, MSGFLT_ALLOW, None);
            }
        }
        theme::apply(h, self.cfg.config.appearance.backdrop, &self.theme);
        theme::apply(self.popup.hwnd, self.cfg.config.appearance.backdrop, &self.theme);
        self.tooltip.set_dark(self.theme.dark);

        let real = shell::find_real_tray();
        REAL_TRAY.store(real.map_or(0, |h| h.0 as isize), Ordering::Relaxed);
        if !self.opts.safe
            && let Some(real) = real
        {
            self.takeover = Takeover::start(real);
        }

        self.set_pins(pinned::read());
        self.windows.enumerate();
        self.foreground = self.root(unsafe { GetForegroundWindow() });
        for i in 0..self.windows.items.len() {
            self.request_icon(self.windows.items[i].hwnd);
        }
        self.refresh_keyboard();
        self.refresh_volume();
        self.refresh_clock();
        self.rebuild_static_texts();
        self.relayout();
        self.reposition();

        self.relay = Some(Relay::start(self.tray_shared.clone()));
        self.keyhook = KeyHook::start(h, WM_APP_WIN_DIGIT, WM_APP_INPUT);
        if let Some(k) = &self.keyhook {
            k.set_digits(self.takeover.is_some());
        }

        unsafe {
            let _ = RegisterShellHookWindow(h);
            for (min, max) in [
                (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
                (EVENT_OBJECT_FOCUS, EVENT_OBJECT_FOCUS),
                (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
            ] {
                let hook = SetWinEventHook(
                    min,
                    max,
                    None,
                    Some(on_winevent),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                );
                if !hook.is_invalid() {
                    self.hooks.push(hook);
                }
            }
            let _ = WTSRegisterSessionNotification(h, NOTIFY_FOR_THIS_SESSION);
            self.power_notify =
                RegisterPowerSettingNotification(HANDLE(h.0), &GUID_CONSOLE_DISPLAY_STATE, DEVICE_NOTIFY_WINDOW_HANDLE)
                    .ok();
            let _ = SetCoalescableTimer(Some(h), TIMER_SLOW, 2000, None, 1000);
        }
        self.register_appbar();
        if let (Some(relay), Some(real)) = (&self.relay, real) {
            relay.attach(real);
        }
        unsafe {
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            SendMessageW(h, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(0)));
        }
        self.update_visibility();
        log::info!("started: {} windows, dpi {}", self.windows.items.len(), self.dpi);
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        unsafe {
            let mut d = APPBARDATA { cbSize: size_of::<APPBARDATA>() as u32, hWnd: self.hwnd, ..Default::default() };
            SHAppBarMessage(ABM_REMOVE, &mut d);
            for hook in self.hooks.drain(..) {
                let _ = UnhookWinEvent(hook);
            }
            let _ = DeregisterShellHookWindow(self.hwnd);
            let _ = WTSUnRegisterSessionNotification(self.hwnd);
            if let Some(n) = self.power_notify.take() {
                let _ = UnregisterPowerSettingNotification(n);
            }
        }
        self.relay = None;
        for ev in self.tray_shared.queue.lock().unwrap().drain(..) {
            tray::destroy_icon(ev.icon);
        }
        self.keyhook = None;
        if let Some(t) = self.takeover.take() {
            t.stop();
        }
        log::info!("stopped");
    }

    fn register_appbar(&self) {
        let mut d = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: self.hwnd,
            uCallbackMessage: WM_APP_APPBAR,
            ..Default::default()
        };
        unsafe { SHAppBarMessage(ABM_NEW, &mut d) };
    }

    fn root(&self, h: HWND) -> HWND {
        if h.is_invalid() {
            return h;
        }
        let r = unsafe { GetAncestor(h, GA_ROOTOWNER) };
        if r.is_invalid() { h } else { r }
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn invalidate_popup(&self) {
        if self.popup.open {
            unsafe {
                let _ = InvalidateRect(Some(self.popup.hwnd), None, false);
            }
        }
    }

    fn px(&self, dip: f32) -> i32 {
        scale(dip, self.dpi)
    }

    fn to_dip(&self, px: i32) -> f32 {
        px as f32 * 96.0 / self.dpi as f32
    }

    fn rebuild_static_texts(&mut self) {
        let t = &mut self.texts;
        t.chevron = self.gfx.text(GLYPH_CHEVRON, Font::Glyph);
        t.clock_slot = clock::WIDTH_CANDIDATES
            .iter()
            .filter_map(|s| self.gfx.text(s, Font::Clock))
            .map(|t| t.width.ceil())
            .fold(0.0, f32::max);
        let glyph = GLYPH_VOLUME.iter().chain([&GLYPH_MUTE]).filter_map(|g| self.gfx.text(g, Font::Glyph));
        t.volume_slot = glyph.map(|t| t.width).fold(0.0, f32::max).ceil();
        self.measure_keyboard_slot();
    }

    /// Widest label any installed layout can show, so switching layouts or
    /// IME modes never resizes the bar.
    fn measure_keyboard_slot(&mut self) {
        let overrides = &self.cfg.config.keyboard.labels;
        let mut labels = Vec::new();
        for hkl in keyboard::installed() {
            let iso = keyboard::iso(hkl);
            labels.push(kbd::label(&iso, overrides, None));
            if kbd::has_ime(&iso) {
                labels.push(kbd::label(&iso, overrides, Some(false)));
            }
        }
        self.texts.keyboard_slot =
            labels.iter().filter_map(|l| self.gfx.text(l, Font::Text)).map(|t| t.width.ceil()).fold(0.0, f32::max);
    }

    fn refresh_clock(&mut self) {
        let t = unsafe { GetLocalTime() };
        let s = (clock::time(t.wHour, t.wMinute), clock::date(t.wYear, t.wMonth, t.wDay));
        if s != self.clock {
            self.texts.clock_time = self.gfx.text(&s.0, Font::Clock);
            self.texts.clock_date = self.gfx.text(&s.1, Font::Clock);
            self.clock = s;
            self.invalidate();
        }
        if !self.paused() {
            unsafe {
                SetTimer(
                    Some(self.hwnd),
                    TIMER_CLOCK,
                    clock::ms_until_next_minute(t.wSecond, t.wMilliseconds) + 20,
                    None,
                );
            }
        }
    }

    fn input_target(&self) -> HWND {
        let fg = unsafe { GetForegroundWindow() };
        if fg.is_invalid() || own_window(fg) || shell_flyout(fg) { self.foreground } else { fg }
    }

    fn refresh_keyboard(&mut self) {
        let target = self.input_target();
        let hkl = keyboard::current(target);
        if hkl == 0 {
            return;
        }
        if hkl != self.keyboard_layout {
            self.keyboard_layout = hkl;
            self.keyboard_iso = keyboard::iso(hkl);
            self.ime_native = None;
        }
        self.ime_query = self.ime_query.wrapping_add(1);
        if !kbd::has_ime(&self.keyboard_iso) || !keyboard::query_ime_open(target, self.ime_query, on_ime_open) {
            self.ime_native = None;
        }
        self.update_keyboard_label();
    }

    fn on_ime_open(&mut self, ime: HWND, query: usize, open: bool) {
        if query != self.ime_query {
            return;
        }
        if !open {
            self.ime_native = Some(false);
        } else if !keyboard::query_ime_mode(ime, query, on_ime_mode) {
            self.ime_native = None;
        }
        self.update_keyboard_label();
    }

    fn on_ime_mode(&mut self, query: usize, native: bool) {
        if query == self.ime_query {
            self.ime_native = Some(native);
            self.update_keyboard_label();
        }
    }

    fn update_keyboard_label(&mut self) {
        let label = kbd::label(&self.keyboard_iso, &self.cfg.config.keyboard.labels, self.ime_native);
        if label != self.keyboard_label {
            log::debug!("keyboard label {label}");
            self.texts.keyboard = self.gfx.text(&label, Font::Text);
            self.keyboard_label = label;
            self.relayout();
        }
    }

    fn refresh_volume(&mut self) {
        let level = self.volume.level();
        let glyph = if !self.volume.available() || self.volume.muted() {
            GLYPH_MUTE
        } else {
            GLYPH_VOLUME[match level {
                l if l < 0.005 => 0,
                l if l < 0.34 => 1,
                l if l < 0.67 => 2,
                _ => 3,
            }]
        };
        if glyph != self.volume_glyph {
            self.texts.volume_glyph = self.gfx.text(glyph, Font::Glyph);
            self.volume_glyph = glyph;
            self.invalidate();
        }
        if self.hover == Some(Hit::Volume) {
            self.show_tip();
        }
    }

    fn relayout(&mut self) {
        self.bar_tray = self.tray.bar_icons(&self.cfg.config.tray.pinned);
        self.tasks = self.windows.tasks();
        let input = Input {
            tasks: self.tasks.len(),
            tray: self.bar_tray.len(),
            chevron: !self.tray.popup_icons().is_empty(),
            keyboard_w: self.texts.keyboard.as_ref().map_or(0.0, |t| t.width.ceil()).max(self.texts.keyboard_slot),
            volume_w: self.texts.volume_slot,
            clock_w: self.texts.clock_slot,
        };
        let layout = Layout::compute(&METRICS, &input);
        let resized = (layout.width - self.layout.width).abs() > 0.01;
        self.layout = layout;
        if self.hover.is_some_and(|h| self.layout.rect(h).is_none()) {
            self.set_hover(None);
        }
        if resized {
            self.schedule_reposition();
        } else {
            self.publish_tray_rects();
        }
        self.invalidate();
    }

    /// Coalesces the resizes from a burst of window events, such as a
    /// virtual desktop switch cloaking and uncloaking every window.
    fn schedule_reposition(&mut self) {
        if !self.reposition_pending {
            self.reposition_pending = true;
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_APP_REPOSITION, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn reposition(&mut self) {
        self.reposition_pending = false;
        let mon = primary_monitor();
        let (w, h) = (self.px(self.layout.width), self.px(METRICS.height));
        let (x, y) = if self.drag.is_some() {
            (self.bounds.left, self.bounds.top)
        } else {
            let p = &self.cfg.config.position;
            layout::place(mon, w, h, p.center_x, self.px(p.bottom_gap))
        };
        self.bounds = RECT { left: x, top: y, right: x + w, bottom: y + h };
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
        }
        if let Some(s) = &mut self.surface {
            s.resize(w as u32, h as u32);
        }
        self.publish_tray_rects();
        if self.popup.open {
            self.place_popup();
        }
    }

    fn screen_rect(&self, r: Rect, origin: (i32, i32)) -> RECT {
        RECT {
            left: origin.0 + self.px(r.x),
            top: origin.1 + self.px(r.y),
            right: origin.0 + self.px(r.right()),
            bottom: origin.1 + self.px(r.y + r.h),
        }
    }

    fn bar_origin(&self) -> (i32, i32) {
        (self.bounds.left, self.bounds.top)
    }

    /// Shares icon positions with the relay for `Shell_NotifyIconGetRect`.
    fn publish_tray_rects(&self) {
        let mut rects = self.tray_shared.rects.lock().unwrap();
        rects.icons.clear();
        rects.fallback = self.layout.rect(Hit::Chevron).map_or(self.bounds, |r| self.screen_rect(r, self.bar_origin()));
        for (i, &icon) in self.bar_tray.iter().enumerate() {
            if let (Some(r), Some(icon)) = (self.layout.rect(Hit::Tray(i)), self.tray.icons.get(icon)) {
                rects.icons.push((icon.key, self.screen_rect(r, self.bar_origin())));
            }
        }
        if self.popup.open {
            for (cell, &icon) in self.popup.cells.iter().enumerate() {
                if let Some(icon) = self.tray.icons.get(icon) {
                    rects.icons.push((icon.key, self.screen_rect(self.popup.cell_rect(cell), self.popup.origin)));
                }
            }
        }
    }

    fn window_icon_px(&self) -> u32 {
        self.px(WINDOW_ICON) as u32
    }

    fn request_icon(&mut self, h: HWND) {
        let class_icon = winlist::class_icon(h);
        if class_icon != 0 {
            self.set_window_icon(h, class_icon, IconRank::Class);
        }
        let px = self.window_icon_px();
        if class_name(h) == FRAME_CLASS
            && let Some(id) = winlist::aumid(h)
        {
            self.request_shell_icon(h, apps_folder(&id), IconRank::Package, px);
            return;
        }
        winlist::request_window_icon(h, false, on_window_icon);
        if class_icon == 0
            && let Some(path) = self.windows.index(h).and_then(|i| self.windows.items[i].exe.clone())
        {
            self.request_shell_icon(h, path, IconRank::Exe, px);
        }
    }

    fn request_shell_icon(&mut self, h: HWND, name: String, rank: IconRank, px: u32) {
        self.request_icon_target(IconTarget::Window(h, rank), name, px);
    }

    fn request_icon_target(&mut self, target: IconTarget, name: String, px: u32) {
        let key = (name, px);
        if let Some(img) = self.icon_cache.get(&key) {
            return self.apply_shell_icon(target, img.clone());
        }
        let waiters = self.icon_waiters.entry(key.clone()).or_default();
        waiters.push(target);
        if waiters.len() == 1 {
            let id = self.next_icon_request;
            self.next_icon_request += 1;
            self.icon_worker.request(id, key.0.clone(), px);
            self.icon_requests.insert(id, key);
        }
    }

    /// Builds images for the given tray icons, asking for the owner's exe
    /// icon when the app sent none we can read.
    fn ensure_tray_images(&mut self, icons: &[usize]) {
        let px = self.px(TRAY_ICON) as u32;
        for &i in icons {
            let icon = &mut self.tray.icons[i];
            if icon.ensure_image(&self.gfx, px) || icon.fallback_requested {
                continue;
            }
            icon.fallback_requested = true;
            let key = icon.key;
            if let Some(exe) = icon.exe.clone() {
                self.request_icon_target(IconTarget::Tray(key), exe, px);
            }
        }
    }

    fn set_window_icon(&mut self, h: HWND, icon: isize, rank: IconRank) {
        let px = self.window_icon_px();
        let Some(i) = self.windows.index(h) else { return };
        let w = &self.windows.items[i];
        if rank < w.rank || (rank == w.rank && w.src == icon && w.image.as_ref().is_some_and(|i| i.px == px)) {
            return;
        }
        if let Some(img) = self.gfx.image_from_hicon(HICON(icon as *mut _), px) {
            let w = &mut self.windows.items[i];
            w.image = Some(img);
            w.rank = rank;
            w.src = icon;
            self.invalidate();
        }
    }

    fn on_window_icon(&mut self, h: HWND, small: bool, result: isize) {
        if result != 0 {
            self.set_window_icon(h, result, IconRank::Window);
        } else if !small && self.windows.index(h).is_some() {
            winlist::request_window_icon(h, true, on_window_icon);
        }
    }

    fn on_shell_icon(&mut self, id: u64, bitmap: HBITMAP) {
        let key = self.icon_requests.remove(&id);
        let targets = key.as_ref().and_then(|k| self.icon_waiters.remove(k)).unwrap_or_default();
        if bitmap.is_invalid() {
            return;
        }
        let img = key.as_ref().and_then(|k| self.gfx.image_from_hbitmap(bitmap, k.1));
        unsafe {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
        let (Some(key), Some(img)) = (key, img) else { return };
        for target in targets {
            self.apply_shell_icon(target, img.clone());
        }
        self.icon_cache.insert(key, img);
    }

    fn apply_shell_icon(&mut self, target: IconTarget, img: Image) {
        match target {
            IconTarget::Window(h, rank) => {
                if let Some(i) = self.windows.index(h)
                    && rank >= self.windows.items[i].rank
                {
                    let w = &mut self.windows.items[i];
                    w.image = Some(img);
                    w.rank = rank;
                    w.src = 0;
                    self.invalidate();
                }
            }
            IconTarget::Pin(name) => {
                if let Some(p) = self.pins.iter_mut().find(|p| p.parsing_name == name) {
                    p.image = Some(img);
                    self.invalidate();
                }
            }
            IconTarget::Tray(key) => {
                if let Some(icon) = self.tray.icons.iter_mut().find(|i| i.key.same(&key) && i.image.is_none()) {
                    icon.image = Some(img);
                    self.invalidate();
                    self.invalidate_popup();
                }
            }
        }
    }

    fn prune_icon_cache(&mut self) {
        let live: Vec<u64> = (self.windows.items.iter().map(|w| &w.image))
            .chain(self.pins.iter().map(|p| &p.image))
            .chain(self.tray.icons.iter().map(|i| &i.image))
            .flatten()
            .map(Image::id)
            .collect();
        self.icon_cache.retain(|_, img| live.contains(&img.id()));
    }

    fn refresh_window_icons(&mut self) {
        for h in std::mem::take(&mut self.icon_refresh) {
            if self.windows.index(h).is_some() && class_name(h) != FRAME_CLASS {
                winlist::request_window_icon(h, false, on_window_icon);
            }
        }
    }

    fn refetch_icons(&mut self) {
        self.icon_cache.clear();
        for w in &mut self.windows.items {
            w.rank = IconRank::None;
        }
        for i in 0..self.windows.items.len() {
            self.request_icon(self.windows.items[i].hwnd);
        }
        for i in 0..self.pins.len() {
            self.request_pin_icon(i);
        }
        for icon in &mut self.tray.icons {
            icon.image = None;
            icon.fallback_requested = false;
        }
    }

    fn add_window(&mut self, h: HWND) {
        if self.windows.add(h) {
            self.request_icon(h);
            self.relayout();
        }
    }

    fn remove_window(&mut self, h: HWND) {
        if self.windows.remove(h) {
            self.relayout();
        }
    }

    fn on_foreground(&mut self, h: HWND) {
        if h.is_invalid() || own_window(h) {
            return;
        }
        let root = self.root(h);
        if shell_flyout(root) {
            return;
        }
        self.add_window(root);
        if root != self.foreground {
            self.foreground = root;
            self.invalidate();
        }
        self.refresh_keyboard();
        self.update_visibility();
    }

    fn on_winevent(&mut self, event: u32, h: HWND) {
        match event {
            EVENT_SYSTEM_FOREGROUND => self.on_foreground(h),
            EVENT_OBJECT_CLOAKED => self.remove_window(h),
            EVENT_OBJECT_UNCLOAKED => self.add_window(h),
            EVENT_OBJECT_FOCUS if h != self.focus => {
                self.focus = h;
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_KEYBOARD, 100, None);
                }
            }
            _ => {}
        }
    }

    fn on_shell_hook(&mut self, code: usize, h: HWND) {
        const CREATED: usize = 1;
        const DESTROYED: usize = 2;
        const ACTIVATED: usize = 4;
        const REDRAW: usize = 6;
        const LANGUAGE: usize = 8;
        const REPLACED: usize = 13;
        const REPLACING: usize = 14;
        match code & 0x7FFF {
            CREATED | REPLACING => self.add_window(h),
            DESTROYED => self.remove_window(h),
            REPLACED => {
                if self.windows.replaced(h) {
                    self.relayout();
                }
            }
            ACTIVATED => self.on_foreground(h),
            REDRAW => {
                if self.windows.index(h).is_some() && !self.icon_refresh.contains(&h) {
                    if self.icon_refresh.is_empty() {
                        unsafe {
                            SetTimer(Some(self.hwnd), TIMER_ICONS, 500, None);
                        }
                    }
                    self.icon_refresh.push(h);
                }
            }
            LANGUAGE => self.refresh_keyboard(),
            _ => {}
        }
    }

    fn on_taskbar_created(&mut self) {
        let real = shell::find_real_tray();
        if real == shell::real_tray() {
            return;
        }
        log::info!("explorer restarted");
        REAL_TRAY.store(real.map_or(0, |h| h.0 as isize), Ordering::Relaxed);
        if let (Some(t), Some(real)) = (&mut self.takeover, real) {
            t.apply(real);
        }
        self.close_popup();
        self.tray.icons.clear();
        self.relayout();
        self.register_appbar();
        if let (Some(relay), Some(real)) = (&self.relay, real) {
            relay.attach(real);
        }
    }

    fn on_tray_events(&mut self) {
        let events: Vec<_> = self.tray_shared.queue.lock().unwrap().drain(..).collect();
        let (mut relayout, mut redraw) = (false, false);
        for ev in events {
            match self.tray.apply(ev) {
                Change::None => {}
                Change::Redraw(i) => redraw |= self.bar_tray.contains(&i) || self.popup.cells.contains(&i),
                Change::Relayout => relayout = true,
            }
        }
        if relayout {
            self.after_tray_change();
        } else if redraw {
            self.invalidate();
            self.invalidate_popup();
        }
    }

    fn after_tray_change(&mut self) {
        if self.popup.open {
            self.popup.cells = self.tray.popup_icons();
            if self.popup.cells.is_empty() {
                self.close_popup();
            } else {
                self.place_popup();
                self.invalidate_popup();
            }
        }
        self.relayout();
    }

    fn update_visibility(&mut self) {
        if let Some(f) = fullscreen_foreground() {
            self.fullscreen = f;
        }
        let hide = BREAK.load(Ordering::Relaxed) || self.appbar_fullscreen || self.fullscreen;
        if hide == self.hidden {
            return;
        }
        log::debug!("bar {}", if hide { "hidden" } else { "shown" });
        self.hidden = hide;
        if hide {
            self.close_popup();
            self.set_hover(None);
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, if hide { SW_HIDE } else { SW_SHOWNOACTIVATE });
            if !hide {
                let _ =
                    SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
            }
        }
    }

    fn slow_tick(&mut self) {
        self.update_visibility();
        if let Some(t) = &self.takeover {
            t.ensure_hidden();
        }
        if self.hidden {
            return;
        }
        self.slow_ticks = self.slow_ticks.wrapping_add(1);
        if !self.slow_ticks.is_multiple_of(PRUNE_EVERY) {
            return;
        }
        let mut changed = self.windows.prune();
        if self.tray.prune_dead() {
            self.after_tray_change();
            changed = false;
        }
        if changed {
            self.relayout();
        }
        self.prune_icon_cache();
    }

    fn paused(&self) -> bool {
        self.locked || self.display_off
    }

    fn set_paused(&mut self, locked: bool, display_off: bool) {
        let was = self.paused();
        self.locked = locked;
        self.display_off = display_off;
        let now = self.paused();
        if now == was {
            return;
        }
        unsafe {
            if now {
                let _ = KillTimer(Some(self.hwnd), TIMER_SLOW);
                let _ = KillTimer(Some(self.hwnd), TIMER_CLOCK);
            } else {
                let _ = SetCoalescableTimer(Some(self.hwnd), TIMER_SLOW, 2000, None, 1000);
            }
        }
        if !now {
            self.refresh_clock();
            self.refresh_keyboard();
            self.slow_tick();
        }
    }

    fn on_pins_signal(&mut self) {
        if let Some(w) = &self.pin_watch {
            w.rearm();
        }
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_PINS, 300, None);
        }
    }

    fn set_pins(&mut self, mut pins: Vec<PinnedApp>) {
        for p in &mut pins {
            p.image = self.pins.iter_mut().find(|o| o.parsing_name == p.parsing_name).and_then(|o| o.image.take());
        }
        self.windows.set_pins(pins.iter().map(|p| p.pin.clone()).collect());
        self.pins = pins;
        for i in 0..self.pins.len() {
            if self.pins[i].image.is_none() {
                self.request_pin_icon(i);
            }
        }
    }

    fn request_pin_icon(&mut self, i: usize) {
        let name = self.pins[i].parsing_name.clone();
        self.request_icon_target(IconTarget::Pin(name.clone()), name, self.window_icon_px());
    }

    fn refresh_pins(&mut self) {
        let pins = pinned::read();
        fn key(p: &PinnedApp) -> (&Pin, &str, &str) {
            (&p.pin, &p.parsing_name, &p.name)
        }
        if pins.iter().map(key).eq(self.pins.iter().map(key)) {
            return;
        }
        self.set_pins(pins);
        self.relayout();
    }

    fn on_config_signal(&mut self) {
        self.cfg.rearm_watch();
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_CONFIG, 250, None);
        }
    }

    fn reload_config(&mut self) {
        let old = self.cfg.config.clone();
        if self.cfg.reload() {
            log::info!("config reloaded");
            self.apply_config(&old);
        }
    }

    fn apply_config(&mut self, old: &CapsuleBar) {
        let c = self.cfg.config.clone();
        if c.appearance.backdrop != old.appearance.backdrop {
            theme::apply(self.hwnd, c.appearance.backdrop, &self.theme);
            theme::apply(self.popup.hwnd, c.appearance.backdrop, &self.theme);
        }
        if c.keyboard.labels != old.keyboard.labels {
            self.measure_keyboard_slot();
            self.refresh_keyboard();
        }
        self.relayout();
        self.reposition();
        self.invalidate();
        self.invalidate_popup();
    }

    fn on_theme_change(&mut self) {
        let theme = Theme::read();
        if theme == self.theme {
            return;
        }
        self.theme = theme;
        let backdrop = self.cfg.config.appearance.backdrop;
        theme::apply(self.hwnd, backdrop, &theme);
        theme::apply(self.popup.hwnd, backdrop, &theme);
        self.tooltip.set_dark(theme.dark);
        self.invalidate();
        self.invalidate_popup();
    }

    fn schedule_work_area_check(&self) {
        if self.takeover.is_some() {
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_WORK_AREA, 250, None);
            }
        }
    }

    fn fix_work_area(&mut self) {
        let Some(t) = &self.takeover else { return };
        if self.last_work_area_fix.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            self.schedule_work_area_check();
            return;
        }
        self.last_work_area_fix = Some(Instant::now());
        t.ensure_work_area();
    }

    fn on_timer(&mut self, id: usize) {
        if id != TIMER_SLOW {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), id);
            }
        }
        match id {
            TIMER_CLOCK => self.refresh_clock(),
            TIMER_SLOW => self.slow_tick(),
            TIMER_CONFIG => self.reload_config(),
            TIMER_KEYBOARD => {
                self.refresh_keyboard();
                if self.keyboard_retries > 0 {
                    self.keyboard_retries -= 1;
                    unsafe {
                        SetTimer(Some(self.hwnd), TIMER_KEYBOARD, 200, None);
                    }
                }
            }
            TIMER_WORK_AREA => self.fix_work_area(),
            TIMER_WIN_DIGIT => {
                for vk in std::mem::take(&mut self.pending_win_digits) {
                    keyhook::replay_win_key(vk);
                }
            }
            TIMER_REVEAL => self.conceal_when_idle(),
            TIMER_PINS => self.refresh_pins(),
            TIMER_ICONS => self.refresh_window_icons(),
            TIMER_AUDIO => {
                self.volume.attach();
                self.refresh_volume();
            }
            _ => {}
        }
    }

    fn on_dpi(&mut self, dpi: u32) {
        if dpi == self.dpi {
            return;
        }
        self.dpi = dpi;
        if let Some(s) = &mut self.surface {
            s.set_dpi(dpi);
        }
        if let Some(s) = &mut self.popup.surface {
            s.set_dpi(dpi);
        }
        self.refetch_icons();
        self.reposition();
        self.invalidate();
    }

    fn track_mouse(&self, h: HWND) {
        let mut t = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_HOVER | TME_LEAVE,
            hwndTrack: h,
            dwHoverTime: HOVER_DEFAULT,
        };
        unsafe {
            let _ = TrackMouseEvent(&mut t);
        }
    }

    fn close_icon_popup(&mut self) {
        if let Some(key) = self.tray_popup_open.take()
            && let Some(icon) = self.tray.icons.iter().find(|i| i.key.same(&key))
        {
            let p = cursor();
            icon.send(NIN_POPUPCLOSE, (p.x, p.y));
        }
    }

    fn set_hover(&mut self, hit: Option<Hit>) {
        if hit == self.hover {
            return;
        }
        self.close_icon_popup();
        self.tooltip.hide();
        self.hover = hit;
        if let Some(Hit::Tray(i)) = hit {
            let p = cursor();
            self.tray.icons[self.bar_tray[i]].send(WM_MOUSEMOVE, (p.x, p.y));
        }
        if hit.is_some() {
            self.track_mouse(self.hwnd);
        }
        self.invalidate();
    }

    fn tray_tip(&mut self, icon: usize, x: i32, top: i32, bottom: i32) {
        let i = &self.tray.icons[icon];
        if i.wants_popup() {
            let p = cursor();
            i.send(NIN_POPUPOPEN, (p.x, p.y));
            self.tray_popup_open = Some(i.key);
        } else {
            let tip = i.tip.clone();
            self.tooltip.show(&tip, x, top, bottom);
        }
    }

    fn show_tip(&mut self) {
        let Some(hit) = self.hover else { return };
        let Some(r) = self.layout.rect(hit) else { return };
        let x = self.bounds.left + self.px(r.center_x());
        let text = match hit {
            Hit::Task(i) => match self.tasks[i] {
                Task::Window(w) => winlist::title(self.windows.items[w].hwnd),
                Task::Pin(p) => self.pins[p].name.clone(),
            },
            Hit::Tray(i) => return self.tray_tip(self.bar_tray[i], x, self.bounds.top, self.bounds.bottom),
            Hit::Keyboard => keyboard::display_name(self.keyboard_layout),
            Hit::Volume if !self.volume.available() => "No audio output device".into(),
            Hit::Volume if self.volume.muted() => "Muted".into(),
            Hit::Volume => format!("Volume {}%", (self.volume.level() * 100.0).round()),
            Hit::Clock => long_date(),
            Hit::Start => "Start".into(),
            Hit::Chevron => return,
        };
        self.tooltip.show(&text, x, self.bounds.top, self.bounds.bottom);
    }

    /// Forwards a mouse message to a tray icon, adding explorer's follow-up
    /// notification after button-up.
    fn tray_mouse(&mut self, icon: usize, msg: u32) {
        let i = &self.tray.icons[icon];
        if matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONDBLCLK) {
            i.allow_foreground();
        }
        let p = cursor();
        if !i.send(msg, (p.x, p.y)) {
            self.tooltip.show(ELEVATED_HINT, p.x, p.y - self.px(12.0), p.y + self.px(20.0));
            return;
        }
        let skip = msg == WM_LBUTTONUP && std::mem::take(&mut self.double_click);
        if msg == WM_LBUTTONDBLCLK {
            self.double_click = true;
        }
        if !skip && let Some(f) = i.follow_up(msg) {
            i.send(f, (p.x, p.y));
        }
    }

    fn next_keyboard_layout(&mut self) {
        let layouts = keyboard::installed();
        let next = layouts.iter().position(|&h| h == self.keyboard_layout).map_or(0, |i| (i + 1) % layouts.len());
        let Some(&hkl) = layouts.get(next) else { return };
        keyboard::switch(self.input_target(), hkl);
        self.keyboard_retries = 2;
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_KEYBOARD, 40, None);
        }
    }

    fn foreground_root(&self) -> HWND {
        let fg = self.root(unsafe { GetForegroundWindow() });
        if fg.is_invalid() || own_window(fg) { self.foreground } else { fg }
    }

    fn on_win_digit(&mut self, vk: u16) {
        let Some(t) = &self.takeover else { return keyhook::replay_win_key(vk) };
        self.last_win_digit = Some(Instant::now());
        if t.revealed() && self.pending_win_digits.is_empty() {
            keyhook::replay_win_key(vk);
        } else {
            t.reveal();
            self.pending_win_digits.push(vk);
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_WIN_DIGIT, 60, None);
            }
        }
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_REVEAL, 300, None);
        }
    }

    fn conceal_when_idle(&mut self) {
        let Some(t) = &self.takeover else { return };
        if keyhook::win_held() || self.last_win_digit.is_some_and(|d| d.elapsed() < Duration::from_millis(800)) {
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_REVEAL, 300, None);
            }
        } else {
            t.conceal();
        }
    }

    fn on_bar_mouse(&mut self, msg: u32, lparam: LPARAM) {
        let hit = self.layout.hit(self.to_dip(lparam_point(lparam).0));
        if let Some(d) = &mut self.drag {
            match msg {
                WM_MOUSEMOVE => {
                    let p = cursor();
                    let (dx, dy) = (p.x - d.cursor.x, p.y - d.cursor.y);
                    let threshold = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
                    d.moved |= dx.abs() > threshold.0 || dy.abs() > threshold.1;
                    if d.moved {
                        let origin = d.origin;
                        self.drag_to(origin.x + dx, origin.y + dy);
                    }
                }
                WM_LBUTTONUP => {
                    let (moved, down_hit, double) = (d.moved, d.hit, d.double);
                    self.end_drag();
                    if let Some(h) = down_hit.filter(|&h| !moved && Some(h) == hit) {
                        self.left_click(h, double);
                    }
                }
                _ => {}
            }
            return;
        }
        if msg == WM_MOUSEMOVE {
            self.set_hover(hit);
            return;
        }
        self.tooltip.hide();
        if matches!(msg, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK) {
            let origin = POINT { x: self.bounds.left, y: self.bounds.top };
            self.drag = Some(Drag { cursor: cursor(), origin, moved: false, hit, double: msg == WM_LBUTTONDBLCLK });
            unsafe { SetCapture(self.hwnd) };
            return;
        }
        let Some(hit) = hit else {
            if msg == WM_RBUTTONUP {
                self.deferred.push_back(DeferredMenu::App);
            }
            return;
        };
        match (msg, hit) {
            (WM_MBUTTONUP, Hit::Task(i)) => {
                if let Some(h) = self.task_window(i) {
                    winlist::close(h);
                }
            }
            (WM_RBUTTONUP, Hit::Task(i)) => {
                if let Some(h) = self.task_window(i) {
                    let p = cursor();
                    winlist::system_menu(h, p.x, p.y);
                }
            }
            (WM_RBUTTONUP, Hit::Start) => keyhook::press_win(b'X'),
            (WM_RBUTTONUP, Hit::Keyboard) => self.deferred.push_back(DeferredMenu::Keyboard),
            (WM_RBUTTONUP, Hit::Clock) => self.deferred.push_back(DeferredMenu::App),
            (_, Hit::Tray(i)) => self.tray_mouse(self.bar_tray[i], msg),
            _ => {}
        }
    }

    fn task_window(&self, i: usize) -> Option<HWND> {
        match self.tasks[i] {
            Task::Window(w) => Some(self.windows.items[w].hwnd),
            Task::Pin(_) => None,
        }
    }

    /// A left click that did not turn into a drag.
    fn left_click(&mut self, hit: Hit, double: bool) {
        match hit {
            Hit::Start => keyhook::press_start(),
            Hit::Task(i) => match self.tasks[i] {
                Task::Window(w) => winlist::activate_or_minimize(self.windows.items[w].hwnd, self.foreground_root()),
                Task::Pin(p) => {
                    launcher::open(self.hwnd, &self.pins[p].parsing_name, "");
                }
            },
            Hit::Chevron => self.toggle_popup(),
            Hit::Keyboard => self.next_keyboard_layout(),
            Hit::Volume => keyhook::press_win(b'A'),
            Hit::Clock => keyhook::press_win(b'N'),
            Hit::Tray(i) => {
                let icon = self.bar_tray[i];
                self.tray_mouse(icon, if double { WM_LBUTTONDBLCLK } else { WM_LBUTTONDOWN });
                self.tray_mouse(icon, WM_LBUTTONUP);
            }
        }
    }

    fn drag_to(&mut self, x: i32, y: i32) {
        let mon = primary_monitor();
        let (w, h) = (self.bounds.right - self.bounds.left, self.bounds.bottom - self.bounds.top);
        let (x, y) = layout::snap(mon, x, y, w, h, self.px(SNAP));
        let x = x.clamp(mon.left, mon.right - w);
        let y = y.clamp(mon.top, mon.bottom - h);
        self.bounds = RECT { left: x, top: y, right: x + w, bottom: y + h };
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
        }
    }

    fn end_drag(&mut self) {
        let Some(d) = self.drag.take() else { return };
        unsafe {
            let _ = ReleaseCapture();
        }
        if (d.origin.x, d.origin.y) == (self.bounds.left, self.bounds.top) {
            return;
        }
        let mon = primary_monitor();
        let b = self.bounds;
        let (cx, gap) = layout::anchor(mon, b.left, b.top, b.right - b.left, b.bottom - b.top);
        self.cfg.save_position(cx, self.to_dip(gap));
        self.publish_tray_rects();
    }

    fn on_wheel(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let (sx, sy) = lparam_point(lparam);
        let mut p = POINT { x: sx, y: sy };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut p);
        }
        if self.layout.hit(self.to_dip(p.x)) != Some(Hit::Volume) {
            return;
        }
        self.wheel += (wparam.0 >> 16) as i16 as i32;
        let notches = self.wheel / 120;
        self.wheel %= 120;
        if notches != 0 {
            self.volume.change(notches as f32 * self.cfg.config.volume.step as f32 / 100.0);
        }
    }

    fn place_popup(&mut self) {
        let (w, h) = self.popup.size();
        let (w, h) = (self.px(w), self.px(h));
        let anchor =
            self.layout.rect(Hit::Chevron).map_or(self.bounds.left, |r| self.bounds.left + self.px(r.center_x()));
        let mon = primary_monitor();
        let x = (anchor - w / 2).clamp(mon.left, mon.right - w);
        let above = self.bounds.top - self.px(POPUP_GAP) - h;
        let y = if above >= mon.top { above } else { self.bounds.bottom + self.px(POPUP_GAP) };
        self.popup.origin = (x, y);
        unsafe {
            let _ = SetWindowPos(self.popup.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
        }
        if let Some(s) = &mut self.popup.surface {
            s.resize(w as u32, h as u32);
        }
        self.publish_tray_rects();
    }

    fn toggle_popup(&mut self) {
        if self.popup.open {
            self.close_popup();
            return;
        }
        self.popup.cells = self.tray.popup_icons();
        if self.popup.cells.is_empty() {
            return;
        }
        self.popup.open = true;
        self.popup.hover = None;
        self.place_popup();
        unsafe {
            let _ = ShowWindow(self.popup.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.popup.hwnd);
        }
        self.invalidate_popup();
    }

    fn close_popup(&mut self) {
        if !self.popup.open {
            return;
        }
        self.popup.open = false;
        self.popup.hover = None;
        self.popup.surface = None;
        self.close_icon_popup();
        self.tooltip.hide();
        unsafe {
            let _ = ShowWindow(self.popup.hwnd, SW_HIDE);
        }
        self.publish_tray_rects();
    }

    fn on_popup_mouse(&mut self, msg: u32, lparam: LPARAM) {
        let (x, y) = lparam_point(lparam);
        let cell = self.popup.hit(self.to_dip(x), self.to_dip(y));
        if msg == WM_MOUSEMOVE {
            if cell != self.popup.hover {
                self.close_icon_popup();
                self.tooltip.hide();
                self.popup.hover = cell;
                if let Some(c) = cell {
                    let p = cursor();
                    self.tray.icons[self.popup.cells[c]].send(WM_MOUSEMOVE, (p.x, p.y));
                    self.track_mouse(self.popup.hwnd);
                }
                self.invalidate_popup();
            }
            return;
        }
        self.tooltip.hide();
        let Some(c) = cell else { return };
        let icon = self.popup.cells[c];
        match msg {
            WM_RBUTTONDOWN => {}
            WM_RBUTTONUP => self.deferred.push_back(DeferredMenu::Tray(icon)),
            _ => self.tray_mouse(icon, msg),
        }
    }

    fn popup_tip(&mut self) {
        let Some(c) = self.popup.hover else { return };
        let r = self.popup.cell_rect(c);
        let x = self.popup.origin.0 + self.px(r.center_x());
        let top = self.popup.origin.1 + self.px(r.y);
        let bottom = self.popup.origin.1 + self.px(r.y + r.h);
        self.tray_tip(self.popup.cells[c], x, top, bottom);
    }

    fn toggle_pin(&mut self, icon: usize) {
        let pin = self.tray.icons[icon].pin.clone();
        let pins = &mut self.cfg.config.tray.pinned;
        if let Some(i) = pins.iter().position(|p| *p == pin) {
            pins.remove(i);
        } else {
            pins.insert(0, pin);
        }
        self.cfg.save_pinned();
        self.relayout();
        self.invalidate_popup();
    }

    fn new_surface(&self, hwnd: HWND, w: i32, h: i32) -> Option<Surface> {
        let s = Surface::new(hwnd, w as u32, h as u32, self.dpi);
        if s.is_none() {
            log::error!("surface creation failed");
        }
        s
    }

    fn ensure_surface(&mut self) -> bool {
        if self.surface.is_none() {
            let (w, h) = (self.bounds.right - self.bounds.left, self.bounds.bottom - self.bounds.top);
            self.surface = self.new_surface(self.hwnd, w, h);
        }
        self.surface.is_some()
    }

    fn paint(&mut self) {
        unsafe {
            let _ = ValidateRect(Some(self.hwnd), None);
        }
        if !self.ensure_surface() {
            return;
        }
        self.ensure_tray_images(&self.bar_tray.clone());
        let pal = self.theme.palette(self.cfg.config.appearance.opacity);
        let h = METRICS.height;
        let cy = h / 2.0;
        let s = self.surface.as_mut().unwrap();
        s.begin();
        s.fill(Rect { x: 0.0, y: 0.0, w: self.layout.width, h }, pal.tint, 0.0);
        for &(hit, r) in &self.layout.items {
            let highlight = Rect { x: r.x + 1.0, y: 4.0, w: r.w - 2.0, h: h - 8.0 };
            if self.hover == Some(hit) {
                s.fill(highlight, pal.hover, 4.0);
            }
            let center = |t: &Text| (r.center_x() - t.width / 2.0, cy - t.height / 2.0);
            match hit {
                Hit::Start => {
                    let size = (START_LOGO - START_LOGO_GAP) / 2.0;
                    let (left, top) = (r.center_x() - START_LOGO / 2.0, cy - START_LOGO / 2.0);
                    let squares = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].map(|(col, row)| Rect {
                        x: left + col * (size + START_LOGO_GAP),
                        y: top + row * (size + START_LOGO_GAP),
                        w: size,
                        h: size,
                    });
                    s.fill_gradient(&squares, 1.0, START_LOGO_FROM, START_LOGO_TO);
                }
                Hit::Task(i) => {
                    let image = match self.tasks[i] {
                        Task::Window(w) => {
                            let w = &self.windows.items[w];
                            let (bar_w, color) = if w.hwnd == self.foreground {
                                s.fill(highlight, pal.active, 4.0);
                                (16.0, pal.accent)
                            } else {
                                (6.0, pal.running)
                            };
                            s.fill(Rect { x: r.center_x() - bar_w / 2.0, y: h - 7.0, w: bar_w, h: 3.0 }, color, 1.5);
                            &w.image
                        }
                        Task::Pin(p) => &self.pins[p].image,
                    };
                    let icon_y = cy - TASK_ICON_LIFT;
                    match image {
                        Some(img) => s.image(img, r.center_x(), icon_y),
                        None => s.dot(r.center_x(), icon_y, 3.0, pal.text.alpha(0.5)),
                    }
                }
                Hit::Chevron => {
                    if let Some(t) = &self.texts.chevron {
                        let (x, y) = center(t);
                        s.text(t, x, y, pal.text);
                    }
                }
                Hit::Tray(i) => {
                    if let Some(img) = &self.tray.icons[self.bar_tray[i]].image {
                        s.image(img, r.center_x(), cy);
                    }
                }
                Hit::Keyboard => {
                    if let Some(t) = &self.texts.keyboard {
                        let (x, y) = center(t);
                        s.text(t, x, y, pal.text);
                    }
                }
                Hit::Volume => {
                    if let Some(g) = &self.texts.volume_glyph {
                        let (x, y) = center(g);
                        s.text(g, x, y, pal.text);
                    }
                }
                Hit::Clock => {
                    if let (Some(t), Some(d)) = (&self.texts.clock_time, &self.texts.clock_date) {
                        let right = r.right() - METRICS.text_pad;
                        let top = cy - (t.height + d.height - CLOCK_LINE_OVERLAP) / 2.0;
                        s.text(t, right - t.width, top, pal.text);
                        s.text(d, right - d.width, top + t.height - CLOCK_LINE_OVERLAP, pal.text);
                    }
                }
            }
        }
        s.end();
    }

    fn paint_popup(&mut self) {
        unsafe {
            let _ = ValidateRect(Some(self.popup.hwnd), None);
        }
        if !self.popup.open {
            return;
        }
        if self.popup.surface.is_none() {
            let (w, h) = self.popup.size();
            self.popup.surface = self.new_surface(self.popup.hwnd, self.px(w), self.px(h));
        }
        self.ensure_tray_images(&self.popup.cells.clone());
        let pal = self.theme.palette(self.cfg.config.appearance.opacity);
        let (w, h) = self.popup.size();
        let pins = &self.cfg.config.tray.pinned;
        let cells: Vec<(Rect, usize)> =
            self.popup.cells.iter().enumerate().map(|(c, &icon)| (self.popup.cell_rect(c), icon)).collect();
        let hover = self.popup.hover;
        let Some(s) = self.popup.surface.as_mut() else { return };
        s.begin();
        s.fill(Rect { x: 0.0, y: 0.0, w, h }, pal.tint, 0.0);
        for (c, &(r, icon)) in cells.iter().enumerate() {
            if hover == Some(c) {
                s.fill(Rect { x: r.x + 2.0, y: r.y + 2.0, w: r.w - 4.0, h: r.h - 4.0 }, pal.hover, 4.0);
            }
            let i = &self.tray.icons[icon];
            if let Some(img) = &i.image {
                s.image(img, r.center_x(), r.y + r.h / 2.0 - 1.0);
            }
            if pins.contains(&i.pin) {
                s.dot(r.center_x(), r.y + r.h - 5.0, 1.5, pal.text.alpha(0.7));
            }
        }
        s.end();
    }

    fn bar_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        let handled = LRESULT(0);
        match msg {
            WM_PAINT => self.paint(),
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP
            | WM_MBUTTONDOWN | WM_MBUTTONUP => self.on_bar_mouse(msg, lparam),
            WM_MOUSEHOVER => self.show_tip(),
            WM_MOUSELEAVE => {
                if self.drag.is_none() {
                    self.set_hover(None);
                }
            }
            WM_MOUSEWHEEL => self.on_wheel(wparam, lparam),
            WM_CAPTURECHANGED => self.end_drag(),
            WM_TIMER => self.on_timer(wparam.0),
            WM_APP_STOP => unsafe {
                let _ = EndMenu();
            },
            WM_APP_BREAK => self.update_visibility(),
            WM_APP_RELOAD_CONFIG => self.reload_config(),
            WM_APP_REPOSITION => {
                if self.reposition_pending {
                    self.reposition();
                }
            }
            WM_DPICHANGED => self.on_dpi((wparam.0 & 0xFFFF) as u32),
            WM_DISPLAYCHANGE => {
                self.reposition();
                self.schedule_work_area_check();
            }
            WM_SETTINGCHANGE => {
                if wparam.0 == SPI_SETWORKAREA.0 as usize {
                    self.schedule_work_area_check();
                } else if wparam.0 == SPI_SETHIGHCONTRAST.0 as usize {
                    self.on_theme_change();
                } else if lparam.0 != 0 {
                    let area = unsafe { PCWSTR(lparam.0 as *const u16).to_string() }.unwrap_or_default();
                    if area == "ImmersiveColorSet" {
                        self.on_theme_change();
                    }
                }
                return None;
            }
            WM_TIMECHANGE => self.refresh_clock(),
            WM_POWERBROADCAST => {
                if wparam.0 == PBT_APMRESUMEAUTOMATIC {
                    self.refresh_clock();
                    self.refresh_keyboard();
                } else if wparam.0 == PBT_POWERSETTINGCHANGE {
                    let s = unsafe { &*(lparam.0 as *const POWERBROADCAST_SETTING) };
                    if s.PowerSetting == GUID_CONSOLE_DISPLAY_STATE {
                        self.set_paused(self.locked, s.Data[0] == 0);
                    }
                }
                return Some(LRESULT(1));
            }
            WM_WTSSESSION_CHANGE => match wparam.0 {
                WTS_SESSION_LOCK => self.set_paused(true, self.display_off),
                WTS_SESSION_UNLOCK => self.set_paused(false, self.display_off),
                _ => {}
            },
            WM_ENDSESSION if wparam.0 != 0 => self.stop(),
            WM_DESTROY => unsafe { PostQuitMessage(0) },
            WM_APP_TRAY => self.on_tray_events(),
            WM_APP_VOLUME => self.refresh_volume(),
            WM_APP_AUDIO_DEVICE => unsafe {
                SetTimer(Some(self.hwnd), TIMER_AUDIO, 200, None);
            },
            WM_APP_ICON => self.on_shell_icon(wparam.0 as u64, HBITMAP(lparam.0 as *mut _)),
            WM_APP_APPBAR => {
                if wparam.0 == ABN_FULLSCREENAPP {
                    self.appbar_fullscreen = lparam.0 != 0;
                    self.update_visibility();
                }
            }
            WM_APP_WINEVENT => self.on_winevent(wparam.0 as u32, HWND(lparam.0 as *mut _)),
            WM_APP_WIN_DIGIT => self.on_win_digit(wparam.0 as u16),
            WM_APP_INPUT => {
                keyhook::input_handled();
                self.keyboard_retries = 2;
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_KEYBOARD, 40, None);
                }
            }
            m if m == self.shell_msg && m != 0 => self.on_shell_hook(wparam.0, HWND(lparam.0 as *mut _)),
            m if m == self.taskbar_created_msg && m != 0 => self.on_taskbar_created(),
            _ => return None,
        }
        Some(handled)
    }

    fn popup_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_PAINT => self.paint_popup(),
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP
            | WM_MBUTTONDOWN | WM_MBUTTONUP => self.on_popup_mouse(msg, lparam),
            WM_MOUSEHOVER => self.popup_tip(),
            WM_MOUSELEAVE => {
                self.close_icon_popup();
                self.tooltip.hide();
                self.popup.hover = None;
                self.invalidate_popup();
            }
            WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => self.close_popup(),
            WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => self.close_popup(),
            _ => return None,
        }
        Some(LRESULT(0))
    }
}

fn menu_anchor(a: &App, hit: Hit) -> (i32, i32) {
    let x = a.layout.rect(hit).map_or(a.bounds.left, |r| a.bounds.left + a.px(r.center_x()));
    (x, a.bounds.top - a.px(4.0))
}

fn app_menu() {
    let Some((owner, prev, y)) = with_app(|a| (a.hwnd, a.foreground, a.bounds.top - a.px(4.0))) else { return };
    let x = cursor().x;
    let items = [menu::command(MENU_RESTART, "Restart capsule bar"), menu::command(MENU_QUIT, "Quit capsule bar")];
    let id = menu::show(owner, &items, x, y);
    if !prev.is_invalid() {
        unsafe {
            let _ = SetForegroundWindow(prev);
        }
    }
    with_app(|a| match id {
        MENU_RESTART => a.opts.post_owner(a.opts.restart_msg),
        MENU_QUIT => a.opts.post_owner(a.opts.disable_msg),
        _ => {}
    });
}

fn keyboard_menu() {
    let Some((owner, target, current, (x, y))) =
        with_app(|a| (a.hwnd, a.input_target(), a.keyboard_layout, menu_anchor(a, Hit::Keyboard)))
    else {
        return;
    };
    let layouts = keyboard::installed();
    let names: Vec<String> = layouts.iter().map(|&h| keyboard::display_name(h)).collect();
    let items: Vec<Item> = layouts
        .iter()
        .zip(&names)
        .enumerate()
        .map(|(i, (&h, name))| Item { id: i as u32 + 1, text: name, checked: h == current })
        .collect();
    let id = menu::show(owner, &items, x, y);
    if !target.is_invalid() {
        unsafe {
            let _ = SetForegroundWindow(target);
        }
    }
    if let Some(&hkl) = (id as usize).checked_sub(1).and_then(|i| layouts.get(i)) {
        keyboard::switch(target, hkl);
        with_app(|a| unsafe {
            SetTimer(Some(a.hwnd), TIMER_KEYBOARD, 150, None);
        });
    }
}

fn tray_menu(icon: usize) {
    let Some((owner, key, pinned)) = with_app(|a| {
        let i = a.tray.icons.get(icon)?;
        Some((a.popup.hwnd, i.key, a.cfg.config.tray.pinned.contains(&i.pin)))
    })
    .flatten() else {
        return;
    };
    let p = cursor();
    let items = [menu::command(1, if pinned { "Unpin from bar" } else { "Pin to bar" }), menu::command(2, "App menu")];
    let id = menu::show(owner, &items, p.x, p.y);
    with_app(|a| {
        let Some(icon) = a.tray.icons.iter().position(|i| i.key.same(&key)) else { return };
        match id {
            1 => a.toggle_pin(icon),
            2 => {
                a.tray_mouse(icon, WM_RBUTTONDOWN);
                a.tray_mouse(icon, WM_RBUTTONUP);
            }
            _ => {}
        }
    });
}
