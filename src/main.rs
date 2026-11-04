#![windows_subsystem = "windows"]

mod capsule;
mod render;

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tiny_skia::Pixmap;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::RegisterSuspendResumeNotification;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use capsule::launcher::{self, AppCommand};
use capsule_core::launcher::hotkey;
use render::{Digits, IconFace, Overlay};

const WORK_SECONDS: f64 = 20.0 * 60.0;
const BREAK_SECONDS: f64 = 20.0;
const COUNTDOWN_SECONDS: u32 = 10;
const SLEEP_GAP_THRESHOLD: f64 = 30.0;
const STARTUP_BAR_DELAY: f64 = 4.0;
const STARTUP_PROGRESS: [(f64, f32); 15] = [
    (0.0, 0.00),
    (STARTUP_BAR_DELAY, 0.00),
    (5.2, 0.03),
    (5.6, 0.14),
    (7.0, 0.22),
    (8.5, 0.32),
    (10.0, 0.44),
    (11.5, 0.53),
    (13.0, 0.60),
    (14.5, 0.69),
    (16.0, 0.78),
    (17.2, 0.85),
    (18.0, 0.92),
    (19.2, 0.98),
    (BREAK_SECONDS, 1.00),
];

const MAIN_CLASS: PCWSTR = w!("202020");
const OVERLAY_CLASS: PCWSTR = w!("202020Overlay");
const WM_TRAY: u32 = WM_APP + 1;
const WM_DISMISS: u32 = WM_APP + 2;
const WM_BAR_EXITED: u32 = WM_APP + 3;
const WM_BAR_DISABLE: u32 = WM_APP + 4;
const WM_BAR_RESTART: u32 = WM_APP + 5;
const WM_LAUNCHER_CMD: u32 = WM_APP + 6;
const TRAY_ID: u32 = 1;
const HOTKEY_TOGGLE: i32 = 1;
const HOTKEY_LAUNCHER: i32 = 2;
const TIMER_TICK: usize = 1;
const TIMER_FRAME: usize = 2;
const TIMER_BREAK_END: usize = 3;
const CMD_TOGGLE: i32 = 1;
const CMD_BREAK: i32 = 2;
const CMD_RESET: i32 = 3;
const CMD_QUIT: i32 = 4;
const CMD_BAR: i32 = 5;
const CMD_EDIT_CONFIG: i32 = 6;
const CMD_RELOAD_CONFIG: i32 = 7;
const CMD_LAUNCHER: i32 = 8;

const LAUNCHER_COMMANDS: &[AppCommand] = &[
    AppCommand { id: CMD_TOGGLE as u32, name: "Pause or resume timer  暂停 继续计时" },
    AppCommand { id: CMD_BREAK as u32, name: "Break now  立即休息" },
    AppCommand { id: CMD_RESET as u32, name: "Reset timer  重置计时" },
    AppCommand { id: CMD_BAR as u32, name: "Toggle capsule bar  切换胶囊栏" },
    AppCommand { id: CMD_EDIT_CONFIG as u32, name: "Edit config  编辑配置" },
    AppCommand { id: CMD_RELOAD_CONFIG as u32, name: "Reload config  重新加载配置" },
];

static MAIN_HWND: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|app| app.try_borrow_mut().ok().and_then(|mut app| app.as_mut().map(f)))
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

fn minutes_left(seconds: u32) -> u32 {
    seconds.div_ceil(60).max(1)
}

/// Seconds until the tray icon changes, `left` seconds before the break. The
/// boundaries follow the rounding in `App::remaining`.
fn next_face_change(left: f64) -> f64 {
    if left <= 0.0 {
        return 1.0;
    }
    let rem = (left + 0.5).floor() as u32;
    let at = if rem > COUNTDOWN_SECONDS {
        (60 * (minutes_left(rem) - 1)).max(COUNTDOWN_SECONDS) as f64 + 0.5
    } else if rem > 0 {
        rem as f64 - 0.5
    } else {
        0.0
    };
    left - at
}

fn startup_progress_at(elapsed: f64) -> f32 {
    for pair in STARTUP_PROGRESS.windows(2) {
        let ((t0, v0), (t1, v1)) = (pair[0], pair[1]);
        if elapsed <= t1 {
            return v0 + (v1 - v0) * ((elapsed - t0) / (t1 - t0)) as f32;
        }
    }
    1.0
}

