//! Geometry for showing one pane of a window: the window is clipped to the
//! pane and placed so the pane covers a chosen frame.

use crate::layout::PxRect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grip {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

pub const GRIPS: [Grip; 4] = [Grip::TopLeft, Grip::TopRight, Grip::BottomLeft, Grip::BottomRight];

/// The window rect that puts `pane`, a part of `window`, over `frame`, and
/// the pane's rect within that window for its region. The pane keeps its
/// distance from the window's top and left edges.
pub fn fit(frame: PxRect, window: PxRect, pane: PxRect) -> (PxRect, PxRect) {
    let (dx, dy) = (pane.left - window.left, pane.top - window.top);
    let (left, top) = (frame.left - dx, frame.top - dy);
    let placed = PxRect {
        left,
        top,
        right: left + frame.width() + window.width() - pane.width(),
        bottom: top + frame.height() + window.height() - pane.height(),
    };
    (placed, PxRect { left: dx, top: dy, right: dx + frame.width(), bottom: dy + frame.height() })
}

pub fn croppable(window: PxRect, pane: PxRect) -> bool {
    pane.width() > 0
        && pane.height() > 0
        && pane != window
        && pane.left >= window.left
        && pane.top >= window.top
        && pane.right <= window.right
        && pane.bottom <= window.bottom
}

pub fn insets(outer: PxRect, inner: PxRect) -> PxRect {
    PxRect {
        left: inner.left - outer.left,
        top: inner.top - outer.top,
        right: outer.right - inner.right,
        bottom: outer.bottom - inner.bottom,
    }
}

pub fn shrink(r: PxRect, by: PxRect) -> PxRect {
    PxRect { left: r.left + by.left, top: r.top + by.top, right: r.right - by.right, bottom: r.bottom - by.bottom }
}

pub fn grip_rect(frame: PxRect, grip: Grip, size: i32) -> PxRect {
    let left = if matches!(grip, Grip::TopLeft | Grip::BottomLeft) { frame.left } else { frame.right - size };
    let top = if matches!(grip, Grip::TopLeft | Grip::TopRight) { frame.top } else { frame.bottom - size };
    PxRect { left, top, right: left + size, bottom: top + size }
}

/// `start` after dragging `grip` by `(dx, dy)`. The top-left grip moves the
/// frame; the others move their corner's two edges, keeping the frame at
/// least `min` pixels wide and high.
pub fn drag(start: PxRect, grip: Grip, dx: i32, dy: i32, min: i32) -> PxRect {
    let mut r = start;
    match grip {
        Grip::TopLeft => return r.offset(dx, dy),
        Grip::TopRight | Grip::BottomRight => r.right = (r.right + dx).max(r.left + min),
        Grip::BottomLeft => r.left = (r.left + dx).min(r.right - min),
    }
    match grip {
        Grip::TopRight => r.top = (r.top + dy).min(r.bottom - min),
        _ => r.bottom = (r.bottom + dy).max(r.top + min),
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn rect(left: i32, top: i32, right: i32, bottom: i32) -> PxRect {
        PxRect { left, top, right, bottom }
    }

    #[test]
    fn fit_covers_the_frame_with_the_pane() {
        let window = rect(92, 42, 1108, 858);
        let pane = rect(100, 150, 1100, 850);
        let frame = rect(100, 50, 1100, 850);
        let (placed, region) = fit(frame, window, pane);
        assert_eq!(placed, rect(92, -58, 1108, 858));
        assert_eq!(region, rect(8, 108, 1008, 908));
        let pane_after = rect(100, 50, 1100, 850);
        assert_eq!(fit(frame, placed, pane_after), (placed, region));
    }

    #[test]
    fn fit_follows_a_new_frame() {
        let placed = rect(92, -58, 1108, 858);
        let pane = rect(100, 50, 1100, 850);
        let (moved, region) = fit(rect(0, 0, 500, 400), placed, pane);
        assert_eq!(moved, rect(-8, -108, 508, 408));
        assert_eq!(region, rect(8, 108, 508, 508));
    }

    #[test]
    fn only_inner_panes_crop() {
        let window = rect(0, 0, 800, 600);
        assert!(croppable(window, rect(0, 80, 800, 600)));
        assert!(!croppable(window, window));
        assert!(!croppable(window, rect(0, 80, 800, 80)));
        assert!(!croppable(window, rect(-1, 80, 800, 600)));
    }

    #[test]
    fn insets_and_shrink_round_trip() {
        let outer = rect(92, 42, 1108, 858);
        let inner = rect(100, 42, 1100, 850);
        let by = insets(outer, inner);
        assert_eq!(by, rect(8, 0, 8, 8));
        assert_eq!(shrink(outer, by), inner);
    }

    #[test]
    fn grips_sit_in_the_corners() {
        let frame = rect(100, 50, 1100, 850);
        assert_eq!(grip_rect(frame, Grip::TopLeft, 12), rect(100, 50, 112, 62));
        assert_eq!(grip_rect(frame, Grip::BottomRight, 12), rect(1088, 838, 1100, 850));
        assert_eq!(grip_rect(frame, Grip::TopRight, 12), rect(1088, 50, 1100, 62));
        assert_eq!(grip_rect(frame, Grip::BottomLeft, 12), rect(100, 838, 112, 850));
    }

    #[test]
    fn dragging_moves_or_resizes() {
        let start = rect(100, 100, 600, 500);
        assert_eq!(drag(start, Grip::TopLeft, -50, 20, 100), rect(50, 120, 550, 520));
        assert_eq!(drag(start, Grip::BottomRight, 30, -40, 100), rect(100, 100, 630, 460));
        assert_eq!(drag(start, Grip::TopRight, 10, 30, 100), rect(100, 130, 610, 500));
        assert_eq!(drag(start, Grip::BottomLeft, -10, 5, 100), rect(90, 100, 600, 505));
        assert_eq!(drag(start, Grip::BottomRight, -1000, -1000, 100), rect(100, 100, 200, 200));
        assert_eq!(drag(start, Grip::TopRight, -1000, 1000, 100), rect(100, 400, 200, 500));
        assert_eq!(drag(start, Grip::BottomLeft, 1000, -1000, 100), rect(500, 100, 600, 200));
    }
}
