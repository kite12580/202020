use crate::launcher::hotkey::{self, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};

mod jobs;
mod layer;
pub mod vk;

pub use jobs::{alpha_step, alpha_toggle, copy_accept, hotstring, line_start_probe, window_preset, wrap_text};
pub use layer::{Effect, Event, Layer};

pub const CTRL: u8 = MOD_CONTROL as u8;
pub const SHIFT: u8 = MOD_SHIFT as u8;
pub const ALT: u8 = MOD_ALT as u8;
pub const WIN: u8 = MOD_WIN as u8;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    CopyOrLine,
    PastePlain,
    /// Surround the selection with `open` and `close`, or insert both with
    /// the caret between them. `at_end` first moves to the end of the line,
    /// and presses Enter afterwards.
    Wrap {
        open: String,
        close: String,
        at_end: bool,
    },
    /// Insert text through the clipboard, so the IME and auto-closing
    /// brackets leave it alone.
    Paste(String),
    DeleteLine,
    Hotstring,
    /// Esc, or with the mouse at the screen's left edge, place the window.
    EscapeOrMove,
    PowerOptions,
    Topmost,
    Caption,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Keys(Box<[Chord]>),
    Click(Button),
    /// Wheel notches, positive is up.
    Wheel(i32),
    Job(Job),
    Reload,
    Transparency,
    SwitchDesktop,
    VirgoDesktop,
}

const NAMED: &[(&str, Action)] = &[
    ("copy_or_line", Action::Job(Job::CopyOrLine)),
    ("paste_plain", Action::Job(Job::PastePlain)),
    ("delete_line", Action::Job(Job::DeleteLine)),
    ("hotstring", Action::Job(Job::Hotstring)),
    ("escape_or_move", Action::Job(Job::EscapeOrMove)),
    ("power_options", Action::Job(Job::PowerOptions)),
    ("topmost", Action::Job(Job::Topmost)),
    ("caption", Action::Job(Job::Caption)),
    ("reload", Action::Reload),
    ("transparency", Action::Transparency),
    ("switch_desktop", Action::SwitchDesktop),
    ("virgo_desktop", Action::VirgoDesktop),
    ("left_button", Action::Click(Button::Left)),
    ("middle_button", Action::Click(Button::Middle)),
    ("right_button", Action::Click(Button::Right)),
];

/// A named action, or chords separated by spaces such as `End Shift+Home`.
pub fn action(s: &str) -> Option<Action> {
    if let Some((_, a)) = NAMED.iter().find(|(name, _)| *name == s) {
        return Some(a.clone());
    }
    let chords = s
        .split_whitespace()
        .map(|s| hotkey::parse(s).map(|(mods, vk)| Chord { mods: mods as u8, vk: vk as u16 }))
        .collect::<Option<Box<[Chord]>>>()?;
    (!chords.is_empty()).then_some(Action::Keys(chords))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub action: Action,
    pub terminal: Option<Action>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Keymap(Box<[(u16, Binding)]>);

impl FromIterator<(u16, Binding)> for Keymap {
    fn from_iter<I: IntoIterator<Item = (u16, Binding)>>(iter: I) -> Self {
        let mut bindings: Vec<_> = iter.into_iter().collect();
        bindings.sort_by_key(|&(key, _)| key);
        bindings.dedup_by_key(|&mut (key, _)| key);
        Keymap(bindings.into_boxed_slice())
    }
}

impl Keymap {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn binds(&self, action: &Action) -> bool {
        self.0.iter().any(|(_, b)| b.action == *action || b.terminal.as_ref() == Some(action))
    }

    pub fn get(&self, key: u16, target: impl FnOnce() -> Target) -> Option<&Action> {
        let i = self.0.binary_search_by_key(&key, |&(k, _)| k).ok()?;
        let b = &self.0[i].1;
        Some(match &b.terminal {
            Some(t) if target() == Target::Terminal => t,
            _ => &b.action,
        })
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

#[cfg(test)]
mod tests {
    use super::vk::*;
    use super::*;

    #[test]
    fn targets() {
        let keys = |s: &str| Some(action(s).unwrap());
        let cfg = crate::config::Hyper::default();
        let get = |key, t| cfg.keys.get(key, || t).cloned();
        assert_eq!(get(W, Target::Terminal), keys("Ctrl+W"));
        assert_eq!(get(D, Target::Terminal), keys("Ctrl+U"));
        assert_eq!(get(D, Target::Other), Some(Action::Job(Job::DeleteLine)));
        assert_eq!(get(U, Target::Terminal), keys("Ctrl+/"));
        assert_eq!(get(U, Target::Other), keys("Ctrl+Z"));
        assert_eq!(get(M, Target::Terminal), keys("Ctrl+Tab"));
        assert_eq!(cfg.keys.get(H, || unreachable!()), keys("Left").as_ref());
        assert_eq!(cfg.alt_keys.get(D1, || unreachable!()), None);
    }

    #[test]
    fn actions() {
        assert_eq!(action("End Shift+Home"), Some(Action::Keys(Box::new([k(END), s(HOME)]))));
        assert_eq!(action("Ctrl+Shift+Alt+Win+,"), Some(Action::Keys(Box::new([Chord { mods: 15, vk: COMMA }]))));
        assert_eq!(action("left_button"), Some(Action::Click(Button::Left)));
        for bad in ["", " ", "Left Bogus", "Ctrl+", "Left_Button"] {
            assert_eq!(action(bad), None, "{bad:?}");
        }
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
