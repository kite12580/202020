use std::cell::{Cell, OnceCell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;

use ab_glyph::{Font as _, FontRef, GlyphId, PxScale, VariableFont, point};
use capsule_core::layout::Rect;
use capsule_core::shell_proto::Bitmap;
use tiny_skia::{
    FillRule, GradientStop, IntSize, LinearGradient, Paint, PathBuilder, Pixmap, PixmapMut, PixmapPaint, Point,
    SpreadMode, Transform,
};
use windows::Win32::Foundation::{GENERIC_READ, HWND};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GdiFlush, GetDC, GetDIBits, GetObjectW, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, GetFileSizeEx, OPEN_EXISTING,
};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, PAGE_READONLY, UnmapViewOfFile,
};
use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, HICON, ICONINFO};
use windows::core::PCWSTR;

use crate::capsule::util::{OwnedHandle, wide};
use crate::render::rounded_rect;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);

impl Color {
    pub fn alpha(self, a: f32) -> Color {
        Color(self.0, self.1, self.2, self.3 * a)
    }

    /// Surfaces hold BGRA pixels, which tiny-skia treats as RGBA.
    fn skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba(self.2, self.1, self.0, self.3).unwrap_or(tiny_skia::Color::TRANSPARENT)
    }

    fn paint(self) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color(self.skia());
        p.anti_alias = true;
        p
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Font {
    Text,
    Clock,
    Glyph,
    Small,
    Title,
    Search,
    SearchGlyph,
}

const TEXT_FONTS: &[&str] = &["SegUIVar.ttf", "segoeui.ttf", "msyh.ttc", "YuGothM.ttc", "malgun.ttf", "seguisym.ttf"];
const GLYPH_FONTS: &[&str] = &["SegoeIcons.ttf", "segmdl2.ttf"];

struct View(MEMORY_MAPPED_VIEW_ADDRESS);

impl Drop for View {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.0);
        }
    }
}

/// A font file mapped from the Fonts folder. `font` borrows `_view`, so it
/// must stay the first field.
struct Face {
    font: FontRef<'static>,
    _view: View,
}

fn map_face(file: &str) -> Option<Face> {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let path = wide(&format!(r"{dir}\Fonts\{file}"));
    unsafe {
        let file = OwnedHandle(
            CreateFileW(
                PCWSTR(path.as_ptr()),
                GENERIC_READ.0,
                FILE_SHARE_READ,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
            .ok()?,
        );
        let mut size = 0i64;
        GetFileSizeEx(file.0, &mut size).ok()?;
        let mapping = OwnedHandle(CreateFileMappingW(file.0, None, PAGE_READONLY, 0, 0, PCWSTR::null()).ok()?);
        let view = View(MapViewOfFile(mapping.0, FILE_MAP_READ, 0, 0, 0));
        if view.0.Value.is_null() {
            return None;
        }
        let data = std::slice::from_raw_parts(view.0.Value as *const u8, size as usize);
        let mut font = FontRef::try_from_slice(data).ok()?;
        font.set_variation(b"wght", 400.0);
        font.set_variation(b"opsz", 10.5);
        Some(Face { font, _view: view })
    }
}

/// Fonts tried in order for each character, mapped on first use.
struct Chain {
    files: &'static [&'static str],
    faces: Vec<OnceCell<Option<Rc<Face>>>>,
}

impl Chain {
    fn new(files: &'static [&'static str]) -> Self {
        Self { files, faces: files.iter().map(|_| OnceCell::new()).collect() }
    }

    fn face(&self, i: usize) -> Option<&Rc<Face>> {
        self.faces[i].get_or_init(|| map_face(self.files[i]).map(Rc::new)).as_ref()
    }

    fn find(&self, c: char) -> Option<(&Rc<Face>, GlyphId)> {
        (0..self.files.len()).find_map(|i| {
            let face = self.face(i)?;
            let id = face.font.glyph_id(c);
            (id.0 != 0).then_some((face, id))
        })
    }
}

struct Placed {
    face: Rc<Face>,
    id: GlyphId,
    x: f32,
}

/// A rasterized glyph, positioned relative to its text's pixel origin.
struct Coverage {
    x: i32,
    y: i32,
    width: usize,
    alpha: Vec<u8>,
}

