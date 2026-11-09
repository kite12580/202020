use capsule_core::layout::{self, Input, Layout, METRICS, Rect};
use windows::Win32::Foundation::{LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos};

use super::{App, WM_APP_REPOSITION};
use crate::capsule::util::primary_monitor;

impl App {
    pub(super) fn relayout(&mut self) {
        self.bar_tray = self.tray.bar_icons(&self.cfg.config.tray.pinned);
        self.tasks = self.windows.tasks();
        let input = Input {
            tasks: self.tasks.len(),
            tray: self.bar_tray.len(),
            chevron: !self.tray.popup_icons().is_empty(),
            keyboard_w: self.texts.keyboard.as_ref().map_or(0.0, |t| t.width.ceil()).max(self.texts.keyboard_slot),
            volume_w: self.texts.volume_slot,
            clock_w: self.texts.clock_slot,
        };
        let layout = Layout::compute(&METRICS, &input);
        let resized = (layout.width - self.layout.width).abs() > 0.01;
        self.layout = layout;
        if self.hover.is_some_and(|h| self.layout.rect(h).is_none()) {
            self.set_hover(None);
        }
        if resized {
            self.schedule_reposition();
        } else {
            self.publish_tray_rects();
        }
        self.invalidate();
    }

    /// Coalesces the resizes from a burst of window events, such as a
    /// virtual desktop switch cloaking and uncloaking every window.
    fn schedule_reposition(&mut self) {
        if !self.reposition_pending {
            self.reposition_pending = true;
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_APP_REPOSITION, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub(super) fn reposition(&mut self) {
        self.reposition_pending = false;
        let mon = primary_monitor();
        let (w, h) = (self.px(self.layout.width), self.px(METRICS.height));
        let (x, y) = if self.drag.is_some() {
            (self.bounds.left, self.bounds.top)
        } else {
            let p = &self.cfg.config.position;
            layout::place(mon, w, h, p.center_x, self.px(p.bottom_gap))
        };
        self.bounds = RECT { left: x, top: y, right: x + w, bottom: y + h };
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
        }
        if let Some(s) = &mut self.surface {
            s.resize(w as u32, h as u32);
        }
        self.publish_tray_rects();
        if self.popup.open {
            self.place_popup();
        }
    }

    pub(super) fn screen_rect(&self, r: Rect, origin: (i32, i32)) -> RECT {
        RECT {
            left: origin.0 + self.px(r.x),
            top: origin.1 + self.px(r.y),
            right: origin.0 + self.px(r.right()),
            bottom: origin.1 + self.px(r.y + r.h),
        }
    }

    pub(super) fn bar_origin(&self) -> (i32, i32) {
        (self.bounds.left, self.bounds.top)
    }

    pub(super) fn on_dpi(&mut self, dpi: u32) {
        if dpi == self.dpi {
            return;
        }
        self.dpi = dpi;
        if let Some(s) = &mut self.surface {
            s.set_dpi(dpi);
        }
        if let Some(s) = &mut self.popup.surface {
            s.set_dpi(dpi);
        }
        self.refetch_icons();
        self.reposition();
        self.invalidate();
    }
}
