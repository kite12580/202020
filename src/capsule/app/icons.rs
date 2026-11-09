use std::collections::HashMap;

use capsule_core::shell_proto::{Bitmap, Request};
use windows::Win32::Foundation::{HWND, LRESULT};
use windows::Win32::UI::WindowsAndMessaging::HICON;

use super::{App, TIMER_ICONS, with_app};
use crate::capsule::gfx::Image;
use crate::capsule::hook;
use crate::capsule::tray::TrayKey;
use crate::capsule::util::{apps_folder, class_name};
use crate::capsule::windows_list::{self as winlist, IconRank};

const WINDOW_ICON: f32 = 20.0;
const TRAY_ICON: f32 = 16.0;
const FRAME_CLASS: &str = "ApplicationFrameWindow";

type IconKey = (String, u32);

enum IconTarget {
    Window(HWND, IconRank),
    Tray(TrayKey),
    Pin(String),
}

pub(super) struct IconRequests {
    requests: HashMap<u64, IconKey>,
    waiters: HashMap<IconKey, Vec<IconTarget>>,
    cache: HashMap<IconKey, Image>,
    next_request: u64,
    refresh: Vec<HWND>,
}

impl Default for IconRequests {
    fn default() -> Self {
        Self {
            requests: HashMap::new(),
            waiters: HashMap::new(),
            cache: HashMap::new(),
            next_request: 1,
            refresh: Vec::new(),
        }
    }
}

unsafe extern "system" fn on_window_icon(h: HWND, _msg: u32, small: usize, result: LRESULT) {
    with_app(|a| a.on_window_icon(h, small != 0, result.0));
}

impl App {
    fn window_icon_px(&self) -> u32 {
        self.px(WINDOW_ICON) as u32
    }

    pub(super) fn request_icon(&mut self, h: HWND) {
        let class_icon = winlist::class_icon(h);
        if class_icon != 0 {
            self.set_window_icon(h, class_icon, IconRank::Class);
        }
        let px = self.window_icon_px();
        if class_name(h) == FRAME_CLASS
            && let Some(id) = winlist::aumid(h)
        {
            self.request_icon_target(IconTarget::Window(h, IconRank::Package), apps_folder(&id), px);
            return;
        }
        winlist::request_window_icon(h, false, on_window_icon);
        if class_icon == 0
            && let Some(path) = self.windows.index(h).and_then(|i| self.windows.items[i].exe.clone())
        {
            self.request_icon_target(IconTarget::Window(h, IconRank::Exe), path, px);
        }
    }

    fn request_icon_target(&mut self, target: IconTarget, name: String, px: u32) {
        let key = (name, px);
        if let Some(img) = self.icons.cache.get(&key) {
            return self.apply_shell_icon(target, img.clone());
        }
        let waiters = self.icons.waiters.entry(key.clone()).or_default();
        waiters.push(target);
        if waiters.len() == 1 {
            self.send_icon_request(key);
        }
    }

    pub(super) fn resend_icon_requests(&mut self) {
        self.icons.requests.clear();
        let keys: Vec<IconKey> = self.icons.waiters.keys().cloned().collect();
        for key in keys {
            self.send_icon_request(key);
        }
    }

    /// Requests that are not sent stay in `icons.waiters` until the hook attaches.
    fn send_icon_request(&mut self, key: IconKey) {
        let id = self.icons.next_request;
        self.icons.next_request += 1;
        if hook::request(self.hwnd, &Request::Icon { id, px: key.1, name: key.0.clone() }) {
            self.icons.requests.insert(id, key);
        }
    }

    /// Builds images for the given tray icons, asking for the owner's exe
    /// icon when the app sent none we can read.
    pub(super) fn ensure_tray_images(&mut self, icons: &[usize]) {
        let px = self.px(TRAY_ICON) as u32;
        for &i in icons {
            let icon = &mut self.tray.icons[i];
            if icon.ensure_image(&self.gfx, px) || icon.fallback_requested {
                continue;
            }
            icon.fallback_requested = true;
            let key = icon.key;
            if let Some(exe) = icon.exe.clone() {
                self.request_icon_target(IconTarget::Tray(key), exe, px);
            }
        }
    }

