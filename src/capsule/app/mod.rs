mod icons;
mod language;
mod menus;
mod placement;
mod pointer;
mod render;
mod system;
mod taskbar;
mod tray_area;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use capsule_core::layout::{Hit, Layout};
use capsule_core::taskband::Task;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    HPOWERNOTIFY, RegisterPowerSettingNotification, UnregisterPowerSettingNotification,
};
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Controls::{
    ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, WM_MOUSEHOVER, WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::Shell::{ABM_NEW, ABM_REMOVE, APPBARDATA, SHAppBarMessage};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, ChangeWindowMessageFilterEx, CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW,
    DeregisterShellHookWindow, DestroyWindow, DispatchMessageW, EndMenu, GA_ROOTOWNER, GetAncestor, GetCursorPos,
    GetForegroundWindow, KillTimer, MA_NOACTIVATE, MSG, MSGFLT_ALLOW, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx,
    PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage, QS_ALLINPUT, RegisterShellHookWindow,
    RegisterWindowMessageW, SW_SHOWNOACTIVATE, SendMessageW, SetCoalescableTimer, SetTimer, ShowWindow,
    TranslateMessage, WA_INACTIVE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_ACTIVATE, WM_APP,
    WM_CAPTURECHANGED, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCACTIVATE, WM_PAINT, WM_POWERBROADCAST, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETTINGCHANGE,
    WM_TIMECHANGE, WM_TIMER, WM_WTSSESSION_CHANGE, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::{GUID, w};

use crate::capsule::config_store::ConfigStore;
use crate::capsule::gfx::{Gfx, Surface};
use crate::capsule::hook::{Hook, Rects, TrayShared};
use crate::capsule::input;
use crate::capsule::keyhook::{self, KeyHook};
use crate::capsule::menu;
use crate::capsule::pinned::{self, PinnedApp};
use crate::capsule::popup::Popup;
use crate::capsule::shell::{self, REAL_TRAY, Takeover};
use crate::capsule::theme::{self, Theme};
use crate::capsule::tooltip::Tooltip;
use crate::capsule::tray::{self, Tray, TrayKey};
use crate::capsule::util::{hwnd, register_class, scale};
use crate::capsule::volume::Volume;
use crate::capsule::windows_list::WindowList;
use icons::IconRequests;
use language::KeyboardState;
use menus::{DeferredMenu, run_deferred};
use pointer::Drag;
use render::Texts;
use taskbar::on_winevent;

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
const WM_APP_HOOK: u32 = WM_APP + 4;
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

const ABN_FULLSCREENAPP: usize = 2;
const WTS_SESSION_LOCK: usize = 7;
const WTS_SESSION_UNLOCK: usize = 8;

const GUID_CONSOLE_DISPLAY_STATE: GUID = GUID::from_u128(0x6fe69556_704a_47a0_8f24_c28d936fda47);

const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const EVENT_OBJECT_FOCUS: u32 = 0x8005;
const EVENT_OBJECT_CLOAKED: u32 = 0x8017;
const EVENT_OBJECT_UNCLOAKED: u32 = 0x8018;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app unless it is already borrowed further up the stack,
/// which happens when a handler makes a call that dispatches messages.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| cell.try_borrow_mut().ok().and_then(|mut a| a.as_mut().map(f)))
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
    tray_shared: Arc<TrayShared>,
    hook: Option<Hook>,
    takeover: Option<Takeover>,
    volume: Volume,
    keyhook: Option<KeyHook>,
    pin_watch: Option<pinned::Watch>,
    kbd: KeyboardState,
    volume_glyph: &'static str,
    pending_win_digits: Vec<u16>,
    last_win_digit: Option<Instant>,
    clock: (String, String),
    texts: Texts,
    icons: IconRequests,
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

static BAR: AtomicIsize = AtomicIsize::new(0);

fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

impl App {
    fn new(hwnd: HWND, popup: HWND, opts: Options) -> App {
        BAR.store(hwnd.0 as isize, Ordering::Relaxed);
        let cfg = ConfigStore::open();
        let theme = Theme::read();
        let taskbar_created_msg = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        let tray_shared = Arc::new(TrayShared {
            queue: Mutex::new(VecDeque::new()),
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
            hook: None,
            takeover: None,
            volume: Volume::new(hwnd.0 as isize, WM_APP_VOLUME, WM_APP_AUDIO_DEVICE),
            keyhook: None,
            pin_watch: pinned::Watch::new(),
            kbd: KeyboardState::default(),
            volume_glyph: "",
            pending_win_digits: Vec::new(),
            last_win_digit: None,
            clock: Default::default(),
            texts: Texts::default(),
            icons: IconRequests::default(),
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

        self.keyhook = KeyHook::start(h, WM_APP_WIN_DIGIT, WM_APP_INPUT);
        if let Some(k) = &self.keyhook {
            k.set_digits(self.takeover.is_some());
        }

        self.register_notifications();
        self.start_slow_timer();
        self.register_appbar();
        self.hook = Some(Hook::acquire(h, WM_APP_HOOK, Some(self.tray_shared.clone())));
        unsafe {
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            SendMessageW(h, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(0)));
        }
        self.update_visibility();
        log::info!("started: {} windows, dpi {}", self.windows.items.len(), self.dpi);
    }

    fn register_notifications(&mut self) {
        let h = self.hwnd;
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
        }
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        unsafe {
            let mut d = APPBARDATA { cbSize: size_of::<APPBARDATA>() as u32, hWnd: self.hwnd, ..Default::default() };
            SHAppBarMessage(ABM_REMOVE, &mut d);
        }
        self.unregister_notifications();
        self.hook = None;
        for ev in self.tray_shared.queue.lock().unwrap().drain(..) {
            tray::destroy_icon(ev.icon);
        }
        self.keyhook = None;
        if let Some(t) = self.takeover.take() {
            t.stop();
        }
        log::info!("stopped");
    }

    fn unregister_notifications(&mut self) {
        unsafe {
            for hook in self.hooks.drain(..) {
                let _ = UnhookWinEvent(hook);
            }
            let _ = DeregisterShellHookWindow(self.hwnd);
            let _ = WTSUnRegisterSessionNotification(self.hwnd);
            if let Some(n) = self.power_notify.take() {
                let _ = UnregisterPowerSettingNotification(n);
            }
        }
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

    fn set_timer(&self, id: usize, ms: u32) {
        unsafe {
            SetTimer(Some(self.hwnd), id, ms, None);
        }
    }

    fn start_slow_timer(&self) {
        unsafe {
            let _ = SetCoalescableTimer(Some(self.hwnd), TIMER_SLOW, 2000, None, 1000);
        }
    }

    fn kill_timer(&self, id: usize) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), id);
        }
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

    fn on_timer(&mut self, id: usize) {
        if id != TIMER_SLOW {
            self.kill_timer(id);
        }
        match id {
            TIMER_CLOCK => self.refresh_clock(),
            TIMER_SLOW => self.slow_tick(),
            TIMER_CONFIG => self.reload_config(),
            TIMER_KEYBOARD => self.on_keyboard_timer(),
            TIMER_WORK_AREA => self.fix_work_area(),
            TIMER_WIN_DIGIT => {
                for vk in std::mem::take(&mut self.pending_win_digits) {
                    input::replay_win_key(vk);
                }
            }
            TIMER_REVEAL => self.conceal_when_idle(),
            TIMER_PINS => self.request_pins(),
            TIMER_ICONS => self.refresh_window_icons(),
            TIMER_AUDIO => {
                self.volume.attach();
                self.refresh_volume();
            }
            _ => {}
        }
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
                self.on_setting_change(wparam, lparam);
                return None;
            }
            WM_TIMECHANGE => self.refresh_clock(),
            WM_POWERBROADCAST => {
                self.on_power(wparam, lparam);
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
            WM_APP_AUDIO_DEVICE => self.set_timer(TIMER_AUDIO, 200),
            WM_APP_HOOK => self.on_hook_events(),
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
                self.refresh_keyboard_soon();
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
