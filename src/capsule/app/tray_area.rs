use capsule_core::layout::Hit;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::UI::WindowsAndMessaging::{
    SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP,
};

use super::menus::DeferredMenu;
use super::{App, cursor};
use crate::capsule::tray::{Change, NIN_POPUPCLOSE, NIN_POPUPOPEN};
use crate::capsule::util::{lparam_point, primary_monitor};

const POPUP_GAP: f32 = 6.0;
const ELEVATED_HINT: &str = "This program runs as administrator. Run 202020 as administrator to use its icon.";

impl App {
    /// Shares icon positions with the relay for `Shell_NotifyIconGetRect`.
    pub(super) fn publish_tray_rects(&self) {
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

    pub(super) fn on_tray_events(&mut self) {
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

    pub(super) fn after_tray_change(&mut self) {
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

    pub(super) fn close_icon_popup(&mut self) {
        if let Some(key) = self.tray_popup_open.take()
            && let Some(icon) = self.tray.icons.iter().find(|i| i.key.same(&key))
        {
            let p = cursor();
            icon.send(NIN_POPUPCLOSE, (p.x, p.y));
        }
    }

    pub(super) fn tray_tip(&mut self, icon: usize, x: i32, top: i32, bottom: i32) {
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

    /// Forwards a mouse message to a tray icon, adding explorer's follow-up
    /// notification after button-up.
    pub(super) fn tray_mouse(&mut self, icon: usize, msg: u32) {
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

    pub(super) fn place_popup(&mut self) {
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

    pub(super) fn toggle_popup(&mut self) {
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

    pub(super) fn close_popup(&mut self) {
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

    pub(super) fn on_popup_mouse(&mut self, msg: u32, lparam: LPARAM) {
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

    pub(super) fn popup_tip(&mut self) {
        let Some(c) = self.popup.hover else { return };
        let r = self.popup.cell_rect(c);
        let x = self.popup.origin.0 + self.px(r.center_x());
        let top = self.popup.origin.1 + self.px(r.y);
        let bottom = self.popup.origin.1 + self.px(r.y + r.h);
        self.tray_tip(self.popup.cells[c], x, top, bottom);
    }

    pub(super) fn toggle_pin(&mut self, icon: usize) {
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
}
