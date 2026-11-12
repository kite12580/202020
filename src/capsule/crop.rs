//! Shows one pane of a window, such as a browser's page: the window is
//! clipped to the pane and grown so the pane fills the window's frame.
//! Corner grips stand in for the hidden title bar and borders. Runs on the
//! main thread.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::OnceLock;

use capsule_core::crop::{GRIPS, Grip, croppable, drag, fit, grip_rect, insets, shrink};
use capsule_core::layout::PxRect;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, CreateSolidBrush, DeleteObject, GetMonitorInfoW, GetWindowRgnBox,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, RGN_DIFF, SIMPLEREGION, ScreenToClient, SetWindowRgn,
};
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_NOREPEAT, RegisterHotKey, ReleaseCapture, SetCapture, UnregisterHotKey, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, CWP_SKIPINVISIBLE, CallNextHookEx, ChildWindowFromPointEx, CreateWindowExW, DefWindowProcW,
    DestroyWindow, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE,
    EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART,
    EnumChildWindows, GA_PARENT, GA_ROOT, GW_HWNDPREV, GWL_EXSTYLE, GWLP_USERDATA, GetAncestor, GetCursorPos,
    GetWindow, GetWindowLongPtrW, GetWindowRect, HC_ACTION, HHOOK, HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST, IDC_SIZEALL,
    IDC_SIZENESW, IDC_SIZENWSE, IsHungAppWindow, IsIconic, IsWindow, IsWindowVisible, IsZoomed, LWA_ALPHA, LoadCursorW,
    MA_NOACTIVATE, MB_OK, MSLLHOOKSTRUCT, OBJID_WINDOW, PostMessageW, SET_WINDOW_POS_FLAGS, SW_HIDE, SW_MAXIMIZE,
    SW_RESTORE, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SetCursor, SetLayeredWindowAttributes, SetWindowLongPtrW,
    SetWindowPos, SetWindowsHookExW, ShowWindow, ShowWindowAsync, UnhookWindowsHookEx, WH_MOUSE_LL,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_CAPTURECHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WindowFromPoint,
};
use windows::core::{PCWSTR, w};

use crate::capsule::desktops::alive;
use crate::capsule::theme::Theme;
use crate::capsule::util::{class_name, foreground, hwnd, own_window, px_rect, register_class, scale, window_ids};
use crate::capsule::watchdog;
use crate::capsule::windows_list::{cloaked, collect, qualifies};

pub const TOGGLE: usize = 0;
pub const CANCEL: usize = 1;
const POINTED: usize = 2;
const PICKED: usize = 3;
const OUTWARD: usize = 4;
const INWARD: usize = 5;
const CHANGED: usize = 6;

const GRIP_DIP: f32 = 12.0;
const MIN_DIP: f32 = 120.0;
const OUTLINE_DIP: f32 = 3.0;
const GRIP_CLASS: PCWSTR = w!("202020CropGrip");
const OUTLINE_CLASS: PCWSTR = w!("202020CropOutline");

