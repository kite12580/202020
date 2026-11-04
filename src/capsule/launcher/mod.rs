//! Spotlight-style search window. Lives on the main thread; everything but the
//! launch history is freed after `idle_unload_minutes` without an open.

mod clipboard;
mod index;
mod launch;
mod sources;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use capsule_core::config::{Appearance, Launcher as Config};
use capsule_core::launcher::history::{History, split_key};
use capsule_core::launcher::input::TextInput;
use capsule_core::launcher::rank::{Match, Searcher, rank, top};
use capsule_core::launcher::{Source, query};
use capsule_core::layout::Rect;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, DeleteObject, HBITMAP, HGDIOBJ, InvalidateRect, ValidateRect};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
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
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCaretBlinkTime, GetCursorPos, IDC_ARROW, KillTimer, LoadCursorW,
    RegisterClassExW, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow, SetTimer, SetWindowPos,
    ShowWindow, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_CHAR, WM_CLOSE, WM_CONTEXTMENU, WM_DPICHANGED, WM_ERASEBKGND,
    WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT,
    WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::capsule::config_store;
use crate::capsule::gfx::{Color, Font, Gfx, Image, Surface};
use crate::capsule::icons::IconWorker;
use crate::capsule::theme::{self, Theme};
use crate::capsule::util::{apps_folder, lparam_point, primary_monitor, scale};
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
const DIR_GLYPH: char = '\u{E8B7}';

/// Index into `Launcher::lists`.
const RECENT: usize = 4;

fn list_of(source: Source) -> usize {
    match source {
        Source::App => 0,
        Source::Window => 1,
        Source::Command => 2,
        Source::Folder => 3,
    }
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

/// Writes the launch history if it changed.
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
    appearance: Appearance,
    cfg_stamp: Option<SystemTime>,
    visible: bool,
    input: TextInput,
    high_surrogate: Option<u16>,
    searcher: Searcher,
    /// Apps, windows, commands, folders, and history entries not in the others.
    lists: [Vec<Entry>; 5],
    by_key: HashMap<String, (usize, usize)>,
    scanned: [bool; 4],
    generation: u64,
    scan: Option<(Receiver<Batch>, Arc<AtomicBool>)>,
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

fn register_class() {
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(launcher_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&class);
    }
}

fn icon_key(e: &Entry) -> String {
    if e.source != Source::Folder {
        return e.icon.clone();
    }
    if e.glyph == Some(DIR_GLYPH) {
        return "<folder>".into();
    }
    match e.name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()) {
        Some(ext) if !matches!(ext.as_str(), "exe" | "lnk" | "ico" | "url" | "appref-ms" | "msc" | "cpl") => {
            format!(".{ext}")
        }
        _ => e.icon.clone(),
    }
}

/// An entry for a history key whose source hasn't been scanned yet.
fn remembered(key: &str, name: &str) -> Option<Entry> {
    let (source, id) = split_key(key)?;
    match source {
        Source::App => {
            let target = apps_folder(id);
            Some(Entry {
                source,
                key: key.to_string(),
                keys: capsule_core::launcher::haystack::Keys::new(name, &[]),
                name: name.to_string(),
                subtitle: String::new(),
                icon: target.clone(),
                glyph: None,
                action: Action::Open(target),
            })
        }
        Source::Folder => Some(index::file_entry(id.to_string(), name.to_string(), false)),
        Source::Window | Source::Command => None,
    }
}

