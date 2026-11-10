//! Two desktops made by hiding the other one's windows. Whatever is visible
//! when switching belongs to the desktop being left. Runs on the main thread.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOTOWNER, GetAncestor, IsHungAppWindow, IsWindowVisible, SW_HIDE, SW_SHOWNA, SetForegroundWindow, ShowWindow,
    ShowWindowAsync,
};

use crate::capsule::util::{foreground, window_ids};
use crate::capsule::watchdog;
use crate::capsule::windows_list::{cloaked, force_foreground, qualifies, top_level};

#[derive(Default)]
struct Desktops {
    /// The other desktop's windows with their process ids, top first.
    hidden: Vec<(HWND, u32)>,
    other_active: Option<HWND>,
}

thread_local! {
    static STATE: RefCell<Desktops> = RefCell::default();
}

fn root(h: HWND) -> HWND {
    let r = unsafe { GetAncestor(h, GA_ROOTOWNER) };
    if r.is_invalid() { h } else { r }
}

fn alive(&(h, pid): &(HWND, u32)) -> bool {
    window_ids(h).1 == pid
}

/// Visible windows of the apps on screen, owned ones included, top first.
fn on_screen() -> Vec<(HWND, u32)> {
    let mut roots = HashMap::new();
    top_level()
        .into_iter()
        .filter(|&h| {
            unsafe { IsWindowVisible(h) }.as_bool()
                && !cloaked(h)
                && (qualifies(h) || *roots.entry(root(h).0 as isize).or_insert_with(|| qualifies(root(h))))
        })
        .map(|h| (h, window_ids(h).1))
        .collect()
}

/// Hides `windows`, returning those it could; elevated ones can't be.
fn hide(windows: Vec<(HWND, u32)>) -> Vec<(HWND, u32)> {
    windows.into_iter().filter(|&(h, _)| unsafe { ShowWindowAsync(h, SW_HIDE) }.as_bool()).collect()
}

fn focus(h: HWND) {
    unsafe {
        if IsHungAppWindow(h).as_bool() {
            let _ = ShowWindowAsync(h, SW_SHOWNA);
        } else {
            let _ = ShowWindow(h, SW_SHOWNA);
        }
        if !SetForegroundWindow(h).as_bool() {
            force_foreground(h);
        }
    }
}

pub fn toggle() {
    STATE.with_borrow_mut(|d| {
        let active = foreground();
        let leaving = on_screen();
        if !watchdog::set_hidden(&[&d.hidden[..], &leaving[..]].concat()) {
            return;
        }
        let old = std::mem::replace(&mut d.hidden, hide(leaving));
        let returning: Vec<_> = old.into_iter().filter(|w| alive(w) && !d.hidden.contains(w)).collect();
        let target =
            d.other_active.filter(|&a| returning.iter().any(|&(h, _)| h == a)).or(returning.first().map(|w| w.0));
        for &(h, _) in returning.iter().rev().filter(|&&(h, _)| Some(h) != target) {
            unsafe {
                let _ = ShowWindowAsync(h, SW_SHOWNA);
            }
        }
        if let Some(h) = target {
            focus(h);
        }
        d.other_active = active;
        watchdog::set_hidden(&d.hidden);
    })
}

pub fn move_active() {
    let Some(active) = foreground().map(root) else { return };
    let moving: Vec<_> = on_screen().into_iter().filter(|&(h, _)| root(h) == active).collect();
    if moving.is_empty() {
        return;
    }
    STATE.with_borrow_mut(|d| {
        if !watchdog::set_hidden(&[&moving[..], &d.hidden[..]].concat()) {
            return;
        }
        let moved = hide(moving);
        d.hidden.retain(|w| !moved.contains(w));
        d.hidden.splice(0..0, moved);
        watchdog::set_hidden(&d.hidden);
    })
}

pub fn restore_all() {
    STATE.with_borrow_mut(|d| {
        for w in d.hidden.drain(..).rev().filter(alive) {
            unsafe {
                let _ = ShowWindowAsync(w.0, SW_SHOWNA);
            }
        }
        d.other_active = None;
        watchdog::set_hidden(&[]);
    })
}