const PROCESS_EVENTS: [(u32, u32); 4] = [
    (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
    (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
    (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
    (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
];

pub struct Options {
    pub owner: isize,
    pub msg: u32,
    pub cancel_hotkey: i32,
}

static OPTIONS: OnceLock<Options> = OnceLock::new();

#[derive(Clone, Copy)]
pub struct Saved {
    pub rect: RECT,
    pub maximized: bool,
}

/// A cropped window, its process id and what uncropping puts back.
pub type Cropped = (HWND, u32, Saved);

struct Pane {
    hwnd: HWND,
    parent: HWND,
    class: String,
}

struct Entry {
    target: HWND,
    pid: u32,
    pane: Pane,
    /// Where the pane shows, on screen.
    frame: PxRect,
    /// The target's invisible resize borders.
    border: PxRect,
    /// The target's rect as last placed here; any other rect was set elsewhere.
    applied: PxRect,
    /// The rect last asked for, which the target may have adjusted.
    asked: PxRect,
    /// The pane within the target, as last clipped.
    region: PxRect,
    saved: Saved,
    grips: [HWND; 4],
}

struct Pick {
    hook: HHOOK,
    outline: Option<HWND>,
    /// The windows under the cursor, deepest first, ending at the top-level one.
    chain: Vec<HWND>,
    level: usize,
}

struct Drag {
    grip: HWND,
    from: POINT,
    frame: PxRect,
}

#[derive(Default)]
struct Crops {
    entries: Vec<Entry>,
    pick: Option<Pick>,
    drag: Option<Drag>,
    hooks: Vec<(u32, Vec<HWINEVENTHOOK>)>,
    foreground_hook: Option<HWINEVENTHOOK>,
}

thread_local! {
    static STATE: RefCell<Crops> = RefCell::default();
    static CURSOR: Cell<POINT> = const { Cell::new(POINT { x: 0, y: 0 }) };
    static POINTED_POSTED: Cell<bool> = const { Cell::new(false) };
}

pub fn init(opts: Options) {
    let accent = Theme::read().accent;
    let color = COLORREF(u32::from(accent[0]) | u32::from(accent[1]) << 8 | u32::from(accent[2]) << 16);
    register_class(WNDCLASSEXW {
        lpfnWndProc: Some(outline_proc),
        hbrBackground: unsafe { CreateSolidBrush(color) },
        lpszClassName: OUTLINE_CLASS,
        ..Default::default()
    });
    register_class(WNDCLASSEXW { lpfnWndProc: Some(grip_proc), lpszClassName: GRIP_CLASS, ..Default::default() });
    let _ = OPTIONS.set(opts);
}

fn post(request: usize, lparam: isize) {
    if let Some(o) = OPTIONS.get() {
        unsafe {
            let _ = PostMessageW(Some(hwnd(o.owner)), o.msg, WPARAM(request), LPARAM(lparam));
        }
    }
}

pub fn on_request(request: usize, lparam: isize) {
    if request == POINTED {
        POINTED_POSTED.set(false);
    }
    STATE.with_borrow_mut(|c| match request {
        TOGGLE => c.toggle(),
        CANCEL => c.end_pick(),
        POINTED => c.point(CURSOR.get()),
        PICKED => c.picked(),
        OUTWARD | INWARD => c.step(request == OUTWARD),
        CHANGED => c.changed(hwnd(lparam)),
        _ => {}
    })
}

pub fn restore_all() {
    STATE.with_borrow_mut(|c| {
        c.end_pick();
        while !c.entries.is_empty() {
            c.uncrop(c.entries.len() - 1);
        }
    })
}

pub fn undo(h: HWND, saved: &Saved) {
    let r = saved.rect;
    unsafe {
        SetWindowRgn(h, None, true);
        let _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
        if saved.maximized {
            let _ = ShowWindowAsync(h, SW_MAXIMIZE);
        }
    }
}

fn beep() {
    unsafe {
        let _ = MessageBeep(MB_OK);
    }
}

fn window_rect(h: HWND) -> PxRect {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    px_rect(r)
}

/// The part of the window that is drawn, without its invisible resize borders.
fn visible_rect(h: HWND) -> PxRect {
    let mut r = RECT::default();
    let size = size_of::<RECT>() as u32;
    match unsafe { DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut RECT as *mut c_void, size) } {
        Ok(()) => px_rect(r),
        Err(_) => window_rect(h),
    }
}

fn work_area(h: HWND) -> PxRect {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut info);
    }
    px_rect(info.rcWork)
}

fn dpi(h: HWND) -> u32 {
    unsafe { GetDpiForWindow(h) }.max(96)
}

fn place(h: HWND, after: Option<HWND>, r: PxRect, flags: SET_WINDOW_POS_FLAGS) {
    unsafe {
        let _ = SetWindowPos(h, after, r.left, r.top, r.width(), r.height(), flags | SWP_NOACTIVATE);
    }
}

/// The windows under `pt`, deepest first, ending at the top-level one.
fn chain_at(pt: POINT) -> Vec<HWND> {
    let root = unsafe { GetAncestor(WindowFromPoint(pt), GA_ROOT) };
    if root.is_invalid() || own_window(root) || !qualifies(root) {
        return Vec::new();
    }
    let mut chain = vec![root];
    loop {
        let parent = chain[chain.len() - 1];
        let mut client = pt;
        let child = unsafe {
            let _ = ScreenToClient(parent, &mut client);
            ChildWindowFromPointEx(parent, client, CWP_SKIPINVISIBLE)
        };
        if child.is_invalid() || child == parent {
            break;
        }
        chain.push(child);
    }
    chain.reverse();
    chain
}

fn usable(h: HWND, target: HWND) -> bool {
    unsafe { IsWindow(Some(h)).as_bool() && IsWindowVisible(h).as_bool() && GetAncestor(h, GA_ROOT) == target }
}

impl Pane {
    fn new(h: HWND) -> Pane {
        Pane { hwnd: h, parent: unsafe { GetAncestor(h, GA_PARENT) }, class: class_name(h) }
    }

    /// The pane, or when it was hidden or destroyed, as when a browser
    /// switches tabs, the largest shown window of its kind beside it.
    fn current(&mut self, target: HWND) -> Option<HWND> {
        if usable(self.hwnd, target) {
            return Some(self.hwnd);
        }
        let mut all = Vec::new();
        unsafe {
            let _ = EnumChildWindows(Some(self.parent), Some(collect), LPARAM(&mut all as *mut _ as isize));
        }
        let found = all
            .into_iter()
            .filter(|&h| unsafe { GetAncestor(h, GA_PARENT) } == self.parent && usable(h, target))
            .filter(|&h| class_name(h) == self.class)
            .max_by_key(|&h| {
                let r = window_rect(h);
                i64::from(r.width()) * i64::from(r.height())
            })?;
        self.hwnd = found;
        Some(found)
    }
}

impl Entry {
    /// Brings the crop in line with the target. Returns false once the
    /// target is gone.
    fn sync(&mut self) -> bool {
        let target = self.target;
        if !alive(&(target, self.pid)) {
            return false;
        }
        let shown = unsafe { IsWindowVisible(target).as_bool() && !IsIconic(target).as_bool() } && !cloaked(target);
        if !shown {
            self.hide_grips();
            return true;
        }
        if unsafe { IsHungAppWindow(target) }.as_bool() {
            return true;
        }
        if unsafe { IsZoomed(target) }.as_bool() {
            self.frame = work_area(target);
            unsafe {
                let _ = ShowWindow(target, SW_RESTORE);
            }
            self.applied = window_rect(target);
        }
        let window = window_rect(target);
        if window != self.applied {
            self.frame = shrink(window, self.border);
        }
        let pane = match self.pane.current(target).map(window_rect).filter(|&p| croppable(window, p)) {
            Some(pane) => pane,
            None if self.region.width() > 0 => PxRect {
                left: window.left + self.region.left,
                top: window.top + self.region.top,
                right: window.left + self.region.right,
                bottom: window.top + self.region.bottom,
            },
            None => return true,
        };
        let (placed, region) = fit(self.frame, window, pane);
        if placed != window && !(placed == self.asked && window == self.applied) {
            place(target, None, placed, SWP_NOZORDER);
            self.asked = placed;
        }
        self.applied = window_rect(target);
        self.region = region;
        clip(target, region);
        self.place_grips();
        true
    }

    /// Shows the grips just above the target. A window placed after a
    /// topmost one becomes topmost, so when the window above the target
    /// differs in that, the grips go to the top of the target's band instead.
    fn place_grips(&self) {
        let size = scale(GRIP_DIP, dpi(self.target));
        let on_top = topmost(self.target);
        let above = unsafe { GetWindow(self.target, GW_HWNDPREV) }.ok().filter(|&h| topmost(h) == on_top);
        for (&g, grip) in self.grips.iter().zip(GRIPS) {
            let after = match above {
                Some(h) => h,
                None if on_top => HWND_TOPMOST,
                None if topmost(g) => HWND_NOTOPMOST,
                None => HWND_TOP,
            };
            let flags = if g == after { SWP_SHOWWINDOW | SWP_NOZORDER } else { SWP_SHOWWINDOW };
            place(g, Some(after), grip_rect(self.frame, grip, size), flags);
        }
    }

    fn hide_grips(&self) {
        for &g in &self.grips {
            unsafe {
                let _ = ShowWindow(g, SW_HIDE);
            }
        }
    }

    fn cropped(&self) -> Cropped {
        (self.target, self.pid, self.saved)
    }
}

fn topmost(h: HWND) -> bool {
    let ex = unsafe { GetWindowLongPtrW(h, GWL_EXSTYLE) };
    ex & WS_EX_TOPMOST.0 as isize != 0
}

fn clip(h: HWND, region: PxRect) {
    let mut current = RECT::default();
    unsafe {
        if GetWindowRgnBox(h, &mut current) == SIMPLEREGION && px_rect(current) == region {
            return;
        }
        SetWindowRgn(h, Some(CreateRectRgn(region.left, region.top, region.right, region.bottom)), true);
    }
}

fn create_grips() -> Option<[HWND; 4]> {
    let instance = unsafe { GetModuleHandleW(None) }.ok()?;
    let mut grips = [HWND::default(); 4];
    for (i, g) in grips.iter_mut().enumerate() {
        let created = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                GRIP_CLASS,
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
        };
        let Ok(h) = created else {
            destroy(&grips[..i]);
            return None;
        };
        unsafe {
            SetWindowLongPtrW(h, GWLP_USERDATA, i as isize);
            let _ = SetLayeredWindowAttributes(h, COLORREF(0), 1, LWA_ALPHA);
        }
        *g = h;
    }
    Some(grips)
}

