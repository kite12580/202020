//! Spotlight-style search window. Lives on the main thread; everything but the
//! launch history is freed after `idle_unload_minutes` without an open.

mod clipboard;
mod index;
mod launch;
mod sources;

pub(crate) use launch::open;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use capsule_core::config::{Backdrop, Launcher as Config};
use capsule_core::launcher::history::{History, split_key};
use capsule_core::launcher::input::TextInput;
use capsule_core::launcher::rank::{Match, Searcher, rank, top};
use capsule_core::launcher::{Source, query};
use capsule_core::layout::Rect;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, DeleteObject, HBITMAP, HGDIOBJ, InvalidateRect, ValidateRect};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentProcess, INFINITE, SetProcessWorkingSetSize};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::Ime::{
    CANDIDATEFORM, CFS_CANDIDATEPOS, CFS_POINT, COMPOSITIONFORM, ImmGetContext, ImmReleaseContext,
    ImmSetCandidateWindow, ImmSetCompositionWindow,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_A, VK_BACK, VK_C, VK_CONTROL, VK_DELETE, VK_DOWN,
    VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP, VK_V, VK_X,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCaretBlinkTime, GetCursorPos, KillTimer, SW_HIDE, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, WA_INACTIVE, WM_ACTIVATE,
    WM_APP, WM_CHAR, WM_CLOSE, WM_CONTEXTMENU, WM_DPICHANGED, WM_ERASEBKGND, WM_IME_STARTCOMPOSITION, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::capsule::config_store;
use crate::capsule::gfx::{Color, Font, Gfx, Image, Surface, Text};
use crate::capsule::icons::IconWorker;
use crate::capsule::theme::{self, Palette, Theme};
use crate::capsule::util::{hwnd, lparam_point, primary_monitor, register_class, scale};
use crate::capsule::windows_list as winlist;
use index::{Action, Batch, Entry};

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
const WM_ICON: u32 = WM_APP + 2;
const TIMER_IDLE: usize = 1;
const TIMER_CARET: usize = 2;

const WIDTH: f32 = 640.0;
const PAD: f32 = 8.0;
const SEARCH_H: f32 = 56.0;
const TEXT_X: f32 = 52.0;
const ROW_H: f32 = 44.0;
const ICON: f32 = 24.0;
const NAME_X: f32 = PAD + 48.0;
const TAG_W: f32 = 72.0;
const LIST_TOP: f32 = SEARCH_H + 1.0 + PAD / 2.0;
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

struct Row {
    list: usize,
    index: usize,
    marks: Vec<u32>,
    icon: Option<String>,
    name: Option<Text>,
    sub: Option<Text>,
    tag: Option<Text>,
    glyph: Option<Text>,
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
    scan: Option<(Receiver<Batch>, Arc<AtomicBool>)>,
    scan_started: Option<Instant>,
    /// Matches for `found_query`, which a longer query only narrows.
    found: Vec<(usize, usize, Match)>,
    found_query: Option<(Option<Source>, String)>,
    rows: Vec<Row>,
    selected: usize,
    hover: Option<usize>,
    pressed: Option<usize>,
    icons: IconWorker,
    images: HashMap<String, Option<Image>>,
    pending: HashMap<u64, String>,
    next_icon: u64,
    caret_on: bool,
    deferred: Option<Deferred>,
}

fn row_rect(i: usize) -> Rect {
    Rect { x: PAD, y: LIST_TOP + i as f32 * ROW_H, w: WIDTH - 2.0 * PAD, h: ROW_H }
}

/// An entry for a history key whose source hasn't been scanned yet.
fn remembered(key: &str, name: &str) -> Option<Entry> {
    let (source, id) = split_key(key)?;
    match source {
        Source::App => Some(index::app_entry(name.to_string(), id)),
        Source::Folder => {
            let path = id.trim_end_matches('\\');
            Some(index::file_entry(path, name.to_string(), path.len() != id.len()))
        }
        Source::Window | Source::Command => None,
    }
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
            dpi: unsafe { GetDpiForWindow(hwnd) }.max(96),
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
            scan: None,
            scan_started: None,
            found: Vec::new(),
            found_query: None,
            rows: Vec::new(),
            selected: 0,
            hover: None,
            pressed: None,
            icons: IconWorker::new(hwnd.0 as isize, WM_ICON),
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
        let scanned = |c: &Config| (c.apps.enabled, c.folders.enabled, c.folders.roots.clone());
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
        self.lists_changed();
        if rescan && let Some((_, cancel)) = self.scan.take() {
            cancel.store(true, Ordering::Relaxed);
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
        self.found = Vec::new();
        self.found_query = None;
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

    fn start_scan(&mut self) {
        if self.scan.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        index::spawn_scan(self.cfg.clone(), tx, self.hwnd.0 as isize, WM_SCAN, cancel.clone());
        self.scan = Some((rx, cancel));
        self.scan_started = Some(Instant::now());
    }

    fn on_scan(&mut self) {
        let Some((rx, _)) = &self.scan else { return };
        let batches: Vec<Batch> = rx.try_iter().collect();
        for b in batches {
            let list = list_of(b.source);
            self.lists[list] = b.entries;
            self.scanned[list] = true;
            if b.last {
                self.scan = None;
            }
        }
        self.lists_changed();
        if self.visible {
            let keep = self.rows.get(self.selected).map(|r| self.entry(r).key.clone());
            self.refresh();
            if let Some(k) = keep.filter(|k| !k.is_empty())
                && let Some(i) = self.rows.iter().position(|r| self.entry(r).key == k)
            {
                self.selected = i;
            }
        }
    }

    fn lists_changed(&mut self) {
        self.found_query = None;
        self.by_key.clear();
        let (lists, by_key) = (&self.lists, &mut self.by_key);
        with_history(|h| {
            for (l, list) in lists[..RECENT].iter().enumerate() {
                for (i, e) in list.iter().enumerate() {
                    if h.contains(&e.key) {
                        by_key.insert(e.key.clone(), (l, i));
                    }
                }
            }
        });
    }

    fn row(&self, list: usize, index: usize, marks: Vec<u32>) -> Row {
        let e = &self.lists[list][index];
        let room = WIDTH - NAME_X - TAG_W - PAD * 2.0;
        Row {
            list,
            index,
            marks,
            icon: e.icon_key(),
            name: self.gfx.text_fit(&e.name, Font::Title, room),
            sub: Some(e.subtitle()).filter(|s| !s.is_empty()).and_then(|s| self.gfx.text_fit(&s, Font::Small, room)),
            tag: self.gfx.text(e.source.label(), Font::Small),
            glyph: e.glyph.and_then(|g| self.gfx.text(g.encode_utf8(&mut [0; 4]), Font::Glyph)),
        }
    }

    /// Re-runs the query and resizes the window to fit the results.
    fn refresh(&mut self) {
        let text = self.input.text();
        self.search = self.gfx.text(&text, Font::Search);
        let (only, q) = query::split(&text, &self.cfg.prefixes);
        self.searcher.set_query(q);
        let wanted = |s: Source| only.is_none_or(|o| o == s);
        let k = self.cfg.max_results;
        let now = now();
        self.rows.clear();
        self.lists[RECENT].clear();
        if self.searcher.is_empty() {
            self.found.clear();
            self.found_query = None;
            let recent: Vec<(String, String)> =
                with_history(|h| h.top(now).into_iter().map(|(key, e)| (key.to_string(), e.name.clone())).collect());
            for (key, name) in recent {
                if self.rows.len() >= k {
                    break;
                }
                let Some((source, _)) = split_key(&key) else { continue };
                if !wanted(source) || !self.enabled(source) {
                    continue;
                }
                if let Some(&(list, index)) = self.by_key.get(&key) {
                    let row = self.row(list, index, Vec::new());
                    self.rows.push(row);
                } else if !self.scanned[list_of(source)]
                    && let Some(e) = remembered(&key, &name)
                {
                    self.lists[RECENT].push(e);
                    let row = self.row(RECENT, self.lists[RECENT].len() - 1, Vec::new());
                    self.rows.push(row);
                }
            }
            if let Some(source) = only {
                let list = list_of(source);
                for index in 0..self.lists[list].len() {
                    if self.rows.len() >= k {
                        break;
                    }
                    if !self.rows.iter().any(|r| r.list == list && r.index == index) {
                        let row = self.row(list, index, Vec::new());
                        self.rows.push(row);
                    }
                }
            }
        } else {
            let narrow = self
                .found_query
                .as_ref()
                .is_some_and(|(o, prev)| *o == only && q.starts_with(prev.as_str()) && !prev.ends_with('\\'));
            let prev = std::mem::take(&mut self.found);
            let mut found = Vec::new();
            let mut ranked: Vec<(f64, usize, usize)> = Vec::new();
            let weights: Vec<Option<f32>> =
                SOURCES.iter().map(|&s| (wanted(s) && self.enabled(s)).then(|| self.weight(s))).collect();
            let (lists, searcher) = (&self.lists, &mut self.searcher);
            with_history(|h| {
                let mut consider = |l: usize, i: usize| {
                    let Some(weight) = weights[l] else { return };
                    let e = &lists[l][i];
                    if let Some(m) = searcher.score(&e.keys) {
                        let boost = if e.key.is_empty() { 1.0 } else { h.boost(&e.key, now) };
                        ranked.push((rank(m.score, weight, boost), e.name.len(), found.len()));
                        found.push((l, i, m));
                    }
                };
                if narrow {
                    prev.iter().for_each(|&(l, i, _)| consider(l, i));
                } else {
                    for (l, list) in lists[..RECENT].iter().enumerate() {
                        if weights[l].is_some() {
                            (0..list.len()).for_each(|i| consider(l, i));
                        }
                    }
                }
            });
            for f in top(&mut ranked, k) {
                let (list, index, m) = found[f];
                let marks = self.searcher.highlight(&self.lists[list][index].keys, m);
                let row = self.row(list, index, marks);
                self.rows.push(row);
            }
            self.found = found;
            self.found_query = Some((only, q.to_string()));
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        self.request_icons();
        self.place();
        self.invalidate();
    }

    fn height(&self) -> f32 {
        if self.rows.is_empty() { SEARCH_H } else { LIST_TOP + self.rows.len() as f32 * ROW_H + PAD }
    }

    fn place(&mut self) {
        let mon = primary_monitor();
        let (w, h) = (self.px(WIDTH), self.px(self.height()));
        let x = (mon.left + mon.right - w) / 2;
        let y = mon.top + (mon.bottom - mon.top) * 22 / 100;
        if self.bounds == (x, y, w, h) {
            return;
        }
        self.bounds = (x, y, w, h);
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
        }
        if let Some(s) = &mut self.surface {
            s.resize(w as u32, h as u32);
        }
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn edited(&mut self) {
        self.selected = 0;
        self.caret_on = true;
        self.refresh();
        self.place_ime();
    }

    fn caret_offset(&self) -> f32 {
        self.search.as_ref().map_or(0.0, |t| t.x_at(self.input.caret))
    }

    /// Left edge of the search text, shifted so the caret stays in view.
    fn text_origin(&self) -> f32 {
        let room = WIDTH - TEXT_X - PAD * 2.0;
        TEXT_X - (self.caret_offset() - room).max(0.0)
    }

    fn place_ime(&self) {
        let pt = POINT { x: self.px(self.text_origin() + self.caret_offset()), y: self.px(SEARCH_H / 2.0 - 12.0) };
        unsafe {
            let imc = ImmGetContext(self.hwnd);
            if imc.is_invalid() {
                return;
            }
            let comp = COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: pt, ..Default::default() };
            let _ = ImmSetCompositionWindow(imc, &comp);
            let below = POINT { x: pt.x, y: self.px(SEARCH_H / 2.0 + 14.0) };
            let cand =
                CANDIDATEFORM { dwIndex: 0, dwStyle: CFS_CANDIDATEPOS, ptCurrentPos: below, ..Default::default() };
            let _ = ImmSetCandidateWindow(imc, &cand);
            let _ = ImmReleaseContext(self.hwnd, imc);
        }
    }

    fn row_at(&self, y: f32) -> Option<usize> {
        (0..self.rows.len()).find(|&i| {
            let r = row_rect(i);
            y >= r.y && y < r.y + r.h
        })
    }

    fn request_icons(&mut self) {
        let px = self.px(ICON) as u32;
        for row in &self.rows {
            let Some(key) = row.icon.as_ref().filter(|k| !self.images.contains_key(*k)) else { continue };
            let Some(name) = self.lists[row.list][row.index].icon() else { continue };
            let id = self.next_icon;
            self.next_icon += 1;
            self.images.insert(key.clone(), None);
            self.pending.insert(id, key.clone());
            self.icons.request(id, name, px);
        }
    }

    fn on_icon(&mut self, id: u64, bitmap: HBITMAP) {
        let image = if bitmap.is_invalid() { None } else { self.gfx.image_from_hbitmap(bitmap, self.px(ICON) as u32) };
        if !bitmap.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
        }
        if let Some(key) = self.pending.remove(&id) {
            self.images.insert(key, image);
            self.invalidate();
        }
    }

    fn paint_search(&self, s: &mut Surface, pal: &Palette) {
        let origin = self.text_origin();
        let (selection, caret) = (self.input.selection(), self.input.caret);
        s.fill(Rect { x: 0.0, y: 0.0, w: WIDTH, h: SEARCH_H }, pal.tint, 0.0);
        let cy = SEARCH_H / 2.0;
        if let Some(t) = &self.lens {
            s.text(t, 20.0, cy - t.height / 2.0, pal.text.alpha(0.6));
        }
        match &self.search {
            Some(t) if self.input.end() > 0 => {
                let ty = cy - t.height / 2.0;
                if selection.0 != selection.1 {
                    let (a, b) = (origin + t.x_at(selection.0), origin + t.x_at(selection.1));
                    s.fill(Rect { x: a, y: cy - 13.0, w: b - a, h: 26.0 }, pal.accent.alpha(0.35), 2.0);
                }
                s.text_colored(t, origin, ty, |i| {
                    if origin + t.x_at(i) < TEXT_X - 2.0 { Color(0.0, 0.0, 0.0, 0.0) } else { pal.text }
                });
                if self.caret_on {
                    s.fill(Rect { x: origin + t.x_at(caret), y: cy - 12.0, w: 1.5, h: 24.0 }, pal.text, 0.0);
                }
            }
            _ => {
                if let Some(p) = &self.placeholder {
                    s.text(p, TEXT_X, cy - p.height / 2.0, pal.text.alpha(0.4));
                }
                if self.caret_on {
                    s.fill(Rect { x: TEXT_X, y: cy - 12.0, w: 1.5, h: 24.0 }, pal.text, 0.0);
                }
            }
        }
    }

    fn blink(&mut self) {
        self.caret_on = !self.caret_on;
        let Some(theme) = self.theme else { return };
        let Some(mut s) = self.surface.take() else { return };
        s.begin_band(0.0, SEARCH_H);
        self.paint_search(&mut s, &theme.palette(1.0));
        s.end_band(0.0, SEARCH_H);
        self.surface = Some(s);
    }

    fn paint(&mut self) {
        unsafe {
            let _ = ValidateRect(Some(self.hwnd), None);
        }
        if !self.visible {
            return;
        }
        let (w, h) = (self.px(WIDTH) as u32, self.px(self.height()) as u32);
        if self.surface.is_none() {
            self.surface = Surface::new(self.hwnd, w, h, self.dpi);
        }
        let Some(theme) = self.theme else { return };
        let pal = theme.palette(1.0);
        let dim = pal.text.alpha(0.6);
        let accent = if theme.high_contrast { pal.text } else { pal.accent };
        let height = self.to_dip(h as i32);
        let Some(mut s) = self.surface.take() else { return };
        s.resize(w, h);
        s.begin();
        s.fill(Rect { x: 0.0, y: SEARCH_H, w: WIDTH, h: height - SEARCH_H }, pal.tint, 0.0);
        if !self.rows.is_empty() {
            s.fill(Rect { x: 0.0, y: SEARCH_H, w: WIDTH, h: 1.0 }, pal.text.alpha(0.08), 0.0);
        }
        for (i, row) in self.rows.iter().enumerate() {
            let r = row_rect(i);
            if i == self.selected {
                s.fill(r, pal.active, 6.0);
            } else if self.hover == Some(i) {
                s.fill(r, pal.hover, 6.0);
            }
            let rcy = r.y + r.h / 2.0;
            let icx = r.x + 12.0 + ICON / 2.0;
            if let Some(Some(img)) = row.icon.as_ref().and_then(|k| self.images.get(k)) {
                s.image(img, icx, rcy);
            } else if let Some(g) = &row.glyph {
                s.text(g, icx - g.width / 2.0, rcy - g.height / 2.0, dim);
            }
            let paint_name = |s: &mut Surface, t: &Text, y: f32| {
                s.text_colored(t, NAME_X, y, |c| {
                    if row.marks.binary_search(&(c as u32)).is_ok() { accent } else { pal.text }
                });
            };
            match (&row.name, &row.sub) {
                (Some(n), Some(st)) => {
                    let top = rcy - (n.height + st.height) / 2.0;
                    paint_name(&mut s, n, top);
                    s.text(st, NAME_X, top + n.height, dim);
                }
                (Some(n), None) => paint_name(&mut s, n, rcy - n.height / 2.0),
                _ => {}
            }
            if let Some(t) = &row.tag {
                s.text(t, r.x + r.w - 12.0 - t.width, rcy - t.height / 2.0, pal.text.alpha(0.45));
            }
        }
        self.paint_search(&mut s, &pal);
        s.end();
        self.surface = Some(s);
    }

    fn select(&mut self, i: usize) {
        if i != self.selected && i < self.rows.len() {
            self.selected = i;
            self.invalidate();
        }
    }

    fn step(&mut self, delta: isize) {
        let n = self.rows.len() as isize;
        if n > 0 {
            self.select((self.selected as isize + delta).rem_euclid(n) as usize);
        }
    }

    fn on_key(&mut self, vk: u16) -> bool {
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        let edit = |l: &mut Launcher, f: &dyn Fn(&mut TextInput)| {
            let before = l.input.text();
            f(&mut l.input);
            l.caret_on = true;
            if l.input.text() != before {
                l.edited();
            } else {
                l.place_ime();
                l.invalidate();
            }
        };
        match vk {
            v if v == VK_ESCAPE.0 => self.hide(),
            v if v == VK_RETURN.0 => self.deferred = Some(Deferred::Activate(self.selected)),
            v if v == VK_UP.0 => self.step(-1),
            v if v == VK_DOWN.0 => self.step(1),
            v if v == VK_TAB.0 => self.step(if shift { -1 } else { 1 }),
            v if v == VK_PRIOR.0 => self.select(0),
            v if v == VK_NEXT.0 => self.select(self.rows.len().saturating_sub(1)),
            v if v == VK_LEFT.0 => edit(self, &|t| t.left(ctrl, shift)),
            v if v == VK_RIGHT.0 => edit(self, &|t| t.right(ctrl, shift)),
            v if v == VK_HOME.0 => edit(self, &|t| t.move_to(0, shift)),
            v if v == VK_END.0 => edit(self, &|t| t.move_to(t.end(), shift)),
            v if v == VK_BACK.0 => edit(self, &|t| t.backspace(ctrl)),
            v if v == VK_DELETE.0 => edit(self, &|t| t.delete(ctrl)),
            v if ctrl && v == VK_A.0 => edit(self, &|t| t.select_all()),
            v if ctrl && v == VK_C.0 => clipboard::set(self.hwnd, &self.input.selected_text()),
            v if ctrl && v == VK_X.0 => {
                clipboard::set(self.hwnd, &self.input.selected_text());
                edit(self, &|t| {
                    t.delete_selection();
                });
            }
            v if ctrl && v == VK_V.0 => {
                let pasted = clipboard::get(self.hwnd).unwrap_or_default();
                let line = pasted.lines().next().unwrap_or("").to_string();
                edit(self, &|t| t.insert(&line));
            }
            _ => return false,
        }
        true
    }

    fn on_char(&mut self, unit: u16) {
        let c = match (self.high_surrogate.take(), unit) {
            (_, 0xD800..=0xDBFF) => {
                self.high_surrogate = Some(unit);
                return;
            }
            (Some(hi), 0xDC00..=0xDFFF) => char::decode_utf16([hi, unit]).next().and_then(Result::ok),
            (_, u) => char::from_u32(u32::from(u)),
        };
        let Some(c) = c.filter(|c| !c.is_control()) else { return };
        self.input.insert(&c.to_string());
        self.edited();
    }

    fn caret_from_x(&mut self, x: f32, extend: bool) {
        let origin = self.text_origin();
        let Some(t) = &self.search else { return };
        let i = (0..=t.len())
            .min_by(|&a, &b| (origin + t.x_at(a) - x).abs().total_cmp(&(origin + t.x_at(b) - x).abs()))
            .unwrap_or(0);
        self.input.move_to(i, extend);
        self.caret_on = true;
        self.place_ime();
        self.invalidate();
    }

    fn menu_point(&self, i: usize) -> (i32, i32) {
        let r = row_rect(i);
        let mut pt = POINT { x: self.px(NAME_X), y: self.px(r.y + r.h) };
        unsafe {
            let _ = ClientToScreen(self.hwnd, &mut pt);
        }
        (pt.x, pt.y)
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
            WM_SCAN => self.on_scan(),
            WM_ICON => self.on_icon(wparam.0 as u64, HBITMAP(lparam.0 as *mut _)),
            WM_DPICHANGED => {
                self.dpi = (wparam.0 & 0xFFFF) as u32;
                if let Some(s) = &mut self.surface {
                    s.set_dpi(self.dpi);
                }
                self.images.clear();
                self.pending.clear();
                self.request_icons();
                self.place();
                self.invalidate();
            }
            WM_MOUSEMOVE => {
                let (_, y) = lparam_point(lparam);
                let row = self.row_at(self.to_dip(y));
                if row != self.hover {
                    self.hover = row;
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut tme);
                    }
                    self.invalidate();
                }
            }
            WM_MOUSELEAVE => {
                self.hover = None;
                self.invalidate();
            }
            WM_LBUTTONDOWN => {
                let (x, y) = lparam_point(lparam);
                let (x, y) = (self.to_dip(x), self.to_dip(y));
                if y < SEARCH_H {
                    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
                    self.caret_from_x(x, shift);
                } else {
                    self.pressed = self.row_at(y);
                    if let Some(i) = self.pressed {
                        self.select(i);
                    }
                }
            }
            WM_LBUTTONUP => {
                let (_, y) = lparam_point(lparam);
                let row = self.row_at(self.to_dip(y));
                if let Some(i) = row.filter(|&i| self.pressed.take() == Some(i)) {
                    self.deferred = Some(Deferred::Activate(i));
                }
            }
            WM_RBUTTONUP => {
                let (_, y) = lparam_point(lparam);
                if let Some(i) = self.row_at(self.to_dip(y)) {
                    self.select(i);
                    let mut pt = POINT::default();
                    unsafe {
                        let _ = GetCursorPos(&mut pt);
                    }
                    self.deferred = Some(Deferred::Menu(i, pt.x, pt.y));
                }
            }
            WM_CONTEXTMENU => {
                if lparam.0 == -1 && self.selected < self.rows.len() {
                    let (x, y) = self.menu_point(self.selected);
                    self.deferred = Some(Deferred::Menu(self.selected, x, y));
                }
            }
            WM_MOUSEWHEEL => {
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
        if let Some((_, cancel)) = &self.scan {
            cancel.store(true, Ordering::Relaxed);
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
            launch::post(p.owner, p.command_msg, *id);
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