struct Break {
    hwnd: HWND,
    hook: Option<HHOOK>,
    started_at: f64,
    overlay: Overlay,
}

struct App {
    hwnd: HWND,
    digits: Digits,
    icon: HICON,
    paused: bool,
    deadline: f64,
    paused_remaining: f64,
    tick_due: f64,
    last_face: Option<IconFace>,
    brk: Option<Break>,
    bar_opts: capsule::Options,
    bar: BarState,
    launcher: LauncherState,
    restarting: bool,
    quitting: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LauncherState {
    Off,
    On,
    /// Enabled, but the hotkey is invalid or taken by another program.
    NoHotkey,
}

enum BarState {
    Off,
    Running(capsule::Bar),
    Stopping(capsule::Bar),
}

impl App {
    fn remaining(&self) -> u32 {
        let seconds = if self.paused { self.paused_remaining } else { self.deadline - now() };
        (seconds + 0.5).floor().max(0.0) as u32
    }

    fn face(&self) -> IconFace {
        let rem = self.remaining();
        if self.paused {
            IconFace::Paused
        } else if rem > 0 && rem <= COUNTDOWN_SECONDS {
            IconFace::Countdown(rem)
        } else {
            IconFace::Minutes(minutes_left(rem))
        }
    }

    fn notify(&self, message: NOTIFY_ICON_MESSAGE) {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: self.icon,
            ..Default::default()
        };
        for (dst, src) in data.szTip.iter_mut().zip("202020".encode_utf16()) {
            *dst = src;
        }
        unsafe {
            let _ = Shell_NotifyIconW(message, &data);
        }
    }