impl Coverage {
    fn of(g: &Placed, size: f32, scale: f32) -> Option<Coverage> {
        let font = &g.face.font;
        let px = PxScale::from(size * scale * font.height_unscaled() / font.units_per_em().unwrap_or(1000.0));
        let outlined = font.outline_glyph(g.id.with_scale_and_position(px, point(g.x * scale, 0.0)))?;
        let b = outlined.px_bounds();
        let width = b.width() as usize;
        let mut alpha = vec![0; width * b.height() as usize];
        outlined.draw(|x, y, c| alpha[y as usize * width + x as usize] = (c.min(1.0) * 255.0).round() as u8);
        Some(Coverage { x: b.min.x as i32, y: b.min.y as i32, width, alpha })
    }
}

/// A laid-out line of text, in DIPs.
pub struct Text {
    glyphs: Vec<Placed>,
    size: f32,
    ascent: f32,
    pub width: f32,
    pub height: f32,
    raster: RefCell<Option<(f32, Vec<Option<Coverage>>)>>,
}

impl Text {
    /// Left edge of character `i`, or the end of the line past the last one.
    pub fn x_at(&self, i: usize) -> f32 {
        self.glyphs.get(i).map_or(self.width, |g| g.x)
    }

    pub fn len(&self) -> usize {
        self.glyphs.len()
    }
}

/// Device-independent icon pixels, pre-scaled to their on-screen size.
#[derive(Clone)]
pub struct Image {
    id: u64,
    pixmap: Rc<Pixmap>,
    pub px: u32,
}

impl Image {
    pub fn id(&self) -> u64 {
        self.id
    }
}

pub struct Gfx {
    text: Chain,
    glyph: Chain,
    next_id: Cell<u64>,
}

impl Gfx {
    pub fn new() -> Self {
        Self { text: Chain::new(TEXT_FONTS), glyph: Chain::new(GLYPH_FONTS), next_id: Cell::new(1) }
    }

    pub fn text(&self, s: &str, font: Font) -> Option<Text> {
        let (chain, size) = match font {
            Font::Text => (&self.text, 12.0),
            Font::Clock | Font::Small => (&self.text, 11.0),
            Font::Glyph => (&self.glyph, 12.0),
            Font::Title => (&self.text, 14.0),
            Font::Search => (&self.text, 20.0),
            Font::SearchGlyph => (&self.glyph, 18.0),
        };
        let primary = chain.face(0)?;
        let units = |f: &FontRef| size / f.units_per_em().unwrap_or(1000.0);
        let tabular = (font == Font::Clock).then(|| {
            let f = &primary.font;
            ('0'..='9').map(|c| f.h_advance_unscaled(f.glyph_id(c))).fold(0.0, f32::max) * units(f)
        });
        let mut glyphs = Vec::new();
        let mut pen = 0.0;
        let mut prev: Option<(&Rc<Face>, GlyphId)> = None;
        for c in s.chars() {
            let (face, id) = chain.find(c).unwrap_or((primary, GlyphId(0)));
            let k = units(&face.font);
            if let Some((pf, pid)) = prev
                && Rc::ptr_eq(pf, face)
            {
                pen += face.font.kern_unscaled(pid, id) * k;
            }
            let mut advance = face.font.h_advance_unscaled(id) * k;
            let mut x = pen;
            if let Some(cell) = tabular
                && c.is_ascii_digit()
            {
                x += (cell - advance) / 2.0;
                advance = cell;
            }
            glyphs.push(Placed { face: face.clone(), id, x });
            pen += advance;
            prev = Some((face, id));
        }
        let f = &primary.font;
        let k = units(f);
        Some(Text {
            glyphs,
            size,
            ascent: f.ascent_unscaled() * k,
            width: pen,
            height: (f.height_unscaled() + f.line_gap_unscaled()) * k,
            raster: RefCell::default(),
        })
    }

