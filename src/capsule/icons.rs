//! Shell icon extraction on a background STA thread. Shell lookups can touch
//! the disk, so they stay off the UI thread.

use std::sync::mpsc::{Receiver, Sender, channel};

use windows::Win32::Foundation::{LPARAM, SIZE, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::PCWSTR;

use crate::capsule::util::{hwnd, wide};

struct Request {
    id: u64,
    name: String,
    px: u32,
}

pub struct IconWorker {
    tx: Option<Sender<Request>>,
    notify: isize,
    msg: u32,
}

impl IconWorker {
    pub fn new(notify: isize, msg: u32) -> Self {
        Self { tx: None, notify, msg }
    }

    /// Posts `msg(id, HBITMAP)` to the notify window when done; the receiver
    /// owns the bitmap. `name` is a path or a `shell:AppsFolder\…` name.
    pub fn request(&mut self, id: u64, name: String, px: u32) {
        let (notify, msg) = (self.notify, self.msg);
        let tx = self.tx.get_or_insert_with(|| {
            let (tx, rx) = channel();
            std::thread::spawn(move || run(rx, notify, msg));
            tx
        });
        let _ = tx.send(Request { id, name, px });
    }
}

fn run(rx: Receiver<Request>, notify: isize, msg: u32) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    for req in rx {
        let px = req.px as i32;
        let bitmap = unsafe {
            SHCreateItemFromParsingName::<_, _, IShellItemImageFactory>(PCWSTR(wide(&req.name).as_ptr()), None)
                .and_then(|f| f.GetImage(SIZE { cx: px, cy: px }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK))
        };
        match bitmap {
            Ok(b) => unsafe {
                let _ = PostMessageW(Some(hwnd(notify)), msg, WPARAM(req.id as usize), LPARAM(b.0 as isize));
            },
            Err(e) => log::debug!("icon {}: {e}", req.name),
        }
    }
}
