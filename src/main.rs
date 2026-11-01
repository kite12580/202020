#![windows_subsystem = "windows"]

mod capsule;
mod render;

use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ab_glyph::FontVec;
use tiny_skia::Pixmap;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::RegisterSuspendResumeNotification;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::core::{PCWSTR, w};

use render::{IconFace, Overlay};

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
const WM_BAR_QUIT: u32 = WM_APP + 4;
const WM_BAR_DISABLE: u32 = WM_APP + 5;
const TRAY_ID: u32 = 1;
const HOTKEY_TOGGLE: i32 = 1;
const TIMER_TICK: usize = 1;
const TIMER_FRAME: usize = 2;
const TIMER_BREAK_END: usize = 3;
const CMD_TOGGLE: usize = 1;
const CMD_BREAK: usize = 2;
const CMD_RESET: usize = 3;
const CMD_QUIT: usize = 4;
const CMD_BAR: usize = 5;
const CMD_EDIT_CONFIG: usize = 6;

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
    hook: HHOOK,
    started_at: f64,
    overlay: Overlay,
}

struct App {
    hwnd: HWND,
    font: FontVec,
    icon: HICON,
    paused: bool,
    deadline: f64,
    paused_remaining: f64,
    last_tick: f64,
    last_face: Option<IconFace>,
    brk: Option<Break>,
    bar_opts: capsule::Options,
    bar: BarState,
    quitting: bool,
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
        unsafe {
            let mut data: NOTIFYICONDATAW = zeroed();
            data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = self.hwnd;
            data.uID = TRAY_ID;
            data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            data.uCallbackMessage = WM_TRAY;
            data.hIcon = self.icon;
            for (dst, src) in data.szTip.iter_mut().zip("202020".encode_utf16()) {
                *dst = src;
            }
            Shell_NotifyIconW(message, &data);
        }
    }

    fn refresh_icon(&mut self) {
        let face = self.face();
        self.last_face = Some(face);
        unsafe {
            let size = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForWindow(self.hwnd)).max(16) as u32;
            let previous = std::mem::replace(&mut self.icon, create_icon(&render::tray_icon(&self.font, size, face)));
            self.notify(NIM_MODIFY);
            if !previous.is_null() {
                DestroyIcon(previous);
            }
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
            let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut info: MONITORINFO = zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(monitor, &mut info);
            let r = info.rcMonitor;
            let (width, height) = ((r.right - r.left) as u32, (r.bottom - r.top) as u32);
            let (mut dpi_x, mut dpi_y) = (96, 96);
            GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            let overlay = Overlay::new(width, height, dpi_x as f32 / 96.0);

            let instance = GetModuleHandleW(null());
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                OVERLAY_CLASS,
                null(),
                WS_POPUP,
                r.left,
                r.top,
                width as i32,
                height as i32,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), instance, 0);
            self.brk = Some(Break { hwnd, hook, started_at: now(), overlay });
            capsule::set_break(true, self.bar());
            SetTimer(self.hwnd, TIMER_FRAME, 1000 / 30, None);
            SetTimer(self.hwnd, TIMER_BREAK_END, (BREAK_SECONDS * 1000.0) as u32, None);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
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
            unsafe { InvalidateRect(brk.hwnd, &rect, 0) };
        }
    }

    fn dismiss_break(&mut self) {
        let Some(brk) = self.brk.take() else { return };
        unsafe {
            KillTimer(self.hwnd, TIMER_FRAME);
            KillTimer(self.hwnd, TIMER_BREAK_END);
            UnhookWindowsHookEx(brk.hook);
            DestroyWindow(brk.hwnd);
        }
        capsule::set_break(false, self.bar());
        self.deadline = now() + WORK_SECONDS;
        self.refresh_icon();
    }

    fn tick(&mut self) {
        let now = now();
        let gap = now - self.last_tick;
        self.last_tick = now;
        if gap > SLEEP_GAP_THRESHOLD {
            return self.reset_timer();
        }
        if !self.paused && now >= self.deadline && self.brk.is_none() {
            return self.show_break();
        }
        if self.last_face != Some(self.face()) {
            self.refresh_icon();
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

    fn on_bar_exited(&mut self) {
        if let BarState::Running(bar) | BarState::Stopping(bar) = std::mem::replace(&mut self.bar, BarState::Off) {
            bar.join();
        }
        if self.quitting {
            self.quit();
        }
    }

    fn quit(&mut self) {
        self.quitting = true;
        if self.bar().is_some() {
            return self.stop_bar();
        }
        self.dismiss_break();
        self.notify(NIM_DELETE);
        unsafe { DestroyWindow(self.hwnd) };
    }
}

fn bitmap_info(width: i32, height: i32) -> BITMAPINFO {
    let mut bmi: BITMAPINFO = unsafe { zeroed() };
    bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = width;
    bmi.bmiHeader.biHeight = -height;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;
    bmi
}

fn create_icon(pixmap: &Pixmap) -> HICON {
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    unsafe {
        let mut bits: *mut c_void = null_mut();
        let color = CreateDIBSection(null_mut(), &bitmap_info(w, h), DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if color.is_null() {
            return null_mut();
        }
        let dst = std::slice::from_raw_parts_mut(bits as *mut [u8; 4], (w * h) as usize);
        for (dst, px) in dst.iter_mut().zip(pixmap.pixels()) {
            let c = px.demultiply();
            *dst = [c.blue(), c.green(), c.red(), c.alpha()];
        }
        let mask_bits = vec![0u8; (w as usize).div_ceil(16) * 2 * h as usize];
        let mask = CreateBitmap(w, h, 1, 1, mask_bits.as_ptr() as *const c_void);
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

fn paint_overlay(hdc: HDC, overlay: &Overlay) {
    let (w, h) = (overlay.width(), overlay.height());
    unsafe {
        SetDIBitsToDevice(
            hdc,
            0,
            0,
            w,
            h,
            0,
            0,
            0,
            h,
            overlay.pixels().as_ptr() as *const c_void,
            &bitmap_info(w as i32, h as i32),
            DIB_RGB_COLORS,
        );
    }
}

fn show_menu(hwnd: HWND) {
    let Some((paused, bar_flags)) = with_app(|app| {
        let bar_flags = match app.bar {
            BarState::Off => MF_UNCHECKED,
            BarState::Running(_) => MF_CHECKED,
            BarState::Stopping(_) => MF_GRAYED,
        };
        (app.paused, bar_flags)
    }) else {
        return;
    };
    let cmd = unsafe {
        let menu = CreatePopupMenu();
        AppendMenuW(menu, MF_STRING, CMD_TOGGLE, if paused { w!("Resume") } else { w!("Pause") });
        AppendMenuW(menu, MF_STRING, CMD_BREAK, w!("Break now"));
        AppendMenuW(menu, MF_STRING, CMD_RESET, w!("Reset timer"));
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING | bar_flags, CMD_BAR, w!("Capsule bar"));
        AppendMenuW(menu, MF_STRING, CMD_EDIT_CONFIG, w!("Edit config"));
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Quit"));
        let mut pt: POINT = zeroed();
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        cmd as usize
    };
    match cmd {
        CMD_TOGGLE => with_app(App::toggle_pause),
        CMD_BREAK => with_app(App::show_break),
        CMD_RESET => with_app(App::reset_timer),
        CMD_QUIT => with_app(App::quit),
        CMD_BAR => with_app(App::toggle_bar),
        CMD_EDIT_CONFIG => {
            capsule::edit_config();
            None
        }
        _ => None,
    };
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 && wparam == WM_KEYDOWN as usize {
            let vk = (*(lparam as *const KBDLLHOOKSTRUCT)).vkCode;
            if vk == VK_ESCAPE as u32 || vk == VK_SPACE as u32 {
                PostMessageW(MAIN_HWND.load(Ordering::Relaxed), WM_DISMISS, 0, 0);
                return 1;
            }
        }
        CallNextHookEx(null_mut(), code, wparam, lparam)
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            with_app(|app| match wparam {
                TIMER_TICK => app.tick(),
                TIMER_FRAME => app.advance_overlay(),
                TIMER_BREAK_END => app.dismiss_break(),
                _ => {}
            });
        }
        WM_HOTKEY => {
            with_app(App::toggle_pause);
        }
        WM_DISMISS => {
            with_app(App::dismiss_break);
        }
        WM_BAR_EXITED => {
            with_app(App::on_bar_exited);
        }
        WM_BAR_QUIT => {
            with_app(App::quit);
        }
        WM_BAR_DISABLE => {
            with_app(App::disable_bar);
        }
        WM_POWERBROADCAST if wparam == PBT_APMRESUMEAUTOMATIC as usize => {
            with_app(App::reset_timer);
        }
        WM_TRAY => {
            if matches!(lparam as u32, WM_LBUTTONUP | WM_RBUTTONUP) {
                show_menu(hwnd);
            }
        }
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        _ if msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            if with_app(|app| app.notify(NIM_ADD)).is_none() {
                unsafe { PostMessageW(hwnd, msg, wparam, lparam) };
            }
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    0
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_PAINT {
            let painted = APP.with(|app| {
                let Ok(app) = app.try_borrow() else { return false };
                let mut ps: PAINTSTRUCT = zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                if let Some(brk) = app.as_ref().and_then(|app| app.brk.as_ref()) {
                    paint_overlay(hdc, &brk.overlay);
                }
                EndPaint(hwnd, &ps);
                true
            });
            if painted {
                return 0;
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

fn register_class(instance: HINSTANCE, name: PCWSTR, proc: WNDPROC, background: HBRUSH) {
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: proc,
            hInstance: instance,
            lpszClassName: name,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: background,
            ..zeroed()
        };
        RegisterClassW(&class);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let verbose = flag("--verbose");
    match args.first().map(String::as_str) {
        Some("--watchdog") => {
            if verbose {
                capsule::init_logging("watchdog.txt", log::LevelFilter::Debug);
            }
            std::process::exit(capsule::run_watchdog(&args[1..]));
        }
        Some("--restore") => {
            capsule::restore_taskbar();
            return;
        }
        _ => {}
    }
    let diag = flag("--diag");
    if verbose || diag {
        capsule::init_logging("log.txt", log::LevelFilter::Debug);
        std::panic::set_hook(Box::new(|info| {
            log::error!("panic: {info}");
            log::logger().flush();
        }));
    }
    unsafe {
        CreateMutexW(null(), 0, w!("Local\\202020"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        let instance = GetModuleHandleW(null());
        register_class(instance, MAIN_CLASS, Some(main_proc), null_mut());
        register_class(instance, OVERLAY_CLASS, Some(overlay_proc), GetStockObject(BLACK_BRUSH) as HBRUSH);
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            MAIN_CLASS,
            w!("202020"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        MAIN_HWND.store(hwnd, Ordering::Relaxed);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);

        let now = now();
        let mut app = App {
            hwnd,
            font: render::load_font(),
            icon: null_mut(),
            paused: false,
            deadline: now + WORK_SECONDS,
            paused_remaining: WORK_SECONDS,
            last_tick: now,
            last_face: None,
            brk: None,
            bar_opts: capsule::Options {
                safe: flag("--safe"),
                diag,
                owner: hwnd as isize,
                exited_msg: WM_BAR_EXITED,
                quit_msg: WM_BAR_QUIT,
                disable_msg: WM_BAR_DISABLE,
            },
            bar: BarState::Off,
            quitting: false,
        };
        app.refresh_icon();
        app.notify(NIM_ADD);
        APP.with(|cell| *cell.borrow_mut() = Some(app));
        if capsule::enabled() {
            with_app(App::start_bar);
        }

        SetTimer(hwnd, TIMER_TICK, 1000, None);
        RegisterHotKey(hwnd, HOTKEY_TOGGLE, MOD_WIN | MOD_ALT | MOD_NOREPEAT, 'C' as u32);
        RegisterSuspendResumeNotification(hwnd, DEVICE_NOTIFY_WINDOW_HANDLE);

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
