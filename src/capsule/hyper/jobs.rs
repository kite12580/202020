use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use capsule_core::config::Hyper;
use capsule_core::hyper::{
    Chord, Job, alpha_step, alpha_toggle, c, copy_accept, hotstring, k, line_start_probe, s, vk, window_preset,
    wrap_text,
};
use windows::Win32::Foundation::{COLORREF, CloseHandle, HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME,
    RDW_INVALIDATE, RedrawWindow,
};
use windows::Win32::System::DataExchange::{AddClipboardFormatListener, RemoveClipboardFormatListener};
use windows::Win32::System::Threading::{CreateProcessW, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTUPINFOW};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, CreateWindowExW, DestroyWindow, DispatchMessageW, GWL_EXSTYLE, GWL_STYLE,
    GetCursorPos, GetLayeredWindowAttributes, GetWindowLongPtrW, HWND_MESSAGE, HWND_NOTOPMOST, HWND_TOPMOST, IsZoomed,
    LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT,
    SW_RESTORE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetLayeredWindowAttributes,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CAPTION, WS_EX_LAYERED,
    WS_EX_TOPMOST, WS_THICKFRAME,
};
use windows::core::{PCWSTR, PWSTR, w};

use crate::capsule::util::{foreground, hwnd, px_rect, wide};
use crate::capsule::{clipboard, input};

const COPY_TIMEOUT: Duration = Duration::from_millis(100);
const PASTE_DELAY: Duration = Duration::from_millis(150);
const FRAME: isize = (WS_CAPTION.0 | WS_THICKFRAME.0) as isize;

static BUSY: AtomicBool = AtomicBool::new(false);

const COPY: Chord = c(vk::INSERT);
const PASTE: Chord = s(vk::INSERT);

pub fn spawn(job: Job, cfg: Arc<Hyper>) {
    if BUSY.swap(true, Relaxed) {
        return;
    }
    let spawned = std::thread::Builder::new().name("hyper job".into()).stack_size(256 << 10).spawn(move || {
        run(job, &cfg);
        BUSY.store(false, Relaxed);
    });
    if spawned.is_err() {
        BUSY.store(false, Relaxed);
    }
}

fn run(job: Job, cfg: &Hyper) {
    match job {
        Job::Topmost => toggle_topmost(),
        Job::Caption => toggle_caption(),
        Job::EscapeOrMove => escape_or_move(),
        Job::PowerOptions => launch("control.exe", "/name Microsoft.PowerOptions"),
        Job::CopyOrLine => {
            if let Some(clip) = Clip::new() {
                clip.copy_or_line();
            }
        }
        _ => {
            if let Some(clip) = Clip::new() {
                clip.edit(job, cfg);
            }
        }
    }
}

