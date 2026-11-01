use ab_glyph::{Font, FontVec, OutlineCurve};
use tiny_skia::{Color, FillRule, IntRect, Paint, Path, PathBuilder, Pixmap, Rect, Transform};

const ICON_CANVAS: f32 = 64.0;
const KAPPA: f32 = 0.552_284_8;

const LOGO_VIEWBOX_HEIGHT: f32 = 29.0;
const LOGO_PATHS: [&str; 2] = [
    "M14.08184 24.54825C14.92661 25.6053 15.50041 27.02485 15.34827 28.47485C14.11227 28.41395 12.60386 27.6585 11.73012 26.60145C10.94621 25.6952 10.25214 24.2162 10.43326 22.8271C11.81995 22.7053 13.20665 23.52165 14.08184 24.54825Z",
    "M15.33277 22.55523C13.31721 22.67558 11.60449 21.41118 10.6409 21.41118C9.678768 21.41118 8.205135 22.49433 6.609786 22.46533C4.536267 22.43488 2.610546 21.26183 1.558572 19.39423C-0.6076833 15.65903 0.9862167 10.11858 3.091614 7.07648C4.114608 5.57283 5.347707 3.91548 6.970587 3.97493C8.506527 4.03583 9.107862 4.96963 10.97272 4.96963C12.83614 4.96963 13.37806 3.97493 15.00094 4.00538C16.68613 4.03583 17.73955 5.51048 18.76255 7.01703C19.93479 8.73238 20.41586 10.38828 20.44629 10.47963C20.41586 10.51008 17.19763 11.74548 17.16865 15.44878C17.13822 18.55033 19.69426 20.02498 19.81452 20.11778C18.37132 22.25363 16.11523 22.49433 15.33277 22.55523Z",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IconFace {
    Minutes(u32),
    Countdown(u32),
    Paused,
}

pub fn load_font() -> FontVec {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let data = std::fs::read(format!(r"{windir}\Fonts\seguisb.ttf")).expect("Segoe UI Semibold font");
    FontVec::try_from_vec(data).expect("valid font")
}

fn paint(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(r, g, b, a);
    paint.anti_alias = true;
    paint
}

fn gray(white: f32) -> Paint<'static> {
    let v = (white * 255.0) as u8;
    paint(v, v, v, 255)
}

fn rounded_rect(rect: Rect, radius: f32) -> Option<Path> {
    let (x, y, w, h) = (rect.x(), rect.y(), rect.width(), rect.height());
    let r = radius.min(w / 2.0).min(h / 2.0);
    let c = r * (1.0 - KAPPA);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - c, y, x + w, y + c, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - c, x + w - c, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + c, y + h, x, y + h - c, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + c, x + c, y, x + r, y);
    pb.close();
    pb.finish()
}

fn fill_rounded(pixmap: &mut Pixmap, rect: Option<Rect>, radius: f32, paint: &Paint, t: Transform) {
    if let Some(path) = rect.and_then(|r| rounded_rect(r, radius)) {
        pixmap.fill_path(&path, paint, FillRule::Winding, t, None);
    }
}

fn text_path(font: &FontVec, text: &str, size: f32) -> Option<(Path, f32)> {
    let scale = size / font.units_per_em()?;
    let mut pb = PathBuilder::new();
    let mut pen = 0.0;
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(prev) = prev {
            pen += font.kern_unscaled(prev, id) * scale;
        }
        if let Some(outline) = font.outline(id) {
            let pt = |p: &ab_glyph::Point| (pen + p.x * scale, -p.y * scale);
            let mut last = None;
            for curve in &outline.curves {
                let (from, to) = match curve {
                    OutlineCurve::Line(a, b) => (a, b),
                    OutlineCurve::Quad(a, _, c) => (a, c),
                    OutlineCurve::Cubic(a, _, _, d) => (a, d),
                };
                if last != Some(*from) {
                    let (x, y) = pt(from);
                    pb.move_to(x, y);
                }
                match curve {
                    OutlineCurve::Line(_, b) => {
                        let (x, y) = pt(b);
                        pb.line_to(x, y);
                    }
                    OutlineCurve::Quad(_, b, c) => {
                        let ((x1, y1), (x, y)) = (pt(b), pt(c));
                        pb.quad_to(x1, y1, x, y);
                    }
                    OutlineCurve::Cubic(_, b, c, d) => {
                        let ((x1, y1), (x2, y2), (x, y)) = (pt(b), pt(c), pt(d));
                        pb.cubic_to(x1, y1, x2, y2, x, y);
                    }
                }
                last = Some(*to);
            }
        }
        pen += font.h_advance_unscaled(id) * scale;
        prev = Some(id);
    }
    Some((pb.finish()?, pen))
}