    fn refresh_icon(&mut self) {
        let face = self.face();
        self.last_face = Some(face);
        unsafe {
            let size = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForWindow(self.hwnd)).max(16) as u32;
            let previous = std::mem::replace(&mut self.icon, create_icon(&render::tray_icon(&self.digits, size, face)));
            self.notify(NIM_MODIFY);
            if !previous.is_invalid() {
                let _ = DestroyIcon(previous);
            }
        }
        self.schedule_tick();
    }

    fn schedule_tick(&mut self) {
        unsafe {
            if self.paused || self.brk.is_some() {
                let _ = KillTimer(Some(self.hwnd), TIMER_TICK);
                return;
            }
            let now = now();
            let delay = next_face_change(self.deadline - now);
            self.tick_due = now + delay;
            SetTimer(Some(self.hwnd), TIMER_TICK, (delay * 1000.0).ceil() as u32 + 20, None);
        }
    }

    fn toggle_pause(&mut self) {
        if self.paused {
            self.deadline = now() + self.paused_remaining;
        } else {
            self.paused_remaining = (self.deadline - now()).max(0.0);
        }
        self.paused = !self.paused;
        self.refresh_icon();
    }

    fn reset_timer(&mut self) {
        self.deadline = now() + WORK_SECONDS;
        if self.paused {
            self.paused_remaining = WORK_SECONDS;
        }
        self.refresh_icon();
    }

    fn show_break(&mut self) {
        if self.brk.is_some() {
            return;
        }
        unsafe {
            let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(monitor, &mut info);
            let r = info.rcMonitor;
            let (width, height) = ((r.right - r.left) as u32, (r.bottom - r.top) as u32);
            let (mut dpi_x, mut dpi_y) = (96, 96);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            let overlay = Overlay::new(width, height, dpi_x as f32 / 96.0);

            let Ok(instance) = GetModuleHandleW(None) else { return };
            let Ok(hwnd) = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                OVERLAY_CLASS,
                PCWSTR::null(),
                WS_POPUP,
                r.left,
                r.top,
                width as i32,
                height as i32,
                None,
                None,
                Some(instance.into()),
                None,
            ) else {
                return;
            };
            launcher::hide();
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(instance.into()), 0).ok();
            self.brk = Some(Break { hwnd, hook, started_at: now(), overlay });
            capsule::set_break(true, self.bar());
            SetTimer(Some(self.hwnd), TIMER_FRAME, 1000 / 30, None);
            SetTimer(Some(self.hwnd), TIMER_BREAK_END, (BREAK_SECONDS * 1000.0) as u32, None);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }

    fn advance_overlay(&mut self) {
        let Some(brk) = &mut self.brk else { return };
        let elapsed = now() - brk.started_at;
        if elapsed < STARTUP_BAR_DELAY {
            return;
        }
        if let Some(dirty) = brk.overlay.draw_progress(startup_progress_at(elapsed)) {
            let rect = RECT { left: dirty.left(), top: dirty.top(), right: dirty.right(), bottom: dirty.bottom() };
            unsafe {
                let _ = InvalidateRect(Some(brk.hwnd), Some(&rect), false);
            }
        }
    }

    fn dismiss_break(&mut self) {
        let Some(brk) = self.brk.take() else { return };
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_FRAME);
            let _ = KillTimer(Some(self.hwnd), TIMER_BREAK_END);
            if let Some(hook) = brk.hook {
                let _ = UnhookWindowsHookEx(hook);
            }
            let _ = DestroyWindow(brk.hwnd);
        }
        capsule::set_break(false, self.bar());
        self.deadline = now() + WORK_SECONDS;
        self.refresh_icon();
    }

    fn tick(&mut self) {
        let now = now();
        if now - self.tick_due > SLEEP_GAP_THRESHOLD {
            return self.reset_timer();
        }
        if !self.paused && now >= self.deadline && self.brk.is_none() {
            self.show_break();
            return self.schedule_tick();
        }
        if self.last_face != Some(self.face()) {
            self.refresh_icon();
        } else {
            self.schedule_tick();
        }
    }

    fn bar(&self) -> Option<&capsule::Bar> {
        match &self.bar {
            BarState::Running(bar) | BarState::Stopping(bar) => Some(bar),
            BarState::Off => None,
        }
    }

    fn start_bar(&mut self) {
        if let Some(bar) = capsule::Bar::start(self.bar_opts) {
            self.bar = BarState::Running(bar);
        }
    }

    fn stop_bar(&mut self) {
        self.bar = match std::mem::replace(&mut self.bar, BarState::Off) {
            BarState::Running(bar) => {
                bar.stop();
                BarState::Stopping(bar)
            }
            state => state,
        };
    }

    fn set_bar(&mut self, on: bool) {
        let running = match self.bar {
            BarState::Off => false,
            BarState::Running(_) => true,
            BarState::Stopping(_) => return,
        };
        if on == running {
            return;
        }
        capsule::set_enabled(on);
        if on { self.start_bar() } else { self.stop_bar() }
    }

    fn toggle_bar(&mut self) {
        self.set_bar(matches!(self.bar, BarState::Off));
    }

    fn disable_bar(&mut self) {
        self.set_bar(false);
    }

    fn restart_bar(&mut self) {
        if matches!(self.bar, BarState::Running(_)) {
            self.restarting = true;
            self.stop_bar();
        }
    }

    fn reload_config(&mut self) {
        if let BarState::Running(bar) = &self.bar {
            bar.reload_config();
        }
        self.apply_launcher();
    }

    /// Registers the launcher hotkey from the config, or frees the launcher.
    fn apply_launcher(&mut self) {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_LAUNCHER);
        }
        let (cfg, _) = capsule::launcher_config();
        if !cfg.enabled {
            self.launcher = LauncherState::Off;
            launcher::unload();
            return;
        }
        let registered = hotkey::parse(&cfg.hotkey).is_some_and(|(mods, vk)| unsafe {
            RegisterHotKey(Some(self.hwnd), HOTKEY_LAUNCHER, HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT, vk).is_ok()
        });
        if !registered {
            log::warn!("launcher hotkey {:?} is invalid or in use", cfg.hotkey);
        }
        self.launcher = if registered { LauncherState::On } else { LauncherState::NoHotkey };
    }

    fn toggle_launcher(&mut self) {
        capsule::set_launcher_enabled(self.launcher == LauncherState::Off);
        self.apply_launcher();
    }

    fn on_bar_exited(&mut self) {
        if let BarState::Running(bar) | BarState::Stopping(bar) = std::mem::replace(&mut self.bar, BarState::Off) {
            bar.join();
        }
        if self.quitting {
            self.quit();
        } else if std::mem::take(&mut self.restarting) {
            self.start_bar();
        }
    }

    fn quit(&mut self) {
        self.quitting = true;
        if self.bar().is_some() {
            return self.stop_bar();
        }
        self.dismiss_break();
        launcher::flush_history();
        launcher::unload();
        self.notify(NIM_DELETE);
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn bitmap_info(width: i32, height: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn create_icon(pixmap: &Pixmap) -> HICON {
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    unsafe {
        let mut bits: *mut c_void = null_mut();
        let Ok(color) = CreateDIBSection(None, &bitmap_info(w, h), DIB_RGB_COLORS, &mut bits, None, 0) else {
            return HICON::default();
        };
        let dst = std::slice::from_raw_parts_mut(bits as *mut [u8; 4], (w * h) as usize);
        for (dst, px) in dst.iter_mut().zip(pixmap.pixels()) {
            let c = px.demultiply();
            *dst = [c.blue(), c.green(), c.red(), c.alpha()];
        }
        let mask_bits = vec![0u8; (w as usize).div_ceil(16) * 2 * h as usize];
        let mask = CreateBitmap(w, h, 1, 1, Some(mask_bits.as_ptr() as *const c_void));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

fn paint_overlay(hdc: HDC, overlay: &Overlay) {
    for patch in overlay.patches() {
        let (w, h) = (patch.rect.width(), patch.rect.height());
        unsafe {
            SetDIBitsToDevice(
                hdc,
                patch.rect.x(),
                patch.rect.y(),
                w,
                h,
                0,
                0,
                0,
                h,
                patch.pixels.as_ptr() as *const c_void,
                &bitmap_info(w as i32, h as i32),
                DIB_RGB_COLORS,
            );
        }
    }
}

fn show_menu(hwnd: HWND) {
    let Some((paused, bar_flags, launcher)) = with_app(|app| {
        let bar_flags = match app.bar {
            BarState::Off => MF_UNCHECKED,
            BarState::Running(_) => MF_CHECKED,
            BarState::Stopping(_) => MF_GRAYED,
        };
        (app.paused, bar_flags, app.launcher)
    }) else {
        return;
    };
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let items = [
            (MF_STRING | bar_flags, CMD_BAR, w!("Capsule bar")),
            match launcher {
                LauncherState::Off => (MF_STRING | MF_UNCHECKED, CMD_LAUNCHER, w!("Launcher")),
                LauncherState::On => (MF_STRING | MF_CHECKED, CMD_LAUNCHER, w!("Launcher")),
                LauncherState::NoHotkey => (MF_STRING | MF_CHECKED, CMD_LAUNCHER, w!("Launcher (hotkey unavailable)")),
            },
            (MF_SEPARATOR, 0, PCWSTR::null()),
            (MF_STRING, CMD_TOGGLE, if paused { w!("Resume") } else { w!("Pause") }),
            (MF_STRING, CMD_BREAK, w!("Break now")),
            (MF_STRING, CMD_RESET, w!("Reset timer")),
            (MF_SEPARATOR, 0, PCWSTR::null()),
            (MF_STRING, CMD_EDIT_CONFIG, w!("Edit config")),
            (MF_STRING, CMD_RELOAD_CONFIG, w!("Reload config")),
            (MF_SEPARATOR, 0, PCWSTR::null()),
            (MF_STRING, CMD_QUIT, w!("Quit")),
        ];
        for (flags, id, text) in items {
            let _ = AppendMenuW(menu, flags, id as usize, text);
        }
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        cmd.0
    };
    run_command(cmd);
}

fn run_command(cmd: i32) {
    match cmd {
        CMD_TOGGLE => with_app(App::toggle_pause),
        CMD_BREAK => with_app(App::show_break),
        CMD_RESET => with_app(App::reset_timer),
        CMD_QUIT => with_app(App::quit),
        CMD_BAR => with_app(App::toggle_bar),
        CMD_LAUNCHER => with_app(App::toggle_launcher),
        CMD_EDIT_CONFIG => {
            capsule::edit_config();
            None
        }
        CMD_RELOAD_CONFIG => with_app(App::reload_config),
        _ => None,
    };
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 && wparam.0 == WM_KEYDOWN as usize {
            let vk = (*(lparam.0 as *const KBDLLHOOKSTRUCT)).vkCode;
            if vk == u32::from(VK_ESCAPE.0) || vk == u32::from(VK_SPACE.0) {
                let _ = PostMessageW(Some(HWND(MAIN_HWND.load(Ordering::Relaxed))), WM_DISMISS, WPARAM(0), LPARAM(0));
                return LRESULT(1);
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            with_app(|app| match wparam.0 {
                TIMER_TICK => app.tick(),
                TIMER_FRAME => app.advance_overlay(),
                TIMER_BREAK_END => app.dismiss_break(),
                _ => {}
            });
        }
        WM_HOTKEY if wparam.0 as i32 == HOTKEY_LAUNCHER => {
            if with_app(|app| app.brk.is_none()) == Some(true) {
                launcher::toggle();
            }
        }
        WM_HOTKEY => {
            with_app(App::toggle_pause);
        }
        WM_LAUNCHER_CMD => run_command(wparam.0 as i32),
        WM_ENDSESSION if wparam.0 != 0 => launcher::flush_history(),
        WM_DISMISS => {
            with_app(App::dismiss_break);
        }
        WM_BAR_EXITED => {
            with_app(App::on_bar_exited);
        }
        WM_BAR_DISABLE => {
            with_app(App::disable_bar);
        }
        WM_BAR_RESTART => {
            with_app(App::restart_bar);
        }
        WM_POWERBROADCAST if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize => {
            with_app(App::reset_timer);
        }
        WM_TRAY => {
            if matches!(lparam.0 as u32, WM_LBUTTONUP | WM_RBUTTONUP) {
                show_menu(hwnd);
            }
        }
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        _ if msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            if with_app(|app| app.notify(NIM_ADD)).is_none() {
                unsafe {
                    let _ = PostMessageW(Some(hwnd), msg, wparam, lparam);
                }
            }
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_PAINT {
            let painted = APP.with(|app| {
                let Ok(app) = app.try_borrow() else { return false };
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if let Some(brk) = app.as_ref().and_then(|app| app.brk.as_ref()) {
                    paint_overlay(hdc, &brk.overlay);
                }
                let _ = EndPaint(hwnd, &ps);
                true
            });
            if painted {
                return LRESULT(0);
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    match args.first().map(String::as_str) {
        Some("--watchdog") => {
            capsule::init_logging("watchdog.txt");
            std::process::exit(capsule::run_watchdog(&args[1..]));
        }
        Some("--restore") => {
            capsule::restore_taskbar();
            return;
        }
        _ => {}
    }
    capsule::init_logging("log.txt");
    unsafe {
        let _ = CreateMutexW(None, false, w!("Local\\202020"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        let Ok(instance) = GetModuleHandleW(None) else { return };
        let instance = HINSTANCE::from(instance);
        capsule::register_class(WNDCLASSEXW {
            lpfnWndProc: Some(main_proc),
            lpszClassName: MAIN_CLASS,
            ..Default::default()
        });
        capsule::register_class(WNDCLASSEXW {
            lpfnWndProc: Some(overlay_proc),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszClassName: OVERLAY_CLASS,
            ..Default::default()
        });
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            MAIN_CLASS,
            w!("202020"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        ) else {
            return;
        };
        MAIN_HWND.store(hwnd.0, Ordering::Relaxed);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);

        let now = now();
        let mut app = App {
            hwnd,
            digits: render::load_digits(),
            icon: HICON::default(),
            paused: false,
            deadline: now + WORK_SECONDS,
            paused_remaining: WORK_SECONDS,
            tick_due: now,
            last_face: None,
            brk: None,
            bar_opts: capsule::Options {
                safe: flag("--safe"),
                owner: hwnd.0 as isize,
                exited_msg: WM_BAR_EXITED,
                disable_msg: WM_BAR_DISABLE,
                restart_msg: WM_BAR_RESTART,
            },
            bar: BarState::Off,
            launcher: LauncherState::Off,
            restarting: false,
            quitting: false,
        };
        app.refresh_icon();
        app.notify(NIM_ADD);
        APP.with(|cell| *cell.borrow_mut() = Some(app));
        if capsule::enabled() {
            with_app(App::start_bar);
        }
        launcher::init(launcher::Options {
            owner: hwnd.0 as isize,
            command_msg: WM_LAUNCHER_CMD,
            commands: LAUNCHER_COMMANDS,
        });
        with_app(App::apply_launcher);

        let _ = RegisterHotKey(Some(hwnd), HOTKEY_TOGGLE, MOD_WIN | MOD_ALT | MOD_NOREPEAT, 'C' as u32);
        let _ = RegisterSuspendResumeNotification(HANDLE(hwnd.0), DEVICE_NOTIFY_WINDOW_HANDLE);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
