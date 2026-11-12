//! Spotlight-style search window. Lives on the main thread; everything but the
//! launch history is freed after `idle_unload_minutes` without an open.

mod index;
mod interact;
mod launch;
mod search;
mod sources;
mod view;

pub(crate) use launch::open;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use capsule_core::config::{Backdrop, FolderRoot, Launcher as Config};
use capsule_core::launcher::Source;
use capsule_core::launcher::history::History;
use capsule_core::launcher::input::TextInput;
use capsule_core::launcher::rank::{Match, Searcher};
use capsule_core::shell_proto::Answer;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentProcess, INFINITE, SetProcessWorkingSetSize};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCaretBlinkTime, KillTimer, SW_HIDE, SW_SHOW,
    SetForegroundWindow, SetTimer, ShowWindow, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_CHAR, WM_CLOSE, WM_CONTEXTMENU,
    WM_DPICHANGED, WM_ERASEBKGND, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::capsule::config_store;
use crate::capsule::gfx::{Font, Gfx, Image, Surface, Text};
use crate::capsule::hook::{Hook, HookEvent};
use crate::capsule::theme::{self, Theme};
use crate::capsule::util::{dpi, hwnd, post, register_class, scale};
use crate::capsule::windows_list as winlist;
use index::{Action, Batch, Entry};
use search::Row;

/// A 202020 action offered as a command; `id` is posted as the wparam of
/// `Options::command_msg`.
pub struct AppCommand {
    pub id: u32,
    pub name: &'static str,
}

#[derive(Clone, Copy)]
pub struct Options {
    pub owner: isize,
    pub command_msg: u32,
    pub commands: &'static [AppCommand],
}

const CLASS: PCWSTR = w!("202020Launcher");
const HISTORY_FILE: &str = "launcher_history.tsv";
const WM_SCAN: u32 = WM_APP + 1;
const WM_HOOK: u32 = WM_APP + 2;
const TIMER_IDLE: usize = 1;
const TIMER_CARET: usize = 2;
const TIMER_MATCH: usize = 3;

const RESCAN_AFTER: Duration = Duration::from_secs(60);

const SOURCES: [Source; 4] = [Source::App, Source::Window, Source::Command, Source::Folder];

/// Index into `Launcher::lists`.
const RECENT: usize = SOURCES.len();

fn list_of(source: Source) -> usize {
    SOURCES.iter().position(|&s| s == source).unwrap_or(RECENT)
}

thread_local! {
    static OPTS: Cell<Option<Options>> = const { Cell::new(None) };
    static STATE: RefCell<Option<Launcher>> = const { RefCell::new(None) };
    static HISTORY: RefCell<Option<History>> = const { RefCell::new(None) };
}

fn with_launcher<R>(f: impl FnOnce(&mut Launcher) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.as_mut().map(f)))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn history_path() -> PathBuf {
    config_store::app_dir().join(HISTORY_FILE)
}

fn with_history<R>(f: impl FnOnce(&mut History) -> R) -> R {
    HISTORY.with(|h| {
        let mut h = h.borrow_mut();
        let h = h.get_or_insert_with(|| History::parse(&std::fs::read_to_string(history_path()).unwrap_or_default()));
        f(h)
    })
}

pub fn init(opts: Options) {
    OPTS.set(Some(opts));
}

pub fn toggle() {
    if with_launcher(|l| l.visible) == Some(true) {
        hide();
        return;
    }
    if STATE.with(|s| s.borrow().is_none()) {
        let Some(l) = OPTS.get().and_then(Launcher::create) else { return };
        STATE.with(|s| *s.borrow_mut() = Some(l));
    }
    with_launcher(Launcher::show);
}

pub fn hide() {
    with_launcher(Launcher::hide);
}

/// Frees the window, index and icons.
pub fn unload() {
    let Some(l) = STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.take())) else { return };
    drop(l);
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

