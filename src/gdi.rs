use std::ffi::c_void;
use std::ptr::null_mut;

use tiny_skia::Pixmap;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::render::Overlay;

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

pub(super) fn create_icon(pixmap: &Pixmap) -> HICON {
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    unsafe {
        let mut bits: *mut c_void = null_mut();
        let Ok(color) = CreateDIBSection(None, &bitmap_info(w, h), DIB_RGB_COLORS, &mut bits, None, 0) else {
            return HICON::default();
        };
        let dst = std::slice::from_raw_parts_mut(bits as *mut [u8; 4], (w * h) as usize);
        for (dst, px) in dst.iter_mut().zip(pixmap.pixels()) {
            let c = px.demultiply();
            *dst = [c.blue(), c.green(), c.red(), c.alpha()];
        }
        let mask_bits = vec![0u8; (w as usize).div_ceil(16) * 2 * h as usize];
        let mask = CreateBitmap(w, h, 1, 1, Some(mask_bits.as_ptr() as *const c_void));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

pub(super) fn paint_overlay(hdc: HDC, overlay: &Overlay) {
    for patch in overlay.patches() {
        let (w, h) = (patch.rect.width(), patch.rect.height());
        unsafe {
            SetDIBitsToDevice(
                hdc,
                patch.rect.x(),
                patch.rect.y(),
                w,
                h,
                0,
                0,
                0,
                h,
                patch.pixels.as_ptr() as *const c_void,
                &bitmap_info(w as i32, h as i32),
                DIB_RGB_COLORS,
            );
        }
    }
}
