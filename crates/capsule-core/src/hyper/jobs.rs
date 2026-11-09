use std::collections::BTreeMap;

use crate::layout::PxRect;

/// A copy made to read the selection counts only if it doesn't end in a line
/// break: editors such as VS Code copy the whole line when nothing is selected.
pub fn copy_accept(text: &str) -> bool {
    !text.is_empty() && !text.ends_with('\n')
}

/// Reads the copy taken after Shift+Left: `(at line start, something selected)`.
/// Nothing copied means the caret is at the start of the document.
pub fn line_start_probe(copied: Option<&str>) -> (bool, bool) {
    match copied {
        None => (true, false),
        Some("\r\n" | "\n" | "\r") => (true, true),
        Some(t) if t.ends_with('\n') => (true, false),
        Some(_) => (false, true),
    }
}

/// Text to paste and how many characters to step back afterwards.
pub fn wrap_text(open: &str, close: &str, selection: Option<&str>) -> (String, usize) {
    match selection {
        Some(sel) => (format!("{open}{sel}{close}"), 0),
        None => (format!("{open}{close}"), close.chars().count()),
    }
}

/// The longest key that ends `line`, ignoring case: `(characters to delete,
/// replacement)`.
pub fn hotstring<'a>(line: &str, table: &'a BTreeMap<String, String>) -> Option<(usize, &'a str)> {
    let line = line.to_lowercase();
    table
        .iter()
        .filter(|(key, _)| !key.is_empty() && line.ends_with(&key.to_lowercase()))
        .max_by_key(|(key, _)| key.chars().count())
        .map(|(key, value)| (key.chars().count(), value.as_str()))
}

/// Window alpha after a tap of Caps+F4; `None` is opaque.
pub fn alpha_toggle(alpha: Option<u8>) -> Option<u8> {
    if alpha.is_some() { None } else { Some(170) }
}

/// Window alpha after a wheel notch while Caps+F4 is held.
pub fn alpha_step(alpha: Option<u8>, up: bool) -> Option<u8> {
    if up {
        alpha.and_then(|a| a.checked_add(10)).filter(|&a| a < 255)
    } else {
        Some(alpha.unwrap_or(245).saturating_sub(10).max(15))
    }
}

/// Window placement for Caps+Alt+F with the mouse at the left edge, chosen by
/// the cursor's height on the monitor: `(rect, topmost)`.
pub fn window_preset(monitor: PxRect, work: PxRect, cursor_y: i32) -> (PxRect, bool) {
    let (w, h) = (work.width() as f32, work.height() as f32);
    let rect = |x: f32, y: f32, rw: f32, rh: f32| {
        let left = work.left + (x * w).round() as i32;
        let top = work.top + (y * h).round() as i32;
        PxRect { left, top, right: left + (rw * w).round() as i32, bottom: top + (rh * h).round() as i32 }
    };
    let rel = (cursor_y - monitor.top) as f32 / monitor.height().max(1) as f32;
    if rel < 0.2 {
        (rect(0.1, 0.1, 0.72, 0.82), false)
    } else if rel < 0.8 {
        (rect(0.05, 0.0, 0.9, 1.0), false)
    } else {
        (rect(0.6, 0.55, 0.4, 0.45), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert!(copy_accept("abc") && !copy_accept("") && !copy_accept("line\r\n"));
        assert_eq!(line_start_probe(None), (true, false));
        assert_eq!(line_start_probe(Some("\r\n")), (true, true));
        assert_eq!(line_start_probe(Some("whole line\r\n")), (true, false));
        assert_eq!(line_start_probe(Some("x")), (false, true));
        assert_eq!(wrap_text("(", ")", Some("a b")), ("(a b)".into(), 0));
        assert_eq!(wrap_text(" + \"", "\" + ", None), (" + \"\" + ".into(), 4));

        let table: BTreeMap<String, String> =
            [("jos", "j@x"), ("os", "short"), ("x", "")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        assert_eq!(hotstring("mail: JOS", &table), Some((3, "j@x")));
        assert_eq!(hotstring("kos", &table), Some((2, "short")));
        assert_eq!(hotstring("nothing", &table), None);

        assert_eq!(alpha_toggle(None), Some(170));
        assert_eq!(alpha_toggle(Some(200)), None);
        assert_eq!(alpha_step(None, false), Some(235));
        assert_eq!(alpha_step(Some(20), false), Some(15));
        assert_eq!(alpha_step(None, true), None);
        assert_eq!(alpha_step(Some(240), true), Some(250));
        assert_eq!(alpha_step(Some(250), true), None);

        let m = PxRect { left: 0, top: 0, right: 1000, bottom: 1000 };
        let w = PxRect { bottom: 900, ..m };
        assert_eq!(window_preset(m, w, 100), (PxRect { left: 100, top: 90, right: 820, bottom: 828 }, false));
        assert_eq!(window_preset(m, w, 500), (PxRect { left: 50, top: 0, right: 950, bottom: 900 }, false));
        let (r, topmost) = window_preset(m, w, 900);
        assert!(topmost);
        assert_eq!((r.right, r.bottom), (1000, 900));
    }
}