pub fn flush_history() {
    HISTORY.with(|h| {
        let Ok(mut h) = h.try_borrow_mut() else { return };
        if let Some(h) = h.as_mut()
            && h.take_dirty()
            && let Err(e) = std::fs::write(history_path(), h.serialize())
        {
            log::error!("launcher history: {e}");
        }
    });
}

enum Deferred {
    Activate(usize),
    Menu(usize, i32, i32),
}

struct Launcher {
    opts: Options,
    hwnd: HWND,
    dpi: u32,
    gfx: Gfx,
    surface: Option<Surface>,
    theme: Option<Theme>,
    cfg: Config,
    cfg_stamp: Option<SystemTime>,
    visible: bool,
    bounds: (i32, i32, i32, i32),
    input: TextInput,
    search: Option<Text>,
    placeholder: Option<Text>,
    lens: Option<Text>,
    high_surrogate: Option<u16>,
    searcher: Searcher,
    /// Apps, windows, commands, folders, and history entries not in the others.
    lists: [Vec<Entry>; RECENT + 1],
    /// Positions of the entries that have history.
    by_key: HashMap<String, (usize, usize)>,
    scanned: [bool; RECENT],
    /// Entries per folder root, in order, in the folders list.
    root_lens: Vec<usize>,
    scan: Option<Scan>,
    scan_started: Option<Instant>,
    /// Matches for `found_query`, which a longer query only narrows. Still
    /// incomplete while `recheck` or the lists from `next` are unscored.
    found: Vec<(usize, usize, Match)>,
    found_query: Option<(Option<Source>, String)>,
    recheck: Vec<(usize, usize)>,
    next: (usize, usize),
    rows: Vec<Row>,
    selected: usize,
    hover: Option<usize>,
    pressed: Option<usize>,
    hook: Hook,
    /// Whether the apps list is wanted from explorer.
    apps_pending: bool,
    images: HashMap<String, Option<Image>>,
    pending: HashMap<u64, String>,
    next_icon: u64,
    caret_on: bool,
    deferred: Option<Deferred>,
}

struct Scan {
    rx: Receiver<Batch>,
    cancel: Arc<AtomicBool>,
}