    /// Lays out `s`, cut short with an ellipsis if wider than `max`.
    pub fn text_fit(&self, s: &str, font: Font, max: f32) -> Option<Text> {
        let mut t = self.text(s, font)?;
        if t.width <= max {
            return Some(t);
        }
        let ellipsis = self.text("…", font)?;
        let keep = t.glyphs.iter().take_while(|g| g.x + ellipsis.width <= max).count().saturating_sub(1);
        let x = t.x_at(keep);
        t.glyphs.truncate(keep);
        t.glyphs.extend(ellipsis.glyphs.into_iter().map(|g| Placed { x: x + g.x, ..g }));
        t.width = x + ellipsis.width;
        Some(t)
    }

    fn finish(&self, w: u32, h: u32, pixels: Vec<u8>, px: u32) -> Option<Image> {
        let pixels = if (w, h) == (px, px) { pixels } else { resample(&pixels, w, h, px) };
        let pixmap = Pixmap::from_vec(pixels, IntSize::from_wh(px, px)?)?;
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        Some(Image { id, pixmap: Rc::new(pixmap), px })
    }

    pub fn image_from_hicon(&self, icon: HICON, px: u32) -> Option<Image> {
        let mut info = ICONINFO::default();
        unsafe { GetIconInfo(icon, &mut info) }.ok()?;
        let color = if info.hbmColor.is_invalid() { None } else { read_bitmap(info.hbmColor) };
        let mask = read_bitmap(info.hbmMask);
        unsafe {
            let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
            let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
        }
        let Bitmap { width: w, height: h, mut pixels, .. } = color?;
        if pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
            let mask = mask?.pixels;
            for (p, m) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(mask.as_chunks::<4>().0) {
                p[3] = if m[0] == 0 { 255 } else { 0 };
            }
        }
        premultiply(&mut pixels);
        self.finish(w, h, pixels, px)
    }

    pub fn image_from_bitmap(&self, bitmap: Bitmap, px: u32) -> Option<Image> {
        let Bitmap { width: w, height: h, bpp, mut pixels } = bitmap;
        if bpp < 32 {
            pixels.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
        } else if !premultiplied(&pixels) {
            premultiply(&mut pixels);
        }
        self.finish(w, h, pixels, px)
    }
}

