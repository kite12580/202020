use capsule_core::kbd;
use windows::Win32::Foundation::{HWND, LRESULT};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use super::system::shell_flyout;
use super::{App, TIMER_KEYBOARD, with_app};
use crate::capsule::gfx::Font;
use crate::capsule::keyboard;
use crate::capsule::util::own_window;

#[derive(Default)]
pub(super) struct KeyboardState {
    pub(super) layout: isize,
    iso: String,
    label: String,
    retries: u8,
    ime_native: Option<bool>,
    ime_query: usize,
}

unsafe extern "system" fn on_ime_open(ime: HWND, _msg: u32, query: usize, open: LRESULT) {
    with_app(|a| a.on_ime_open(ime, query, open.0 != 0));
}

unsafe extern "system" fn on_ime_mode(_ime: HWND, _msg: u32, query: usize, mode: LRESULT) {
    with_app(|a| a.on_ime_mode(query, keyboard::native_mode(mode.0 as usize)));
}

impl App {
    /// Widest label any installed layout can show, so switching layouts or
    /// IME modes never resizes the bar.
    pub(super) fn measure_keyboard_slot(&mut self) {
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

    pub(super) fn input_target(&self) -> HWND {
        let fg = unsafe { GetForegroundWindow() };
        if fg.is_invalid() || own_window(fg) || shell_flyout(fg) { self.foreground } else { fg }
    }

    pub(super) fn refresh_keyboard(&mut self) {
        let target = self.input_target();
        let hkl = keyboard::current(target);
        if hkl == 0 {
            return;
        }
        if hkl != self.kbd.layout {
            self.kbd.layout = hkl;
            self.kbd.iso = keyboard::iso(hkl);
            self.kbd.ime_native = None;
        }
        self.kbd.ime_query = self.kbd.ime_query.wrapping_add(1);
        if !kbd::has_ime(&self.kbd.iso) || !keyboard::query_ime_open(target, self.kbd.ime_query, on_ime_open) {
            self.kbd.ime_native = None;
        }
        self.update_keyboard_label();
    }

    fn on_ime_open(&mut self, ime: HWND, query: usize, open: bool) {
        if query != self.kbd.ime_query {
            return;
        }
        if !open {
            self.kbd.ime_native = Some(false);
        } else if !keyboard::query_ime_mode(ime, query, on_ime_mode) {
            self.kbd.ime_native = None;
        }
        self.update_keyboard_label();
    }

    fn on_ime_mode(&mut self, query: usize, native: bool) {
        if query == self.kbd.ime_query {
            self.kbd.ime_native = Some(native);
            self.update_keyboard_label();
        }
    }

    fn update_keyboard_label(&mut self) {
        let label = kbd::label(&self.kbd.iso, &self.cfg.config.keyboard.labels, self.kbd.ime_native);
        if label != self.kbd.label {
            log::debug!("keyboard label {label}");
            self.texts.keyboard = self.gfx.text(&label, Font::Text);
            self.kbd.label = label;
            self.relayout();
        }
    }

    pub(super) fn next_keyboard_layout(&mut self) {
        let layouts = keyboard::installed();
        let next = layouts.iter().position(|&h| h == self.kbd.layout).map_or(0, |i| (i + 1) % layouts.len());
        let Some(&hkl) = layouts.get(next) else { return };
        keyboard::switch(self.input_target(), hkl);
        self.refresh_keyboard_soon();
    }

    pub(super) fn on_keyboard_timer(&mut self) {
        self.refresh_keyboard();
        if self.kbd.retries > 0 {
            self.kbd.retries -= 1;
            self.set_timer(TIMER_KEYBOARD, 200);
        }
    }

    pub(super) fn refresh_keyboard_soon(&mut self) {
        self.kbd.retries = 2;
        self.set_timer(TIMER_KEYBOARD, 40);
    }
}
