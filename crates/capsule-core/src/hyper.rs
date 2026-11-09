//! CapsLock as a hyper key: tap for Esc, hold for a layer of editing and
//! window shortcuts, add left Alt for a second layer.

use std::collections::BTreeMap;

use crate::layout::PxRect;

pub mod vk {
    pub const BACK: u16 = 0x08;
    pub const TAB: u16 = 0x09;
    pub const RETURN: u16 = 0x0D;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const CAPITAL: u16 = 0x14;
    pub const ESCAPE: u16 = 0x1B;
    pub const SPACE: u16 = 0x20;
    pub const PRIOR: u16 = 0x21;
    pub const NEXT: u16 = 0x22;
    pub const END: u16 = 0x23;
    pub const HOME: u16 = 0x24;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const INSERT: u16 = 0x2D;
    pub const DELETE: u16 = 0x2E;
    pub const LWIN: u16 = 0x5B;
    pub const RWIN: u16 = 0x5C;
    pub const APPS: u16 = 0x5D;
    pub const F1: u16 = 0x70;
    pub const F2: u16 = 0x71;
    pub const F3: u16 = 0x72;
    pub const F4: u16 = 0x73;
    pub const F5: u16 = 0x74;
    pub const F9: u16 = 0x78;
    pub const F12: u16 = 0x7B;
    pub const LSHIFT: u16 = 0xA0;
    pub const RSHIFT: u16 = 0xA1;
    pub const LCONTROL: u16 = 0xA2;
    pub const RCONTROL: u16 = 0xA3;
    pub const LMENU: u16 = 0xA4;
    pub const RMENU: u16 = 0xA5;
    pub const BROWSER_BACK: u16 = 0xA6;
    pub const BROWSER_FORWARD: u16 = 0xA7;
    pub const MEDIA_NEXT_TRACK: u16 = 0xB0;
    pub const MEDIA_PREV_TRACK: u16 = 0xB1;
    pub const MEDIA_PLAY_PAUSE: u16 = 0xB3;
    /// `;`
    pub const OEM_1: u16 = 0xBA;
    /// `=`
    pub const OEM_PLUS: u16 = 0xBB;
    pub const OEM_COMMA: u16 = 0xBC;
    pub const OEM_MINUS: u16 = 0xBD;
    pub const OEM_PERIOD: u16 = 0xBE;
    /// `/`
    pub const OEM_2: u16 = 0xBF;
    /// `` ` ``
    pub const OEM_3: u16 = 0xC0;
    /// `[`
    pub const OEM_4: u16 = 0xDB;
    /// `\`
    pub const OEM_5: u16 = 0xDC;
    /// `]`
    pub const OEM_6: u16 = 0xDD;
    /// `'`
    pub const OEM_7: u16 = 0xDE;

    pub const fn letter(c: u8) -> u16 {
        c.to_ascii_uppercase() as u16
    }
}

pub const CTRL: u8 = 1;
pub const SHIFT: u8 = 2;
pub const ALT: u8 = 4;
pub const WIN: u8 = 8;

const TAP_MS: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub mods: u8,
    pub vk: u16,
}

const fn k(vk: u16) -> Chord {
    Chord { mods: 0, vk }
}

const fn c(vk: u16) -> Chord {
    Chord { mods: CTRL, vk }
}

const fn s(vk: u16) -> Chord {
    Chord { mods: SHIFT, vk }
}

const fn cs(vk: u16) -> Chord {
    Chord { mods: CTRL | SHIFT, vk }
}