fn bitmap_info(width: i32, height: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn read_bitmap(bitmap: HBITMAP) -> Option<Bitmap> {
    let mut bm = BITMAP::default();
    let size = size_of::<BITMAP>() as i32;
    if unsafe { GetObjectW(HGDIOBJ(bitmap.0), size, Some(&mut bm as *mut BITMAP as *mut c_void)) } == 0
        || bm.bmWidth <= 0
        || bm.bmHeight <= 0
    {
        return None;
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight);
    let mut info = bitmap_info(w, h);
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    let rows = unsafe {
        let dc = CreateCompatibleDC(None);
        let rows =
            GetDIBits(dc, bitmap, 0, h as u32, Some(pixels.as_mut_ptr() as *mut c_void), &mut info, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        rows
    };
    (rows == h).then_some(Bitmap { width: w as u32, height: h as u32, bpp: bm.bmBitsPixel, pixels })
}

fn premultiplied(pixels: &[u8]) -> bool {
    pixels.as_chunks::<4>().0.iter().all(|p| p[0].max(p[1]).max(p[2]) <= p[3])
}

fn premultiply(pixels: &mut [u8]) {
    for p in pixels.as_chunks_mut::<4>().0 {
        let a = u16::from(p[3]);
        for c in &mut p[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
}

/// Tent-filter taps for resampling `src` samples to `dst`, widened when
/// shrinking so every source pixel contributes.
fn taps(src: u32, dst: u32) -> Vec<(usize, Vec<f32>)> {
    let ratio = src as f32 / dst as f32;
    let support = ratio.max(1.0);
    (0..dst)
        .map(|i| {
            let center = (i as f32 + 0.5) * ratio;
            let first = (center - support).floor().max(0.0) as usize;
            let last = ((center + support).ceil() as usize).min(src as usize);
            let mut w: Vec<f32> =
                (first..last).map(|j| (1.0 - ((j as f32 + 0.5 - center) / support).abs()).max(0.0)).collect();
            let sum: f32 = w.iter().sum();
            if sum > 0.0 {
                w.iter_mut().for_each(|v| *v /= sum);
            }
            (first, w)
        })
        .collect()
}

fn resample(src: &[u8], w: u32, h: u32, px: u32) -> Vec<u8> {
    let (w, h, n) = (w as usize, h as usize, px as usize);
    let mut rows = vec![0f32; n * h * 4];
    for (x, (first, weights)) in taps(w as u32, px).iter().enumerate() {
        for y in 0..h {
            let d = (y * n + x) * 4;
            for (k, wt) in weights.iter().enumerate() {
                let s = (y * w + first + k) * 4;
                for c in 0..4 {
                    rows[d + c] += f32::from(src[s + c]) * wt;
                }
            }
        }
    }
    let mut out = vec![0u8; n * n * 4];
    for (y, (first, weights)) in taps(h as u32, px).iter().enumerate() {
        for x in 0..n {
            let mut acc = [0f32; 4];
            for (k, wt) in weights.iter().enumerate() {
                let s = ((first + k) * n + x) * 4;
                for c in 0..4 {
                    acc[c] += rows[s + c] * wt;
                }
            }
            let a = acc[3].round().clamp(0.0, 255.0);
            let d = (y * n + x) * 4;
            for c in 0..3 {
                out[d + c] = acc[c].round().clamp(0.0, a) as u8;
            }
            out[d + 3] = a as u8;
        }
    }
    out
}

/// A DIB the size of one window, copied to it by `end`. Coordinates are DIPs.
pub struct Surface {
    hwnd: HWND,
    dc: HDC,
    bitmap: HBITMAP,
    bits: *mut u8,
    width: u32,
    height: u32,
    dpi: u32,
}

impl Surface {
    pub fn new(hwnd: HWND, width: u32, height: u32, dpi: u32) -> Option<Self> {
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            return None;
        }
        let mut s =
            Surface { hwnd, dc, bitmap: HBITMAP::default(), bits: std::ptr::null_mut(), width: 0, height: 0, dpi };
        s.resize(width, height);
        (!s.bits.is_null()).then_some(s)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return;
        }
        let info = bitmap_info(width as i32, height as i32);
        let mut bits = std::ptr::null_mut();
        let Ok(bitmap) = (unsafe { CreateDIBSection(Some(self.dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) }) else {
            return;
        };
        unsafe {
            SelectObject(self.dc, HGDIOBJ(bitmap.0));
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            }
        }
        self.bitmap = bitmap;
        self.bits = bits as *mut u8;
        self.width = width;
        self.height = height;
    }

    pub fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi;
    }

    fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    fn data(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.width as usize * self.height as usize * 4) }
    }

    fn pixmap(&mut self) -> PixmapMut<'_> {
        let (w, h) = (self.width, self.height);
        PixmapMut::from_bytes(self.data(), w, h).expect("surface size matches its DIB")
    }

    pub fn begin(&mut self) {
        self.clear(0, self.height);
    }

    fn clear(&mut self, from: u32, to: u32) {
        unsafe {
            let _ = GdiFlush();
        }
        let row = self.width as usize * 4;
        self.data()[from as usize * row..to as usize * row].fill(0);
    }

    fn band(&self, top: f32, height: f32) -> (u32, u32) {
        let s = self.scale();
        let from = ((top * s).floor().max(0.0) as u32).min(self.height);
        (from, (((top + height) * s).ceil().max(0.0) as u32).clamp(from, self.height))
    }

    pub fn begin_band(&mut self, top: f32, height: f32) {
        let (from, to) = self.band(top, height);
        self.clear(from, to);
    }

    pub fn end_band(&mut self, top: f32, height: f32) {
        let (from, to) = self.band(top, height);
        self.present(from, to);
    }

    pub fn fill(&mut self, r: Rect, c: Color, radius: f32) {
        let Some(rect) = tiny_skia::Rect::from_xywh(r.x, r.y, r.w, r.h) else { return };
        let t = Transform::from_scale(self.scale(), self.scale());
        let paint = c.paint();
        let mut pm = self.pixmap();
        if radius <= 0.0 {
            pm.fill_rect(rect, &paint, t, None);
        } else if let Some(path) = rounded_rect(rect, radius) {
            pm.fill_path(&path, &paint, FillRule::Winding, t, None);
        }
    }

    /// Fills `rects` with a single gradient running from the top-left to the
    /// bottom-right corner of their bounds.
    pub fn fill_gradient(&mut self, rects: &[Rect], radius: f32, from: Color, to: Color) {
        let left = rects.iter().map(|r| r.x).fold(f32::INFINITY, f32::min);
        let top = rects.iter().map(|r| r.y).fold(f32::INFINITY, f32::min);
        let right = rects.iter().map(Rect::right).fold(f32::NEG_INFINITY, f32::max);
        let bottom = rects.iter().map(|r| r.y + r.h).fold(f32::NEG_INFINITY, f32::max);
        let stops = vec![GradientStop::new(0.0, from.skia()), GradientStop::new(1.0, to.skia())];
        let Some(shader) = LinearGradient::new(
            Point::from_xy(left, top),
            Point::from_xy(right, bottom),
            stops,
            SpreadMode::Pad,
            Transform::identity(),
        ) else {
            return;
        };
        let paint = Paint { shader, anti_alias: true, ..Default::default() };
        let t = Transform::from_scale(self.scale(), self.scale());
        let mut pm = self.pixmap();
        for r in rects {
            let path = tiny_skia::Rect::from_xywh(r.x, r.y, r.w, r.h).and_then(|rect| rounded_rect(rect, radius));
            if let Some(path) = path {
                pm.fill_path(&path, &paint, FillRule::Winding, t, None);
            }
        }
    }

    pub fn dot(&mut self, x: f32, y: f32, radius: f32, c: Color) {
        let Some(path) = PathBuilder::from_circle(x, y, radius) else { return };
        let t = Transform::from_scale(self.scale(), self.scale());
        self.pixmap().fill_path(&path, &c.paint(), FillRule::Winding, t, None);
    }

    pub fn text(&mut self, t: &Text, x: f32, y: f32, c: Color) {
        self.text_colored(t, x, y, |_| c);
    }

    /// Draws character `i` of `t` in `color(i)`.
    pub fn text_colored(&mut self, t: &Text, x: f32, y: f32, color: impl Fn(usize) -> Color) {
        let s = self.scale();
        let (ox, oy) = ((x * s).round() as i32, ((y + t.ascent) * s).round() as i32);
        let (w, h) = (self.width as i32, self.height as i32);
        let mut raster = t.raster.borrow_mut();
        if raster.as_ref().is_none_or(|(rs, _)| *rs != s) {
            *raster = Some((s, t.glyphs.iter().map(|g| Coverage::of(g, t.size, s)).collect()));
        }
        let Some((_, glyphs)) = raster.as_ref() else { return };
        let data = self.data();
        for (i, g) in glyphs.iter().enumerate() {
            let Some(g) = g else { continue };
            let c = color(i);
            for (j, &coverage) in g.alpha.iter().enumerate() {
                let (px, py) = (ox + g.x + (j % g.width) as i32, oy + g.y + (j / g.width) as i32);
                if coverage == 0 || px < 0 || py < 0 || px >= w || py >= h {
                    continue;
                }
                let a = c.3 * f32::from(coverage) / 255.0;
                let i = (py * w + px) as usize * 4;
                for (d, v) in data[i..i + 4].iter_mut().zip([c.2, c.1, c.0, 1.0]) {
                    *d = (v * a * 255.0 + f32::from(*d) * (1.0 - a)).round() as u8;
                }
            }
        }
    }

    /// Draws `img` centered on `(cx, cy)` at its native pixel size.
    pub fn image(&mut self, img: &Image, cx: f32, cy: f32) {
        let s = self.scale();
        let half = img.px as f32 / 2.0;
        let (x, y) = ((cx * s - half).round() as i32, (cy * s - half).round() as i32);
        self.pixmap().draw_pixmap(x, y, (*img.pixmap).as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }

    pub fn end(&mut self) {
        self.present(0, self.height);
    }

    fn present(&self, from: u32, to: u32) {
        let (y, h) = (from as i32, (to - from) as i32);
        unsafe {
            let dc = GetDC(Some(self.hwnd));
            let _ = BitBlt(dc, 0, y, self.width as i32, h, Some(self.dc), 0, y, SRCCOPY);
            ReleaseDC(Some(self.hwnd), dc);
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.dc);
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            }
        }
    }
}
