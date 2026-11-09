use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use capsule_core::config::CapsuleBar;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::System::Power::POWERBROADCAST_SETTING;
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetForegroundWindow, GetWindowLongW, GetWindowRect, HWND_TOPMOST, IsZoomed, SPI_SETHIGHCONTRAST,
    SPI_SETWORKAREA, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow,
    WS_CAPTION,
};
use windows::core::PCWSTR;

use super::{
    App, BREAK, GUID_CONSOLE_DISPLAY_STATE, TIMER_CLOCK, TIMER_CONFIG, TIMER_REVEAL, TIMER_SLOW, TIMER_WIN_DIGIT,
    TIMER_WORK_AREA,
};
use crate::capsule::input;
use crate::capsule::shell::{self, REAL_TRAY};
use crate::capsule::theme::{self, Theme};
use crate::capsule::util::{class_name, own_window, primary_monitor};

const PRUNE_EVERY: u32 = 5;

const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
const PBT_POWERSETTINGCHANGE: usize = 0x8013;

const CORE_WINDOW_CLASS: &str = "Windows.UI.Core.CoreWindow";

/// Shell flyouts such as the input switcher and the IME mode indicator are
/// top-level CoreWindows that take the foreground and can cover the whole
/// monitor without being fullscreen apps.
pub(super) fn shell_flyout(h: HWND) -> bool {
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

impl App {
    pub(super) fn on_taskbar_created(&mut self) {
        let real = shell::find_real_tray();
        if real == shell::real_tray() {
            return;
        }
        log::info!("explorer restarted");
        REAL_TRAY.store(real.map_or(0, |h| h.0 as isize), Ordering::Relaxed);
        if let (Some(t), Some(real)) = (&mut self.takeover, real) {
            t.apply(real);
        }
        self.register_appbar();
    }

    pub(super) fn update_visibility(&mut self) {
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

    pub(super) fn slow_tick(&mut self) {
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

    pub(super) fn paused(&self) -> bool {
        self.locked || self.display_off
    }

    pub(super) fn set_paused(&mut self, locked: bool, display_off: bool) {
        let was = self.paused();
        self.locked = locked;
        self.display_off = display_off;
        let now = self.paused();
        if now == was {
            return;
        }
        if now {
            self.kill_timer(TIMER_SLOW);
            self.kill_timer(TIMER_CLOCK);
        } else {
            self.start_slow_timer();
            self.refresh_clock();
            self.refresh_keyboard();
            self.slow_tick();
        }
    }

    pub(super) fn on_config_signal(&mut self) {
        self.cfg.rearm_watch();
        self.set_timer(TIMER_CONFIG, 250);
    }

    pub(super) fn reload_config(&mut self) {
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

    pub(super) fn on_setting_change(&mut self, wparam: WPARAM, lparam: LPARAM) {
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
    }

    pub(super) fn on_power(&mut self, wparam: WPARAM, lparam: LPARAM) {
        if wparam.0 == PBT_APMRESUMEAUTOMATIC {
            self.refresh_clock();
            self.refresh_keyboard();
        } else if wparam.0 == PBT_POWERSETTINGCHANGE {
            let s = unsafe { &*(lparam.0 as *const POWERBROADCAST_SETTING) };
            if s.PowerSetting == GUID_CONSOLE_DISPLAY_STATE {
                self.set_paused(self.locked, s.Data[0] == 0);
            }
        }
    }

    pub(super) fn schedule_work_area_check(&self) {
        if self.takeover.is_some() {
            self.set_timer(TIMER_WORK_AREA, 250);
        }
    }

    pub(super) fn fix_work_area(&mut self) {
        let Some(t) = &self.takeover else { return };
        if self.last_work_area_fix.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            self.schedule_work_area_check();
            return;
        }
        self.last_work_area_fix = Some(Instant::now());
        t.ensure_work_area();
    }

    pub(super) fn on_win_digit(&mut self, vk: u16) {
        let Some(t) = &self.takeover else { return input::replay_win_key(vk) };
        self.last_win_digit = Some(Instant::now());
        if t.revealed() && self.pending_win_digits.is_empty() {
            input::replay_win_key(vk);
        } else {
            t.reveal();
            self.pending_win_digits.push(vk);
            self.set_timer(TIMER_WIN_DIGIT, 60);
        }
        self.set_timer(TIMER_REVEAL, 300);
    }

    pub(super) fn conceal_when_idle(&mut self) {
        let Some(t) = &self.takeover else { return };
        if input::win_held() || self.last_win_digit.is_some_and(|d| d.elapsed() < Duration::from_millis(800)) {
            self.set_timer(TIMER_REVEAL, 300);
        } else {
            t.conceal();
        }
    }
}