fn destroy(windows: &[HWND]) {
    for &h in windows {
        unsafe {
            let _ = DestroyWindow(h);
        }
    }
}

fn create_outline() -> Option<HWND> {
    unsafe {
        let h = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            OUTLINE_CLASS,
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            GetModuleHandleW(None).ok().map(Into::into),
            None,
        )
        .ok()?;
        let _ = SetLayeredWindowAttributes(h, COLORREF(0), 255, LWA_ALPHA);
        Some(h)
    }
}

fn show_ring(outline: HWND, r: PxRect, dpi: u32) {
    let t = scale(OUTLINE_DIP, dpi);
    let (w, h) = (r.width(), r.height());
    unsafe {
        let ring = CreateRectRgn(0, 0, w, h);
        let hole = CreateRectRgn(t, t, w - t, h - t);
        CombineRgn(Some(ring), Some(ring), Some(hole), RGN_DIFF);
        let _ = DeleteObject(hole.into());
        SetWindowRgn(outline, Some(ring), false);
    }
    place(outline, Some(HWND_TOPMOST), r, SWP_SHOWWINDOW);
}

impl Crops {
    fn find(&self, target: HWND) -> Option<usize> {
        self.entries.iter().position(|e| e.target == target)
    }

    fn toggle(&mut self) {
        if self.pick.is_some() {
            return self.end_pick();
        }
        match foreground().and_then(|h| self.find(h)) {
            Some(i) => self.uncrop(i),
            None => self.start_pick(),
        }
    }

