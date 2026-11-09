use capsule_core::clock;
use capsule_core::layout::{Hit, METRICS, Rect};
use capsule_core::taskband::Task;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::SystemInformation::GetLocalTime;

use super::{App, TIMER_CLOCK};
use crate::capsule::gfx::{Color, Font, Surface, Text};
use crate::capsule::theme::Palette;

const TASK_ICON_LIFT: f32 = 1.5;
const START_LOGO: f32 = 16.0;
const START_LOGO_GAP: f32 = 1.0;
const START_LOGO_FROM: Color = Color(0.29, 0.79, 1.0, 1.0);
const START_LOGO_TO: Color = Color(0.0, 0.40, 0.80, 1.0);
const CLOCK_LINE_OVERLAP: f32 = 2.0;

const GLYPH_CHEVRON: &str = "\u{E70E}";
const GLYPH_MUTE: &str = "\u{E74F}";
const GLYPH_VOLUME: [&str; 4] = ["\u{E992}", "\u{E993}", "\u{E994}", "\u{E995}"];

#[derive(Default)]
pub(super) struct Texts {
    pub(super) keyboard: Option<Text>,
    pub(super) chevron: Option<Text>,
    pub(super) volume_glyph: Option<Text>,
    pub(super) clock_time: Option<Text>,
    pub(super) clock_date: Option<Text>,
    pub(super) keyboard_slot: f32,
    pub(super) volume_slot: f32,
    pub(super) clock_slot: f32,
}

fn paint_start_logo(s: &mut Surface, cx: f32, cy: f32) {
    let size = (START_LOGO - START_LOGO_GAP) / 2.0;
    let (left, top) = (cx - START_LOGO / 2.0, cy - START_LOGO / 2.0);
    let squares = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].map(|(col, row)| Rect {
        x: left + col * (size + START_LOGO_GAP),
        y: top + row * (size + START_LOGO_GAP),
        w: size,
        h: size,
    });
    s.fill_gradient(&squares, 1.0, START_LOGO_FROM, START_LOGO_TO);
}

impl App {
    pub(super) fn rebuild_static_texts(&mut self) {
        let t = &mut self.texts;
        t.chevron = self.gfx.text(GLYPH_CHEVRON, Font::Glyph);
        t.clock_slot = clock::WIDTH_CANDIDATES
            .iter()
            .filter_map(|s| self.gfx.text(s, Font::Clock))
            .map(|t| t.width.ceil())
            .fold(0.0, f32::max);
        let glyph = GLYPH_VOLUME.iter().chain([&GLYPH_MUTE]).filter_map(|g| self.gfx.text(g, Font::Glyph));
        t.volume_slot = glyph.map(|t| t.width).fold(0.0, f32::max).ceil();
        self.measure_keyboard_slot();
    }

    pub(super) fn refresh_clock(&mut self) {
        let t = unsafe { GetLocalTime() };
        let s = (clock::time(t.wHour, t.wMinute), clock::date(t.wYear, t.wMonth, t.wDay));
        if s != self.clock {
            self.texts.clock_time = self.gfx.text(&s.0, Font::Clock);
            self.texts.clock_date = self.gfx.text(&s.1, Font::Clock);
            self.clock = s;
            self.invalidate();
        }
        if !self.paused() {
            self.set_timer(TIMER_CLOCK, clock::ms_until_next_minute(t.wSecond, t.wMilliseconds) + 20);
        }
    }

    pub(super) fn refresh_volume(&mut self) {
        let level = self.volume.level();
        let glyph = if !self.volume.available() || self.volume.muted() {
            GLYPH_MUTE
        } else {
            GLYPH_VOLUME[match level {
                l if l < 0.005 => 0,
                l if l < 0.34 => 1,
                l if l < 0.67 => 2,
                _ => 3,
            }]
        };
        if glyph != self.volume_glyph {
            self.texts.volume_glyph = self.gfx.text(glyph, Font::Glyph);
            self.volume_glyph = glyph;
            self.invalidate();
        }
        if self.hover == Some(Hit::Volume) {
            self.show_tip();
        }
    }

    fn new_surface(&self, hwnd: HWND, w: i32, h: i32) -> Option<Surface> {
        let s = Surface::new(hwnd, w as u32, h as u32, self.dpi);
        if s.is_none() {
            log::error!("surface creation failed");
        }
        s
    }

    fn ensure_surface(&mut self) -> bool {
        if self.surface.is_none() {
            let (w, h) = (self.bounds.right - self.bounds.left, self.bounds.bottom - self.bounds.top);
            self.surface = self.new_surface(self.hwnd, w, h);
        }
        self.surface.is_some()
    }

