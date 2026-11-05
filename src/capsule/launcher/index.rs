//! Searchable entries. Apps and folders are scanned on worker threads, since
//! both touch the disk; windows and commands are listed on the UI thread.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use capsule_core::config::{FolderRoot, Launcher as Config};
use capsule_core::launcher::Source;
use capsule_core::launcher::folders::{excluded, wants_file};
use capsule_core::launcher::haystack::{Keys, app_keyword};
use capsule_core::launcher::history::{key, split_key};
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
use crate::capsule::windows_list as winlist;

#[derive(Clone)]
pub enum Action {
    /// Opens the Apps folder id or path in the entry's history key.
    Item,
    /// A URI or program, opened with `ShellExecuteEx`.
    Open(&'static str),
    Window {
        hwnd: isize,
        exe: String,
    },
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
    /// History key; empty for entries that aren't remembered. Ends in `\`
    /// for directories.
    pub key: String,
    pub name: String,
    pub keys: Keys,
    pub glyph: Option<char>,
    pub action: Action,
}

const DIR_GLYPH: char = '\u{E8B7}';
const OWN_ICON_EXTENSIONS: &[&str] = &["exe", "lnk", "ico", "url", "appref-ms", "msc", "cpl"];

pub fn exe_stem(exe: &str) -> &str {
    exe.rsplit('\\').next().and_then(|f| f.rsplit_once('.')).map_or("", |(s, _)| s)
}

impl Entry {
    fn folder_path(&self) -> Option<&str> {
        match split_key(&self.key)? {
            (Source::Folder, path) => Some(path.trim_end_matches('\\')),
            _ => None,
        }
    }

    fn is_dir(&self) -> bool {
        self.source == Source::Folder && self.key.ends_with('\\')
    }

    /// What `Action::Item` and `Action::Open` open.
    pub fn target(&self) -> Option<String> {
        match &self.action {
            Action::Open(t) => Some(t.to_string()),
            Action::Item => match split_key(&self.key)? {
                (Source::App, id) => Some(apps_folder(id)),
                _ => self.folder_path().map(str::to_string),
            },
            _ => None,
        }
    }

    pub fn subtitle(&self) -> Cow<'_, str> {
        match &self.action {
            Action::Window { exe, .. } => exe_stem(exe).into(),
            Action::App(_) => "202020".into(),
            Action::Run { command, args } if args.is_empty() => command.as_str().into(),
            Action::Run { command, args } => format!("{command} {args}").into(),
            _ => self.folder_path().and_then(|p| p.rsplit_once('\\')).map_or("", |(parent, _)| parent).into(),
        }
    }

    /// Shell parsing name of the icon.
    pub fn icon(&self) -> Option<String> {
        match &self.action {
            Action::Item => self.target(),
            Action::Window { hwnd: h, exe } => {
                Some(winlist::aumid(hwnd(*h)).map_or_else(|| exe.clone(), |id| apps_folder(&id)))
            }
            Action::Run { command, .. } => Some(command.clone()),
            _ => None,
        }
    }

    /// Entries with the same key share an icon.
    pub fn icon_key(&self) -> Option<String> {
        if let Action::Window { hwnd, .. } = self.action {
            return Some(format!("<window {hwnd}>"));
        }
        if self.is_dir() {
            return Some("<folder>".into());
        }
        if self.source == Source::Folder
            && let Some((_, ext)) = self.name.rsplit_once('.')
            && !OWN_ICON_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext))
        {
            return Some(format!(".{}", ext.to_ascii_lowercase()));
        }
        self.icon()
    }
}

pub struct Batch {
    pub source: Source,
    /// Index into the folder roots; 0 for other sources.
    pub root: usize,
    pub entries: Vec<Entry>,
    /// Whether this is the source's final batch.
    pub last: bool,
}

#[derive(Clone)]
struct Outbox {
    tx: Sender<Batch>,
    notify: isize,
    msg: u32,
    cancel: Arc<AtomicBool>,
}

impl Outbox {
    fn send(&self, batch: Batch) -> bool {
        if self.cancel.load(Ordering::Relaxed) || self.tx.send(batch).is_err() {
            return false;
        }
        unsafe {
            let _ = PostMessageW(Some(hwnd(self.notify)), self.msg, WPARAM(0), LPARAM(0));
        }
        true
    }
}

/// Scans apps and folders on separate threads, sending apps as one batch and
/// folders as one batch per root, and posting `msg` to `notify` after each.
/// Stops early once `cancel` is set.
pub fn spawn_scan(cfg: Config, tx: Sender<Batch>, notify: isize, msg: u32, cancel: Arc<AtomicBool>) {
    let out = Outbox { tx, notify, msg, cancel };
    let apps = out.clone();
    let apps_enabled = cfg.apps.enabled;
    std::thread::spawn(move || {
        let entries = if apps_enabled { scan_apps() } else { Vec::new() };
        apps.send(Batch { source: Source::App, root: 0, entries, last: true });
    });
    std::thread::spawn(move || {
        let roots = if cfg.folders.enabled { &cfg.folders.roots[..] } else { &[] };
        if roots.is_empty() {
            out.send(Batch { source: Source::Folder, root: 0, entries: Vec::new(), last: true });
        }
        for (i, root) in roots.iter().enumerate() {
            let entries = scan_root(root, &out.cancel);
            if !out.send(Batch { source: Source::Folder, root: i, entries, last: i + 1 == roots.len() }) {
                return;
            }
        }
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
    let keyword = app_keyword(id);
    let hidden: &[&str] = if keyword.eq_ignore_ascii_case(&name) { &[] } else { &[keyword] };
    Entry {
        source: Source::App,
        key: key(Source::App, id),
        keys: Keys::new(&name, hidden),
        name,
        glyph: None,
        action: Action::Item,
    }
}

pub fn file_entry(path: &str, name: String, dir: bool) -> Entry {
    let mut key = key(Source::Folder, path);
    if dir {
        key.push('\\');
    }
    Entry {
        source: Source::Folder,
        key,
        keys: Keys::new(&name, &[]),
        name,
        glyph: Some(if dir { DIR_GLYPH } else { '\u{E8A5}' }),
        action: Action::Item,
    }
}

fn scan_root(root: &FolderRoot, cancel: &AtomicBool) -> Vec<Entry> {
    let mut out = Vec::new();
    let base = super::launch::env(&root.path);
    let base = base.trim_end_matches(['\\', '/']).to_string();
    let mut stack = vec![(base, 0)];
    'walk: while let Some((dir, depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        for (name, attrs) in list_dir(&dir) {
            if out.len() >= root.max_items {
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
                    out.push(file_entry(&path, name, true));
                }
            } else if wants_file(root, &name) {
                out.push(file_entry(&path, name, false));
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