const fn l(ch: u8) -> u16 {
    vk::letter(ch)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Other,
    /// A program from the configured terminal list.
    Terminal,
    /// gvim or nvim-qt.
    Vim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// Copy the selection, or the current line when nothing is selected.
    CopyOrLine,
    PastePlain,
    /// Surround the selection with `open` and `close`, or insert both with
    /// the caret between them. `at_end` first moves to the end of the line,
    /// and presses Enter afterwards.
    Wrap {
        open: &'static str,
        close: &'static str,
        at_end: bool,
    },
    /// Insert text through the clipboard, so the IME and auto-closing
    /// brackets leave it alone.
    Paste(&'static str),
    DeleteLine,
    Hotstring,
    /// Esc, or with the mouse at the screen's left edge, place the window.
    EscapeOrMove,
    Terminal,
    PowerOptions,
    Topmost,
    Caption,
    Reload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Keys(&'static [Chord]),
    Click(Button),
    /// Wheel notches, positive is up.
    Wheel(i32),
    Job(Job),
    Transparency,
    SwitchDesktop,
    Noop,
}

macro_rules! keys {
    ($($c:expr),* $(,)?) => {
        Action::Keys({
            const K: &[Chord] = &[$($c),*];
            K
        })
    };
}

const fn wrap(open: &'static str, close: &'static str) -> Action {
    Action::Job(Job::Wrap { open, close, at_end: false })
}

pub fn lookup(key: u16, alt: bool, target: impl FnOnce() -> Target) -> Action {
    use vk::*;
    if alt {
        return match key {
            _ if key == l(b'h') => keys![s(LEFT)],
            _ if key == l(b'j') => keys![s(DOWN)],
            _ if key == l(b'k') => keys![s(UP)],
            _ if key == l(b'l') => keys![s(RIGHT)],
            _ if key == l(b'e') => keys![cs(RIGHT)],
            _ if key == l(b'b') => keys![cs(LEFT)],
            OEM_1 => keys![s(HOME)],
            OEM_7 => keys![s(END)],
            _ if key == l(b'g') => keys![c(HOME)],
            _ if key == l(b'q') => Action::Wheel(-2),
            _ if key == l(b'r') => Action::Wheel(2),
            _ if key == l(b'o') => keys![k(HOME), k(RETURN), k(UP)],
            _ if key == l(b'u') => keys![c(l(b'y'))],
            _ if key == l(b'z') => keys![cs(l(b's'))],
            _ if key == l(b'v') => Action::SwitchDesktop,
            _ if key == l(b'w') => keys![k(END), s(DOWN), s(HOME), k(BACK)],
            _ if key == l(b'd') => keys![k(END), s(HOME), k(DELETE)],
            _ if key == l(b'x') => keys![c(INSERT), k(BACK)],
            _ if key == l(b'y') => Action::Job(Job::CopyOrLine),
            _ if key == l(b'p') => Action::Job(Job::PastePlain),
            _ if key == u16::from(b'9') => wrap("\"", "\""),
            _ if key == u16::from(b'0') => wrap("'", "'"),
            OEM_4 => wrap("[", "]"),
            _ if key == l(b'f') => Action::Job(Job::EscapeOrMove),
            _ if key == l(b't') => Action::Job(Job::Terminal),
            F1 => Action::Job(Job::Caption),
            F2 => keys![k(MEDIA_PLAY_PAUSE)],
            F3 => keys![k(MEDIA_PREV_TRACK)],
            _ => Action::Noop,
        };
    }
    match key {
        _ if key == l(b'h') => keys![k(LEFT)],
        _ if key == l(b'j') => keys![k(DOWN)],
        _ if key == l(b'k') => keys![k(UP)],
        _ if key == l(b'l') => keys![k(RIGHT)],
        _ if key == l(b'e') => keys![c(RIGHT)],
        _ if key == l(b'b') => keys![c(LEFT)],
        OEM_1 => keys![k(HOME)],
        OEM_7 | RETURN => keys![k(END)],
        SPACE => keys![k(RIGHT)],
        _ if key == l(b'g') => keys![c(END)],
        _ if key == l(b'q') => keys![k(NEXT)],
        _ if key == l(b'r') => keys![k(PRIOR)],
        _ if key == l(b'n') => match target() {
            Target::Vim => keys![k(F2)],
            _ => keys![cs(TAB)],
        },
        _ if key == l(b'm') => match target() {
            Target::Vim => keys![k(F3)],
            _ => keys![c(TAB)],
        },
        _ if key == l(b'o') => keys![k(END), k(RETURN)],
        _ if key == l(b'u') => match target() {
            Target::Terminal => keys![c(OEM_2)],
            _ => keys![c(l(b'z'))],
        },
        _ if key == l(b'z') => keys![c(l(b's'))],
        _ if key == l(b'c') => keys![Chord { mods: ALT, vk: TAB }],
        _ if key == l(b'v') => keys![cs(l(b'v'))],
        OEM_2 => keys![c(l(b'f'))],
        _ if key == u16::from(b'1') => Action::Click(Button::Left),
        _ if key == u16::from(b'2') => Action::Click(Button::Middle),
        _ if key == u16::from(b'3') => Action::Click(Button::Right),
        OEM_COMMA => keys![k(BROWSER_BACK)],
        OEM_PERIOD => keys![k(BROWSER_FORWARD)],
        OEM_3 => keys![k(APPS)],
        _ if key == l(b'w') => match target() {
            Target::Other => keys![cs(LEFT), k(BACK)],
            _ => keys![c(l(b'w'))],
        },
        BACK => keys![c(LEFT), cs(RIGHT), k(BACK)],
        _ if key == l(b'd') => match target() {
            Target::Other => Action::Job(Job::DeleteLine),
            _ => keys![c(l(b'u'))],
        },
        _ if key == l(b'x') => keys![c(INSERT), k(DELETE)],
        _ if key == l(b'y') => Action::Job(Job::CopyOrLine),
        _ if key == l(b'p') => keys![s(INSERT)],
        _ if key == u16::from(b'9') => wrap("(", ")"),
        _ if key == u16::from(b'0') => Action::Job(Job::Paste(")")),
        OEM_4 => Action::Job(Job::Wrap { open: " {", close: "}", at_end: true }),
        OEM_PLUS => wrap(" + \"", "\" + "),
        TAB => Action::Job(Job::Hotstring),
        F1 => Action::Job(Job::Topmost),
        F3 => keys![k(MEDIA_NEXT_TRACK)],
        F4 => Action::Transparency,
        F5 => Action::Job(Job::Reload),
        F9 => Action::Job(Job::PowerOptions),
        _ => Action::Noop,
    }
}

/// Keys the layer takes over while CapsLock is held, mapped or not.
pub fn in_layer(key: u16) -> bool {
    use vk::*;
    matches!(key, 0x30..=0x39 | 0x41..=0x5A)
        || (F1..=F12).contains(&key)
        || matches!(
            key,
            SPACE
                | TAB
                | RETURN
                | ESCAPE
                | BACK
                | OEM_1
                | OEM_PLUS
                | OEM_COMMA
                | OEM_MINUS
                | OEM_PERIOD
                | OEM_2
                | OEM_3
                | OEM_4
                | OEM_5
                | OEM_6
                | OEM_7
        )
}

fn is_modifier(key: u16) -> bool {
    use vk::*;
    matches!(key, SHIFT | CONTROL | LSHIFT | RSHIFT | LCONTROL | RCONTROL | LWIN | RWIN)
}

fn button_bit(b: Button) -> u8 {
    1 << b as u8
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub vk: u16,
    pub down: bool,
    /// Milliseconds, as in `KBDLLHOOKSTRUCT::time`.
    pub time: u32,
    /// `CTRL`, `SHIFT`, `WIN` and `ALT` (left Alt) as physically held.
    pub mods: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Esc,
    Run(Action),
    Button(Button, bool),
    SlowMouse(bool),
    AlphaBegin,
    AlphaEnd { tap: bool },
}

#[derive(Debug, Default)]
pub struct Layer {
    caps_at: Option<u32>,
    used: bool,
    /// Left Alt was down before CapsLock, so the layer stays out of the way.
    passthrough: bool,
    alt: bool,
    swallowed: [u64; 4],
    buttons: u8,
    alpha_at: Option<u32>,
    desktop_right: bool,
}

impl Layer {
    /// Whether events other than CapsLock need `on_key`.
    pub fn engaged(&self) -> bool {
        self.caps_at.is_some() || self.swallowed != [0; 4]
    }

    pub fn held(&self) -> bool {
        self.caps_at.is_some()
    }

    /// Marks `key` as swallowed; returns whether it already was (auto-repeat).
    fn mark(&mut self, key: u16) -> bool {
        let (i, bit) = (usize::from(key >> 6) & 3, 1u64 << (key & 63));
        let was = self.swallowed[i] & bit != 0;
        self.swallowed[i] |= bit;
        was
    }

    fn take(&mut self, key: u16) -> bool {
        let (i, bit) = (usize::from(key >> 6) & 3, 1u64 << (key & 63));
        let was = self.swallowed[i] & bit != 0;
        self.swallowed[i] &= !bit;
        was
    }

    /// Returns whether to swallow the event.
    pub fn on_key(&mut self, ev: Event, target: impl FnOnce() -> Target, emit: &mut impl FnMut(Effect)) -> bool {
        let Event { vk: key, down, time, mods } = ev;
        if !down {
            if !self.take(key) {
                return false;
            }
            match key {
                vk::CAPITAL => self.release_caps(time, emit),
                vk::LMENU if self.alt => {
                    self.alt = false;
                    emit(Effect::SlowMouse(false));
                }
                vk::F4 => self.end_alpha(time, emit),
                _ => {}
            }
            for (b, bkey) in [(Button::Left, b'1'), (Button::Middle, b'2'), (Button::Right, b'3')] {
                if key == u16::from(bkey) && self.buttons & button_bit(b) != 0 {
                    self.buttons &= !button_bit(b);
                    emit(Effect::Button(b, false));
                }
            }
            return true;
        }
        if key == vk::CAPITAL {
            if !self.mark(key) && self.caps_at.is_none() {
                self.caps_at = Some(time);
                self.used = mods != 0;
                self.passthrough = mods & ALT != 0;
            }
            return true;
        }
        if self.caps_at.is_none() {
            return false;
        }
        self.used = true;
        if is_modifier(key) {
            return false;
        }
        if key == vk::LMENU {
            if self.passthrough {
                return false;
            }
            if !self.mark(key) && !self.alt {
                self.alt = true;
                emit(Effect::SlowMouse(true));
            }
            return true;
        }
        if key == vk::RMENU {
            self.mark(key);
            return true;
        }
        if self.passthrough || mods & (CTRL | SHIFT | WIN) != 0 || !in_layer(key) {
            return false;
        }
        let repeat = self.mark(key);
        match lookup(key, self.alt, target) {
            a @ (Action::Keys(_) | Action::Wheel(_)) => emit(Effect::Run(a)),
            _ if repeat => {}
            Action::Click(b) => {
                self.buttons |= button_bit(b);
                emit(Effect::Button(b, true));
            }
            a @ Action::Job(_) => emit(Effect::Run(a)),
            Action::Transparency => {
                self.alpha_at = Some(time);
                emit(Effect::AlphaBegin);
            }
            Action::SwitchDesktop => {
                self.desktop_right = !self.desktop_right;
                emit(Effect::Run(if self.desktop_right {
                    keys![Chord { mods: WIN | CTRL, vk: vk::RIGHT }]
                } else {
                    keys![Chord { mods: WIN | CTRL, vk: vk::LEFT }]
                }));
            }
            Action::Noop => {}
        }
        true
    }

    fn release_caps(&mut self, time: u32, emit: &mut impl FnMut(Effect)) {
        let Some(at) = self.caps_at.take() else { return };
        if self.alt {
            self.alt = false;
            emit(Effect::SlowMouse(false));
        }
        self.end_alpha(time, emit);
        if !self.used && time.wrapping_sub(at) < TAP_MS {
            emit(Effect::Esc);
        }
        self.passthrough = false;
    }

    fn end_alpha(&mut self, time: u32, emit: &mut impl FnMut(Effect)) {
        if let Some(at) = self.alpha_at.take() {
            emit(Effect::AlphaEnd { tap: time.wrapping_sub(at) < TAP_MS });
        }
    }
}

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
    use super::vk::*;
    use super::*;

    fn ev(vk: u16, down: bool, time: u32) -> Event {
        Event { vk, down, time, mods: 0 }
    }

    fn run(layer: &mut Layer, e: Event) -> (bool, Vec<Effect>) {
        let mut out = Vec::new();
        let swallow = layer.on_key(e, || Target::Other, &mut |x| out.push(x));
        (swallow, out)
    }

    #[test]
    fn tap_sends_esc_only_when_quick_and_alone() {
        let mut layer = Layer::default();
        assert_eq!(run(&mut layer, ev(CAPITAL, true, 1000)), (true, vec![]));
        assert_eq!(run(&mut layer, ev(CAPITAL, true, 1030)), (true, vec![]));
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 1100)), (true, vec![Effect::Esc]));
        assert!(!layer.engaged());

        run(&mut layer, ev(CAPITAL, true, 2000));
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 2400)), (true, vec![]));

        run(&mut layer, ev(CAPITAL, true, 3000));
        assert!(!run(&mut layer, ev(LSHIFT, true, 3010)).0);
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 3020)), (true, vec![]));

        run(&mut layer, ev(CAPITAL, true, u32::MAX - 10));
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 20)).1, vec![Effect::Esc]);
    }

    #[test]
    fn layer_maps_and_swallows_the_release() {
        let mut layer = Layer::default();
        run(&mut layer, ev(CAPITAL, true, 0));
        let (swallow, fx) = run(&mut layer, ev(l(b'h'), true, 10));
        assert!(swallow);
        assert_eq!(fx, vec![Effect::Run(keys![k(LEFT)])]);
        assert_eq!(run(&mut layer, ev(l(b'h'), true, 40)).1.len(), 1);
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 50)), (true, vec![]));
        assert!(layer.engaged());
        assert_eq!(run(&mut layer, ev(l(b'h'), false, 60)), (true, vec![]));
        assert!(!layer.engaged());
        assert!(!run(&mut layer, ev(l(b'h'), true, 70)).0);
    }

    #[test]
    fn unmapped_and_outside_keys() {
        let mut layer = Layer::default();
        run(&mut layer, ev(CAPITAL, true, 0));
        assert_eq!(run(&mut layer, ev(l(b'a'), true, 10)), (true, vec![]));
        assert_eq!(run(&mut layer, ev(INSERT, true, 10)), (false, vec![]));
        assert!(run(&mut layer, ev(RMENU, true, 10)).0);
    }

    #[test]
    fn held_modifiers_pass_keys_through() {
        let mut layer = Layer::default();
        run(&mut layer, ev(CAPITAL, true, 0));
        for mods in [CTRL, super::SHIFT, WIN] {
            assert!(!run(&mut layer, Event { vk: l(b'h'), down: true, time: 10, mods }).0);
        }
        let mut layer = Layer::default();
        run(&mut layer, Event { vk: CAPITAL, down: true, time: 0, mods: CTRL });
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 10)), (true, vec![]));
        run(&mut layer, Event { vk: CAPITAL, down: true, time: 0, mods: ALT });
        assert!(!run(&mut layer, ev(LMENU, true, 5)).0);
        assert!(!run(&mut layer, Event { vk: l(b'h'), down: true, time: 10, mods: ALT }).0);
    }

    #[test]
    fn alt_layer_and_slow_mouse() {
        let mut layer = Layer::default();
        run(&mut layer, ev(CAPITAL, true, 0));
        assert_eq!(run(&mut layer, ev(LMENU, true, 10)), (true, vec![Effect::SlowMouse(true)]));
        assert_eq!(run(&mut layer, ev(LMENU, true, 40)), (true, vec![]));
        assert_eq!(run(&mut layer, ev(l(b'h'), true, 50)).1, vec![Effect::Run(keys![s(LEFT)])]);
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 60)).1, vec![Effect::SlowMouse(false)]);
        assert_eq!(run(&mut layer, ev(LMENU, false, 70)), (true, vec![]));
        run(&mut layer, ev(l(b'h'), false, 80));
        assert!(!layer.engaged());
    }

    #[test]
    fn clicks_and_jobs_ignore_repeats() {
        let mut layer = Layer::default();
        run(&mut layer, ev(CAPITAL, true, 0));
        assert_eq!(run(&mut layer, ev(u16::from(b'1'), true, 10)).1, vec![Effect::Button(Button::Left, true)]);
        assert_eq!(run(&mut layer, ev(u16::from(b'1'), true, 40)).1, vec![]);
        assert_eq!(run(&mut layer, ev(u16::from(b'1'), false, 50)).1, vec![Effect::Button(Button::Left, false)]);
        assert_eq!(run(&mut layer, ev(l(b'y'), true, 60)).1, vec![Effect::Run(Action::Job(Job::CopyOrLine))]);
        assert_eq!(run(&mut layer, ev(l(b'y'), true, 90)).1, vec![]);
    }

    #[test]
    fn desktop_toggles_direction() {
        let mut layer = Layer::default();
        run(&mut layer, ev(CAPITAL, true, 0));
        run(&mut layer, ev(LMENU, true, 1));
        let first = run(&mut layer, ev(l(b'v'), true, 10)).1;
        run(&mut layer, ev(l(b'v'), false, 20));
        let second = run(&mut layer, ev(l(b'v'), true, 30)).1;
        assert_eq!(first, vec![Effect::Run(keys![Chord { mods: WIN | CTRL, vk: RIGHT }])]);
        assert_eq!(second, vec![Effect::Run(keys![Chord { mods: WIN | CTRL, vk: LEFT }])]);
    }

    #[test]
    fn transparency_session() {
        let mut layer = Layer::default();
        run(&mut layer, ev(CAPITAL, true, 0));
        assert_eq!(run(&mut layer, ev(F4, true, 10)).1, vec![Effect::AlphaBegin]);
        assert_eq!(run(&mut layer, ev(F4, false, 100)).1, vec![Effect::AlphaEnd { tap: true }]);
        run(&mut layer, ev(F4, true, 200));
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 900)).1, vec![Effect::AlphaEnd { tap: false }]);
        assert_eq!(run(&mut layer, ev(F4, false, 950)), (true, vec![]));
    }

    #[test]
    fn targets() {
        assert_eq!(lookup(l(b'w'), false, || Target::Terminal), keys![c(l(b'w'))]);
        assert_eq!(lookup(l(b'd'), false, || Target::Vim), keys![c(l(b'u'))]);
        assert_eq!(lookup(l(b'd'), false, || Target::Other), Action::Job(Job::DeleteLine));
        assert_eq!(lookup(l(b'u'), false, || Target::Terminal), keys![c(OEM_2)]);
        assert_eq!(lookup(l(b'u'), false, || Target::Vim), keys![c(l(b'z'))]);
        assert_eq!(lookup(l(b'm'), false, || Target::Vim), keys![k(F3)]);
        assert_eq!(lookup(l(b'h'), false, || unreachable!()), keys![k(LEFT)]);
    }

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

    #[test]
    fn layer_keys() {
        for key in [u16::from(b'A'), u16::from(b'0'), F1, F12, SPACE, OEM_7, ESCAPE] {
            assert!(in_layer(key), "{key:#x}");
        }
        for key in [INSERT, LEFT, 0x60, F12 + 1, CAPITAL] {
            assert!(!in_layer(key), "{key:#x}");
        }
    }
}