    fn start_pick(&mut self) {
        let Some(o) = OPTIONS.get() else { return };
        let module = unsafe { GetModuleHandleW(None) }.ok().map(Into::into);
        let Ok(hook) = (unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(pick_proc), module, 0) }) else { return };
        let mut pt = POINT::default();
        unsafe {
            let _ = RegisterHotKey(Some(hwnd(o.owner)), o.cancel_hotkey, MOD_NOREPEAT, u32::from(VK_ESCAPE.0));
            let _ = GetCursorPos(&mut pt);
        }
        self.pick = Some(Pick { hook, outline: create_outline(), chain: Vec::new(), level: 0 });
        self.point(pt);
    }

    fn end_pick(&mut self) {
        let Some(p) = self.pick.take() else { return };
        unsafe {
            let _ = UnhookWindowsHookEx(p.hook);
            if let Some(o) = OPTIONS.get() {
                let _ = UnregisterHotKey(Some(hwnd(o.owner)), o.cancel_hotkey);
            }
        }
        destroy(p.outline.as_slice());
    }

    fn point(&mut self, pt: POINT) {
        let Some(p) = &mut self.pick else { return };
        let chain = chain_at(pt);
        if chain != p.chain {
            p.chain = chain;
            p.level = 0;
        }
        self.show_outline();
    }

    fn step(&mut self, outward: bool) {
        let Some(p) = &mut self.pick else { return };
        p.level = if outward { (p.level + 1).min(p.chain.len().saturating_sub(1)) } else { p.level.saturating_sub(1) };
        self.show_outline();
    }

    fn show_outline(&self) {
        let Some(p) = &self.pick else { return };
        let Some(outline) = p.outline else { return };
        match p.chain.get(p.level) {
            Some(&h) => {
                let r = if p.level + 1 == p.chain.len() { visible_rect(h) } else { window_rect(h) };
                show_ring(outline, r, dpi(h));
            }
            None => unsafe {
                let _ = ShowWindow(outline, SW_HIDE);
            },
        }
    }

    fn picked(&mut self) {
        self.point(CURSOR.get());
        let Some(p) = &self.pick else { return };
        let pane = p.chain.get(p.level).copied();
        let target = p.chain.last().copied();
        self.end_pick();
        match (pane, target) {
            (Some(pane), Some(target)) if pane != target => self.crop(target, pane),
            _ => beep(),
        }
    }

    fn crop(&mut self, target: HWND, pane: HWND) {
        if let Some(i) = self.find(target) {
            let e = &mut self.entries[i];
            e.pane = Pane::new(pane);
            e.region = PxRect::default();
            return self.sync(i);
        }
        if unsafe { IsHungAppWindow(target) }.as_bool() || !croppable(window_rect(target), window_rect(pane)) {
            return beep();
        }
        let maximized = unsafe { IsZoomed(target) }.as_bool();
        if maximized {
            unsafe {
                let _ = ShowWindow(target, SW_RESTORE);
            }
        }
        let window = window_rect(target);
        let visible = visible_rect(target);
        let Some(grips) = create_grips() else { return beep() };
        let pid = window_ids(target).1;
        let saved = Saved {
            rect: RECT { left: window.left, top: window.top, right: window.right, bottom: window.bottom },
            maximized,
        };
        let crops: Vec<Cropped> = self.entries.iter().map(Entry::cropped).chain([(target, pid, saved)]).collect();
        if !watchdog::set_crops(&crops) {
            destroy(&grips);
            undo(target, &saved);
            return beep();
        }
        self.entries.push(Entry {
            target,
            pid,
            pane: Pane::new(pane),
            frame: if maximized { work_area(target) } else { visible },
            border: insets(window, visible),
            applied: window,
            asked: PxRect::default(),
            region: PxRect::default(),
            saved,
            grips,
        });
        self.watch();
        self.sync(self.entries.len() - 1);
    }

    fn uncrop(&mut self, i: usize) {
        let e = self.entries.remove(i);
        destroy(&e.grips);
        if self.drag.as_ref().is_some_and(|d| e.grips.contains(&d.grip)) {
            self.drag = None;
        }
        if alive(&(e.target, e.pid)) {
            undo(e.target, &e.saved);
        }
        self.update_watchers();
    }

    fn sync(&mut self, i: usize) {
        if !self.entries[i].sync() {
            let e = self.entries.remove(i);
            destroy(&e.grips);
            self.update_watchers();
        }
    }

    fn update_watchers(&mut self) {
        let crops: Vec<Cropped> = self.entries.iter().map(Entry::cropped).collect();
        watchdog::set_crops(&crops);
        self.watch();
    }

    fn changed(&mut self, h: HWND) {
        let root = unsafe { GetAncestor(h, GA_ROOT) };
        for i in (0..self.entries.len()).rev() {
            let e = &self.entries[i];
            if e.target == h || e.pane.hwnd == h || e.target == root {
                self.sync(i);
            }
        }
    }

    fn watch(&mut self) {
        let pids: Vec<u32> = self.entries.iter().map(|e| e.pid).collect();
        self.hooks.retain(|(pid, hooks)| {
            let keep = pids.contains(pid);
            if !keep {
                unhook(hooks);
            }
            keep
        });
        for &pid in &pids {
            if self.hooks.iter().any(|(p, _)| *p == pid) {
                continue;
            }
            let hooks = PROCESS_EVENTS
                .iter()
                .map(|&(min, max)| unsafe {
                    SetWinEventHook(min, max, None, Some(on_event), pid, 0, WINEVENT_OUTOFCONTEXT)
                })
                .filter(|h| !h.is_invalid())
                .collect();
            self.hooks.push((pid, hooks));
        }
        match (pids.is_empty(), self.foreground_hook) {
            (true, Some(hook)) => {
                unhook(&[hook]);
                self.foreground_hook = None;
            }
            (false, None) => {
                let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
                let event = EVENT_SYSTEM_FOREGROUND;
                let hook = unsafe { SetWinEventHook(event, event, None, Some(on_event), 0, 0, flags) };
                self.foreground_hook = (!hook.is_invalid()).then_some(hook);
            }
            _ => {}
        }
    }

    fn grip(&self, g: HWND) -> Option<usize> {
        self.entries.iter().position(|e| e.grips.contains(&g))
    }

    fn begin_drag(&mut self, g: HWND) {
        let Some(i) = self.grip(g) else { return };
        let mut from = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut from);
        }
        self.drag = Some(Drag { grip: g, from, frame: self.entries[i].frame });
    }

    fn drag_to(&mut self, g: HWND) {
        let Some(d) = self.drag.as_ref().filter(|d| d.grip == g) else { return };
        let Some(i) = self.grip(g) else { return };
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        let grip = GRIPS[unsafe { GetWindowLongPtrW(g, GWLP_USERDATA) } as usize % GRIPS.len()];
        let min = scale(MIN_DIP, dpi(g));
        self.entries[i].frame = drag(d.frame, grip, pt.x - d.from.x, pt.y - d.from.y, min);
        self.sync(i);
    }
}

