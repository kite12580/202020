use capsule_core::layout::Rect;
use capsule_core::shell_proto::{Bitmap, Request};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{ClientToScreen, ValidateRect};
use windows::Win32::UI::Input::Ime::{
    CANDIDATEFORM, CFS_CANDIDATEPOS, CFS_POINT, COMPOSITIONFORM, ImmGetContext, ImmReleaseContext,
    ImmSetCandidateWindow, ImmSetCompositionWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos};

use super::Launcher;
use crate::capsule::gfx::{Color, Surface, Text};
use crate::capsule::hook;
use crate::capsule::theme::Palette;
use crate::capsule::util::primary_monitor;

pub(super) const WIDTH: f32 = 640.0;
pub(super) const PAD: f32 = 8.0;
pub(super) const SEARCH_H: f32 = 56.0;
const TEXT_X: f32 = 52.0;
const ROW_H: f32 = 44.0;
const ICON: f32 = 24.0;
pub(super) const NAME_X: f32 = PAD + 48.0;
pub(super) const TAG_W: f32 = 72.0;
const LIST_TOP: f32 = SEARCH_H + 1.0 + PAD / 2.0;

fn row_rect(i: usize) -> Rect {
    Rect { x: PAD, y: LIST_TOP + i as f32 * ROW_H, w: WIDTH - 2.0 * PAD, h: ROW_H }
}

impl Launcher {
    fn height(&self) -> f32 {
        if self.rows.is_empty() { SEARCH_H } else { LIST_TOP + self.rows.len() as f32 * ROW_H + PAD }
    }

