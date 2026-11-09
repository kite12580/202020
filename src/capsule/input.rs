use std::ffi::c_void;

use capsule_core::hyper::{ALT, Button, CTRL, Chord, SHIFT, WIN, k, vk};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MOUSE_EVENT_FLAGS, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_WHEEL, MOUSEINPUT, MapVirtualKeyW, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LMENU, VK_LWIN, VK_RWIN,
    VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETMOUSESPEED, SPI_SETMOUSESPEED, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW, WHEEL_DELTA,
};

/// Tags our own `SendInput` events so the hook passes them through while
/// still handling keys injected by remappers such as AutoHotkey.
pub const OWN_INPUT: usize = 0x4342_4152;

const MASK: u16 = 0xE8;

const MODS: [(u8, u16); 4] = [(CTRL, vk::LCONTROL), (SHIFT, vk::LSHIFT), (ALT, vk::LMENU), (WIN, vk::LWIN)];

fn extended(key: u16) -> bool {
    matches!(key, 0x21..=0x28 | vk::INSERT | vk::DELETE | vk::LWIN | vk::RWIN | vk::APPS | vk::RCONTROL | vk::RMENU)
        || (vk::BROWSER_BACK..=0xB7).contains(&key)
}

fn key(key: u16, up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if extended(key) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    let scan = unsafe { MapVirtualKeyW(u32::from(key), MAPVK_VK_TO_VSC) } as u16;
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(key), wScan: scan, dwFlags: flags, dwExtraInfo: OWN_INPUT, time: 0 },
        },
    }
}

fn mouse(flags: MOUSE_EVENT_FLAGS, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { mouseData: data as u32, dwFlags: flags, dwExtraInfo: OWN_INPUT, ..Default::default() },
        },
    }
}

struct Batch {
    inputs: [INPUT; 24],
    len: usize,
}

impl Batch {
    fn new() -> Batch {
        Batch { inputs: [INPUT::default(); 24], len: 0 }
    }

    fn push(&mut self, input: INPUT) {
        if self.len == self.inputs.len() {
            self.flush();
        }
        self.inputs[self.len] = input;
        self.len += 1;
    }

    fn chord(&mut self, c: Chord) {
        for &(bit, m) in &MODS {
            if c.mods & bit != 0 {
                self.push(key(m, false));
            }
        }
        self.push(key(c.vk, false));
        self.push(key(c.vk, true));
        for &(bit, m) in MODS.iter().rev() {
            if c.mods & bit != 0 {
                self.push(key(m, true));
            }
        }
    }

    fn flush(&mut self) {
        if self.len > 0 {
            unsafe { SendInput(&self.inputs[..self.len], size_of::<INPUT>() as i32) };
            self.len = 0;
        }
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        self.flush();
    }
}

pub fn keys(chords: &[Chord]) {
    let mut batch = Batch::new();
    for &c in chords {
        batch.chord(c);
    }
}

pub fn repeat(vk: u16, n: usize) {
    let mut batch = Batch::new();
    for _ in 0..n {
        batch.chord(k(vk));
    }
}

pub fn wheel(notches: i32) {
    let delta = if notches > 0 { WHEEL_DELTA as i32 } else { -(WHEEL_DELTA as i32) };
    let mut batch = Batch::new();
    for _ in 0..notches.unsigned_abs() {
        batch.push(mouse(MOUSEEVENTF_WHEEL, delta));
    }
}

pub fn button(b: Button, down: bool) {
    let flags = match (b, down) {
        (Button::Left, true) => MOUSEEVENTF_LEFTDOWN,
        (Button::Left, false) => MOUSEEVENTF_LEFTUP,
        (Button::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
        (Button::Middle, false) => MOUSEEVENTF_MIDDLEUP,
        (Button::Right, true) => MOUSEEVENTF_RIGHTDOWN,
        (Button::Right, false) => MOUSEEVENTF_RIGHTUP,
    };
    unsafe { SendInput(&[mouse(flags, 0)], size_of::<INPUT>() as i32) };
}

/// Types `chords` while the user may hold Win: chords with Win use the held
/// key, the others are typed with it let go. An unassigned key is pressed
/// before Win is let go and after it is pressed again, or Start would open.
pub fn keys_holding_win(chords: &[Chord]) {
    let wins = [vk::LWIN, vk::RWIN].map(|w| held(VIRTUAL_KEY(w)).then_some(w));
    let mut batch = Batch::new();
    for &c in chords {
        if wins == [None; 2] {
            batch.chord(c);
        } else if c.mods & WIN != 0 {
            batch.chord(Chord { mods: c.mods & !WIN, ..c });
        } else {
            batch.chord(k(MASK));
            for w in wins.into_iter().flatten() {
                batch.push(key(w, true));
            }
            batch.chord(c);
            for w in wins.into_iter().flatten() {
                batch.push(key(w, false));
            }
            batch.chord(k(MASK));
        }
    }
}

/// Types `vk` with Win held, pressing Win only if the user has released it.
pub fn replay_win_key(vk: u16) {
    keys(&[Chord { mods: if win_held() { 0 } else { WIN }, vk }]);
}

pub fn press_start() {
    keys(&[k(vk::LWIN)]);
}

/// Sends the shell shortcut Win+`letter`.
pub fn press_win(letter: u8) {
    keys(&[Chord { mods: WIN, vk: u16::from(letter) }]);
}

pub fn held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
}

pub fn win_held() -> bool {
    held(VK_LWIN) || held(VK_RWIN)
}

/// Modifiers as the system sees them. Left Alt swallowed by the layer reads
/// as released.
pub fn held_mods() -> u8 {
    let mut mods = 0;
    for (bit, down) in [(CTRL, held(VK_CONTROL)), (SHIFT, held(VK_SHIFT)), (WIN, win_held()), (ALT, held(VK_LMENU))] {
        if down {
            mods |= bit;
        }
    }
    mods
}

pub fn caps_lock_off() {
    if unsafe { GetKeyState(i32::from(vk::CAPITAL)) } & 1 != 0 {
        keys(&[k(vk::CAPITAL)]);
    }
}

pub fn mouse_speed() -> u32 {
    let mut speed = 10u32;
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETMOUSESPEED,
            0,
            Some(&mut speed as *mut u32 as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    speed
}

pub fn set_mouse_speed(speed: u32) {
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_SETMOUSESPEED,
            0,
            Some(speed as usize as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
}
