use std::collections::HashMap;

use capsule_core::config::CustomCommand;
use capsule_core::launcher::Source;
use capsule_core::launcher::haystack::Keys;
use capsule_core::launcher::history::key;

use super::AppCommand;
use super::index::{Action, Entry, exe_stem};
use crate::capsule::util::{process_path, window_ids};
use crate::capsule::windows_list::{self as winlist, qualifies};

pub fn windows() -> Vec<Entry> {
    let mut exes: HashMap<u32, String> = HashMap::new();
    winlist::top_level()
        .into_iter()
        .filter(|&h| qualifies(h))
        .filter_map(|h| {
            let title = winlist::title(h);
            if title.is_empty() {
                return None;
            }
            let pid = window_ids(h).1;
            let exe = exes.entry(pid).or_insert_with(|| process_path(pid).unwrap_or_default()).clone();
            Some(Entry {
                source: Source::Window,
                key: String::new(),
                keys: Keys::new(&title, &[exe_stem(&exe)]),
                name: title,
                glyph: Some('\u{E8A7}'),
                action: Action::Window { hwnd: h.0 as isize, exe },
            })
        })
        .collect()
}

struct Builtin {
    id: &'static str,
    name: &'static str,
    glyph: char,
    action: Action,
}

const SYSTEM: &[Builtin] = &[
    Builtin {
        id: "recycle",
        name: "Empty Recycle Bin  清空回收站",
        glyph: '\u{E74D}',
        action: Action::EmptyRecycleBin,
    },
    Builtin { id: "settings", name: "Settings  设置", glyph: '\u{E713}', action: Action::Open("ms-settings:") },
    Builtin {
        id: "taskmgr", name: "Task Manager  任务管理器", glyph: '\u{E9D9}', action: Action::Open("taskmgr.exe")
    },
    Builtin {
        id: "control", name: "Control Panel  控制面板", glyph: '\u{E80F}', action: Action::Open("control.exe")
    },
];

fn command(id: &str, name: &str, glyph: char, action: Action) -> Entry {
    Entry {
        source: Source::Command,
        key: key(Source::Command, id),
        keys: Keys::new(name, &[]),
        name: name.to_string(),
        glyph: Some(glyph),
        action,
    }
}

pub fn commands(app: &[AppCommand], custom: &[CustomCommand]) -> Vec<Entry> {
    let app = app.iter().map(|c| command(&format!("app:{}", c.id), c.name, '\u{E916}', Action::App(c.id)));
    let system = SYSTEM.iter().map(|b| command(&format!("sys:{}", b.id), b.name, b.glyph, b.action.clone()));
    let custom = custom.iter().filter(|c| !c.name.is_empty() && !c.command.is_empty()).map(|c| {
        let action = Action::Run { command: c.command.clone(), args: c.args.clone() };
        command(&format!("custom:{}", c.name), &c.name, '\u{E756}', action)
    });
    app.chain(system).chain(custom).collect()
}
