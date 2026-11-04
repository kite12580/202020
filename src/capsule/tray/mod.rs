pub mod relay;

use capsule_core::pins::pin_key;
use capsule_core::tray_proto::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_STATE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NIS_HIDDEN, handle,
};
use windows::Win32::Foundation::{E_ACCESSDENIED, HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, DestroyIcon, HICON, IsWindow, SendNotifyMessageW, WM_CONTEXTMENU, WM_USER,
};

use crate::capsule::gfx::{Gfx, Image};
use crate::capsule::util::{hwnd, make_long, process_path, window_ids};
use relay::TrayEvent;

pub const NIN_SELECT: u32 = WM_USER;
pub const NIN_POPUPOPEN: u32 = WM_USER + 6;
pub const NIN_POPUPCLOSE: u32 = WM_USER + 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrayKey {
    pub hwnd: u32,
    pub uid: u32,
    pub guid: Option<[u8; 16]>,
}

impl TrayKey {
    pub fn same(&self, o: &TrayKey) -> bool {
        match (self.guid, o.guid) {
            (Some(a), Some(b)) => a == b,
            _ => self.hwnd == o.hwnd && self.uid == o.uid,
        }
    }
}

pub fn destroy_icon(h: isize) {
    if h != 0 {
        unsafe {
            let _ = DestroyIcon(HICON(h as *mut _));
        }
    }
}

pub struct Icon {
    pub key: TrayKey,
    pub pin: String,
    pub callback: u32,
    pub version: u32,
    pub show_tip: bool,
    pub hidden: bool,
    pub tip: String,
    pub exe: Option<String>,
    hicon: isize,
    pub image: Option<Image>,
    /// An exe icon was requested because the app sent no usable icon.
    pub fallback_requested: bool,
}

impl Drop for Icon {
    fn drop(&mut self) {
        destroy_icon(self.hicon);
    }
}

impl Icon {
    pub fn owner(&self) -> HWND {
        hwnd(handle(self.key.hwnd))
    }

    /// Returns `false` when the icon has no image of its own.
    pub fn ensure_image(&mut self, gfx: &Gfx, px: u32) -> bool {
        if self.image.as_ref().is_some_and(|i| i.px == px) {
            return true;
        }
        if self.hicon == 0 {
            return false;
        }
        self.image = gfx.image_from_hicon(HICON(self.hicon as *mut _), px);
        self.image.is_some()
    }

    /// Delivers a mouse message to the icon's owner with the semantics of
    /// the version the icon registered. Returns `false` when UIPI blocks it,
    /// meaning the owner runs elevated and we do not.
    pub fn send(&self, msg: u32, cursor: (i32, i32)) -> bool {
        let (wparam, lparam) = if self.version >= 4 {
            (make_long(cursor.0, cursor.1) as usize, make_long(msg as i32, self.key.uid as i32) as isize)
        } else {
            (self.key.uid as usize, msg as isize)
        };
        let r = unsafe { SendNotifyMessageW(self.owner(), self.callback, WPARAM(wparam), LPARAM(lparam)) };
        log::debug!("tray send {msg:#x} to {:?} cb {:#x} v{} -> {r:?}", self.key, self.callback, self.version);
        !matches!(r, Err(e) if e.code() == E_ACCESSDENIED)
    }

    pub fn allow_foreground(&self) {
        unsafe {
            let _ = AllowSetForegroundWindow(window_ids(self.owner()).1);
        }
    }

    /// The message explorer adds after a button-up for version 3+ icons.
    pub fn follow_up(&self, msg: u32) -> Option<u32> {
        use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONUP, WM_RBUTTONUP};
        if self.version < 3 {
            return None;
        }
        match msg {
            WM_LBUTTONUP => Some(NIN_SELECT),
            WM_RBUTTONUP => Some(WM_CONTEXTMENU),
            _ => None,
        }
    }

    pub fn wants_popup(&self) -> bool {
        self.version >= 4 && !self.show_tip
    }
}

#[derive(Default)]
pub struct Tray {
    pub icons: Vec<Icon>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Change {
    None,
    /// The icon at this index got a new image.
    Redraw(usize),
    Relayout,
}

impl Tray {
    pub fn apply(&mut self, ev: TrayEvent) -> Change {
        let d = &ev.msg.data;
        let key = TrayKey { hwnd: d.hwnd, uid: d.uid, guid: d.guid };
        let pos = self.icons.iter().position(|i| i.key.same(&key));
        log::debug!("tray msg {} {:?} flags {:#x} pos {:?} tip {:?}", ev.msg.message, key, d.flags, pos, d.tip);
        match ev.msg.message {
            NIM_ADD | NIM_MODIFY => {
                let idx = pos.unwrap_or_else(|| {
                    let exe = process_path(window_ids(hwnd(handle(d.hwnd))).1);
                    self.icons.push(Icon {
                        key,
                        pin: pin_key(exe.as_deref().unwrap_or_default(), d.uid, d.guid.as_ref()),
                        callback: 0,
                        version: 0,
                        show_tip: false,
                        hidden: false,
                        tip: String::new(),
                        exe,
                        hicon: 0,
                        image: None,
                        fallback_requested: false,
                    });
                    self.icons.len() - 1
                });
                let icon = &mut self.icons[idx];
                let was_hidden = icon.hidden;
                if d.flags & NIF_MESSAGE != 0 {
                    icon.callback = d.callback_message;
                }
                let new_icon = d.flags & NIF_ICON != 0 && !(ev.same_icon && icon.hicon != 0);
                if new_icon {
                    destroy_icon(icon.hicon);
                    icon.hicon = if ev.same_icon { relay::copy_icon(handle(d.icon)) } else { ev.icon };
                    icon.image = None;
                }
                if d.flags & NIF_TIP != 0 {
                    icon.tip = d.tip.clone();
                }
                if d.flags & NIF_STATE != 0 && d.state_mask & NIS_HIDDEN != 0 {
                    icon.hidden = d.state & NIS_HIDDEN != 0;
                }
                if ev.msg.message == NIM_ADD || d.flags & NIF_SHOWTIP != 0 {
                    icon.show_tip = d.flags & NIF_SHOWTIP != 0;
                }
                if icon.key.guid.is_none() && d.guid.is_some() {
                    icon.key.guid = d.guid;
                }
                if pos.is_none() || icon.hidden != was_hidden {
                    Change::Relayout
                } else if new_icon {
                    Change::Redraw(idx)
                } else {
                    Change::None
                }
            }
            NIM_DELETE => {
                destroy_icon(ev.icon);
                match pos {
                    Some(i) => {
                        self.icons.remove(i);
                        Change::Relayout
                    }
                    None => Change::None,
                }
            }
            NIM_SETVERSION => {
                if let Some(i) = pos {
                    self.icons[i].version = d.version;
                }
                Change::None
            }
            _ => {
                destroy_icon(ev.icon);
                Change::None
            }
        }
    }

    pub fn prune_dead(&mut self) -> bool {
        let before = self.icons.len();
        self.icons.retain(|i| unsafe { IsWindow(Some(i.owner())) }.as_bool());
        before != self.icons.len()
    }

    /// Indices of pinned, visible icons, in pin order.
    pub fn bar_icons(&self, pins: &[String]) -> Vec<usize> {
        pins.iter().filter_map(|p| self.icons.iter().position(|i| !i.hidden && &i.pin == p)).collect()
    }

    pub fn popup_icons(&self) -> Vec<usize> {
        (0..self.icons.len()).filter(|&i| !self.icons[i].hidden).collect()
    }
}
