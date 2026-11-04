use capsule_core::config::CustomCommand;
use capsule_core::launcher::Source;
use capsule_core::launcher::haystack::Keys;
use capsule_core::launcher::history::key;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
use windows::core::BOOL;

use super::AppCommand;
use super::index::{Action, Entry};
use crate::capsule::util::{apps_folder, own_window, process_path, window_ids};
use crate::capsule::windows_list::{self as winlist, qualifies};

unsafe extern "system" fn collect(h: HWND, out: LPARAM) -> BOOL {
    unsafe { (*(out.0 as *mut Vec<HWND>)).push(h) };
    BOOL(1)
}

pub fn windows() -> Vec<Entry> {
    let mut all: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut all as *mut _ as isize));
    }
    all.into_iter()
        .filter(|&h| qualifies(h) && !own_window(h))
        .filter_map(|h| {
            let title = winlist::title(h);
            if title.is_empty() {
                return None;
            }
            let exe = process_path(window_ids(h).1).unwrap_or_default();
            let stem = exe.rsplit('\\').next().and_then(|f| f.rsplit_once('.')).map_or("", |(s, _)| s).to_string();
            let icon = winlist::aumid(h).map_or_else(|| exe.clone(), |id| apps_folder(&id));
            Some(Entry {
                source: Source::Window,
                key: String::new(),
                keys: Keys::new(&title, &[&stem]),
                name: title,
                subtitle: stem,
                icon,
                glyph: Some('\u{E8A7}'),
                action: Action::Window(h.0 as isize),
            })
        })
        .collect()
}

struct Builtin {
    id: &'static str,
    name: &'static str,
    glyph: char,
    action: fn() -> Action,
}

const SYSTEM: &[Builtin] = &[
    Builtin {
        id: "recycle",
        name: "Empty Recycle Bin  清空回收站",
        glyph: '\u{E74D}',
        action: || Action::EmptyRecycleBin,
    },
    Builtin {
        id: "settings",
        name: "Settings  设置",
        glyph: '\u{E713}',
        action: || Action::Open("ms-settings:".into()),
    },
    Builtin {
        id: "taskmgr",
        name: "Task Manager  任务管理器",
        glyph: '\u{E9D9}',
        action: || Action::Open("taskmgr.exe".into()),
    },
    Builtin {
        id: "control",
        name: "Control Panel  控制面板",
        glyph: '\u{E80F}',
        action: || Action::Open("control.exe".into()),
    },
];

fn command(id: String, name: &str, subtitle: &str, glyph: char, action: Action, icon: String) -> Entry {
    Entry {
        source: Source::Command,
        key: key(Source::Command, &id),
        keys: Keys::new(name, &[]),
        name: name.to_string(),
        subtitle: subtitle.to_string(),
        icon,
        glyph: Some(glyph),
        action,
    }
}

pub fn commands(app: &[AppCommand], custom: &[CustomCommand]) -> Vec<Entry> {
    let app = app
        .iter()
        .map(|c| command(format!("app:{}", c.id), c.name, "202020", '\u{E916}', Action::App(c.id), String::new()));
    let system =
        SYSTEM.iter().map(|b| command(format!("sys:{}", b.id), b.name, "", b.glyph, (b.action)(), String::new()));
    let custom = custom.iter().filter(|c| !c.name.is_empty() && !c.command.is_empty()).map(|c| {
        let subtitle = if c.args.is_empty() { c.command.clone() } else { format!("{} {}", c.command, c.args) };
        command(
            format!("custom:{}", c.name),
            &c.name,
            &subtitle,
            '\u{E756}',
            Action::Run { command: c.command.clone(), args: c.args.clone() },
            c.command.clone(),
        )
    });
    app.chain(system).chain(custom).collect()
}
