/// Axis-aligned rectangle in DIPs, relative to the capsule's client area.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn center_x(&self) -> f32 {
        self.x + self.w / 2.0
    }

    pub fn contains_x(&self, x: f32) -> bool {
        x >= self.x && x < self.right()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hit {
    Start,
    Task(usize),
    Chevron,
    Tray(usize),
    Keyboard,
    Volume,
    Clock,
}

pub struct Metrics {
    pub height: f32,
    pub pad: f32,
    pub task_item: f32,
    pub tray_item: f32,
    pub chevron: f32,
    pub text_pad: f32,
    pub tight_pad: f32,
}

pub const METRICS: Metrics =
    Metrics { height: 32.0, pad: 4.0, task_item: 30.0, tray_item: 24.0, chevron: 22.0, text_pad: 6.0, tight_pad: 3.0 };

#[derive(Debug, Clone, Copy, Default)]
pub struct Input {
    pub tasks: usize,
    pub tray: usize,
    pub chevron: bool,
    pub keyboard_w: f32,
    pub volume_w: f32,
    pub clock_w: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub items: Vec<(Hit, Rect)>,
}

impl Layout {
    pub fn compute(m: &Metrics, input: &Input) -> Layout {
        let mut out = Layout::default();
        let mut x = m.pad;
        let push = |out: &mut Layout, hit, w: f32, x: &mut f32| {
            out.items.push((hit, Rect { x: *x, y: 0.0, w, h: m.height }));
            *x += w;
        };

        push(&mut out, Hit::Start, m.task_item, &mut x);
        for i in 0..input.tasks {
            push(&mut out, Hit::Task(i), m.task_item, &mut x);
        }
        if input.chevron {
            push(&mut out, Hit::Chevron, m.chevron, &mut x);
        }
        for i in 0..input.tray {
            push(&mut out, Hit::Tray(i), m.tray_item, &mut x);
        }
        push(&mut out, Hit::Keyboard, input.keyboard_w + 2.0 * m.text_pad, &mut x);
        push(&mut out, Hit::Volume, input.volume_w + 2.0 * m.tight_pad, &mut x);
        push(&mut out, Hit::Clock, m.tight_pad + input.clock_w + m.text_pad, &mut x);
        out.width = x + m.pad;
        out
    }

    pub fn hit(&self, x: f32) -> Option<Hit> {
        self.items.iter().find(|(_, r)| r.contains_x(x)).map(|(h, _)| *h)
    }

    pub fn rect(&self, hit: Hit) -> Option<Rect> {
        self.items.iter().find(|(h, _)| *h == hit).map(|(_, r)| *r)
    }
}

/// Screen rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PxRect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub fn offset(&self, dx: i32, dy: i32) -> PxRect {
        PxRect { left: self.left + dx, top: self.top + dy, right: self.right + dx, bottom: self.bottom + dy }
    }
}

/// Top-left corner for a `w`×`h` capsule anchored at `center_x` (a fraction
/// of the monitor width) and `bottom_gap` pixels above the monitor bottom.
pub fn place(monitor: PxRect, w: i32, h: i32, center_x: f32, bottom_gap: i32) -> (i32, i32) {
    let cx = monitor.left + (center_x.clamp(0.0, 1.0) * monitor.width() as f32).round() as i32;
    let x = (cx - w / 2).clamp(monitor.left, (monitor.right - w).max(monitor.left));
    let y = (monitor.bottom - bottom_gap - h).clamp(monitor.top, (monitor.bottom - h).max(monitor.top));
    (x, y)
}

/// Moves a `w`×`h` capsule at `(x, y)` onto the left edge, the center or the
/// right edge, and onto the bottom, when it is within `threshold` pixels.
pub fn snap(monitor: PxRect, x: i32, y: i32, w: i32, h: i32, threshold: i32) -> (i32, i32) {
    let center = place(monitor, w, h, 0.5, 0).0;
    let x = [monitor.left, center, monitor.right - w].into_iter().find(|t| (x - t).abs() <= threshold).unwrap_or(x);
    let y = if (monitor.bottom - h - y).abs() <= threshold { monitor.bottom - h } else { y };
    (x, y)
}

