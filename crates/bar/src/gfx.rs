use std::cell::Cell;
use std::collections::HashMap;

use capsule_core::layout::Rect;
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_HWND_RENDER_TARGET_PROPERTIES,
    D2D1_PRESENT_OPTIONS_IMMEDIATELY, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap, ID2D1Factory, ID2D1HwndRenderTarget,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_FEATURE, DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_METRICS,
    DWRITE_TEXT_RANGE, DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
    IDWriteTextLayout, IDWriteTypography,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::Graphics::Gdi::HPALETTE;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmap, IWICBitmapSource, IWICImagingFactory,
    WICBitmapCacheOnLoad, WICBitmapDitherTypeNone, WICBitmapInterpolationModeHighQualityCubic,
    WICBitmapPaletteTypeCustom, WICBitmapUsePremultipliedAlpha,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::WindowsAndMessaging::HICON;
use windows::core::{Interface, Result, w};
use windows_numerics::Vector2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);

impl Color {
    pub fn alpha(self, a: f32) -> Color {
        Color(self.0, self.1, self.2, self.3 * a)
    }

    fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.0, g: self.1, b: self.2, a: self.3 }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Font {
    Text,
    Digits,
    Glyph,
}

pub struct Text {
    layout: IDWriteTextLayout,
    pub width: f32,
    pub height: f32,
}

/// Device-independent icon pixels, pre-scaled to their on-screen size.
#[derive(Clone)]
pub struct Image {
    id: u64,
    bitmap: IWICBitmap,
    pub px: u32,
}

pub struct Gfx {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    wic: IWICImagingFactory,
    text: IDWriteTextFormat,
    glyph: IDWriteTextFormat,
    tabular: IDWriteTypography,
    next_id: Cell<u64>,
}

impl Gfx {
    pub fn new() -> Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let format = |family, size| -> Result<IDWriteTextFormat> {
                let f = dwrite.CreateTextFormat(
                    family,
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!(""),
                )?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                Ok(f)
            };
            let text = format(w!("Segoe UI Variable Text"), 12.0)?;
            let glyph = format(w!("Segoe Fluent Icons"), 12.0)?;
            let tabular = dwrite.CreateTypography()?;
            tabular.AddFontFeature(DWRITE_FONT_FEATURE {
                nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
                parameter: 1,
            })?;
            Ok(Self { d2d, dwrite, wic, text, glyph, tabular, next_id: Cell::new(1) })
        }
    }

    pub fn text(&self, s: &str, font: Font) -> Result<Text> {
        let units: Vec<u16> = s.encode_utf16().collect();
        let format = if font == Font::Glyph { &self.glyph } else { &self.text };
        unsafe {
            let layout = self.dwrite.CreateTextLayout(&units, format, 10_000.0, 100.0)?;
            if font == Font::Digits {
                layout
                    .SetTypography(&self.tabular, DWRITE_TEXT_RANGE { startPosition: 0, length: units.len() as u32 })?;
            }
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m)?;
            Ok(Text { layout, width: m.widthIncludingTrailingWhitespace, height: m.height })
        }
    }

    fn finish(&self, source: &IWICBitmapSource, px: u32) -> Result<Image> {
        unsafe {
            let scaler = self.wic.CreateBitmapScaler()?;
            scaler.Initialize(source, px, px, WICBitmapInterpolationModeHighQualityCubic)?;
            let converter = self.wic.CreateFormatConverter()?;
            converter.Initialize(
                &scaler,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )?;
            let bitmap = self.wic.CreateBitmapFromSource(&converter, WICBitmapCacheOnLoad)?;
            let id = self.next_id.get();
            self.next_id.set(id + 1);
            Ok(Image { id, bitmap, px })
        }
    }

    pub fn image_from_hicon(&self, icon: HICON, px: u32) -> Result<Image> {
        let bitmap = unsafe { self.wic.CreateBitmapFromHICON(icon)? };
        self.finish(&bitmap.cast()?, px)
    }

    pub fn image_from_hbitmap(&self, bitmap: HBITMAP, px: u32) -> Result<Image> {
        let bitmap =
            unsafe { self.wic.CreateBitmapFromHBITMAP(bitmap, HPALETTE::default(), WICBitmapUsePremultipliedAlpha)? };
        self.finish(&bitmap.cast()?, px)
    }
}

