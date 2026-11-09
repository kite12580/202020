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
    pub const D0: u16 = 0x30;
    pub const D1: u16 = 0x31;
    pub const D2: u16 = 0x32;
    pub const D3: u16 = 0x33;
    pub const D9: u16 = 0x39;
    pub const A: u16 = 0x41;
    pub const B: u16 = 0x42;
    pub const C: u16 = 0x43;
    pub const D: u16 = 0x44;
    pub const E: u16 = 0x45;
    pub const F: u16 = 0x46;
    pub const G: u16 = 0x47;
    pub const H: u16 = 0x48;
    pub const J: u16 = 0x4A;
    pub const K: u16 = 0x4B;
    pub const L: u16 = 0x4C;
    pub const M: u16 = 0x4D;
    pub const N: u16 = 0x4E;
    pub const O: u16 = 0x4F;
    pub const P: u16 = 0x50;
    pub const Q: u16 = 0x51;
    pub const R: u16 = 0x52;
    pub const S: u16 = 0x53;
    pub const T: u16 = 0x54;
    pub const U: u16 = 0x55;
    pub const V: u16 = 0x56;
    pub const W: u16 = 0x57;
    pub const X: u16 = 0x58;
    pub const Y: u16 = 0x59;
    pub const Z: u16 = 0x5A;
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
    pub const SEMICOLON: u16 = 0xBA;
    pub const EQUALS: u16 = 0xBB;
    pub const COMMA: u16 = 0xBC;
    pub const MINUS: u16 = 0xBD;
    pub const PERIOD: u16 = 0xBE;
    pub const SLASH: u16 = 0xBF;
    pub const BACKTICK: u16 = 0xC0;
    pub const LBRACKET: u16 = 0xDB;
    pub const BACKSLASH: u16 = 0xDC;
    pub const RBRACKET: u16 = 0xDD;
    pub const QUOTE: u16 = 0xDE;
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

pub const fn k(vk: u16) -> Chord {
    Chord { mods: 0, vk }
}

pub const fn c(vk: u16) -> Chord {
    Chord { mods: CTRL, vk }
}

pub const fn s(vk: u16) -> Chord {
    Chord { mods: SHIFT, vk }
}