pub fn tray_icon(font: &FontVec, size: u32, face: IconFace) -> Pixmap {
    let mut pixmap = Pixmap::new(size, size).expect("non-zero icon size");
    let k = size as f32 / ICON_CANVAS;
    let t = Transform::from_scale(k, k);
    let (bg, fg) = match face {
        IconFace::Countdown(_) => (paint(40, 40, 40, 255), paint(255, 150, 40, 255)),
        _ => (paint(0, 150, 220, 255), paint(255, 255, 255, 255)),
    };
    let circle = PathBuilder::from_circle(ICON_CANVAS / 2.0, ICON_CANVAS / 2.0, ICON_CANVAS / 2.0 - 2.0).unwrap();
    pixmap.fill_path(&circle, &bg, FillRule::Winding, t, None);
    match face {
        IconFace::Paused => {
            let (bar_w, bar_h, gap) = (9.0, 28.0, 8.0);
            let x0 = (ICON_CANVAS - (bar_w * 2.0 + gap)) / 2.0;
            let y0 = (ICON_CANVAS - bar_h) / 2.0;
            for i in 0..2 {
                let rect = Rect::from_xywh(x0 + i as f32 * (bar_w + gap), y0, bar_w, bar_h);
                fill_rounded(&mut pixmap, rect, 2.0, &fg, t);
            }
        }
        IconFace::Minutes(value) | IconFace::Countdown(value) => {
            let text = value.to_string();
            let text_size = if text.len() <= 2 { 34.0 } else { 28.0 };
            if let Some((path, advance)) = text_path(font, &text, text_size) {
                let b = path.bounds();
                let t =
                    t.pre_translate((ICON_CANVAS - advance) / 2.0, ICON_CANVAS / 2.0 - (b.top() + b.bottom()) / 2.0);
                pixmap.fill_path(&path, &fg, FillRule::Winding, t, None);
            }
        }
    }
    pixmap
}

fn logo_path() -> Option<Path> {
    let mut pb = PathBuilder::new();
    for d in LOGO_PATHS {
        let spaced: String =
            d.chars().flat_map(|c| if c.is_ascii_alphabetic() { vec![' ', c, ' '] } else { vec![c] }).collect();
        let tokens: Vec<&str> = spaced.split_whitespace().collect();
        let num = |i: usize| tokens[i].parse::<f32>().unwrap();
        let mut i = 0;
        while i < tokens.len() {
            match tokens[i] {
                "M" => {
                    pb.move_to(num(i + 1), num(i + 2));
                    i += 3;
                }
                "C" => {
                    pb.cubic_to(num(i + 1), num(i + 2), num(i + 3), num(i + 4), num(i + 5), num(i + 6));
                    i += 7;
                }
                "Z" => {
                    pb.close();
                    i += 1;
                }
                other => panic!("unsupported logo path token {other}"),
            }
        }
    }
    pb.finish()
}

pub struct Overlay {
    width: u32,
    height: u32,
    scale: f32,
    pixels: Vec<u32>,
    bar: Rect,
    fill: Rect,
}

impl Overlay {
    pub fn new(width: u32, height: u32, scale: f32) -> Self {
        let (w, h) = (width as f32, height as f32);
        let (bar_w, bar_h, border) = (236.0 * scale, 8.0 * scale, scale);
        let bar = Rect::from_xywh((w - bar_w) / 2.0, h * 0.904 - bar_h / 2.0, bar_w, bar_h).unwrap();
        let fill =
            Rect::from_xywh(bar.x() + border, bar.y() + border, bar_w - border * 2.0, bar_h - border * 2.0).unwrap();
        let mut overlay = Self { width, height, scale, pixels: vec![0; (width * height) as usize], bar, fill };

        let (logo_w, logo_h) = (84.0 * scale, 116.0 * scale);
        let logo = Rect::from_xywh((w - logo_w) / 2.0, h * 0.5 - logo_h / 2.0, logo_w, logo_h).unwrap();
        let unit = logo_h / LOGO_VIEWBOX_HEIGHT;
        overlay.paint_region(logo, |pixmap, t| {
            if let Some(path) = logo_path() {
                let t = t.pre_translate(logo.x(), logo.bottom()).pre_scale(unit, -unit);
                pixmap.fill_path(&path, &paint(255, 255, 255, 230), FillRule::Winding, t, None);
            }
        });
        overlay
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    pub fn draw_progress(&mut self, progress: f32) -> Option<IntRect> {
        let (bar, fill, s) = (self.bar, self.fill, self.scale);
        self.paint_region(bar, |pixmap, t| {
            fill_rounded(pixmap, Some(bar), 4.0 * s, &gray(0.20), t);
            fill_rounded(pixmap, Some(fill), 3.0 * s, &gray(0.13), t);
            let done = Rect::from_xywh(fill.x(), fill.y(), fill.width() * progress, fill.height());
            fill_rounded(pixmap, done, 3.0 * s, &gray(0.80), t);
        })
    }

    fn paint_region(&mut self, area: Rect, draw: impl FnOnce(&mut Pixmap, Transform)) -> Option<IntRect> {
        let screen = IntRect::from_xywh(0, 0, self.width, self.height)?;
        let area = area.round_out()?.intersect(&screen)?;
        let mut pixmap = Pixmap::new(area.width(), area.height())?;
        pixmap.fill(Color::BLACK);
        draw(&mut pixmap, Transform::from_translate(-area.x() as f32, -area.y() as f32));
        let row_len = area.width() as usize;
        for (row, src) in pixmap.pixels().chunks(row_len).enumerate() {
            let start = (area.y() as usize + row) * self.width as usize + area.x() as usize;
            for (dst, px) in self.pixels[start..start + row_len].iter_mut().zip(src) {
                *dst = (px.red() as u32) << 16 | (px.green() as u32) << 8 | px.blue() as u32;
            }
        }
        Some(area)
    }
}