/// Inverse of [`place`]: the anchor for a capsule at `(x, y)`. A capsule
/// touching a side edge anchors at 0.0 or 1.0, so it stays on that edge when
/// its width changes.
pub fn anchor(monitor: PxRect, x: i32, y: i32, w: i32, h: i32) -> (f32, i32) {
    let center_x = if x <= monitor.left {
        0.0
    } else if x + w >= monitor.right {
        1.0
    } else {
        ((x + w / 2 - monitor.left) as f32 / monitor.width().max(1) as f32).clamp(0.0, 1.0)
    };
    let bottom_gap = (monitor.bottom - (y + h)).max(0);
    (center_x, bottom_gap)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: PxRect = PxRect { left: 0, top: 0, right: 1920, bottom: 1080 };

    #[test]
    fn layout_sections() {
        let input = Input { tasks: 2, tray: 1, chevron: true, keyboard_w: 10.0, volume_w: 20.0, clock_w: 80.0 };
        let l = Layout::compute(&METRICS, &input);
        let hits: Vec<Hit> = l.items.iter().map(|(h, _)| *h).collect();
        assert_eq!(
            hits,
            [
                Hit::Start,
                Hit::Task(0),
                Hit::Task(1),
                Hit::Chevron,
                Hit::Tray(0),
                Hit::Keyboard,
                Hit::Volume,
                Hit::Clock
            ]
        );
        let m = &METRICS;
        let expected =
            m.pad * 2.0 + 3.0 * m.task_item + m.chevron + m.tray_item + 110.0 + 3.0 * m.text_pad + 3.0 * m.tight_pad;
        assert!((l.width - expected).abs() < 0.01);
        for w in l.items.windows(2) {
            assert!(w[0].1.right() <= w[1].1.x + 0.001);
        }
    }

    #[test]
    fn empty_sections_are_skipped() {
        let l = Layout::compute(&METRICS, &Input::default());
        assert_eq!(l.rect(Hit::Chevron), None);
        assert_eq!(l.hit(1.0), None);
        assert_eq!(l.hit(METRICS.pad + 1.0), Some(Hit::Start));
        assert_eq!(l.hit(METRICS.pad + METRICS.task_item + 1.0), Some(Hit::Keyboard));
    }

    #[test]
    fn placement_round_trips_and_clamps() {
        let (x, y) = place(MON, 400, 48, 0.5, 9);
        assert_eq!((x, y), (760, 1023));
        let (cx, gap) = anchor(MON, x, y, 400, 48);
        assert!((cx - 0.5).abs() < 0.001);
        assert_eq!(gap, 9);
        assert_eq!(place(MON, 400, 48, 0.0, 9).0, 0);
        assert_eq!(place(MON, 400, 48, 1.0, 9).0, 1520);
        assert_eq!(place(MON, 400, 48, 0.5, 5000).1, 0);
    }

    #[test]
    fn edge_anchors_survive_resizes() {
        assert_eq!(anchor(MON, 1520, 1023, 400, 48).0, 1.0);
        assert_eq!(place(MON, 300, 48, 1.0, 9).0 + 300, 1920);
        assert_eq!(anchor(MON, 0, 1023, 400, 48).0, 0.0);
        assert_eq!(place(MON, 500, 48, 0.0, 9).0, 0);
    }

    #[test]
    fn snaps_to_edges_center_and_bottom() {
        assert_eq!(snap(MON, 10, 1020, 400, 48, 12), (0, 1032));
        assert_eq!(snap(MON, 1510, 900, 400, 48, 12), (1520, 900));
        assert_eq!(snap(MON, 770, 1000, 400, 48, 12), (760, 1000));
        assert_eq!(snap(MON, 700, 1000, 401, 48, 12), (700, 1000));
        let (x, _) = snap(MON, 755, 0, 401, 48, 12);
        assert_eq!(anchor(MON, x, 0, 401, 48).0, 0.5);
    }
}
