//! Shell icon extraction on a background STA thread. Shell lookups can touch
//! the disk, so they stay off the UI thread.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use windows::Win32::Foundation::{LPARAM, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP, HGDIOBJ};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::PCWSTR;

use crate::capsule::util::{hwnd, wide};

const IDLE: Duration = Duration::from_secs(10);

struct Request {
    id: u64,
    name: String,
    px: u32,
}

#[derive(Default)]
struct Queue {
    requests: VecDeque<Request>,
    running: bool,
    closed: bool,
}

type Shared = Arc<(Mutex<Queue>, Condvar)>;

/// The thread exits after `IDLE` without requests and restarts on the next one.
pub struct IconWorker {
    queue: Shared,
    notify: isize,
    msg: u32,
}

impl IconWorker {
    pub fn new(notify: isize, msg: u32) -> Self {
        Self { queue: Shared::default(), notify, msg }
    }

    /// Posts `msg(id, HBITMAP)` to the notify window when done; the receiver
    /// owns the bitmap, which is null when the lookup failed. `name` is a path or a `shell:AppsFolder\…` name.
    pub fn request(&mut self, id: u64, name: String, px: u32) {
        let (lock, cvar) = &*self.queue;
        let mut q = lock.lock().unwrap();
        q.requests.push_back(Request { id, name, px });
        if q.running {
            cvar.notify_one();
            return;
        }
        q.running = true;
        let (queue, notify, msg) = (self.queue.clone(), self.notify, self.msg);
        std::thread::spawn(move || run(&queue, notify, msg));
    }
}

impl Drop for IconWorker {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.queue;
        let mut q = lock.lock().unwrap();
        q.closed = true;
        q.requests.clear();
        cvar.notify_one();
    }
}

fn next(queue: &Shared) -> Option<Request> {
    let (lock, cvar) = &**queue;
    let q = lock.lock().unwrap();
    let (mut q, _) = cvar.wait_timeout_while(q, IDLE, |q| q.requests.is_empty() && !q.closed).unwrap();
    let req = q.requests.pop_front();
    q.running = req.is_some();
    req
}

fn run(queue: &Shared, notify: isize, msg: u32) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    while let Some(req) = next(queue) {
        let px = req.px as i32;
        let bitmap = unsafe {
            SHCreateItemFromParsingName::<_, _, IShellItemImageFactory>(PCWSTR(wide(&req.name).as_ptr()), None)
                .and_then(|f| f.GetImage(SIZE { cx: px, cy: px }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK))
        };
        let bitmap = bitmap.unwrap_or_else(|e| {
            log::debug!("icon {}: {e}", req.name);
            HBITMAP::default()
        });
        unsafe {
            if PostMessageW(Some(hwnd(notify)), msg, WPARAM(req.id as usize), LPARAM(bitmap.0 as isize)).is_err() {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
        }
    }
    unsafe { CoUninitialize() };
}
