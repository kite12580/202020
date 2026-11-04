//! Searchable entries. Apps and folders are scanned on a worker thread, since
//! both touch the disk; windows and commands are listed on the UI thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use capsule_core::config::{FolderRoot, Launcher as Config};
use capsule_core::launcher::Source;
use capsule_core::launcher::folders::{excluded, wants_file};
use capsule_core::launcher::haystack::{Keys, app_keyword};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM,
    FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
    WIN32_FIND_DATAW,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT, SHGetKnownFolderItem,
    SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::PCWSTR;

use crate::capsule::util::{apps_folder, from_wide, hwnd, take_string, wide};

#[derive(Clone)]
pub enum Action {
    /// A path or shell parsing name, opened with `ShellExecuteEx`.
    Open(String),
    Window(isize),
    /// A 202020 command, posted to the owner window.
    App(u32),
    Run {
        command: String,
        args: String,
    },
    EmptyRecycleBin,
}

pub struct Entry {
    pub source: Source,
    /// History key; empty for entries that aren't remembered.
    pub key: String,
    pub name: String,
    pub subtitle: String,
    pub keys: Keys,
    /// Shell parsing name of the icon, `""` for none.
    pub icon: String,
    pub glyph: Option<char>,
    pub action: Action,
}

pub const DIR_GLYPH: char = '\u{E8B7}';

pub struct Batch {
    pub source: Source,
    pub entries: Vec<Entry>,
    pub last: bool,
}

/// Scans enabled disk sources, sending one batch per source and posting
/// `msg` to `notify` after each. Stops early once `cancel` is set.
pub fn spawn_scan(cfg: Config, tx: Sender<Batch>, notify: isize, msg: u32, cancel: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let send = |source, entries, last| {
            if cancel.load(Ordering::Relaxed) || tx.send(Batch { source, entries, last }).is_err() {
                return false;
            }
            unsafe {
                let _ = PostMessageW(Some(hwnd(notify)), msg, WPARAM(0), LPARAM(0));
            }
            true
        };
        let apps = if cfg.apps.enabled { scan_apps() } else { Vec::new() };
        if !send(Source::App, apps, false) {
            return;
        }
        let folders = if cfg.folders.enabled { scan_folders(&cfg.folders.roots, &cancel) } else { Vec::new() };
        send(Source::Folder, folders, true);
    });
}

fn scan_apps() -> Vec<Entry> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    let out = enum_apps().unwrap_or_else(|e| {
        log::warn!("apps folder: {e}");
        Vec::new()
    });
    unsafe { CoUninitialize() };
    out
}

fn enum_apps() -> windows::core::Result<Vec<Entry>> {
    let mut out = Vec::new();
    unsafe {
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        loop {
            let mut batch: [Option<IShellItem>; 64] = std::array::from_fn(|_| None);
            let mut fetched = 0;
            items.Next(&mut batch, Some(&mut fetched))?;
            if fetched == 0 {
                break;
            }
            for item in batch.iter().take(fetched as usize).flatten() {
                let Some(name) = item.GetDisplayName(SIGDN_NORMALDISPLAY).ok().and_then(take_string) else {
                    continue;
                };
                let Some(id) = item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).ok().and_then(take_string) else {
                    continue;
                };
                out.push(app_entry(name, &id));
            }
        }
    }
    Ok(out)
}

pub fn app_entry(name: String, id: &str) -> Entry {
    let target = apps_folder(id);
    let keyword = app_keyword(id);
    let hidden: &[&str] = if keyword.eq_ignore_ascii_case(&name) { &[] } else { &[keyword] };
    Entry {
        source: Source::App,
        key: capsule_core::launcher::history::key(Source::App, id),
        keys: Keys::new(&name, hidden),
        name,
        subtitle: String::new(),
        icon: target.clone(),
        glyph: None,
        action: Action::Open(target),
    }
}

pub fn file_entry(path: String, name: String, dir: bool) -> Entry {
    let subtitle = path.rsplit_once('\\').map_or_else(String::new, |(parent, _)| parent.to_string());
    Entry {
        source: Source::Folder,
        key: capsule_core::launcher::history::key(Source::Folder, &path),
        keys: Keys::new(&name, &[]),
        name,
        subtitle,
        icon: path.clone(),
        glyph: Some(if dir { DIR_GLYPH } else { '\u{E8A5}' }),
        action: Action::Open(path),
    }
}

fn scan_folders(roots: &[FolderRoot], cancel: &AtomicBool) -> Vec<Entry> {
    let mut out = Vec::new();
    for root in roots {
        let base = super::launch::env(&root.path);
        let base = base.trim_end_matches(['\\', '/']).to_string();
        let start = out.len();
        let mut stack = vec![(base, 0)];
        'walk: while let Some((dir, depth)) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                return out;
            }
            for (name, attrs) in list_dir(&dir) {
                if out.len() - start >= root.max_items {
                    break 'walk;
                }
                if attrs & (FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0) != 0 || excluded(root, &name) {
                    continue;
                }
                let path = format!(r"{dir}\{name}");
                let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
                if is_dir {
                    if depth < root.depth && attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0 {
                        stack.push((path.clone(), depth + 1));
                    }
                    if root.include_dirs {
                        out.push(file_entry(path, name, true));
                    }
                } else if wants_file(root, &name) {
                    out.push(file_entry(path, name, false));
                }
            }
        }
    }
    out
}

fn list_dir(dir: &str) -> Vec<(String, u32)> {
    let pattern = wide(&format!(r"{dir}\*"));
    let mut data = WIN32_FIND_DATAW::default();
    let mut out = Vec::new();
    unsafe {
        let Ok(find) = FindFirstFileExW(
            PCWSTR(pattern.as_ptr()),
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        ) else {
            return out;
        };
        loop {
            let name = from_wide(&data.cFileName);
            if name != "." && name != ".." {
                out.push((name, data.dwFileAttributes));
            }
            if FindNextFileW(find, &mut data).is_err() {
                break;
            }
        }
        let _ = FindClose(find);
    }
    out
}