impl Launcher {
    fn create(opts: Options) -> Option<Launcher> {
        register_class(WNDCLASSEXW { lpfnWndProc: Some(launcher_proc), lpszClassName: CLASS, ..Default::default() });
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                CLASS,
                w!("202020 launcher"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                GetModuleHandleW(None).ok().map(Into::into),
                None,
            )
        }
        .ok();
        let Some(hwnd) = hwnd else {
            unsafe { CoUninitialize() };
            return None;
        };
        let cfg = config_store::launcher();
        let gfx = Gfx::new();
        Some(Launcher {
            opts,
            hwnd,
            dpi: dpi(hwnd),
            placeholder: gfx.text("Search apps, files, windows and commands", Font::Search),
            lens: gfx.text("\u{E721}", Font::SearchGlyph),
            gfx,
            surface: None,
            theme: None,
            cfg,
            cfg_stamp: config_store::modified(),
            visible: false,
            bounds: (0, 0, 0, 0),
            input: TextInput::default(),
            search: None,
            high_surrogate: None,
            searcher: Searcher::default(),
            lists: Default::default(),
            by_key: HashMap::new(),
            scanned: [false; RECENT],
            root_lens: Vec::new(),
            scan: None,
            scan_started: None,
            found: Vec::new(),
            found_query: None,
            recheck: Vec::new(),
            next: (RECENT, 0),
            rows: Vec::new(),
            selected: 0,
            hover: None,
            pressed: None,
            hook: Hook::acquire(hwnd, WM_HOOK, None),
            apps_pending: false,
            images: HashMap::new(),
            pending: HashMap::new(),
            next_icon: 1,
            caret_on: true,
            deferred: None,
        })
    }

    fn px(&self, dip: f32) -> i32 {
        scale(dip, self.dpi)
    }

    fn to_dip(&self, px: i32) -> f32 {
        px as f32 * 96.0 / self.dpi as f32
    }

    fn weight(&self, source: Source) -> f32 {
        match source {
            Source::App => self.cfg.apps.weight,
            Source::Window => self.cfg.windows.weight,
            Source::Command => self.cfg.commands.weight,
            Source::Folder => self.cfg.folders.weight,
        }
    }

    fn enabled(&self, source: Source) -> bool {
        match source {
            Source::App => self.cfg.apps.enabled,
            Source::Window => self.cfg.windows.enabled,
            Source::Command => self.cfg.commands.enabled,
            Source::Folder => self.cfg.folders.enabled,
        }
    }

    fn entry(&self, row: &Row) -> &Entry {
        &self.lists[row.list][row.index]
    }

    /// Whether the scanned sources changed.
    fn reload_config(&mut self) -> bool {
        let stamp = config_store::modified();
        if stamp == self.cfg_stamp {
            return false;
        }
        self.cfg_stamp = stamp;
        let cfg = config_store::launcher();
        fn scanned(c: &Config) -> (bool, bool, &[FolderRoot]) {
            (c.apps.enabled, c.folders.enabled, &c.folders.roots)
        }
        let rescan = scanned(&cfg) != scanned(&self.cfg);
        self.cfg = cfg;
        self.theme = None;
        rescan
    }

    fn show(&mut self) {
        let rescan = self.reload_config();
        let theme = Theme::read();
        if self.theme != Some(theme) {
            theme::apply(self.hwnd, Backdrop::None, &theme);
            self.theme = Some(theme);
        }
        self.lists[list_of(Source::Window)] = if self.cfg.windows.enabled { sources::windows() } else { Vec::new() };
        self.lists[list_of(Source::Command)] = if self.cfg.commands.enabled {
            sources::commands(self.opts.commands, &self.cfg.commands.custom)
        } else {
            Vec::new()
        };
        self.list_changed(list_of(Source::Window));
        self.list_changed(list_of(Source::Command));
        if rescan && let Some(scan) = self.scan.take() {
            scan.cancel.store(true, Ordering::Relaxed);
        }
        if rescan || self.scan_started.is_none_or(|t| t.elapsed() > RESCAN_AFTER) {
            self.start_scan();
        }
        self.input.select_all();
        self.selected = 0;
        self.hover = None;
        self.refresh();
        self.visible = true;
        self.caret_on = true;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_IDLE);
            let blink = GetCaretBlinkTime();
            if blink != INFINITE {
                SetTimer(Some(self.hwnd), TIMER_CARET, blink.clamp(200, 2000), None);
            }
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.place_ime();
    }

    fn hide(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        self.rows.clear();
        self.lists[list_of(Source::Window)].clear();
        self.stop_matching();
        self.found = Vec::new();
        self.recheck = Vec::new();
        self.hover = None;
        self.pressed = None;
        self.surface = None;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            let _ = KillTimer(Some(self.hwnd), TIMER_CARET);
            let minutes = self.cfg.idle_unload_minutes;
            SetTimer(Some(self.hwnd), TIMER_IDLE, minutes * 60_000, None);
        }
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn on_hook_events(&mut self) {
        for ev in self.hook.take_events() {
            match ev {
                HookEvent::Attached => {
                    self.pending.clear();
                    self.images.retain(|_, image| image.is_some());
                    if self.apps_pending {
                        self.request_apps();
                    }
                    self.request_icons();
                }
                HookEvent::Answer(Answer::Apps(apps)) => self.on_apps(apps),
                HookEvent::Answer(Answer::Icon { id, bitmap }) => self.on_icon(id, bitmap),
                HookEvent::Answer(Answer::Pins(_)) => {}
            }
        }
    }

    fn message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_PAINT => self.paint(),
            WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => self.hide(),
            WM_CLOSE => self.hide(),
            WM_KEYDOWN => {
                if !self.on_key(wparam.0 as u16) {
                    return None;
                }
            }
            WM_CHAR => self.on_char(wparam.0 as u16),
            WM_IME_STARTCOMPOSITION => {
                self.place_ime();
                return None;
            }
            WM_TIMER if wparam.0 == TIMER_CARET => self.blink(),
            WM_TIMER if wparam.0 == TIMER_MATCH => {
                self.kill_match_timer();
                if self.matching() {
                    self.resume_matching();
                }
            }
            WM_SCAN => self.on_scan(),
            WM_HOOK => self.on_hook_events(),
            WM_DPICHANGED => self.on_dpi((wparam.0 & 0xFFFF) as u32),
            WM_MOUSEMOVE => self.on_mouse_move(lparam),
            WM_MOUSELEAVE => {
                self.hover = None;
                self.invalidate();
            }
            WM_LBUTTONDOWN => self.on_left_down(lparam),
            WM_LBUTTONUP => self.on_left_up(lparam),
            WM_RBUTTONUP => self.on_right_up(lparam),
            WM_CONTEXTMENU if lparam.0 == -1 => self.on_menu_key(),
            WM_MOUSEWHEEL => {
                self.settle();
                let delta = (wparam.0 >> 16) as i16;
                self.step(if delta > 0 { -1 } else { 1 });
            }
            _ => return None,
        }
        Some(LRESULT(0))
    }

    /// What a deferred action needs once the launcher is no longer borrowed.
    fn take_pick(&mut self) -> Option<Pick> {
        let (row, menu) = match self.deferred.take()? {
            Deferred::Activate(i) => (i, None),
            Deferred::Menu(i, x, y) => (i, Some((x, y))),
        };
        let e = self.entry(self.rows.get(row)?);
        Some(Pick {
            hwnd: self.hwnd,
            owner: self.opts.owner,
            command_msg: self.opts.command_msg,
            action: e.action.clone(),
            target: e.target(),
            key: e.key.clone(),
            name: e.name.clone(),
            menu,
        })
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        if let Some(scan) = &self.scan {
            scan.cancel.store(true, Ordering::Relaxed);
        }
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            CoUninitialize();
        }
    }
}