    fn set_window_icon(&mut self, h: HWND, icon: isize, rank: IconRank) {
        let px = self.window_icon_px();
        let Some(i) = self.windows.index(h) else { return };
        let w = &self.windows.items[i];
        if rank < w.rank || (rank == w.rank && w.src == icon && w.image.as_ref().is_some_and(|i| i.px == px)) {
            return;
        }
        if let Some(img) = self.gfx.image_from_hicon(HICON(icon as *mut _), px) {
            let w = &mut self.windows.items[i];
            w.image = Some(img);
            w.rank = rank;
            w.src = icon;
            self.invalidate();
        }
    }

    fn on_window_icon(&mut self, h: HWND, small: bool, result: isize) {
        if result != 0 {
            self.set_window_icon(h, result, IconRank::Window);
        } else if !small && self.windows.index(h).is_some() {
            winlist::request_window_icon(h, true, on_window_icon);
        }
    }

    pub(super) fn on_shell_icon(&mut self, id: u64, bitmap: Option<Bitmap>) {
        let Some(key) = self.icons.requests.remove(&id) else { return };
        let targets = self.icons.waiters.remove(&key).unwrap_or_default();
        let Some(img) = bitmap.and_then(|b| self.gfx.image_from_bitmap(b, key.1)) else { return };
        for target in targets {
            self.apply_shell_icon(target, img.clone());
        }
        self.icons.cache.insert(key, img);
    }

    fn apply_shell_icon(&mut self, target: IconTarget, img: Image) {
        match target {
            IconTarget::Window(h, rank) => {
                if let Some(i) = self.windows.index(h)
                    && rank >= self.windows.items[i].rank
                {
                    let w = &mut self.windows.items[i];
                    w.image = Some(img);
                    w.rank = rank;
                    w.src = 0;
                    self.invalidate();
                }
            }
            IconTarget::Pin(name) => {
                if let Some(p) = self.pins.iter_mut().find(|p| p.parsing_name == name) {
                    p.image = Some(img);
                    self.invalidate();
                }
            }
            IconTarget::Tray(key) => {
                if let Some(icon) = self.tray.icons.iter_mut().find(|i| i.key.same(&key) && i.image.is_none()) {
                    icon.image = Some(img);
                    self.invalidate();
                    self.invalidate_popup();
                }
            }
        }
    }

    pub(super) fn prune_icon_cache(&mut self) {
        let live: Vec<u64> = (self.windows.items.iter().map(|w| &w.image))
            .chain(self.pins.iter().map(|p| &p.image))
            .chain(self.tray.icons.iter().map(|i| &i.image))
            .flatten()
            .map(Image::id)
            .collect();
        self.icons.cache.retain(|_, img| live.contains(&img.id()));
    }

    pub(super) fn queue_icon_refresh(&mut self, h: HWND) {
        if self.windows.index(h).is_some() && !self.icons.refresh.contains(&h) {
            if self.icons.refresh.is_empty() {
                self.set_timer(TIMER_ICONS, 500);
            }
            self.icons.refresh.push(h);
        }
    }

    pub(super) fn refresh_window_icons(&mut self) {
        for h in std::mem::take(&mut self.icons.refresh) {
            if self.windows.index(h).is_some() && class_name(h) != FRAME_CLASS {
                winlist::request_window_icon(h, false, on_window_icon);
            }
        }
    }

    pub(super) fn refetch_icons(&mut self) {
        self.icons.cache.clear();
        for w in &mut self.windows.items {
            w.rank = IconRank::None;
        }
        for i in 0..self.windows.items.len() {
            self.request_icon(self.windows.items[i].hwnd);
        }
        for i in 0..self.pins.len() {
            self.request_pin_icon(i);
        }
        for icon in &mut self.tray.icons {
            icon.image = None;
            icon.fallback_requested = false;
        }
    }

    pub(super) fn request_pin_icon(&mut self, i: usize) {
        let name = self.pins[i].parsing_name.clone();
        self.request_icon_target(IconTarget::Pin(name.clone()), name, self.window_icon_px());
    }
}