pub const fn cs(vk: u16) -> Chord {
    Chord { mods: CTRL | SHIFT, vk }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Other,
    Terminal,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Keys(&'static [Chord]),
    Click(Button),
    /// Wheel notches, positive is up.
    Wheel(i32),
    Job(Job),
    Reload,
    Transparency,
    SwitchDesktop,
    Noop,
}

macro_rules! keys {
    ($($c:expr),* $(,)?) => {
        Action::Keys({
            const CHORDS: &[Chord] = &[$($c),*];
            CHORDS
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
            H => keys![s(LEFT)],
            J => keys![s(DOWN)],
            K => keys![s(UP)],
            L => keys![s(RIGHT)],
            E => keys![cs(RIGHT)],
            B => keys![cs(LEFT)],
            SEMICOLON => keys![s(HOME)],
            QUOTE => keys![s(END)],
            G => keys![c(HOME)],
            Q => Action::Wheel(-2),
            R => Action::Wheel(2),
            O => keys![k(HOME), k(RETURN), k(UP)],
            U => keys![c(Y)],
            Z => keys![cs(S)],
            V => Action::SwitchDesktop,
            W => keys![k(END), s(DOWN), s(HOME), k(BACK)],
            D => keys![k(END), s(HOME), k(DELETE)],
            X => keys![c(INSERT), k(BACK)],
            Y => Action::Job(Job::CopyOrLine),
            P => Action::Job(Job::PastePlain),
            D9 => wrap("\"", "\""),
            D0 => wrap("'", "'"),
            LBRACKET => wrap("[", "]"),
            F => Action::Job(Job::EscapeOrMove),
            T => Action::Job(Job::Terminal),
            F1 => Action::Job(Job::Caption),
            F2 => keys![k(MEDIA_PLAY_PAUSE)],
            F3 => keys![k(MEDIA_PREV_TRACK)],
            _ => Action::Noop,
        };
    }
    if let Some(b) = button(key) {
        return Action::Click(b);
    }
    match key {
        H => keys![k(LEFT)],
        J => keys![k(DOWN)],
        K => keys![k(UP)],
        L => keys![k(RIGHT)],
        E => keys![c(RIGHT)],
        B => keys![c(LEFT)],
        SEMICOLON => keys![k(HOME)],
        QUOTE | RETURN => keys![k(END)],
        SPACE => keys![k(RIGHT)],
        G => keys![c(END)],
        Q => keys![k(NEXT)],
        R => keys![k(PRIOR)],
        N => match target() {
            Target::Vim => keys![k(F2)],
            _ => keys![cs(TAB)],
        },
        M => match target() {
            Target::Vim => keys![k(F3)],
            _ => keys![c(TAB)],
        },
        O => keys![k(END), k(RETURN)],
        U => match target() {
            Target::Terminal => keys![c(SLASH)],
            _ => keys![c(Z)],
        },
        Z => keys![c(S)],
        C => keys![Chord { mods: ALT, vk: TAB }],
        V => keys![cs(V)],
        SLASH => keys![c(F)],
        COMMA => keys![k(BROWSER_BACK)],
        PERIOD => keys![k(BROWSER_FORWARD)],
        BACKTICK => keys![k(APPS)],
        W => match target() {
            Target::Other => keys![cs(LEFT), k(BACK)],
            _ => keys![c(W)],
        },
        BACK => keys![c(LEFT), cs(RIGHT), k(BACK)],
        D => match target() {
            Target::Other => Action::Job(Job::DeleteLine),
            _ => keys![c(U)],
        },
        X => keys![c(INSERT), k(DELETE)],
        Y => Action::Job(Job::CopyOrLine),
        P => keys![s(INSERT)],
        D9 => wrap("(", ")"),
        D0 => Action::Job(Job::Paste(")")),
        LBRACKET => Action::Job(Job::Wrap { open: " {", close: "}", at_end: true }),
        EQUALS => wrap(" + \"", "\" + "),
        TAB => Action::Job(Job::Hotstring),
        F1 => Action::Job(Job::Topmost),
        F3 => keys![k(MEDIA_NEXT_TRACK)],
        F4 => Action::Transparency,
        F5 => Action::Reload,
        F9 => Action::Job(Job::PowerOptions),
        _ => Action::Noop,
    }
}

/// Keys the layer takes over while CapsLock is held, mapped or not.
pub fn in_layer(key: u16) -> bool {
    use vk::*;
    matches!(
        key,
        D0..=D9
            | A..=Z
            | F1..=F12
            | SPACE
            | TAB
            | RETURN
            | ESCAPE
            | BACK
            | SEMICOLON
            | EQUALS
            | COMMA
            | MINUS
            | PERIOD
            | SLASH
            | BACKTICK
            | LBRACKET
            | BACKSLASH
            | RBRACKET
            | QUOTE
    )
}

fn is_modifier(key: u16) -> bool {
    use vk::*;
    matches!(key, SHIFT | CONTROL | LSHIFT | RSHIFT | LCONTROL | RCONTROL | LWIN | RWIN)
}

fn button(key: u16) -> Option<Button> {
    match key {
        vk::D1 => Some(Button::Left),
        vk::D2 => Some(Button::Middle),
        vk::D3 => Some(Button::Right),
        _ => None,
    }
}

fn button_bit(b: Button) -> u8 {
    1 << b as u8
}

fn slot(key: u16) -> (usize, u64) {
    (usize::from(key >> 6) & 3, 1u64 << (key & 63))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub vk: u16,
    pub down: bool,
    /// Milliseconds, as in `KBDLLHOOKSTRUCT::time`.
    pub time: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Keys(&'static [Chord]),
    Wheel(i32),
    Job(Job),
    Reload,
    Button(Button, bool),
    SlowMouse(bool),
    AlphaBegin,
    AlphaEnd { tap: bool },
}

const ESC: Effect = Effect::Keys(&[k(vk::ESCAPE)]);

#[derive(Debug)]
pub struct Layer {
    caps_at: Option<u32>,
    used: bool,
    passthrough: bool,
    alt: bool,
    swallowed: [u64; 4],
    buttons: u8,
    alpha_at: Option<u32>,
    desktop_right: bool,
}

impl Default for Layer {
    fn default() -> Self {
        Self::new()
    }
}

impl Layer {
    pub const fn new() -> Self {
        Layer {
            caps_at: None,
            used: false,
            passthrough: false,
            alt: false,
            swallowed: [0; 4],
            buttons: 0,
            alpha_at: None,
            desktop_right: false,
        }
    }

    /// Marks `key` as swallowed; returns whether it already was (auto-repeat).
    fn mark(&mut self, key: u16) -> bool {
        let (i, bit) = slot(key);
        let was = self.swallowed[i] & bit != 0;
        self.swallowed[i] |= bit;
        was
    }

    fn take(&mut self, key: u16) -> bool {
        let (i, bit) = slot(key);
        let was = self.swallowed[i] & bit != 0;
        self.swallowed[i] &= !bit;
        was
    }

    /// Returns whether to swallow the event. `mods` reads `CTRL`, `SHIFT`,
    /// `WIN` and `ALT` (left Alt) as the system sees them.
    pub fn on_key(
        &mut self,
        ev: Event,
        mods: impl FnOnce() -> u8,
        target: impl FnOnce() -> Target,
        emit: &mut impl FnMut(Effect),
    ) -> bool {
        let Event { vk: key, down, time } = ev;
        if !down {
            if !self.take(key) {
                return false;
            }
            match key {
                vk::CAPITAL => self.release_caps(time, emit),
                vk::LMENU => self.end_alt(emit),
                vk::F4 => self.end_alpha(time, emit),
                _ => {}
            }
            if let Some(b) = button(key)
                && self.buttons & button_bit(b) != 0
            {
                self.buttons &= !button_bit(b);
                emit(Effect::Button(b, false));
            }
            return true;
        }
        if key == vk::CAPITAL {
            if !self.mark(key) && self.caps_at.is_none() {
                let mods = mods();
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
        if self.passthrough || !in_layer(key) || mods() & (CTRL | SHIFT | WIN) != 0 {
            return false;
        }
        let repeat = self.mark(key);
        match lookup(key, self.alt, target) {
            Action::Keys(keys) => emit(Effect::Keys(keys)),
            Action::Wheel(n) => emit(Effect::Wheel(n)),
            _ if repeat => {}
            Action::Click(b) => {
                self.buttons |= button_bit(b);
                emit(Effect::Button(b, true));
            }
            Action::Job(job) => emit(Effect::Job(job)),
            Action::Reload => emit(Effect::Reload),
            Action::Transparency => {
                self.alpha_at = Some(time);
                emit(Effect::AlphaBegin);
            }
            Action::SwitchDesktop => {
                self.desktop_right = !self.desktop_right;
                emit(Effect::Keys(if self.desktop_right {
                    &[Chord { mods: WIN | CTRL, vk: vk::RIGHT }]
                } else {
                    &[Chord { mods: WIN | CTRL, vk: vk::LEFT }]
                }));
            }
            Action::Noop => {}
        }
        true
    }

    fn release_caps(&mut self, time: u32, emit: &mut impl FnMut(Effect)) {
        let Some(at) = self.caps_at.take() else { return };
        self.end_alt(emit);
        self.end_alpha(time, emit);
        if !self.used && time.wrapping_sub(at) < TAP_MS {
            emit(ESC);
        }
        self.passthrough = false;
    }

    fn end_alt(&mut self, emit: &mut impl FnMut(Effect)) {
        if self.alt {
            self.alt = false;
            emit(Effect::SlowMouse(false));
        }
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
        Event { vk, down, time }
    }

    fn engaged(layer: &Layer) -> bool {
        layer.caps_at.is_some() || layer.swallowed != [0; 4]
    }

    fn run_with(layer: &mut Layer, e: Event, mods: u8) -> (bool, Vec<Effect>) {
        let mut out = Vec::new();
        let swallow = layer.on_key(e, || mods, || Target::Other, &mut |x| out.push(x));
        (swallow, out)
    }

    fn run(layer: &mut Layer, e: Event) -> (bool, Vec<Effect>) {
        run_with(layer, e, 0)
    }

    #[test]
    fn tap_sends_esc_only_when_quick_and_alone() {
        let mut layer = Layer::new();
        assert_eq!(run(&mut layer, ev(CAPITAL, true, 1000)), (true, vec![]));
        assert_eq!(run(&mut layer, ev(CAPITAL, true, 1030)), (true, vec![]));
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 1100)), (true, vec![ESC]));
        assert!(!engaged(&layer));

        run(&mut layer, ev(CAPITAL, true, 2000));
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 2400)), (true, vec![]));

        run(&mut layer, ev(CAPITAL, true, 3000));
        assert!(!run(&mut layer, ev(LSHIFT, true, 3010)).0);
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 3020)), (true, vec![]));

        run(&mut layer, ev(CAPITAL, true, u32::MAX - 10));
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 20)).1, vec![ESC]);
    }

    #[test]
    fn layer_maps_and_swallows_the_release() {
        let mut layer = Layer::new();
        run(&mut layer, ev(CAPITAL, true, 0));
        let (swallow, fx) = run(&mut layer, ev(H, true, 10));
        assert!(swallow);
        assert_eq!(fx, vec![Effect::Keys(const { &[k(LEFT)] })]);
        assert_eq!(run(&mut layer, ev(H, true, 40)).1.len(), 1);
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 50)), (true, vec![]));
        assert!(engaged(&layer));
        assert_eq!(run(&mut layer, ev(H, false, 60)), (true, vec![]));
        assert!(!engaged(&layer));
        assert!(!run(&mut layer, ev(H, true, 70)).0);
    }

    #[test]
    fn unmapped_and_outside_keys() {
        let mut layer = Layer::new();
        run(&mut layer, ev(CAPITAL, true, 0));
        assert_eq!(run(&mut layer, ev(A, true, 10)), (true, vec![]));
        assert_eq!(run(&mut layer, ev(INSERT, true, 10)), (false, vec![]));
        assert!(run(&mut layer, ev(RMENU, true, 10)).0);
    }

    #[test]
    fn held_modifiers_pass_keys_through() {
        let mut layer = Layer::new();
        run(&mut layer, ev(CAPITAL, true, 0));
        for mods in [CTRL, super::SHIFT, WIN] {
            assert!(!run_with(&mut layer, ev(H, true, 10), mods).0);
        }
        let mut layer = Layer::new();
        run_with(&mut layer, ev(CAPITAL, true, 0), CTRL);
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 10)), (true, vec![]));
        run_with(&mut layer, ev(CAPITAL, true, 0), ALT);
        assert!(!run(&mut layer, ev(LMENU, true, 5)).0);
        assert!(!run_with(&mut layer, ev(H, true, 10), ALT).0);
    }

    #[test]
    fn alt_layer_and_slow_mouse() {
        let mut layer = Layer::new();
        run(&mut layer, ev(CAPITAL, true, 0));
        assert_eq!(run(&mut layer, ev(LMENU, true, 10)), (true, vec![Effect::SlowMouse(true)]));
        assert_eq!(run(&mut layer, ev(LMENU, true, 40)), (true, vec![]));
        assert_eq!(run(&mut layer, ev(H, true, 50)).1, vec![Effect::Keys(const { &[s(LEFT)] })]);
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 60)).1, vec![Effect::SlowMouse(false)]);
        assert_eq!(run(&mut layer, ev(LMENU, false, 70)), (true, vec![]));
        run(&mut layer, ev(H, false, 80));
        assert!(!engaged(&layer));
    }

    #[test]
    fn clicks_and_jobs_ignore_repeats() {
        let mut layer = Layer::new();
        run(&mut layer, ev(CAPITAL, true, 0));
        assert_eq!(run(&mut layer, ev(D1, true, 10)).1, vec![Effect::Button(Button::Left, true)]);
        assert_eq!(run(&mut layer, ev(D1, true, 40)).1, vec![]);
        assert_eq!(run(&mut layer, ev(D1, false, 50)).1, vec![Effect::Button(Button::Left, false)]);
        assert_eq!(run(&mut layer, ev(Y, true, 60)).1, vec![Effect::Job(Job::CopyOrLine)]);
        assert_eq!(run(&mut layer, ev(Y, true, 90)).1, vec![]);
        assert_eq!(run(&mut layer, ev(F5, true, 100)).1, vec![Effect::Reload]);
    }

    #[test]
    fn desktop_toggles_direction() {
        let mut layer = Layer::new();
        run(&mut layer, ev(CAPITAL, true, 0));
        run(&mut layer, ev(LMENU, true, 1));
        let first = run(&mut layer, ev(V, true, 10)).1;
        run(&mut layer, ev(V, false, 20));
        let second = run(&mut layer, ev(V, true, 30)).1;
        assert_eq!(first, vec![Effect::Keys(&[Chord { mods: WIN | CTRL, vk: RIGHT }])]);
        assert_eq!(second, vec![Effect::Keys(&[Chord { mods: WIN | CTRL, vk: LEFT }])]);
    }

    #[test]
    fn transparency_session() {
        let mut layer = Layer::new();
        run(&mut layer, ev(CAPITAL, true, 0));
        assert_eq!(run(&mut layer, ev(F4, true, 10)).1, vec![Effect::AlphaBegin]);
        assert_eq!(run(&mut layer, ev(F4, false, 100)).1, vec![Effect::AlphaEnd { tap: true }]);
        run(&mut layer, ev(F4, true, 200));
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 900)).1, vec![Effect::AlphaEnd { tap: false }]);
        assert_eq!(run(&mut layer, ev(F4, false, 950)), (true, vec![]));
    }

    #[test]
    fn targets() {
        assert_eq!(lookup(W, false, || Target::Terminal), keys![c(W)]);
        assert_eq!(lookup(D, false, || Target::Vim), keys![c(U)]);
        assert_eq!(lookup(D, false, || Target::Other), Action::Job(Job::DeleteLine));
        assert_eq!(lookup(U, false, || Target::Terminal), keys![c(SLASH)]);
        assert_eq!(lookup(U, false, || Target::Vim), keys![c(Z)]);
        assert_eq!(lookup(M, false, || Target::Vim), keys![k(F3)]);
        assert_eq!(lookup(H, false, || unreachable!()), keys![k(LEFT)]);
        assert_eq!(lookup(D1, true, || unreachable!()), Action::Noop);
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
        for key in [A, D0, F1, F12, SPACE, QUOTE, ESCAPE] {
            assert!(in_layer(key), "{key:#x}");
        }
        for key in [INSERT, LEFT, 0x60, F12 + 1, CAPITAL] {
            assert!(!in_layer(key), "{key:#x}");
        }
    }
}