fn unhook(hooks: &[HWINEVENTHOOK]) {
    for &h in hooks {
        unsafe {
            let _ = UnhookWinEvent(h);
        }
    }
}

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    h: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    if h.is_invalid() || object != OBJID_WINDOW.0 || child != CHILDID_SELF as i32 {
        return;
    }
    let handled = STATE.with(|c| c.try_borrow_mut().map(|mut c| c.changed(h)).is_ok());
    if !handled {
        post(CHANGED, h.0 as isize);
    }
}

unsafe extern "system" fn pick_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let m = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        CURSOR.set(m.pt);
        let request = match wparam.0 as u32 {
            WM_MOUSEMOVE => {
                if !POINTED_POSTED.replace(true) {
                    post(POINTED, 0);
                }
                None
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN => Some(None),
            WM_LBUTTONUP => Some(Some(PICKED)),
            WM_RBUTTONUP => Some(Some(CANCEL)),
            WM_MOUSEWHEEL => Some(Some(if (m.mouseData >> 16) as i16 > 0 { OUTWARD } else { INWARD })),
            _ => None,
        };
        if let Some(request) = request {
            if let Some(request) = request {
                post(request, 0);
            }
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn outline_proc(h: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, msg, wparam, lparam) }
}

unsafe extern "system" fn grip_proc(h: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let with = |f: fn(&mut Crops, HWND)| STATE.with(|c| c.try_borrow_mut().map(|mut c| f(&mut c, h)).is_ok());
    match msg {
        WM_SETCURSOR => {
            let cursor = match GRIPS[unsafe { GetWindowLongPtrW(h, GWLP_USERDATA) } as usize % GRIPS.len()] {
                Grip::TopLeft => IDC_SIZEALL,
                Grip::TopRight | Grip::BottomLeft => IDC_SIZENESW,
                Grip::BottomRight => IDC_SIZENWSE,
            };
            unsafe { SetCursor(LoadCursorW(None, cursor).ok()) };
            return LRESULT(1);
        }
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONDOWN => {
            unsafe { SetCapture(h) };
            with(Crops::begin_drag);
        }
        WM_MOUSEMOVE => {
            with(Crops::drag_to);
        }
        WM_LBUTTONUP => unsafe {
            let _ = ReleaseCapture();
        },
        WM_CAPTURECHANGED => {
            with(|c, _| c.drag = None);
        }
        _ => return unsafe { DefWindowProcW(h, msg, wparam, lparam) },
    }
    LRESULT(0)
}
