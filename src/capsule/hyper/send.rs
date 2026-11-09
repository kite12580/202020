use std::ffi::c_void;

use capsule_core::hyper::{ALT, Button, CTRL, Chord, SHIFT, WIN, vk};
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

use crate::capsule::keyhook::OWN_INPUT;

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

fn send(inputs: &[INPUT]) {
    unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
}

fn push(out: &mut Vec<INPUT>, c: Chord) {
    let mods = MODS.iter().filter(|(bit, _)| c.mods & bit != 0);
    out.extend(mods.clone().map(|&(_, m)| key(m, false)));
    out.push(key(c.vk, false));
    out.push(key(c.vk, true));
    out.extend(mods.rev().map(|&(_, m)| key(m, true)));
}

pub fn keys(chords: &[Chord]) {
    let mut inputs = Vec::with_capacity(chords.len() * 4);
    for &c in chords {
        push(&mut inputs, c);
    }
    send(&inputs);
}

pub fn repeat(vk: u16, n: usize) {
    let inputs: Vec<INPUT> = (0..n).flat_map(|_| [key(vk, false), key(vk, true)]).collect();
    send(&inputs);
}

fn mouse(flags: MOUSE_EVENT_FLAGS, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { mouseData: data as u32, dwFlags: flags, dwExtraInfo: OWN_INPUT, ..Default::default() },
        },
    }
}

pub fn wheel(notches: i32) {
    let delta = if notches > 0 { WHEEL_DELTA as i32 } else { -(WHEEL_DELTA as i32) };
    let inputs: Vec<INPUT> = (0..notches.unsigned_abs()).map(|_| mouse(MOUSEEVENTF_WHEEL, delta)).collect();
    send(&inputs);
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
    send(&[mouse(flags, 0)]);
}

fn held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
}

/// Modifiers as the system sees them. Left Alt swallowed by the layer reads
/// as released.
pub fn held_mods() -> u8 {
    let mut mods = 0;
    for (bit, down) in [
        (CTRL, held(VK_CONTROL)),
        (SHIFT, held(VK_SHIFT)),
        (WIN, held(VK_LWIN) || held(VK_RWIN)),
        (ALT, held(VK_LMENU)),
    ] {
        if down {
            mods |= bit;
        }
    }
    mods
}

pub fn caps_lock_off() {
    if unsafe { GetKeyState(i32::from(vk::CAPITAL)) } & 1 != 0 {
        send(&[key(vk::CAPITAL, false), key(vk::CAPITAL, true)]);
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
