use capsule_core::layout::Hit;
use windows::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, WM_RBUTTONDOWN, WM_RBUTTONUP};

use super::{App, TIMER_KEYBOARD, cursor, with_app};
use crate::capsule::keyboard;
use crate::capsule::menu::{self, Item};

const MENU_RESTART: u32 = 1;
const MENU_QUIT: u32 = 2;

/// Menus pump messages and so must run with the app unborrowed.
pub(super) enum DeferredMenu {
    App,
    Keyboard,
    Tray(usize),
}

pub(super) fn run_deferred() {
    while let Some(d) = with_app(|a| a.deferred.pop_front()).flatten() {
        match d {
            DeferredMenu::App => app_menu(),
            DeferredMenu::Keyboard => keyboard_menu(),
            DeferredMenu::Tray(i) => tray_menu(i),
        }
    }
}

fn menu_anchor(a: &App, hit: Hit) -> (i32, i32) {
    let x = a.layout.rect(hit).map_or(a.bounds.left, |r| a.bounds.left + a.px(r.center_x()));
    (x, a.bounds.top - a.px(4.0))
}

fn app_menu() {
    let Some((owner, prev, y)) = with_app(|a| (a.hwnd, a.foreground, a.bounds.top - a.px(4.0))) else { return };
    let x = cursor().x;
    let items = [menu::command(MENU_RESTART, "Restart capsule bar"), menu::command(MENU_QUIT, "Quit capsule bar")];
    let id = menu::show(owner, &items, x, y);
    if !prev.is_invalid() {
        unsafe {
            let _ = SetForegroundWindow(prev);
        }
    }
    with_app(|a| match id {
        MENU_RESTART => a.opts.post_owner(a.opts.restart_msg),
        MENU_QUIT => a.opts.post_owner(a.opts.disable_msg),
        _ => {}
    });
}

fn keyboard_menu() {
    let Some((owner, target, current, (x, y))) =
        with_app(|a| (a.hwnd, a.input_target(), a.kbd.layout, menu_anchor(a, Hit::Keyboard)))
    else {
        return;
    };
    let layouts = keyboard::installed();
    let names: Vec<String> = layouts.iter().map(|&h| keyboard::display_name(h)).collect();
    let items: Vec<Item> = layouts
        .iter()
        .zip(&names)
        .enumerate()
        .map(|(i, (&h, name))| Item { id: i as u32 + 1, text: name, checked: h == current })
        .collect();
    let id = menu::show(owner, &items, x, y);
    if !target.is_invalid() {
        unsafe {
            let _ = SetForegroundWindow(target);
        }
    }
    if let Some(&hkl) = (id as usize).checked_sub(1).and_then(|i| layouts.get(i)) {
        keyboard::switch(target, hkl);
        with_app(|a| a.set_timer(TIMER_KEYBOARD, 150));
    }
}

fn tray_menu(icon: usize) {
    let Some((owner, key, pinned)) = with_app(|a| {
        let i = a.tray.icons.get(icon)?;
        Some((a.popup.hwnd, i.key, a.cfg.config.tray.pinned.contains(&i.pin)))
    })
    .flatten() else {
        return;
    };
    let p = cursor();
    let items = [menu::command(1, if pinned { "Unpin from bar" } else { "Pin to bar" }), menu::command(2, "App menu")];
    let id = menu::show(owner, &items, p.x, p.y);
    with_app(|a| {
        let Some(icon) = a.tray.icons.iter().position(|i| i.key.same(&key)) else { return };
        match id {
            1 => a.toggle_pin(icon),
            2 => {
                a.tray_mouse(icon, WM_RBUTTONDOWN);
                a.tray_mouse(icon, WM_RBUTTONUP);
            }
            _ => {}
        }
    });
}
