use capsule_core::layout::Rect;
use windows::Win32::Foundation::HWND;

use crate::capsule::gfx::Surface;

pub const CELL: f32 = 32.0;
const PAD: f32 = 6.0;
const MAX_COLUMNS: usize = 6;

pub struct Popup {
    pub hwnd: HWND,
    pub surface: Option<Surface>,
    pub open: bool,
    /// Tray icon indices, one per cell.
    pub cells: Vec<usize>,
    pub hover: Option<usize>,
    /// Screen position of the client origin, in pixels.
    pub origin: (i32, i32),
}

impl Popup {
    pub fn new(hwnd: HWND) -> Self {
        Self { hwnd, surface: None, open: false, cells: Vec::new(), hover: None, origin: (0, 0) }
    }

    fn columns(&self) -> usize {
        self.cells.len().clamp(1, MAX_COLUMNS)
    }

    pub fn size(&self) -> (f32, f32) {
        let cols = self.columns();
        let rows = self.cells.len().div_ceil(cols).max(1);
        (cols as f32 * CELL + 2.0 * PAD, rows as f32 * CELL + 2.0 * PAD)
    }

    pub fn cell_rect(&self, i: usize) -> Rect {
        let cols = self.columns();
        Rect { x: PAD + (i % cols) as f32 * CELL, y: PAD + (i / cols) as f32 * CELL, w: CELL, h: CELL }
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        (0..self.cells.len()).find(|&i| {
            let r = self.cell_rect(i);
            r.contains_x(x) && y >= r.y && y < r.y + r.h
        })
    }
}