fn toggle_topmost() {
    let Some(h) = foreground() else { return };
    unsafe {
        let topmost = GetWindowLongPtrW(h, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0;
        let after = if topmost { HWND_NOTOPMOST } else { HWND_TOPMOST };
        let _ = SetWindowPos(h, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

fn toggle_caption() {
    let Some(h) = foreground() else { return };
    unsafe {
        let style = GetWindowLongPtrW(h, GWL_STYLE);
        SetWindowLongPtrW(h, GWL_STYLE, if style & FRAME != 0 { style & !FRAME } else { style | FRAME });
        let flags = SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
        let _ = SetWindowPos(h, None, 0, 0, 0, 0, flags);
    }
}

fn escape_or_move() {
    let mut pt = POINT::default();
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetCursorPos(&mut pt);
        let _ = GetMonitorInfoW(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), &mut info);
    }
    if pt.x > info.rcMonitor.left {
        return input::keys(&[k(vk::ESCAPE)]);
    }
    let Some(h) = foreground() else { return };
    let (r, topmost) = window_preset(px_rect(info.rcMonitor), px_rect(info.rcWork), pt.y);
    unsafe {
        if IsZoomed(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let (after, flags) =
            if topmost { (Some(HWND_TOPMOST), SWP_NOACTIVATE) } else { (None, SWP_NOACTIVATE | SWP_NOZORDER) };
        let _ = SetWindowPos(h, after, r.left, r.top, r.width(), r.height(), flags);
    }
}

fn launch(command: &str, args: &str) {
    let mut line = wide(&format!("\"{command}\" {args}"));
    let startup = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    let mut info = PROCESS_INFORMATION::default();
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        let created = CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            false,
            PROCESS_CREATION_FLAGS(0),
            None,
            PCWSTR::null(),
            &startup,
            &mut info,
        );
        match created {
            Ok(()) => {
                let _ = CloseHandle(info.hThread);
                let _ = CloseHandle(info.hProcess);
            }
            Err(e) => log::warn!("hyper: start {command}: {e}"),
        }
    }
}

pub enum AlphaMsg {
    Wheel(bool),
    End { tap: bool },
}

pub fn alpha_session(h: HWND) -> Option<Sender<AlphaMsg>> {
    let (tx, rx) = mpsc::channel();
    let raw = h.0 as isize;
    std::thread::Builder::new()
        .name("hyper alpha".into())
        .stack_size(64 << 10)
        .spawn(move || {
            let h = hwnd(raw);
            let mut wheeled = false;
            for msg in rx {
                match msg {
                    AlphaMsg::Wheel(up) => {
                        wheeled = true;
                        set_alpha(h, alpha_step(alpha(h), up));
                    }
                    AlphaMsg::End { tap } => {
                        if tap && !wheeled {
                            set_alpha(h, alpha_toggle(alpha(h)));
                        }
                        break;
                    }
                }
            }
        })
        .ok()?;
    Some(tx)
}

fn alpha(h: HWND) -> Option<u8> {
    unsafe {
        if GetWindowLongPtrW(h, GWL_EXSTYLE) & WS_EX_LAYERED.0 as isize == 0 {
            return None;
        }
        let (mut a, mut flags) = (255u8, LAYERED_WINDOW_ATTRIBUTES_FLAGS(0));
        GetLayeredWindowAttributes(h, None, Some(&mut a), Some(&mut flags)).ok()?;
        (flags.contains(LWA_ALPHA) && a < 255).then_some(a)
    }
}

fn set_alpha(h: HWND, alpha: Option<u8>) {
    unsafe {
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let layered = WS_EX_LAYERED.0 as isize;
        match alpha {
            Some(a) => {
                if ex & layered == 0 {
                    SetWindowLongPtrW(h, GWL_EXSTYLE, ex | layered);
                }
                let _ = SetLayeredWindowAttributes(h, COLORREF(0), a, LWA_ALPHA);
            }
            None => {
                let _ = SetLayeredWindowAttributes(h, COLORREF(0), 255, LWA_ALPHA);
                SetWindowLongPtrW(h, GWL_EXSTYLE, ex & !layered);
                let _ = RedrawWindow(Some(h), None, None, RDW_ERASE | RDW_INVALIDATE | RDW_FRAME | RDW_ALLCHILDREN);
            }
        }
    }
}

struct Clip(HWND);

impl Clip {
    fn new() -> Option<Clip> {
        let h = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }
        .ok()?;
        let _ = unsafe { AddClipboardFormatListener(h) };
        Some(Clip(h))
    }

    fn wait_change(&self, since: u32) -> bool {
        let deadline = Instant::now() + COPY_TIMEOUT;
        loop {
            if clipboard::sequence() != since {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            unsafe {
                MsgWaitForMultipleObjects(None, false, left.as_millis() as u32 + 1, QS_ALLINPUT);
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&msg);
                }
            }
        }
    }

    fn copy(&self) -> Option<String> {
        let since = clipboard::sequence();
        input::keys(&[COPY]);
        if self.wait_change(since) { clipboard::get(self.0) } else { None }
    }

    /// Pastes `text`; returns the clipboard sequence number of our write.
    fn paste(&self, text: &str) -> u32 {
        clipboard::set_private(self.0, text);
        let written = clipboard::sequence();
        input::keys(&[PASTE]);
        written
    }

    fn copy_or_line(&self) {
        if !self.copy().is_some_and(|t| copy_accept(&t)) {
            input::keys(&[k(vk::HOME), s(vk::END), COPY, k(vk::END)]);
        }
    }

    fn edit(&self, job: Job, cfg: &Hyper) {
        let saved = clipboard::save(self.0);
        let expected = match job {
            Job::PastePlain => match saved.text() {
                Some(text) => self.paste(&text),
                None => {
                    input::keys(&[PASTE]);
                    return;
                }
            },
            Job::Paste(text) => self.paste(&text),
            Job::Wrap { open, close, at_end } => {
                let selection = if at_end {
                    input::keys(&[k(vk::END)]);
                    None
                } else {
                    self.copy().filter(|t| copy_accept(t))
                };
                let (text, back) = wrap_text(&open, &close, selection.as_deref());
                let written = self.paste(&text);
                input::repeat(vk::LEFT, back);
                if at_end {
                    input::keys(&[k(vk::RETURN)]);
                }
                written
            }
            Job::DeleteLine => {
                input::keys(&[k(vk::HOME), s(vk::LEFT)]);
                let (at_start, selected) = line_start_probe(self.copy().as_deref());
                let copied = clipboard::sequence();
                if selected {
                    input::keys(&[k(vk::RIGHT)]);
                }
                if !at_start {
                    input::keys(&[k(vk::HOME)]);
                }
                input::keys(&[s(vk::END), s(vk::RIGHT), k(vk::DELETE), k(vk::END)]);
                copied
            }
            Job::Hotstring => {
                input::keys(&[s(vk::HOME)]);
                let line = self.copy().filter(|t| copy_accept(t));
                let copied = clipboard::sequence();
                if line.is_some() {
                    input::keys(&[k(vk::RIGHT)]);
                }
                let Some((n, text)) = line.as_deref().and_then(|l| hotstring(l, &cfg.hotstrings)) else {
                    return clipboard::restore(self.0, &saved, copied);
                };
                input::repeat(vk::BACK, n);
                self.paste(text)
            }
            _ => return,
        };
        std::thread::sleep(PASTE_DELAY);
        clipboard::restore(self.0, &saved, expected);
    }
}

impl Drop for Clip {
    fn drop(&mut self) {
        unsafe {
            let _ = RemoveClipboardFormatListener(self.0);
            let _ = DestroyWindow(self.0);
        }
    }
}
