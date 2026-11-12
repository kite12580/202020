use std::collections::HashMap;
use std::ffi::c_void;

use capsule_core::taskband::{self, Pin, Task, slot};
use capsule_core::winfilter::{WindowFacts, taskbar_window};
use windows::Win32::Foundation::{ERROR_SUCCESS, FILETIME, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, GetProcessTimes, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GCLP_HICON, GCLP_HICONSM, GW_OWNER, GWL_EXSTYLE, GetClassLongPtrW, GetForegroundWindow, GetWindow,
    GetWindowLongW, GetWindowTextW, ICON_BIG, ICON_SMALL2, IsIconic, IsWindow, IsWindowVisible, PostMessageW,
    SW_MINIMIZE, SW_RESTORE, SendMessageCallbackW, SetForegroundWindow, ShowWindowAsync, WM_CLOSE, WM_GETICON,
    WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PWSTR};

use crate::capsule::gfx::Image;
use crate::capsule::util::{
    OwnedHandle, PKEY_APP_USER_MODEL_ID, class_name, hwnd, image_path, make_long, open_process, own_window,
    take_string, window_ids,
};

const WM_POPUPSYSTEMMENU: u32 = 0x0313;

/// Icon sources, best last.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum IconRank {
    None,
    Class,
    Exe,
    Window,
    Package,
}

pub struct Window {
    pub hwnd: HWND,
    seq: u64,
    pub image: Option<Image>,
    pub rank: IconRank,
    /// The HICON `image` was made from, to skip unchanged refetches.
    pub src: isize,
    aumids: Vec<String>,
    pub exe: Option<String>,
    /// Index of the taskbar pin the window groups under, or `usize::MAX`.
    slot: usize,
}

impl Window {
    fn order(&self) -> (usize, u64) {
        (self.slot, self.seq)
    }
}

#[derive(Default)]
pub struct WindowList {
    pub items: Vec<Window>,
    seen: HashMap<isize, u64>,
    next_seq: u64,
    replaced_seq: Option<u64>,
    pins: Vec<Pin>,
}

pub fn cloaked(h: HWND) -> bool {
    let mut v = 0u32;
    unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut v as *mut u32 as *mut c_void, 4) }.is_ok() && v != 0
}

pub fn qualifies(h: HWND) -> bool {
    let ex = unsafe { GetWindowLongW(h, GWL_EXSTYLE) } as u32;
    let class = class_name(h);
    let facts = WindowFacts {
        visible: unsafe { IsWindowVisible(h) }.as_bool(),
        cloaked: cloaked(h),
        own_process: own_window(h),
        tool_window: ex & WS_EX_TOOLWINDOW.0 != 0,
        app_window: ex & WS_EX_APPWINDOW.0 != 0,
        no_activate: ex & WS_EX_NOACTIVATE.0 != 0,
        owned: unsafe { GetWindow(h, GW_OWNER) }.is_ok_and(|o| !o.is_invalid()),
        class: &class,
    };
    let r = taskbar_window(&facts);
    if facts.visible {
        log::trace!("filter {:?} {class} -> {r:?}", h.0);
    }
    r.is_ok()
}

fn process_created(pid: u32) -> u64 {
    let Some(p) = open_process(PROCESS_QUERY_LIMITED_INFORMATION, pid) else {
        return u64::MAX;
    };
    let (mut c, mut e, mut k, mut u) =
        (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    if unsafe { GetProcessTimes(p.0, &mut c, &mut e, &mut k, &mut u) }.is_err() {
        return u64::MAX;
    }
    (u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime)
}

/// Enumeration callback pushing each window onto the `Vec<HWND>` at `l`.
pub unsafe extern "system" fn collect(h: HWND, l: LPARAM) -> BOOL {
    let v = unsafe { &mut *(l.0 as *mut Vec<HWND>) };
    v.push(h);
    BOOL(1)
}

pub fn top_level() -> Vec<HWND> {
    let mut all: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut all as *mut _ as isize));
    }
    all
}

impl WindowList {
    /// Fills the list with existing windows. Their true open order is
    /// unknown, so they are ordered by process start time, then bottom-up
    /// z-order.
    pub fn enumerate(&mut self) {
        let all = top_level();
        let mut created: HashMap<u32, u64> = HashMap::new();
        let mut found: Vec<(u64, usize, HWND)> = all
            .iter()
            .enumerate()
            .filter(|(_, h)| qualifies(**h))
            .map(|(z, &h)| {
                let pid = window_ids(h).1;
                let t = *created.entry(pid).or_insert_with(|| process_created(pid));
                (t, usize::MAX - z, h)
            })
            .collect();
        found.sort_by_key(|&(t, z, _)| (t, z));
        for (_, _, h) in found {
            self.insert(h);
        }
    }

    pub fn index(&self, h: HWND) -> Option<usize> {
        self.items.iter().position(|w| w.hwnd == h)
    }

