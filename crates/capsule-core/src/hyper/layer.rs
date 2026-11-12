use super::{ALT, Action, Button, CTRL, Chord, Job, SHIFT, Target, WIN, in_layer, k, vk};
use crate::config::Hyper;

const TAP_MS: u32 = 300;
const HOLD_MS: u32 = 400;

fn is_modifier(key: u16) -> bool {
    use vk::*;
    matches!(key, SHIFT | CONTROL | LSHIFT | RSHIFT | LCONTROL | RCONTROL | LWIN | RWIN)
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
pub enum Effect<'a> {
    Keys(&'a [Chord]),
    WinKeys(&'a [Chord]),
    Wheel(i32),
    Job(&'a Job),
    Reload,
    Button(Button, bool),
    SlowMouse(bool),
    AlphaBegin,
    AlphaEnd { tap: bool },
    SwitchDesktop,
    MaskWin,
    VirgoToggle,
    VirgoMove,
    Crop,
}

const ESC: Effect = Effect::Keys(&[k(vk::ESCAPE)]);
const BUTTONS: [Button; 3] = [Button::Left, Button::Middle, Button::Right];

#[derive(Debug)]
pub struct Layer {
    caps_at: Option<u32>,
    /// CapsLock went down with Shift held and goes to the system until released.
    caps_through: bool,
    used: bool,
    passthrough: bool,
    alt: bool,
    swallowed: [u64; 4],
    /// The key holding each of `BUTTONS` down, or 0.
    buttons: [u16; 3],
    /// The key that began the transparency session, and when.
    alpha: Option<(u16, u32)>,
    virgo: Option<Virgo>,
}

#[derive(Debug, Clone, Copy)]
struct Virgo {
    key: u16,
    at: u32,
    moved: bool,
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
            caps_through: false,
            used: false,
            passthrough: false,
            alt: false,
            swallowed: [0; 4],
            buttons: [0; 3],
            alpha: None,
            virgo: None,
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
    pub fn on_key<'a>(
        &mut self,
        ev: Event,
        cfg: &'a Hyper,
        mods: impl FnOnce() -> u8,
        target: impl FnOnce() -> Target,
        emit: &mut impl FnMut(Effect<'a>),
    ) -> bool {
        let Event { vk: key, down, time } = ev;
        if !down {
            return self.on_release(key, time, emit);
        }
        if key == vk::CAPITAL {
            return self.on_caps_down(time, mods);
        }
        if self.caps_at.is_none() {
            return self.on_win_key(key, time, cfg, mods, target, emit);
        }
        self.used = true;
        match key {
            _ if is_modifier(key) => false,
            vk::LMENU => self.on_alt_down(emit),
            vk::RMENU => {
                self.mark(key);
                true
            }
            _ => self.on_layer_key(key, time, cfg, mods, target, emit),
        }
    }

    fn on_release<'a>(&mut self, key: u16, time: u32, emit: &mut impl FnMut(Effect<'a>)) -> bool {
        if key == vk::CAPITAL {
            self.caps_through = false;
        }
        if !self.take(key) {
            return false;
        }
        match key {
            vk::CAPITAL => self.release_caps(time, emit),
            vk::LMENU => self.end_alt(emit),
            _ if self.alpha.is_some_and(|(k, _)| k == key) => self.end_alpha(time, emit),
            _ if self.virgo.is_some_and(|v| v.key == key) => self.end_virgo(emit),
            _ => {}
        }
        for (b, held) in BUTTONS.into_iter().zip(&mut self.buttons) {
            if *held == key {
                *held = 0;
                emit(Effect::Button(b, false));
            }
        }
        true
    }

    fn on_caps_down(&mut self, time: u32, mods: impl FnOnce() -> u8) -> bool {
        if self.caps_through {
            return false;
        }
        if !self.mark(vk::CAPITAL) && self.caps_at.is_none() {
            let mods = mods();
            if mods & SHIFT != 0 {
                self.take(vk::CAPITAL);
                self.caps_through = true;
                return false;
            }
            self.caps_at = Some(time);
            self.used = mods != 0;
            self.passthrough = mods & ALT != 0;
        }
        true
    }

    fn on_win_key<'a>(
        &mut self,
        key: u16,
        time: u32,
        cfg: &'a Hyper,
        mods: impl FnOnce() -> u8,
        target: impl FnOnce() -> Target,
        emit: &mut impl FnMut(Effect<'a>),
    ) -> bool {
        let Some(action) = cfg.win_keys.get(key, target) else { return false };
        if mods() != WIN || !action.fits_win_key() {
            return false;
        }
        let repeat = self.mark(key);
        match action {
            Action::Keys(chords) => emit(Effect::WinKeys(chords)),
            Action::VirgoDesktop => self.press_virgo(key, time, emit),
            Action::Crop if !repeat => {
                emit(Effect::MaskWin);
                emit(Effect::Crop);
            }
            _ => {}
        }
        true
    }

    fn press_virgo<'a>(&mut self, key: u16, time: u32, emit: &mut impl FnMut(Effect<'a>)) {
        match &mut self.virgo {
            Some(v) if v.key == key && !v.moved && time.wrapping_sub(v.at) >= HOLD_MS => {
                v.moved = true;
                emit(Effect::VirgoMove);
            }
            Some(v) if v.key == key => {}
            _ => {
                self.virgo = Some(Virgo { key, at: time, moved: false });
                emit(Effect::MaskWin);
            }
        }
    }

    fn end_virgo<'a>(&mut self, emit: &mut impl FnMut(Effect<'a>)) {
        if self.virgo.take().is_some_and(|v| !v.moved) {
            emit(Effect::VirgoToggle);
        }
    }

    fn on_alt_down<'a>(&mut self, emit: &mut impl FnMut(Effect<'a>)) -> bool {
        if self.passthrough {
            return false;
        }
        if !self.mark(vk::LMENU) && !self.alt {
            self.alt = true;
            emit(Effect::SlowMouse(true));
        }
        true
    }

    fn on_layer_key<'a>(
        &mut self,
        key: u16,
        time: u32,
        cfg: &'a Hyper,
        mods: impl FnOnce() -> u8,
        target: impl FnOnce() -> Target,
        emit: &mut impl FnMut(Effect<'a>),
    ) -> bool {
        if self.passthrough || mods() & (CTRL | SHIFT | WIN) != 0 {
            return false;
        }
        let action = if self.alt { &cfg.alt_keys } else { &cfg.keys }.get(key, target);
        if action.is_none() && !in_layer(key) {
            return false;
        }
        let repeat = self.mark(key);
        match action {
            None => {}
            Some(Action::Keys(keys)) => emit(Effect::Keys(keys)),
            Some(&Action::Wheel(n)) => emit(Effect::Wheel(n)),
            Some(Action::VirgoDesktop) => self.press_virgo(key, time, emit),
            _ if repeat => {}
            Some(&Action::Click(b)) => {
                let held = &mut self.buttons[b as usize];
                if *held == 0 {
                    *held = key;
                    emit(Effect::Button(b, true));
                }
            }
            Some(Action::Job(job)) => emit(Effect::Job(job)),
            Some(Action::Reload) => emit(Effect::Reload),
            Some(Action::Transparency) => {
                if self.alpha.is_none() {
                    self.alpha = Some((key, time));
                    emit(Effect::AlphaBegin);
                }
            }
            Some(Action::SwitchDesktop) => emit(Effect::SwitchDesktop),
            Some(Action::Crop) => emit(Effect::Crop),
        }
        true
    }

    fn release_caps<'a>(&mut self, time: u32, emit: &mut impl FnMut(Effect<'a>)) {
        let Some(at) = self.caps_at.take() else { return };
        self.end_alt(emit);
        self.end_alpha(time, emit);
        if !self.used && time.wrapping_sub(at) < TAP_MS {
            emit(ESC);
        }
        self.passthrough = false;
    }

    fn end_alt<'a>(&mut self, emit: &mut impl FnMut(Effect<'a>)) {
        if self.alt {
            self.alt = false;
            emit(Effect::SlowMouse(false));
        }
    }

    fn end_alpha<'a>(&mut self, time: u32, emit: &mut impl FnMut(Effect<'a>)) {
        if let Some((_, at)) = self.alpha.take() {
            emit(Effect::AlphaEnd { tap: time.wrapping_sub(at) < TAP_MS });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Binding, Keymap, action, cs, s};
    use super::vk::*;
    use super::*;
    use std::sync::LazyLock;

    static CFG: LazyLock<Hyper> = LazyLock::new(Hyper::default);

    fn ev(vk: u16, down: bool, time: u32) -> Event {
        Event { vk, down, time }
    }

    fn engaged(layer: &Layer) -> bool {
        layer.caps_at.is_some() || layer.swallowed != [0; 4]
    }

    fn run_with(layer: &mut Layer, e: Event, mods: u8) -> (bool, Vec<Effect<'static>>) {
        let mut out = Vec::new();
        let swallow = layer.on_key(e, &CFG, || mods, || Target::Other, &mut |x| out.push(x));
        (swallow, out)
    }

    fn run(layer: &mut Layer, e: Event) -> (bool, Vec<Effect<'static>>) {
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
    fn shift_caps_goes_to_the_system() {
        let mut layer = Layer::new();
        assert_eq!(run_with(&mut layer, ev(CAPITAL, true, 0), super::SHIFT), (false, vec![]));
        assert_eq!(run(&mut layer, ev(CAPITAL, true, 30)), (false, vec![]));
        assert!(!run(&mut layer, ev(H, true, 40)).0);
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 50)), (false, vec![]));
        assert!(!engaged(&layer));
        run(&mut layer, ev(CAPITAL, true, 1000));
        assert_eq!(run(&mut layer, ev(CAPITAL, false, 1010)), (true, vec![ESC]));
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
        assert_eq!(run(&mut layer, ev(Y, true, 60)).1, vec![Effect::Job(&Job::CopyOrLine)]);
        assert_eq!(run(&mut layer, ev(Y, true, 90)).1, vec![]);
        assert_eq!(run(&mut layer, ev(F5, true, 100)).1, vec![Effect::Reload]);
    }

    #[test]
    fn switch_desktop_on_each_press() {
        let mut layer = Layer::new();
        run(&mut layer, ev(CAPITAL, true, 0));
        run(&mut layer, ev(LMENU, true, 1));
        assert_eq!(run(&mut layer, ev(V, true, 10)).1, vec![Effect::SwitchDesktop]);
        assert_eq!(run(&mut layer, ev(V, true, 40)).1, vec![]);
        run(&mut layer, ev(V, false, 50));
        assert_eq!(run(&mut layer, ev(V, true, 60)).1, vec![Effect::SwitchDesktop]);
    }

    #[test]
    fn virgo_tap_toggles_and_hold_moves() {
        let mut layer = Layer::new();
        assert_eq!(run_with(&mut layer, ev(BACKTICK, true, 0), WIN), (true, vec![Effect::MaskWin]));
        assert_eq!(run_with(&mut layer, ev(BACKTICK, false, 100), WIN), (true, vec![Effect::VirgoToggle]));
        assert!(!engaged(&layer));

        assert_eq!(run_with(&mut layer, ev(BACKTICK, true, 1000), WIN).1, vec![Effect::MaskWin]);
        assert_eq!(run_with(&mut layer, ev(BACKTICK, true, 1300), WIN), (true, vec![]));
        assert_eq!(run_with(&mut layer, ev(BACKTICK, true, 1420), WIN).1, vec![Effect::VirgoMove]);
        assert_eq!(run_with(&mut layer, ev(BACKTICK, true, 1450), WIN).1, vec![]);
        assert_eq!(run_with(&mut layer, ev(BACKTICK, false, 1500), WIN), (true, vec![]));
        assert!(!engaged(&layer));

        assert!(!run_with(&mut layer, ev(BACKTICK, true, 2000), WIN | super::SHIFT).0);
        assert!(!run(&mut layer, ev(BACKTICK, true, 2100)).0);
    }

    #[test]
    fn crop_once_per_press() {
        let mut layer = Layer::new();
        run(&mut layer, ev(CAPITAL, true, 0));
        assert_eq!(run(&mut layer, ev(S, true, 10)).1, vec![Effect::Crop]);
        assert_eq!(run(&mut layer, ev(S, true, 40)), (true, vec![]));
        run(&mut layer, ev(S, false, 50));
        run(&mut layer, ev(CAPITAL, false, 60));

        let win_keys: Keymap = [(C, Binding { action: Action::Crop, terminal: None })].into_iter().collect();
        let cfg = Hyper { win_keys, ..Hyper::default() };
        let mut out = Vec::new();
        for (down, time) in [(true, 100), (true, 130), (false, 140)] {
            assert!(layer.on_key(ev(C, down, time), &cfg, || WIN, || Target::Other, &mut |x| out.push(x)));
        }
        assert_eq!(out, [Effect::MaskWin, Effect::Crop]);
        assert!(!engaged(&layer));
    }

    #[test]
    fn win_keys_need_win_alone() {
        let mut layer = Layer::new();
        assert_eq!(
            run_with(&mut layer, ev(E, true, 0), WIN),
            (true, vec![Effect::WinKeys(&[Chord { mods: WIN, vk: D2 }])])
        );
        assert_eq!(run_with(&mut layer, ev(E, false, 10), WIN), (true, vec![]));
        assert_eq!(run_with(&mut layer, ev(T, true, 20), WIN).1, vec![Effect::WinKeys(&[cs(T)])]);
        assert!(run(&mut layer, ev(T, false, 30)).0);
        assert!(!run_with(&mut layer, ev(E, true, 40), WIN | super::SHIFT).0);
        assert!(!run(&mut layer, ev(E, true, 50)).0);
        assert!(!run_with(&mut layer, ev(R, true, 60), WIN).0);
        assert!(!engaged(&layer));
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
    fn remapped_click_and_transparency_follow_their_keys() {
        let keys: Keymap = [(A, "right_button"), (S, "transparency")]
            .into_iter()
            .map(|(key, a)| (key, Binding { action: action(a).unwrap(), terminal: None }))
            .collect();
        let cfg = Hyper { keys: keys.clone(), alt_keys: keys, ..Hyper::default() };
        let mut layer = Layer::new();
        let mut out = Vec::new();
        let mut press = |layer: &mut Layer, vk, down, time| {
            layer.on_key(ev(vk, down, time), &cfg, || 0, || Target::Other, &mut |x| out.push(x))
        };
        press(&mut layer, CAPITAL, true, 0);
        press(&mut layer, A, true, 10);
        press(&mut layer, S, true, 20);
        press(&mut layer, F4, false, 30);
        press(&mut layer, A, false, 40);
        press(&mut layer, S, false, 50);
        assert!(!press(&mut layer, INSERT, true, 60));
        assert_eq!(
            out,
            [
                Effect::Button(Button::Right, true),
                Effect::AlphaBegin,
                Effect::Button(Button::Right, false),
                Effect::AlphaEnd { tap: true },
            ]
        );
    }
}
