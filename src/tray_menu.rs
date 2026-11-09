use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::app::{App, BarState, LauncherState};
use crate::capsule;
use crate::{
    CMD_BAR, CMD_BREAK, CMD_EDIT_CONFIG, CMD_HYPER, CMD_LAUNCHER, CMD_QUIT, CMD_RELOAD_CONFIG, CMD_RESET, CMD_TOGGLE,
    with_app,
};

pub(super) fn show_menu(hwnd: HWND) {
    let Some((paused, bar_flags, launcher)) = with_app(|app| {
        let bar_flags = match app.bar {
            BarState::Off => MF_UNCHECKED,
            BarState::Running(_) => MF_CHECKED,
            BarState::Stopping(_) => MF_GRAYED,
        };
        (app.paused, bar_flags, app.launcher)
    }) else {
        return;
    };
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let items = [
            (MF_STRING | bar_flags, CMD_BAR, w!("Capsule bar")),
            match launcher {
                LauncherState::Off => (MF_STRING | MF_UNCHECKED, CMD_LAUNCHER, w!("Launcher")),
                LauncherState::On => (MF_STRING | MF_CHECKED, CMD_LAUNCHER, w!("Launcher")),
                LauncherState::NoHotkey => (MF_STRING | MF_CHECKED, CMD_LAUNCHER, w!("Launcher (hotkey unavailable)")),
            },
            (MF_STRING | if capsule::hyper::on() { MF_CHECKED } else { MF_UNCHECKED }, CMD_HYPER, w!("CapsLock hyper")),
            (MF_SEPARATOR, 0, PCWSTR::null()),
            (MF_STRING, CMD_TOGGLE, if paused { w!("Resume") } else { w!("Pause") }),
            (MF_STRING, CMD_BREAK, w!("Break now")),
            (MF_STRING, CMD_RESET, w!("Reset timer")),
            (MF_SEPARATOR, 0, PCWSTR::null()),
            (MF_STRING, CMD_EDIT_CONFIG, w!("Edit config")),
            (MF_STRING, CMD_RELOAD_CONFIG, w!("Reload config")),
            (MF_SEPARATOR, 0, PCWSTR::null()),
            (MF_STRING, CMD_QUIT, w!("Quit")),
        ];
        for (flags, id, text) in items {
            let _ = AppendMenuW(menu, flags, id as usize, text);
        }
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        cmd.0
    };
    run_command(cmd);
}

pub(super) fn run_command(cmd: i32) {
    match cmd {
        CMD_TOGGLE => with_app(App::toggle_pause),
        CMD_BREAK => with_app(App::show_break),
        CMD_RESET => with_app(App::reset_timer),
        CMD_QUIT => with_app(App::quit),
        CMD_BAR => with_app(App::toggle_bar),
        CMD_LAUNCHER => with_app(App::toggle_launcher),
        CMD_HYPER => with_app(App::toggle_hyper),
        CMD_EDIT_CONFIG => {
            capsule::edit_config();
            None
        }
        CMD_RELOAD_CONFIG => with_app(App::reload_config),
        _ => None,
    };
}
