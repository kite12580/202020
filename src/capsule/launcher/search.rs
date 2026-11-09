use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::time::Instant;

use capsule_core::launcher::history::split_key;
use capsule_core::launcher::rank::{rank, top};
use capsule_core::launcher::{Source, query};
use capsule_core::shell_proto::{AppInfo, Request};
use windows::Win32::UI::WindowsAndMessaging::{
    GetQueueStatus, KillTimer, QS_KEY, QS_MOUSEBUTTON, SetTimer, USER_TIMER_MINIMUM,
};

use super::index::{self, Batch, Entry};
use super::view::{NAME_X, PAD, TAG_W, WIDTH};
use super::{Launcher, RECENT, SOURCES, Scan, TIMER_MATCH, WM_SCAN, list_of, now, with_history};
use crate::capsule::gfx::{Font, Text};
use crate::capsule::hook;

/// Entries scored between checks for pending input.
const MATCH_CHUNK: u32 = 512;

pub(super) struct Row {
    pub(super) list: usize,
    pub(super) index: usize,
    pub(super) marks: Vec<u32>,
    pub(super) icon: Option<String>,
    pub(super) name: Option<Text>,
    pub(super) sub: Option<Text>,
    pub(super) tag: Option<Text>,
    pub(super) glyph: Option<Text>,
}

/// An entry for a history key whose source hasn't been scanned yet.
fn remembered(key: &str, name: &str) -> Option<Entry> {
    let (source, id) = split_key(key)?;
    match source {
        Source::App => Some(index::app_entry(name.to_string(), id)),
        Source::Folder => {
            let (path, dir) = index::split_dir(id);
            Some(index::file_entry(path, name.to_string(), dir))
        }
        Source::Window | Source::Command => None,
    }
}

impl Launcher {
    pub(super) fn start_scan(&mut self) {
        if self.scan.is_some() {
            return;
        }
        let roots = if self.cfg.folders.enabled { self.cfg.folders.roots.len() } else { 0 };
        if roots < self.root_lens.len() {
            self.root_lens.truncate(roots);
            self.lists[list_of(Source::Folder)].truncate(self.root_lens.iter().sum());
            self.list_changed(list_of(Source::Folder));
        }
        self.root_lens.resize(roots, 0);
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        index::spawn_scan(self.cfg.clone(), tx, self.hwnd.0 as isize, WM_SCAN, cancel.clone());
        self.scan = Some(Scan { rx, cancel });
        self.scan_started = Some(Instant::now());
        self.apps_pending = self.cfg.apps.enabled;
        if self.apps_pending {
            self.request_apps();
        } else {
            self.apply(vec![Batch { source: Source::App, root: 0, entries: Vec::new(), last: true }]);
        }
    }

    pub(super) fn request_apps(&self) {
        hook::request(self.hwnd, &Request::Apps);
    }

    pub(super) fn on_scan(&mut self) {
        let Some(scan) = &self.scan else { return };
        let batches: Vec<Batch> = scan.rx.try_iter().collect();
        if batches.iter().any(|b| b.last) {
            self.scan = None;
        }
        self.apply(batches);
    }

    pub(super) fn on_apps(&mut self, apps: Vec<AppInfo>) {
        if !std::mem::take(&mut self.apps_pending) {
            return;
        }
        let entries = apps.into_iter().map(|a| index::app_entry(a.name, &a.id)).collect();
        self.apply(vec![Batch { source: Source::App, root: 0, entries, last: true }]);
    }

    pub(super) fn apply(&mut self, batches: Vec<Batch>) {
        if batches.is_empty() {
            return;
        }
        let keep = self.rows.get(self.selected).map(|r| self.entry(r).key.clone());
        let mut changed = [false; RECENT];
        for b in batches {
            let list = list_of(b.source);
            if b.source == Source::Folder {
                self.set_root(b.root, b.entries);
            } else {
                self.lists[list] = b.entries;
            }
            if b.last {
                self.scanned[list] = true;
            }
            changed[list] = true;
        }
        for list in (0..RECENT).filter(|&l| changed[l]) {
            self.list_changed(list);
        }
        if self.visible {
            self.refresh();
            if let Some(k) = keep.filter(|k| !k.is_empty())
                && let Some(i) = self.rows.iter().position(|r| self.entry(r).key == k)
            {
                self.selected = i;
            }
        }
    }

    fn set_root(&mut self, root: usize, entries: Vec<Entry>) {
        let Some(&len) = self.root_lens.get(root) else { return };
        let start: usize = self.root_lens[..root].iter().sum();
        self.root_lens[root] = entries.len();
        self.lists[list_of(Source::Folder)].splice(start..start + len, entries);
    }

