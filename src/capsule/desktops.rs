//! Two desktops made by hiding the other one's windows. Whatever is visible
//! when switching belongs to the desktop being left. Runs on the main thread.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    IsHungAppWindow, IsWindowVisible, SW_HIDE, SW_SHOWNA, ShowWindow, ShowWindowAsync,
};

use crate::capsule::util::{class_name, foreground, root_owner, window_ids};
use crate::capsule::watchdog;
use crate::capsule::windows_list::{cloaked, force_foreground, qualifies, top_level};

pub const TOGGLE: usize = 0;
pub const MOVE_ACTIVE: usize = 1;

type Window = (HWND, u32);

/// ConPTY's stand-in console window. Hiding it makes the terminal owning it
/// minimize, which keeps the terminal on the taskbar.
const PSEUDO_CONSOLE: &str = "PseudoConsoleWindow";

#[derive(Default)]
struct Desktops {
    /// The other desktop's windows with their process ids, top first.
    hidden: Vec<Window>,
    other_active: Option<HWND>,
}

thread_local! {
    static STATE: RefCell<Desktops> = RefCell::default();
}

impl Desktops {
    /// Hides `windows` above the hidden ones, returning the hidden ones that
    /// are still alive and weren't among them.
    fn hide(&mut self, windows: Vec<Window>) -> Option<Vec<Window>> {
        if !watchdog::set_hidden(&[&windows[..], &self.hidden[..]].concat()) {
            return None;
        }
        let hidden = windows.into_iter().filter(|&(h, _)| unsafe { ShowWindowAsync(h, SW_HIDE) }.as_bool()).collect();
        let old = std::mem::replace(&mut self.hidden, hidden);
        Some(old.into_iter().filter(|w| !self.hidden.contains(w) && alive(w)).collect())
    }
}

pub fn alive(&(h, pid): &Window) -> bool {
    window_ids(h).1 == pid
}

/// Shows `windows`, given top first.
pub fn show(windows: impl DoubleEndedIterator<Item = Window>) {
    for (h, _) in windows.rev() {
        unsafe {
            let _ = ShowWindowAsync(h, SW_SHOWNA);
        }
    }
}

/// Visible windows of the apps on screen whose root owner passes `keep`,
/// owned ones included, top first.
fn on_screen(keep: impl Fn(HWND) -> bool) -> Vec<Window> {
    let mut roots = HashMap::new();
    top_level()
        .into_iter()
        .filter(|&h| {
            if !unsafe { IsWindowVisible(h) }.as_bool() {
                return false;
            }
            let r = root_owner(h);
            keep(r)
                && (qualifies(h)
                    || (r != h
                        && !cloaked(h)
                        && class_name(h) != PSEUDO_CONSOLE
                        && *roots.entry(r.0 as isize).or_insert_with(|| qualifies(r))))
        })
        .map(|h| (h, window_ids(h).1))
        .collect()
}

fn focus(h: HWND) {
    unsafe {
        if IsHungAppWindow(h).as_bool() {
            let _ = ShowWindowAsync(h, SW_SHOWNA);
        } else {
            let _ = ShowWindow(h, SW_SHOWNA);
        }
    }
    force_foreground(h);
}

pub fn on_request(request: usize) {
    match request {
        TOGGLE => toggle(),
        _ => move_active(),
    }
}

fn toggle() {
    STATE.with_borrow_mut(|d| {
        let active = foreground();
        let Some(returning) = d.hide(on_screen(|_| true)) else { return };
        let target =
            d.other_active.filter(|&a| returning.iter().any(|&(h, _)| h == a)).or(returning.first().map(|w| w.0));
        show(returning.iter().copied().filter(|&(h, _)| Some(h) != target));
        if let Some(h) = target {
            focus(h);
        }
        d.other_active = active;
        watchdog::set_hidden(&d.hidden);
    })
}

fn move_active() {
    let Some(active) = foreground().map(root_owner) else { return };
    let moving = on_screen(|r| r == active);
    if moving.is_empty() {
        return;
    }
    STATE.with_borrow_mut(|d| {
        let Some(rest) = d.hide(moving) else { return };
        d.hidden.extend(rest);
        watchdog::set_hidden(&d.hidden);
    })
}

pub fn restore_all() {
    STATE.with_borrow_mut(|d| {
        d.other_active = None;
        if d.hidden.is_empty() {
            return;
        }
        show(d.hidden.drain(..).filter(alive));
        watchdog::set_hidden(&[]);
    })
}