/// A D2D render target bound to one window. Coordinates are DIPs.
pub struct Surface {
    rt: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
    bitmaps: HashMap<u64, ID2D1Bitmap>,
    used: Vec<u64>,
    dpi: u32,
}

impl Surface {
    pub fn new(gfx: &Gfx, hwnd: HWND, width: u32, height: u32, dpi: u32, software: bool) -> Result<Self> {
        unsafe {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: if software { D2D1_RENDER_TARGET_TYPE_SOFTWARE } else { D2D1_RENDER_TARGET_TYPE_DEFAULT },
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: dpi as f32,
                dpiY: dpi as f32,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: D2D_SIZE_U { width: width.max(1), height: height.max(1) },
                presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
            };
            let rt = gfx.d2d.CreateHwndRenderTarget(&props, &hwnd_props)?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let brush = rt.CreateSolidColorBrush(&Color(0.0, 0.0, 0.0, 0.0).d2d(), None)?;
            Ok(Self { rt, brush, bitmaps: HashMap::new(), used: Vec::new(), dpi })
        }
    }

    pub fn resize(&self, width: u32, height: u32) {
        unsafe {
            let _ = self.rt.Resize(&D2D_SIZE_U { width: width.max(1), height: height.max(1) });
        }
    }

    pub fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi;
        self.bitmaps.clear();
        unsafe { self.rt.SetDpi(dpi as f32, dpi as f32) };
    }

    fn snap(&self, v: f32) -> f32 {
        let s = self.dpi as f32 / 96.0;
        (v * s).round() / s
    }

    pub fn begin(&mut self) {
        self.used.clear();
        unsafe {
            self.rt.BeginDraw();
            self.rt.Clear(Some(&Color(0.0, 0.0, 0.0, 0.0).d2d()));
        }
    }

    fn brush(&self, c: Color) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&c.d2d()) };
        &self.brush
    }

    pub fn fill(&self, r: Rect, c: Color, radius: f32) {
        let rect = D2D_RECT_F { left: r.x, top: r.y, right: r.right(), bottom: r.y + r.h };
        unsafe {
            self.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect, radiusX: radius, radiusY: radius }, self.brush(c));
        }
    }

    pub fn dot(&self, x: f32, y: f32, radius: f32, c: Color) {
        let e = D2D1_ELLIPSE { point: Vector2 { X: x, Y: y }, radiusX: radius, radiusY: radius };
        unsafe { self.rt.FillEllipse(&e, self.brush(c)) };
    }

    pub fn vline(&self, x: f32, y0: f32, y1: f32, c: Color) {
        let x = self.snap(x) + 0.5 * 96.0 / self.dpi as f32;
        unsafe {
            self.rt.DrawLine(
                Vector2 { X: x, Y: y0 },
                Vector2 { X: x, Y: y1 },
                self.brush(c),
                96.0 / self.dpi as f32,
                None,
            );
        }
    }

    pub fn text(&self, t: &Text, x: f32, y: f32, c: Color) {
        unsafe {
            self.rt.DrawTextLayout(
                Vector2 { X: self.snap(x), Y: self.snap(y) },
                &t.layout,
                self.brush(c),
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }

    /// Draws `img` centered on `(cx, cy)` at its native pixel size.
    pub fn image(&mut self, img: &Image, cx: f32, cy: f32, opacity: f32) {
        let size = img.px as f32 * 96.0 / self.dpi as f32;
        let x = self.snap(cx - size / 2.0);
        let y = self.snap(cy - size / 2.0);
        let bitmap = match self.bitmaps.get(&img.id) {
            Some(b) => b.clone(),
            None => {
                let Ok(b) = (unsafe { self.rt.CreateBitmapFromWicBitmap(&img.bitmap, None) }) else {
                    return;
                };
                self.bitmaps.insert(img.id, b.clone());
                b
            }
        };
        self.used.push(img.id);
        let dest = D2D_RECT_F { left: x, top: y, right: x + size, bottom: y + size };
        unsafe {
            self.rt.DrawBitmap(&bitmap, Some(&dest), opacity, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None);
        }
    }

    /// Returns `false` when the device was lost and the surface must be recreated.
    pub fn end(&mut self) -> bool {
        let used = std::mem::take(&mut self.used);
        self.bitmaps.retain(|id, _| used.contains(id));
        self.used = used;
        !matches!(unsafe { self.rt.EndDraw(None, None) }, Err(e) if e.code() == D2DERR_RECREATE_TARGET)
    }
}