    pub(super) fn place(&mut self) {
        let mon = primary_monitor();
        let (w, h) = (self.px(WIDTH), self.px(self.height()));
        let x = (mon.left + mon.right - w) / 2;
        let y = mon.top + (mon.bottom - mon.top) * 22 / 100;
        if self.bounds == (x, y, w, h) {
            return;
        }
        self.bounds = (x, y, w, h);
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
        }
        if let Some(s) = &mut self.surface {
            s.resize(w as u32, h as u32);
        }
    }

    fn caret_offset(&self) -> f32 {
        self.search.as_ref().map_or(0.0, |t| t.x_at(self.input.caret))
    }

    /// Left edge of the search text, shifted so the caret stays in view.
    pub(super) fn text_origin(&self) -> f32 {
        let room = WIDTH - TEXT_X - PAD * 2.0;
        TEXT_X - (self.caret_offset() - room).max(0.0)
    }

    pub(super) fn place_ime(&self) {
        let pt = POINT { x: self.px(self.text_origin() + self.caret_offset()), y: self.px(SEARCH_H / 2.0 - 12.0) };
        unsafe {
            let imc = ImmGetContext(self.hwnd);
            if imc.is_invalid() {
                return;
            }
            let comp = COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: pt, ..Default::default() };
            let _ = ImmSetCompositionWindow(imc, &comp);
            let below = POINT { x: pt.x, y: self.px(SEARCH_H / 2.0 + 14.0) };
            let cand =
                CANDIDATEFORM { dwIndex: 0, dwStyle: CFS_CANDIDATEPOS, ptCurrentPos: below, ..Default::default() };
            let _ = ImmSetCandidateWindow(imc, &cand);
            let _ = ImmReleaseContext(self.hwnd, imc);
        }
    }

    pub(super) fn row_at(&self, y: f32) -> Option<usize> {
        (0..self.rows.len()).find(|&i| {
            let r = row_rect(i);
            y >= r.y && y < r.y + r.h
        })
    }

    pub(super) fn request_icons(&mut self) {
        let px = self.px(ICON) as u32;
        for row in &self.rows {
            let Some(key) = row.icon.as_ref().filter(|k| !self.images.contains_key(*k)) else { continue };
            let Some(name) = self.lists[row.list][row.index].icon() else { continue };
            let id = self.next_icon;
            self.next_icon += 1;
            if hook::request(self.hwnd, &Request::Icon { id, px, name }) {
                self.images.insert(key.clone(), None);
                self.pending.insert(id, key.clone());
            }
        }
    }

    pub(super) fn on_icon(&mut self, id: u64, bitmap: Option<Bitmap>) {
        let image = bitmap.and_then(|b| self.gfx.image_from_bitmap(b, self.px(ICON) as u32));
        if let Some(key) = self.pending.remove(&id) {
            self.images.insert(key, image);
            self.invalidate();
        }
    }

    fn paint_search(&self, s: &mut Surface, pal: &Palette) {
        let origin = self.text_origin();
        let (selection, caret) = (self.input.selection(), self.input.caret);
        s.fill(Rect { x: 0.0, y: 0.0, w: WIDTH, h: SEARCH_H }, pal.tint, 0.0);
        let cy = SEARCH_H / 2.0;
        if let Some(t) = &self.lens {
            s.text(t, 20.0, cy - t.height / 2.0, pal.text.alpha(0.6));
        }
        match &self.search {
            Some(t) if self.input.end() > 0 => {
                let ty = cy - t.height / 2.0;
                if selection.0 != selection.1 {
                    let (a, b) = (origin + t.x_at(selection.0), origin + t.x_at(selection.1));
                    s.fill(Rect { x: a, y: cy - 13.0, w: b - a, h: 26.0 }, pal.accent.alpha(0.35), 2.0);
                }
                s.text_colored(t, origin, ty, |i| {
                    if origin + t.x_at(i) < TEXT_X - 2.0 { Color(0.0, 0.0, 0.0, 0.0) } else { pal.text }
                });
                if self.caret_on {
                    s.fill(Rect { x: origin + t.x_at(caret), y: cy - 12.0, w: 1.5, h: 24.0 }, pal.text, 0.0);
                }
            }
            _ => {
                if let Some(p) = &self.placeholder {
                    s.text(p, TEXT_X, cy - p.height / 2.0, pal.text.alpha(0.4));
                }
                if self.caret_on {
                    s.fill(Rect { x: TEXT_X, y: cy - 12.0, w: 1.5, h: 24.0 }, pal.text, 0.0);
                }
            }
        }
    }

    pub(super) fn blink(&mut self) {
        self.caret_on = !self.caret_on;
        self.repaint_search();
    }

    pub(super) fn repaint_search(&mut self) {
        let Some(theme) = self.theme.filter(|_| self.visible) else { return };
        let Some(mut s) = self.surface.take() else { return };
        s.begin_band(0.0, SEARCH_H);
        self.paint_search(&mut s, &theme.palette(1.0));
        s.end_band(0.0, SEARCH_H);
        self.surface = Some(s);
    }

    pub(super) fn paint(&mut self) {
        unsafe {
            let _ = ValidateRect(Some(self.hwnd), None);
        }
        if !self.visible {
            return;
        }
        let (w, h) = (self.px(WIDTH) as u32, self.px(self.height()) as u32);
        if self.surface.is_none() {
            self.surface = Surface::new(self.hwnd, w, h, self.dpi);
        }
        let Some(theme) = self.theme else { return };
        let pal = theme.palette(1.0);
        let accent = if theme.high_contrast { pal.text } else { pal.accent };
        let height = self.to_dip(h as i32);
        let Some(mut s) = self.surface.take() else { return };
        s.resize(w, h);
        s.begin();
        s.fill(Rect { x: 0.0, y: SEARCH_H, w: WIDTH, h: height - SEARCH_H }, pal.tint, 0.0);
        if !self.rows.is_empty() {
            s.fill(Rect { x: 0.0, y: SEARCH_H, w: WIDTH, h: 1.0 }, pal.text.alpha(0.08), 0.0);
        }
        for i in 0..self.rows.len() {
            self.paint_row(&mut s, &pal, accent, i);
        }
        self.paint_search(&mut s, &pal);
        s.end();
        self.surface = Some(s);
    }

    fn paint_row(&self, s: &mut Surface, pal: &Palette, accent: Color, i: usize) {
        let (row, r) = (&self.rows[i], row_rect(i));
        let dim = pal.text.alpha(0.6);
        if i == self.selected {
            s.fill(r, pal.active, 6.0);
        } else if self.hover == Some(i) {
            s.fill(r, pal.hover, 6.0);
        }
        let rcy = r.y + r.h / 2.0;
        let icx = r.x + 12.0 + ICON / 2.0;
        if let Some(Some(img)) = row.icon.as_ref().and_then(|k| self.images.get(k)) {
            s.image(img, icx, rcy);
        } else if let Some(g) = &row.glyph {
            s.text(g, icx - g.width / 2.0, rcy - g.height / 2.0, dim);
        }
        let paint_name =
            |s: &mut Surface, t: &Text, y: f32| {
                s.text_colored(t, NAME_X, y, |c| {
                    if row.marks.binary_search(&(c as u32)).is_ok() { accent } else { pal.text }
                });
            };
        match (&row.name, &row.sub) {
            (Some(n), Some(st)) => {
                let top = rcy - (n.height + st.height) / 2.0;
                paint_name(s, n, top);
                s.text(st, NAME_X, top + n.height, dim);
            }
            (Some(n), None) => paint_name(s, n, rcy - n.height / 2.0),
            _ => {}
        }
        if let Some(t) = &row.tag {
            s.text(t, r.x + r.w - 12.0 - t.width, rcy - t.height / 2.0, pal.text.alpha(0.45));
        }
    }

    pub(super) fn menu_point(&self, i: usize) -> (i32, i32) {
        let r = row_rect(i);
        let mut pt = POINT { x: self.px(NAME_X), y: self.px(r.y + r.h) };
        unsafe {
            let _ = ClientToScreen(self.hwnd, &mut pt);
        }
        (pt.x, pt.y)
    }

    pub(super) fn on_dpi(&mut self, dpi: u32) {
        self.dpi = dpi;
        if let Some(s) = &mut self.surface {
            s.set_dpi(dpi);
        }
        self.images.clear();
        self.pending.clear();
        self.request_icons();
        self.place();
        self.invalidate();
    }
}
