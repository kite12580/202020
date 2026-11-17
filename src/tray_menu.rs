use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::app::{AWAKE_MINUTES, App, Awake, BarState, LauncherState, now};
use crate::capsule;
use crate::{
    CMD_AWAKE, CMD_AWAKE_OFF, CMD_BAR, CMD_BREAK, CMD_EDIT_CONFIG, CMD_HYPER, CMD_LAUNCHER, CMD_QUIT,
    CMD_RELOAD_CONFIG, CMD_RESET, CMD_TIMER, CMD_TOGGLE, with_app,
};

fn duration(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

fn awake_label(awake: Option<Awake>) -> String {
    match awake {
        None => "Keep awake".into(),
        Some(Awake { until: None, .. }) => "Keep awake (on)".into(),
        Some(Awake { until: Some(until), .. }) => {
            let minutes = ((until - now()) / 60.0).ceil().max(1.0) as u32;
            format!("Keep awake ({} left)", duration(minutes))
        }
    }
}

unsafe fn fill_awake_menu(menu: HMENU, awake: Option<Awake>) {
    unsafe {
        for (i, minutes) in AWAKE_MINUTES.iter().enumerate() {
            let checked = if awake.is_some_and(|a| a.choice == i) { MF_CHECKED } else { MF_UNCHECKED };
            let text = match minutes {
                Some(30) => "30 minutes".into(),
                Some(60) => "1 hour".into(),
                Some(m) => format!("{} hours", m / 60),
                None => "Until turned off".into(),
            };
            let text: Vec<u16> = text.encode_utf16().chain([0]).collect();
            let _ = AppendMenuW(menu, MF_STRING | checked, (CMD_AWAKE + i as i32) as usize, PCWSTR(text.as_ptr()));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let off = if awake.is_some() { MF_ENABLED } else { MF_GRAYED };
        let _ = AppendMenuW(menu, MF_STRING | off, CMD_AWAKE_OFF as usize, w!("Off"));
    }
}

pub(super) fn show_menu(hwnd: HWND) {
    let Some((timer_on, paused, bar_flags, launcher, awake)) = with_app(|app| {
        let bar_flags = match app.bar {
            BarState::Off => MF_UNCHECKED,
            BarState::Running(_) => MF_CHECKED,
            BarState::Stopping(_) => MF_GRAYED,
        };
        (app.timer_on, app.paused, bar_flags, app.launcher, app.awake)
    }) else {
        return;
    };
    let awake_text: Vec<u16> = awake_label(awake).encode_utf16().chain([0]).collect();
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let Ok(awake_menu) = CreatePopupMenu() else {
            let _ = DestroyMenu(menu);
            return;
        };
        fill_awake_menu(awake_menu, awake);
        let toggles = [
            (MF_STRING | if timer_on { MF_CHECKED } else { MF_UNCHECKED }, CMD_TIMER, w!("20-20-20 timer")),
            (MF_STRING | bar_flags, CMD_BAR, w!("Capsule bar")),
            match launcher {
                LauncherState::Off => (MF_STRING | MF_UNCHECKED, CMD_LAUNCHER, w!("Launcher")),
                LauncherState::On => (MF_STRING | MF_CHECKED, CMD_LAUNCHER, w!("Launcher")),
                LauncherState::NoHotkey => (MF_STRING | MF_CHECKED, CMD_LAUNCHER, w!("Launcher (hotkey unavailable)")),
            },
            (MF_STRING | if capsule::hyper::on() { MF_CHECKED } else { MF_UNCHECKED }, CMD_HYPER, w!("CapsLock hyper")),
        ];
        let rest = [
            (MF_SEPARATOR, 0, PCWSTR::null()),
            (
                MF_STRING | if timer_on { MF_ENABLED } else { MF_GRAYED },
                CMD_TOGGLE,
                if paused { w!("Resume") } else { w!("Pause") },
            ),
            (MF_STRING, CMD_BREAK, w!("Break now")),
            (MF_STRING, CMD_RESET, w!("Reset timer")),
            (MF_SEPARATOR, 0, PCWSTR::null()),
            (MF_STRING, CMD_EDIT_CONFIG, w!("Edit config")),
            (MF_STRING, CMD_RELOAD_CONFIG, w!("Reload config")),
            (MF_SEPARATOR, 0, PCWSTR::null()),
            (MF_STRING, CMD_QUIT, w!("Quit")),
        ];
        for (flags, id, text) in toggles {
            let _ = AppendMenuW(menu, flags, id as usize, text);
        }
        let checked = if awake.is_some() { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING | MF_POPUP | checked, awake_menu.0 as usize, PCWSTR(awake_text.as_ptr()));
        for (flags, id, text) in rest {
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
        CMD_TIMER => with_app(App::toggle_timer),
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
        CMD_AWAKE_OFF => with_app(App::stop_awake),
        _ if cmd >= CMD_AWAKE => with_app(|app| app.keep_awake((cmd - CMD_AWAKE) as usize)),
        _ => None,
    };
}