    /// Drops what refers to positions in `list`.
    pub(super) fn list_changed(&mut self, list: usize) {
        self.stop_matching();
        let rows = self.rows.len();
        self.rows.retain(|r| r.list != list);
        if self.rows.len() != rows {
            self.selected = self.selected.min(self.rows.len().saturating_sub(1));
            self.hover = None;
            if self.visible {
                self.place();
                self.invalidate();
            }
        }
        self.by_key.retain(|_, &mut (l, _)| l != list);
        let (entries, by_key) = (&self.lists[list], &mut self.by_key);
        with_history(|h| {
            for (i, e) in entries.iter().enumerate() {
                if h.contains(&e.key) {
                    by_key.insert(e.key.clone(), (list, i));
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

    /// Re-runs the query and resizes the window to fit the results. A search
    /// through many entries yields to pending input and finishes on
    /// `TIMER_MATCH`.
    pub(super) fn refresh(&mut self) {
        let text = self.input.text();
        self.search = self.gfx.text(&text, Font::Search);
        let (only, q) = query::split(&text, &self.cfg.prefixes);
        self.searcher.set_query(q);
        if self.searcher.is_empty() {
            self.stop_matching();
            self.rows.clear();
            self.lists[RECENT].clear();
            self.list_recent(only);
            self.rows_changed();
            return;
        }
        let narrow = self
            .found_query
            .as_ref()
            .is_some_and(|(o, prev)| *o == only && q.starts_with(prev.as_str()) && !prev.ends_with('\\'));
        if narrow {
            self.recheck.extend(self.found.drain(..).map(|(l, i, _)| (l, i)));
        } else {
            self.stop_matching();
            self.next = (0, 0);
        }
        self.found_query = Some((only, q.to_string()));
        self.resume_matching();
        if self.matching() {
            self.repaint_search();
        }
    }

    fn list_recent(&mut self, only: Option<Source>) {
        let wanted = |s: Source| only.is_none_or(|o| o == s);
        let k = self.cfg.max_results;
        let recent: Vec<(String, String)> =
            with_history(|h| h.top(now()).into_iter().map(|(key, e)| (key.to_string(), e.name.clone())).collect());
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
    }

    pub(super) fn matching(&self) -> bool {
        self.found_query.is_some() && (!self.recheck.is_empty() || self.next.0 < RECENT)
    }

    pub(super) fn kill_match_timer(&self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_MATCH);
        }
    }

    pub(super) fn stop_matching(&mut self) {
        self.kill_match_timer();
        self.found_query = None;
        self.found.clear();
        self.recheck.clear();
    }

    pub(super) fn resume_matching(&mut self) {
        if self.score_entries(true) {
            self.show_matches();
        } else {
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_MATCH, USER_TIMER_MINIMUM, None);
            }
        }
    }

    /// Completes a search that yielded, so keys act on its results.
    pub(super) fn settle(&mut self) {
        if self.matching() {
            self.kill_match_timer();
            self.score_entries(false);
            self.show_matches();
        }
    }

    /// Scores entries for `found_query`. Returns false if it stopped for
    /// pending key or mouse button input.
    fn score_entries(&mut self, yield_to_input: bool) -> bool {
        let only = self.found_query.as_ref().and_then(|(o, _)| *o);
        let wanted = SOURCES.map(|s| only.is_none_or(|o| o == s) && self.enabled(s));
        let (lists, searcher, found) = (&self.lists, &mut self.searcher, &mut self.found);
        let mut consider = |l: usize, i: usize| {
            if wanted[l]
                && let Some(m) = searcher.score(&lists[l][i].keys)
            {
                found.push((l, i, m));
            }
        };
        let mut budget = MATCH_CHUNK;
        loop {
            budget -= 1;
            if budget == 0 {
                budget = MATCH_CHUNK;
                if yield_to_input && unsafe { GetQueueStatus(QS_KEY | QS_MOUSEBUTTON) } >> 16 != 0 {
                    return false;
                }
            }
            if let Some((l, i)) = self.recheck.pop() {
                consider(l, i);
                continue;
            }
            let (l, i) = self.next;
            if l >= RECENT {
                return true;
            }
            if !wanted[l] || i >= lists[l].len() {
                self.next = (l + 1, 0);
                continue;
            }
            consider(l, i);
            self.next.1 += 1;
        }
    }

    fn show_matches(&mut self) {
        let now = now();
        let weights = SOURCES.map(|s| self.weight(s));
        let (lists, found) = (&self.lists, &self.found);
        let mut ranked: Vec<(f64, usize, usize)> = with_history(|h| {
            found
                .iter()
                .enumerate()
                .map(|(n, &(l, i, m))| {
                    let e = &lists[l][i];
                    let boost = if e.key.is_empty() { 1.0 } else { h.boost(&e.key, now) };
                    (rank(m.score, weights[l], boost), e.name.len(), n)
                })
                .collect()
        });
        self.rows.clear();
        self.lists[RECENT].clear();
        for f in top(&mut ranked, self.cfg.max_results) {
            let (list, index, m) = self.found[f];
            let marks = self.searcher.highlight(&self.lists[list][index].keys, m);
            let row = self.row(list, index, marks);
            self.rows.push(row);
        }
        self.rows_changed();
    }

    fn rows_changed(&mut self) {
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        self.request_icons();
        self.place();
        self.invalidate();
    }
}
