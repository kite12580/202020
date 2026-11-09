use capsule_core::layout::{self, Hit};
use capsule_core::taskband::Task;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Globalization::{DATE_LONGDATE, GetDateFormatEx, GetTimeFormatEx, TIME_FORMAT_FLAGS};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Controls::HOVER_DEFAULT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_HOVER, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetSystemMetrics, SM_CXDRAG, SM_CYDRAG, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONUP,
};
use windows::core::{PCWSTR, w};

use super::menus::DeferredMenu;
use super::{App, cursor};
use crate::capsule::input;
use crate::capsule::keyboard;
use crate::capsule::launcher;
use crate::capsule::util::{from_wide, lparam_point, own_window, primary_monitor};
use crate::capsule::windows_list::{self as winlist};

const SNAP: f32 = 12.0;

pub(super) struct Drag {
    cursor: POINT,
    origin: POINT,
    moved: bool,
    hit: Option<Hit>,
    double: bool,
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
    pub(super) fn track_mouse(&self, h: HWND) {
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

    pub(super) fn set_hover(&mut self, hit: Option<Hit>) {
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

    pub(super) fn show_tip(&mut self) {
        let Some(hit) = self.hover else { return };
        let Some(r) = self.layout.rect(hit) else { return };
        let x = self.bounds.left + self.px(r.center_x());
        let text = match hit {
            Hit::Task(i) => match self.tasks[i] {
                Task::Window(w) => winlist::title(self.windows.items[w].hwnd),
                Task::Pin(p) => self.pins[p].name.clone(),
            },
            Hit::Tray(i) => return self.tray_tip(self.bar_tray[i], x, self.bounds.top, self.bounds.bottom),
            Hit::Keyboard => keyboard::display_name(self.kbd.layout),
            Hit::Volume if !self.volume.available() => "No audio output device".into(),
            Hit::Volume if self.volume.muted() => "Muted".into(),
            Hit::Volume => format!("Volume {}%", (self.volume.level() * 100.0).round()),
            Hit::Clock => long_date(),
            Hit::Start => "Start".into(),
            Hit::Chevron => return,
        };
        self.tooltip.show(&text, x, self.bounds.top, self.bounds.bottom);
    }

    fn foreground_root(&self) -> HWND {
        let fg = self.root(unsafe { GetForegroundWindow() });
        if fg.is_invalid() || own_window(fg) { self.foreground } else { fg }
    }

    pub(super) fn on_bar_mouse(&mut self, msg: u32, lparam: LPARAM) {
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
            (WM_RBUTTONUP, Hit::Start) => input::press_win(b'X'),
            (WM_RBUTTONUP, Hit::Keyboard) => self.deferred.push_back(DeferredMenu::Keyboard),
            (WM_RBUTTONUP, Hit::Clock) => self.deferred.push_back(DeferredMenu::App),
            (_, Hit::Tray(i)) => self.tray_mouse(self.bar_tray[i], msg),
            _ => {}
        }
    }

    /// A left click that did not turn into a drag.
    fn left_click(&mut self, hit: Hit, double: bool) {
        match hit {
            Hit::Start => input::press_start(),
            Hit::Task(i) => match self.tasks[i] {
                Task::Window(w) => winlist::activate_or_minimize(self.windows.items[w].hwnd, self.foreground_root()),
                Task::Pin(p) => {
                    launcher::open(self.hwnd, &self.pins[p].parsing_name, "");
                }
            },
            Hit::Chevron => self.toggle_popup(),
            Hit::Keyboard => self.next_keyboard_layout(),
            Hit::Volume => input::press_win(b'A'),
            Hit::Clock => input::press_win(b'N'),
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

    pub(super) fn end_drag(&mut self) {
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

    pub(super) fn on_wheel(&mut self, wparam: WPARAM, lparam: LPARAM) {
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
}