impl Launcher {
    fn create(opts: Options) -> Option<Launcher> {
        register_class();
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
        let (cfg, appearance) = config_store::launcher();
        Some(Launcher {
            opts,
            hwnd,
            dpi: unsafe { GetDpiForWindow(hwnd) }.max(96),
            gfx: Gfx::new(),
            surface: None,
            theme: None,
            cfg,
            appearance,
            cfg_stamp: config_store::modified(),
            visible: false,
            input: TextInput::default(),
            high_surrogate: None,
            searcher: Searcher::default(),
            lists: Default::default(),
            by_key: HashMap::new(),
            scanned: [false; 4],
            generation: 0,
            scan: None,
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

    fn reload_config(&mut self) {
        let stamp = config_store::modified();
        if stamp == self.cfg_stamp {
            return;
        }
        self.cfg_stamp = stamp;
        (self.cfg, self.appearance) = config_store::launcher();
        self.theme = None;
    }

    fn show(&mut self) {
        self.reload_config();
        let theme = Theme::read();
        if self.theme != Some(theme) {
            theme::apply(self.hwnd, self.appearance.backdrop, &theme);
            self.theme = Some(theme);
        }
        self.lists[list_of(Source::Window)] = if self.cfg.windows.enabled { sources::windows() } else { Vec::new() };
        self.lists[list_of(Source::Command)] = if self.cfg.commands.enabled {
            sources::commands(self.opts.commands, &self.cfg.commands.custom)
        } else {
            Vec::new()
        };
        self.rebuild_keys();
        self.start_scan();
        self.input.select_all();
        self.selected = 0;
        self.hover = None;
        self.refresh();
        self.visible = true;
        self.caret_on = true;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_IDLE);
            SetTimer(Some(self.hwnd), TIMER_CARET, GetCaretBlinkTime().clamp(200, 2000), None);
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
        self.rebuild_keys();
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
        self.generation += 1;
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        index::spawn_scan(self.cfg.clone(), self.generation, tx, self.hwnd.0 as isize, WM_SCAN, cancel.clone());
        self.scan = Some((rx, cancel));
    }

    fn on_scan(&mut self) {
        let Some((rx, _)) = &self.scan else { return };
        let batches: Vec<Batch> = rx.try_iter().collect();
        for b in batches {
            if b.generation != self.generation {
                continue;
            }
            let list = list_of(b.source);
            self.lists[list] = b.entries;
            self.scanned[list] = true;
            if b.last {
                self.scan = None;
            }
        }
        self.rebuild_keys();
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

    fn rebuild_keys(&mut self) {
        self.by_key.clear();
        for (l, list) in self.lists[..RECENT].iter().enumerate() {
            for (i, e) in list.iter().enumerate() {
                if !e.key.is_empty() {
                    self.by_key.insert(e.key.clone(), (l, i));
                }
            }
        }
    }

    /// Re-runs the query and resizes the window to fit the results.
    fn refresh(&mut self) {
        let text = self.input.text();
        let (only, q) = query::split(&text, &self.cfg.prefixes);
        self.searcher.set_query(q);
        let wanted = |s: Source| only.is_none_or(|o| o == s);
        let k = self.cfg.max_results;
        let now = now();
        self.rows.clear();
        self.lists[RECENT].clear();
        if self.searcher.is_empty() {
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
                    self.rows.push(Row { list, index, marks: Vec::new() });
                } else if !self.scanned[list_of(source)]
                    && let Some(e) = remembered(&key, &name)
                {
                    self.lists[RECENT].push(e);
                    self.rows.push(Row { list: RECENT, index: self.lists[RECENT].len() - 1, marks: Vec::new() });
                }
            }
            if let Some(source) = only {
                let list = list_of(source);
                for index in 0..self.lists[list].len() {
                    if self.rows.len() >= k {
                        break;
                    }
                    if !self.rows.iter().any(|r| r.list == list && r.index == index) {
                        self.rows.push(Row { list, index, marks: Vec::new() });
                    }
                }
            }
        } else {
            let mut found: Vec<(usize, usize, Match)> = Vec::new();
            let mut ranked: Vec<(f64, usize, usize)> = Vec::new();
            let sources: Vec<Option<f32>> = self.lists[..RECENT]
                .iter()
                .map(|list| {
                    let source = list.first()?.source;
                    (wanted(source) && self.enabled(source)).then(|| self.weight(source))
                })
                .collect();
            let (lists, searcher) = (&self.lists, &mut self.searcher);
            with_history(|h| {
                for (l, list) in lists[..RECENT].iter().enumerate() {
                    let Some(weight) = sources[l] else { continue };
                    for (i, e) in list.iter().enumerate() {
                        if let Some(m) = searcher.score(&e.keys) {
                            let boost = if e.key.is_empty() { 1.0 } else { h.boost(&e.key, now) };
                            ranked.push((rank(m.score, weight, boost), e.name.len(), found.len()));
                            found.push((l, i, m));
                        }
                    }
                }
            });
            for f in top(&mut ranked, k) {
                let (list, index, m) = found[f];
                let marks = self.searcher.highlight(&self.lists[list][index].keys, m);
                self.rows.push(Row { list, index, marks });
            }
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        self.place();
        self.invalidate();
    }

    fn height(&self) -> f32 {
        if self.rows.is_empty() { SEARCH_H } else { SEARCH_H + 1.0 + PAD / 2.0 + self.rows.len() as f32 * ROW_H + PAD }
    }

    fn place(&mut self) {
        let mon = primary_monitor();
        let (w, h) = (self.px(WIDTH), self.px(self.height()));
        let x = (mon.left + mon.right - w) / 2;
        let y = mon.top + (mon.bottom - mon.top) * 22 / 100;
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

    /// Left edge of the search text, shifted so the caret stays in view.
    fn text_origin(&self) -> f32 {
        let room = WIDTH - TEXT_X - PAD * 2.0;
        let caret = self.gfx.text(&self.input.text(), Font::Search).map_or(0.0, |t| t.x_at(self.input.caret));
        TEXT_X - (caret - room).max(0.0)
    }

    fn caret_x(&self) -> f32 {
        let t = self.gfx.text(&self.input.text(), Font::Search);
        self.text_origin() + t.map_or(0.0, |t| t.x_at(self.input.caret))
    }

    fn place_ime(&self) {
        let pt = POINT { x: self.px(self.caret_x()), y: self.px(SEARCH_H / 2.0 - 12.0) };
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

    fn row_rect(&self, i: usize) -> Rect {
        Rect { x: PAD, y: SEARCH_H + 1.0 + PAD / 2.0 + i as f32 * ROW_H, w: WIDTH - 2.0 * PAD, h: ROW_H }
    }

    fn row_at(&self, y: f32) -> Option<usize> {
        (0..self.rows.len()).find(|&i| {
            let r = self.row_rect(i);
            y >= r.y && y < r.y + r.h
        })
    }

    fn request_icon(&mut self, e_list: usize, e_index: usize) {
        let e = &self.lists[e_list][e_index];
        if e.icon.is_empty() {
            return;
        }
        let key = icon_key(e);
        if self.images.contains_key(&key) {
            return;
        }
        let (name, id, px) = (e.icon.clone(), self.next_icon, self.px(ICON) as u32);
        self.next_icon += 1;
        self.images.insert(key.clone(), None);
        self.pending.insert(id, key);
        self.icons.request(id, name, px);
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
        let wanted: Vec<(usize, usize)> = self.rows.iter().map(|r| (r.list, r.index)).collect();
        for (l, i) in wanted {
            self.request_icon(l, i);
        }
        let Some(theme) = self.theme else { return };
        let pal = theme.palette(self.appearance.opacity);
        let dim = pal.text.alpha(0.6);
        let text = self.input.text();
        let origin = self.text_origin();
        let search = self.gfx.text(&text, Font::Search);
        let placeholder = self.gfx.text("Search apps, files, windows and commands", Font::Search);
        let lens = self.gfx.text("\u{E721}", Font::SearchGlyph);
        let selection = self.input.selection();
        let caret = self.input.caret;
        let caret_on = self.caret_on;
        let ixs = self.rows.len();
        let mut rows = Vec::with_capacity(ixs);
        for (i, row) in self.rows.iter().enumerate() {
            let e = self.entry(row);
            let has_sub = !e.subtitle.is_empty();
            let name = self.gfx.text_fit(&e.name, Font::Title, WIDTH - NAME_X - TAG_W - PAD * 2.0);
            let sub = has_sub.then(|| self.gfx.text_fit(&e.subtitle, Font::Small, WIDTH - NAME_X - TAG_W - PAD * 2.0));
            let tag = self.gfx.text(e.source.label(), Font::Small);
            let image = self.images.get(&icon_key(e)).cloned().flatten();
            let glyph = match (&image, e.glyph) {
                (None, Some(g)) => self.gfx.text(&g.to_string(), Font::Glyph),
                _ => None,
            };
            rows.push((self.row_rect(i), name, sub.flatten(), tag, image, glyph, &row.marks));
        }
        let selected = self.selected;
        let hover = self.hover;
        let Some(s) = self.surface.as_mut() else { return };
        s.resize(w, h);
        s.begin();
        let height = h as f32 * 96.0 / self.dpi as f32;
        s.fill(Rect { x: 0.0, y: 0.0, w: WIDTH, h: height }, pal.tint, 0.0);
        let cy = SEARCH_H / 2.0;
        if let Some(t) = &lens {
            s.text(t, 20.0, cy - t.height / 2.0, dim);
        }
        match &search {
            Some(t) if !text.is_empty() => {
                let ty = cy - t.height / 2.0;
                if selection.0 != selection.1 {
                    let (a, b) = (origin + t.x_at(selection.0), origin + t.x_at(selection.1));
                    s.fill(Rect { x: a, y: cy - 13.0, w: b - a, h: 26.0 }, pal.accent.alpha(0.35), 2.0);
                }
                s.text_colored(t, origin, ty, |i| {
                    if origin + t.x_at(i) < TEXT_X - 2.0 { Color(0.0, 0.0, 0.0, 0.0) } else { pal.text }
                });
                if caret_on {
                    s.fill(Rect { x: origin + t.x_at(caret), y: cy - 12.0, w: 1.5, h: 24.0 }, pal.text, 0.0);
                }
            }
            _ => {
                if let Some(p) = &placeholder {
                    s.text(p, TEXT_X, cy - p.height / 2.0, pal.text.alpha(0.4));
                }
                if caret_on {
                    s.fill(Rect { x: TEXT_X, y: cy - 12.0, w: 1.5, h: 24.0 }, pal.text, 0.0);
                }
            }
        }
        if !rows.is_empty() {
            s.fill(Rect { x: 0.0, y: SEARCH_H, w: WIDTH, h: 1.0 }, pal.text.alpha(0.08), 0.0);
        }
        for (i, (r, name, sub, tag, image, glyph, marks)) in rows.iter().enumerate() {
            if i == selected {
                s.fill(*r, pal.active, 6.0);
            } else if hover == Some(i) {
                s.fill(*r, pal.hover, 6.0);
            }
            let rcy = r.y + r.h / 2.0;
            let icx = r.x + 12.0 + ICON / 2.0;
            if let Some(img) = image {
                s.image(img, icx, rcy);
            } else if let Some(g) = glyph {
                s.text(g, icx - g.width / 2.0, rcy - g.height / 2.0, dim);
            }
            let accent = if theme.high_contrast { pal.text } else { pal.accent };
            let paint_name =
                |s: &mut Surface, t: &crate::capsule::gfx::Text, y: f32| {
                    s.text_colored(t, NAME_X, y, |c| {
                        if marks.binary_search(&(c as u32)).is_ok() { accent } else { pal.text }
                    });
                };
            match (name, sub) {
                (Some(n), Some(st)) => {
                    let top = rcy - (n.height + st.height) / 2.0;
                    paint_name(s, n, top);
                    s.text(st, NAME_X, top + n.height, dim);
                }
                (Some(n), None) => paint_name(s, n, rcy - n.height / 2.0),
                _ => {}
            }
            if let Some(t) = tag {
                s.text(t, r.x + r.w - 12.0 - t.width, rcy - t.height / 2.0, pal.text.alpha(0.45));
            }
        }
        s.end();
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
        let Some(t) = self.gfx.text(&self.input.text(), Font::Search) else { return };
        let i = (0..=t.len())
            .min_by(|&a, &b| (origin + t.x_at(a) - x).abs().total_cmp(&(origin + t.x_at(b) - x).abs()))
            .unwrap_or(0);
        self.input.move_to(i, extend);
        self.caret_on = true;
        self.place_ime();
        self.invalidate();
    }

    fn menu_point(&self, i: usize) -> (i32, i32) {
        let r = self.row_rect(i);
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
            WM_TIMER if wparam.0 == TIMER_CARET => {
                self.caret_on = !self.caret_on;
                self.invalidate();
            }
            WM_SCAN => self.on_scan(),
            WM_ICON => self.on_icon(wparam.0 as u64, HBITMAP(lparam.0 as *mut _)),
            WM_DPICHANGED => {
                self.dpi = (wparam.0 & 0xFFFF) as u32;
                if let Some(s) = &mut self.surface {
                    s.set_dpi(self.dpi);
                }
                self.images.clear();
                self.pending.clear();
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
    key: String,
    name: String,
    menu: Option<(i32, i32)>,
}

fn run_deferred() {
    let Some(p) = with_launcher(Launcher::take_pick).flatten() else { return };
    let done = match (p.menu, &p.action) {
        (Some((x, y)), Action::Window(h)) => {
            winlist::system_menu(HWND(*h as *mut _), x, y);
            true
        }
        (Some((x, y)), Action::Open(target)) => launch::context_menu(p.hwnd, target, x, y),
        (Some(_), _) => false,
        (None, Action::Open(target)) => launch::open(p.hwnd, target, ""),
        (None, Action::Run { command, args }) => launch::open(p.hwnd, command, args),
        (None, Action::Window(h)) => {
            winlist::activate_or_minimize(HWND(*h as *mut _), HWND::default());
            true
        }
        (None, Action::App(id)) => {
            launch::post(p.owner, p.command_msg, *id);
            true
        }
        (None, Action::EmptyRecycleBin) => {
            hide();
            launch::empty_recycle_bin(p.hwnd);
            true
        }
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
