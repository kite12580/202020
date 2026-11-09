use std::sync::atomic::Ordering;

use capsule_core::shell_proto::{Answer, PinInfo, Request};
use capsule_core::taskband::{Pin, Task};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use super::system::shell_flyout;
use super::{
    App, BAR, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_FOCUS, EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND,
    TIMER_KEYBOARD, TIMER_PINS, WM_APP_WINEVENT, with_app,
};
use crate::capsule::hook::{self, Hook, HookEvent};
use crate::capsule::pinned::PinnedApp;
use crate::capsule::util::{hwnd, own_window};

pub(super) unsafe extern "system" fn on_winevent(
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

impl App {
    pub(super) fn request_pins(&self) {
        hook::request(self.hwnd, &Request::Pins);
    }

    pub(super) fn on_hook_events(&mut self) {
        let events = self.hook.as_ref().map(Hook::take_events).unwrap_or_default();
        for ev in events {
            match ev {
                HookEvent::Attached => self.on_hook_attached(),
                HookEvent::Answer(Answer::Pins(pins)) => self.on_pins(pins),
                HookEvent::Answer(Answer::Icon { id, bitmap }) => self.on_shell_icon(id, bitmap),
                HookEvent::Answer(Answer::Apps(_)) => {}
            }
        }
    }

    /// Explorer's tray icons are re-added once the hook relays them.
    fn on_hook_attached(&mut self) {
        self.close_popup();
        self.tray.icons.clear();
        self.relayout();
        self.request_pins();
        self.resend_icon_requests();
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

    pub(super) fn on_winevent(&mut self, event: u32, h: HWND) {
        match event {
            EVENT_SYSTEM_FOREGROUND => self.on_foreground(h),
            EVENT_OBJECT_CLOAKED => self.remove_window(h),
            EVENT_OBJECT_UNCLOAKED => self.add_window(h),
            EVENT_OBJECT_FOCUS if h != self.focus => {
                self.focus = h;
                self.set_timer(TIMER_KEYBOARD, 100);
            }
            _ => {}
        }
    }

    pub(super) fn on_shell_hook(&mut self, code: usize, h: HWND) {
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
            REDRAW => self.queue_icon_refresh(h),
            LANGUAGE => self.refresh_keyboard(),
            _ => {}
        }
    }

    pub(super) fn on_pins_signal(&mut self) {
        if let Some(w) = &self.pin_watch {
            w.rearm();
        }
        self.set_timer(TIMER_PINS, 300);
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

    fn on_pins(&mut self, pins: Vec<PinInfo>) {
        log::debug!(
            "taskbar pins {:?}",
            pins.iter().map(|p| (&p.aumid, &p.target, &p.parsing_name)).collect::<Vec<_>>()
        );
        let pins: Vec<PinnedApp> = pins.into_iter().map(PinnedApp::from).collect();
        fn key(p: &PinnedApp) -> (&Pin, &str, &str) {
            (&p.pin, &p.parsing_name, &p.name)
        }
        if pins.iter().map(key).eq(self.pins.iter().map(key)) {
            return;
        }
        self.set_pins(pins);
        self.relayout();
    }

    pub(super) fn task_window(&self, i: usize) -> Option<HWND> {
        match self.tasks[i] {
            Task::Window(w) => Some(self.windows.items[w].hwnd),
            Task::Pin(_) => None,
        }
    }
}