    fn insert(&mut self, h: HWND) {
        let seq = match self.replaced_seq.take() {
            Some(seq) => *self.seen.entry(h.0 as isize).insert_entry(seq).get(),
            None => *self.seen.entry(h.0 as isize).or_insert_with(|| {
                self.next_seq += 1;
                self.next_seq
            }),
        };
        let process = open_process(PROCESS_QUERY_LIMITED_INFORMATION, window_ids(h).1);
        let aumids: Vec<String> = aumid(h).into_iter().chain(process.as_ref().and_then(package_aumid)).collect();
        let exe = process.as_ref().and_then(image_path);
        let slot = slot(&self.pins, &aumids, exe.as_deref()).unwrap_or(usize::MAX);
        log::debug!("window {:?} {aumids:?} {exe:?} -> pin {slot}", h.0);
        let w = Window { hwnd: h, seq, image: None, rank: IconRank::None, src: 0, aumids, exe, slot };
        let at = self.items.partition_point(|o| o.order() < w.order());
        self.items.insert(at, w);
    }

    /// Regroups windows behind the native taskbar's pins.
    pub fn set_pins(&mut self, pins: Vec<Pin>) {
        self.pins = pins;
        for w in &mut self.items {
            w.slot = slot(&self.pins, &w.aumids, w.exe.as_deref()).unwrap_or(usize::MAX);
        }
        self.items.sort_by_key(Window::order);
    }

    pub fn tasks(&self) -> Vec<Task> {
        let slots: Vec<usize> = self.items.iter().map(|w| w.slot).collect();
        taskband::tasks(&slots, self.pins.len())
    }

    pub fn add(&mut self, h: HWND) -> bool {
        if self.index(h).is_some() || !qualifies(h) {
            return false;
        }
        self.insert(h);
        true
    }

    pub fn remove(&mut self, h: HWND) -> bool {
        match self.index(h) {
            Some(i) => {
                self.items.remove(i);
                true
            }
            None => false,
        }
    }

    /// `HSHELL_WINDOWREPLACED`: the next added window takes `h`'s place.
    pub fn replaced(&mut self, h: HWND) -> bool {
        let i = self.index(h);
        self.replaced_seq = i.map(|i| self.items.remove(i).seq);
        i.is_some()
    }

    /// Drops windows whose state changed without an event reaching us.
    pub fn prune(&mut self) -> bool {
        let before = self.items.len();
        self.items.retain(|w| qualifies(w.hwnd));
        self.seen.retain(|&h, _| unsafe { IsWindow(Some(hwnd(h))) }.as_bool());
        before != self.items.len()
    }
}

/// The window's explicit AppUserModelID.
pub fn aumid(h: HWND) -> Option<String> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(h).ok()?;
        let value = store.GetValue(&PKEY_APP_USER_MODEL_ID).ok()?;
        take_string(PropVariantToStringAlloc(&value).ok()?).filter(|s| !s.is_empty())
    }
}

fn package_aumid(p: &OwnedHandle) -> Option<String> {
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    if unsafe { GetApplicationUserModelId(p.0, &mut len, Some(PWSTR(buf.as_mut_ptr()))) } != ERROR_SUCCESS {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..len.saturating_sub(1) as usize]))
}

pub fn request_window_icon(h: HWND, small: bool, callback: unsafe extern "system" fn(HWND, u32, usize, LRESULT)) {
    let which = if small { ICON_SMALL2 } else { ICON_BIG };
    unsafe {
        let _ = SendMessageCallbackW(h, WM_GETICON, WPARAM(which as usize), LPARAM(0), Some(callback), small as usize);
    }
}

pub fn class_icon(h: HWND) -> isize {
    unsafe {
        match GetClassLongPtrW(h, GCLP_HICON) {
            0 => GetClassLongPtrW(h, GCLP_HICONSM) as isize,
            v => v as isize,
        }
    }
}

pub fn title(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

pub fn activate_or_minimize(h: HWND, foreground: HWND) {
    unsafe {
        let iconic = IsIconic(h).as_bool();
        if h == foreground && !iconic {
            let _ = ShowWindowAsync(h, SW_MINIMIZE);
            return;
        }
        if iconic {
            let _ = ShowWindowAsync(h, SW_RESTORE);
        }
    }
    force_foreground(h);
}

pub fn force_foreground(h: HWND) {
    if unsafe { SetForegroundWindow(h) }.as_bool() {
        return;
    }
    let fg = window_ids(unsafe { GetForegroundWindow() }).0;
    let me = unsafe { GetCurrentThreadId() };
    if fg == 0 || fg == me || !unsafe { AttachThreadInput(me, fg, true) }.as_bool() {
        return;
    }
    unsafe {
        let _ = SetForegroundWindow(h);
        let _ = AttachThreadInput(me, fg, false);
    }
}

pub fn close(h: HWND) {
    unsafe {
        let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

pub fn system_menu(h: HWND, x: i32, y: i32) {
    unsafe {
        let _ = SetForegroundWindow(h);
        let _ = PostMessageW(Some(h), WM_POPUPSYSTEMMENU, WPARAM(0), LPARAM(make_long(x, y) as isize));
    }
}
