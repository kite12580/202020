use capsule_core::launcher::input::TextInput;
use windows::Win32::Foundation::{LPARAM, POINT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VIRTUAL_KEY, VK_A, VK_BACK, VK_C, VK_CONTROL, VK_DELETE,
    VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_INSERT, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB,
    VK_UP, VK_V, VK_X,
};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use super::view::SEARCH_H;
use super::{Deferred, Launcher};
use crate::capsule::clipboard;
use crate::capsule::util::lparam_point;

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk.0 as i32) < 0 }
}

impl Launcher {
    fn edited(&mut self) {
        self.selected = 0;
        self.caret_on = true;
        self.refresh();
        self.place_ime();
    }

    fn select(&mut self, i: usize) {
        if i != self.selected && i < self.rows.len() {
            self.selected = i;
            self.invalidate();
        }
    }

    pub(super) fn step(&mut self, delta: isize) {
        let n = self.rows.len() as isize;
        if n > 0 {
            self.select((self.selected as isize + delta).rem_euclid(n) as usize);
        }
    }

    pub(super) fn on_key(&mut self, vk: u16) -> bool {
        let (ctrl, shift) = (key_down(VK_CONTROL), key_down(VK_SHIFT));
        let edit = |l: &mut Launcher, f: &dyn Fn(&mut TextInput)| {
            let before = l.input.text();
            f(&mut l.input);
            l.caret_on = true;
            if l.input.text() != before {
                l.edited();
            } else {
                l.place_ime();
                l.repaint_search();
            }
        };
        if [VK_RETURN, VK_UP, VK_DOWN, VK_TAB, VK_PRIOR, VK_NEXT].iter().any(|k| k.0 == vk) {
            self.settle();
        }
        match vk {
            v if v == VK_ESCAPE.0 => self.hide(),
            v if v == VK_RETURN.0 => self.deferred = Some(Deferred::Activate(self.selected)),
            v if v == VK_UP.0 => self.step(-1),
            v if v == VK_DOWN.0 => self.step(1),
            v if v == VK_TAB.0 => self.step(if shift { -1 } else { 1 }),
            v if v == VK_PRIOR.0 => self.select(0),
            v if v == VK_NEXT.0 => self.select(self.rows.len().saturating_sub(1)),
            v if v == VK_LEFT.0 => edit(self, &|t| t.left(ctrl, shift)),
            v if v == VK_RIGHT.0 => edit(self, &|t| t.right(ctrl, shift)),
            v if v == VK_HOME.0 => edit(self, &|t| t.move_to(0, shift)),
            v if v == VK_END.0 => edit(self, &|t| t.move_to(t.end(), shift)),
            v if v == VK_BACK.0 => edit(self, &|t| t.backspace(ctrl)),
            v if v == VK_DELETE.0 => edit(self, &|t| t.delete(ctrl)),
            v if ctrl && v == VK_A.0 => edit(self, &|t| t.select_all()),
            v if ctrl && (v == VK_C.0 || v == VK_INSERT.0) => clipboard::set(self.hwnd, &self.input.selected_text()),
            v if ctrl && v == VK_X.0 => {
                clipboard::set(self.hwnd, &self.input.selected_text());
                edit(self, &|t| {
                    t.delete_selection();
                });
            }
            v if (ctrl && v == VK_V.0) || (shift && v == VK_INSERT.0) => {
                let pasted = clipboard::get(self.hwnd).unwrap_or_default();
                let line = pasted.lines().next().unwrap_or("").to_string();
                edit(self, &|t| t.insert(&line));
            }
            _ => return false,
        }
        true
    }

    pub(super) fn on_char(&mut self, unit: u16) {
        let c = match (self.high_surrogate.take(), unit) {
            (_, 0xD800..=0xDBFF) => {
                self.high_surrogate = Some(unit);
                return;
            }
            (Some(hi), 0xDC00..=0xDFFF) => char::decode_utf16([hi, unit]).next().and_then(Result::ok),
            (_, u) => char::from_u32(u32::from(u)),
        };
        let Some(c) = c.filter(|c| !c.is_control()) else { return };
        self.input.insert(&c.to_string());
        self.edited();
    }

    fn caret_from_x(&mut self, x: f32, extend: bool) {
        let origin = self.text_origin();
        let Some(t) = &self.search else { return };
        let i = (0..=t.len())
            .min_by(|&a, &b| (origin + t.x_at(a) - x).abs().total_cmp(&(origin + t.x_at(b) - x).abs()))
            .unwrap_or(0);
        self.input.move_to(i, extend);
        self.caret_on = true;
        self.place_ime();
        self.repaint_search();
    }

    fn row_under(&self, lparam: LPARAM) -> Option<usize> {
        self.row_at(self.to_dip(lparam_point(lparam).1))
    }

    pub(super) fn on_mouse_move(&mut self, lparam: LPARAM) {
        let row = self.row_under(lparam);
        if row == self.hover {
            return;
        }
        self.hover = row;
        let mut tme = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.hwnd,
            dwHoverTime: 0,
        };
        unsafe {
            let _ = TrackMouseEvent(&mut tme);
        }
        self.invalidate();
    }

    pub(super) fn on_left_down(&mut self, lparam: LPARAM) {
        let (x, y) = lparam_point(lparam);
        let (x, y) = (self.to_dip(x), self.to_dip(y));
        if y < SEARCH_H {
            self.caret_from_x(x, key_down(VK_SHIFT));
        } else {
            self.pressed = self.row_at(y);
            if let Some(i) = self.pressed {
                self.select(i);
            }
        }
    }

    pub(super) fn on_left_up(&mut self, lparam: LPARAM) {
        if let Some(i) = self.row_under(lparam).filter(|&i| self.pressed.take() == Some(i)) {
            self.deferred = Some(Deferred::Activate(i));
        }
    }

    pub(super) fn on_right_up(&mut self, lparam: LPARAM) {
        let Some(i) = self.row_under(lparam) else { return };
        self.select(i);
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        self.deferred = Some(Deferred::Menu(i, pt.x, pt.y));
    }

    pub(super) fn on_menu_key(&mut self) {
        self.settle();
        if self.selected < self.rows.len() {
            let (x, y) = self.menu_point(self.selected);
            self.deferred = Some(Deferred::Menu(self.selected, x, y));
        }
    }
}