struct Pick {
    hwnd: HWND,
    owner: isize,
    command_msg: u32,
    action: Action,
    target: Option<String>,
    key: String,
    name: String,
    menu: Option<(i32, i32)>,
}

fn run_deferred() {
    let Some(p) = with_launcher(Launcher::take_pick).flatten() else { return };
    let done = match (p.menu, &p.action, &p.target) {
        (Some((x, y)), Action::Window { hwnd: h, .. }, _) => {
            winlist::system_menu(hwnd(*h), x, y);
            true
        }
        (Some((x, y)), _, Some(target)) => launch::context_menu(p.hwnd, target, x, y),
        (Some(_), _, None) => false,
        (None, _, Some(target)) => launch::open(p.hwnd, target, ""),
        (None, Action::Run { command, args }, None) => launch::open(p.hwnd, command, args),
        (None, Action::Window { hwnd: h, .. }, None) => {
            winlist::activate_or_minimize(hwnd(*h), HWND::default());
            true
        }
        (None, Action::App(id), None) => {
            post(p.owner, p.command_msg, *id as usize);
            true
        }
        (None, Action::EmptyRecycleBin, None) => {
            hide();
            launch::empty_recycle_bin(p.hwnd);
            true
        }
        (None, Action::Item | Action::Open(_), None) => false,
    };
    if done {
        if p.menu.is_none() && !p.key.is_empty() {
            with_history(|h| h.record(&p.key, &p.name, now()));
        }
        hide();
    }
}

unsafe extern "system" fn launcher_proc(h: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(r) = launch::forward_menu_message(msg, wparam, lparam) {
        return r;
    }
    match msg {
        WM_ERASEBKGND => return LRESULT(1),
        WM_TIMER if wparam.0 == TIMER_IDLE => {
            unsafe {
                let _ = KillTimer(Some(h), TIMER_IDLE);
            }
            unload();
            return LRESULT(0);
        }
        _ => {}
    }
    let r = with_launcher(|l| l.message(msg, wparam, lparam)).flatten();
    run_deferred();
    r.unwrap_or_else(|| unsafe { DefWindowProcW(h, msg, wparam, lparam) })
}