    pub(super) fn paint(&mut self) {
        unsafe {
            let _ = ValidateRect(Some(self.hwnd), None);
        }
        if !self.ensure_surface() {
            return;
        }
        self.ensure_tray_images(&self.bar_tray.clone());
        let pal = self.theme.palette(self.cfg.config.appearance.opacity);
        let Some(mut s) = self.surface.take() else { return };
        s.begin();
        s.fill(Rect { x: 0.0, y: 0.0, w: self.layout.width, h: METRICS.height }, pal.tint, 0.0);
        for &(hit, r) in &self.layout.items {
            self.paint_item(&mut s, &pal, hit, r);
        }
        s.end();
        self.surface = Some(s);
    }

    fn paint_item(&self, s: &mut Surface, pal: &Palette, hit: Hit, r: Rect) {
        let cy = METRICS.height / 2.0;
        let highlight = Rect { x: r.x + 1.0, y: 4.0, w: r.w - 2.0, h: METRICS.height - 8.0 };
        if self.hover == Some(hit) {
            s.fill(highlight, pal.hover, 4.0);
        }
        let centered = |s: &mut Surface, t: &Option<Text>| {
            if let Some(t) = t {
                s.text(t, r.center_x() - t.width / 2.0, cy - t.height / 2.0, pal.text);
            }
        };
        match hit {
            Hit::Start => paint_start_logo(s, r.center_x(), cy),
            Hit::Task(i) => self.paint_task(s, pal, i, r, highlight),
            Hit::Chevron => centered(s, &self.texts.chevron),
            Hit::Tray(i) => {
                if let Some(img) = &self.tray.icons[self.bar_tray[i]].image {
                    s.image(img, r.center_x(), cy);
                }
            }
            Hit::Keyboard => centered(s, &self.texts.keyboard),
            Hit::Volume => centered(s, &self.texts.volume_glyph),
            Hit::Clock => self.paint_clock(s, pal, r),
        }
    }

    fn paint_task(&self, s: &mut Surface, pal: &Palette, i: usize, r: Rect, highlight: Rect) {
        let image = match self.tasks[i] {
            Task::Window(w) => {
                let w = &self.windows.items[w];
                let (bar_w, color) = if w.hwnd == self.foreground {
                    s.fill(highlight, pal.active, 4.0);
                    (16.0, pal.accent)
                } else {
                    (6.0, pal.running)
                };
                s.fill(Rect { x: r.center_x() - bar_w / 2.0, y: METRICS.height - 7.0, w: bar_w, h: 3.0 }, color, 1.5);
                &w.image
            }
            Task::Pin(p) => &self.pins[p].image,
        };
        let icon_y = METRICS.height / 2.0 - TASK_ICON_LIFT;
        match image {
            Some(img) => s.image(img, r.center_x(), icon_y),
            None => s.dot(r.center_x(), icon_y, 3.0, pal.text.alpha(0.5)),
        }
    }

    fn paint_clock(&self, s: &mut Surface, pal: &Palette, r: Rect) {
        let (Some(t), Some(d)) = (&self.texts.clock_time, &self.texts.clock_date) else { return };
        let right = r.right() - METRICS.text_pad;
        let top = METRICS.height / 2.0 - (t.height + d.height - CLOCK_LINE_OVERLAP) / 2.0;
        s.text(t, right - t.width, top, pal.text);
        s.text(d, right - d.width, top + t.height - CLOCK_LINE_OVERLAP, pal.text);
    }

    pub(super) fn paint_popup(&mut self) {
        unsafe {
            let _ = ValidateRect(Some(self.popup.hwnd), None);
        }
        if !self.popup.open {
            return;
        }
        if self.popup.surface.is_none() {
            let (w, h) = self.popup.size();
            self.popup.surface = self.new_surface(self.popup.hwnd, self.px(w), self.px(h));
        }
        self.ensure_tray_images(&self.popup.cells.clone());
        let pal = self.theme.palette(self.cfg.config.appearance.opacity);
        let (w, h) = self.popup.size();
        let pins = &self.cfg.config.tray.pinned;
        let cells: Vec<(Rect, usize)> =
            self.popup.cells.iter().enumerate().map(|(c, &icon)| (self.popup.cell_rect(c), icon)).collect();
        let hover = self.popup.hover;
        let Some(s) = self.popup.surface.as_mut() else { return };
        s.begin();
        s.fill(Rect { x: 0.0, y: 0.0, w, h }, pal.tint, 0.0);
        for (c, &(r, icon)) in cells.iter().enumerate() {
            if hover == Some(c) {
                s.fill(Rect { x: r.x + 2.0, y: r.y + 2.0, w: r.w - 4.0, h: r.h - 4.0 }, pal.hover, 4.0);
            }
            let i = &self.tray.icons[icon];
            if let Some(img) = &i.image {
                s.image(img, r.center_x(), r.y + r.h / 2.0 - 1.0);
            }
            if pins.contains(&i.pin) {
                s.dot(r.center_x(), r.y + r.h - 5.0, 1.5, pal.text.alpha(0.7));
            }
        }
        s.end();
    }
}
