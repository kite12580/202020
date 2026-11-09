use std::time::{SystemTime, UNIX_EPOCH};

use capsule_core::config::Launcher;
use capsule_core::launcher::hotkey;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use crate::capsule::{self, launcher};
use crate::gdi::create_icon;
use crate::render::{self, Digits, IconFace, Overlay};
use crate::{
    HOTKEY_LAUNCHER, OVERLAY_CLASS, TIMER_BREAK_END, TIMER_FRAME, TIMER_TICK, TRAY_ID, WM_BAR_DISABLE, WM_BAR_EXITED,
    WM_BAR_RESTART, WM_TRAY, keyboard_hook,
};

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

pub(super) fn now() -> f64 {
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

pub(super) struct Break {
    hwnd: HWND,
    hook: Option<HHOOK>,
    started_at: f64,
    pub(super) overlay: Overlay,
}

pub(super) struct App {
    hwnd: HWND,
    digits: Digits,
    icon: HICON,
    pub(super) paused: bool,
    deadline: f64,
    paused_remaining: f64,
    tick_due: f64,
    last_face: Option<IconFace>,
    pub(super) brk: Option<Break>,
    bar_opts: capsule::Options,
    pub(super) bar: BarState,
    pub(super) launcher: LauncherState,
    restarting: bool,
    quitting: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LauncherState {
    Off,
    On,
    /// Enabled, but the hotkey is invalid or taken by another program.
    NoHotkey,
}

pub(super) enum BarState {
    Off,
    Running(capsule::Bar),
    Stopping(capsule::Bar),
}

impl App {
    pub(super) fn new(hwnd: HWND, safe: bool) -> App {
        let now = now();
        App {
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
                safe,
                owner: hwnd.0 as isize,
                exited_msg: WM_BAR_EXITED,
                disable_msg: WM_BAR_DISABLE,
                restart_msg: WM_BAR_RESTART,
            },
            bar: BarState::Off,
            launcher: LauncherState::Off,
            restarting: false,
            quitting: false,
        }
    }

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

    pub(super) fn notify(&self, message: NOTIFY_ICON_MESSAGE) {
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

    pub(super) fn refresh_icon(&mut self) {
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

    pub(super) fn toggle_pause(&mut self) {
        if self.paused {
            self.deadline = now() + self.paused_remaining;
        } else {
            self.paused_remaining = (self.deadline - now()).max(0.0);
        }
        self.paused = !self.paused;
        self.refresh_icon();
    }

    pub(super) fn reset_timer(&mut self) {
        self.deadline = now() + WORK_SECONDS;
        if self.paused {
            self.paused_remaining = WORK_SECONDS;
        }
        self.refresh_icon();
    }

    pub(super) fn show_break(&mut self) {
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

    pub(super) fn advance_overlay(&mut self) {
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

    pub(super) fn dismiss_break(&mut self) {
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

    pub(super) fn tick(&mut self) {
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

    pub(super) fn bar(&self) -> Option<&capsule::Bar> {
        match &self.bar {
            BarState::Running(bar) | BarState::Stopping(bar) => Some(bar),
            BarState::Off => None,
        }
    }

    pub(super) fn start_bar(&mut self) {
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

    pub(super) fn toggle_bar(&mut self) {
        self.set_bar(matches!(self.bar, BarState::Off));
    }

    pub(super) fn disable_bar(&mut self) {
        self.set_bar(false);
    }

    pub(super) fn restart_bar(&mut self) {
        if matches!(self.bar, BarState::Running(_)) {
            self.restarting = true;
            self.stop_bar();
        }
    }

    pub(super) fn reload_config(&mut self) {
        if let BarState::Running(bar) = &self.bar {
            bar.reload_config();
        }
        let cfg = capsule::config();
        self.apply_launcher(cfg.launcher);
        capsule::hyper::apply(cfg.hyper);
    }

    /// Registers the launcher hotkey from the config, or frees the launcher.
    pub(super) fn apply_launcher(&mut self, cfg: Launcher) {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_LAUNCHER);
        }
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

    pub(super) fn toggle_launcher(&mut self) {
        capsule::set_launcher_enabled(self.launcher == LauncherState::Off);
        self.apply_launcher(capsule::launcher_config());
    }

    pub(super) fn toggle_hyper(&mut self) {
        capsule::set_hyper_enabled(!capsule::hyper::on());
        capsule::hyper::apply(capsule::config().hyper);
    }

    pub(super) fn on_bar_exited(&mut self) {
        if let BarState::Running(bar) | BarState::Stopping(bar) = std::mem::replace(&mut self.bar, BarState::Off) {
            bar.join();
        }
        if self.quitting {
            self.quit();
        } else if std::mem::take(&mut self.restarting) {
            self.start_bar();
        }
    }

    pub(super) fn quit(&mut self) {
        self.quitting = true;
        if self.bar().is_some() {
            return self.stop_bar();
        }
        self.dismiss_break();
        capsule::hyper::stop();
        launcher::flush_history();
        launcher::unload();
        self.notify(NIM_DELETE);
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
