//! A helper process that undoes 202020's changes outside itself (the taken
//! over taskbar, windows hidden on the other desktop) if 202020 dies without
//! cleaning up. It runs only while one of them is in effect and reads them
//! from memory shared with 202020.

use std::os::windows::io::AsRawHandle;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicIsize, AtomicU32, Ordering::Relaxed};

use windows::Win32::Foundation::{HANDLE, HWND, INVALID_HANDLE_VALUE, RECT, WAIT_OBJECT_0};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP, FILE_MAP_ALL_ACCESS, FILE_MAP_READ, MapViewOfFile, OpenFileMappingW, PAGE_READWRITE,
};
use windows::Win32::System::Threading::{
    CreateEventW, EVENT_MODIFY_STATE, GetCurrentProcess, GetCurrentProcessId, INFINITE, OpenEventW,
    PROCESS_SYNCHRONIZE, SYNCHRONIZATION_SYNCHRONIZE, SetEvent, SetProcessWorkingSetSize, WaitForMultipleObjects,
    WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{SW_SHOWNA, ShowWindowAsync};
use windows::core::PCWSTR;

use crate::capsule::hook;
use crate::capsule::shell::{self, Saved};
use crate::capsule::util::{OwnedHandle, hwnd, open_process, wide, window_ids};

pub const CAPACITY: usize = 1024;
const READY_TIMEOUT_MS: u32 = 1000;

#[repr(C)]
struct Slot {
    hwnd: AtomicIsize,
    pid: AtomicU32,
}

#[repr(C)]
struct Shared {
    takeover: AtomicU32,
    ab_state: AtomicU32,
    work_area: [AtomicI32; 4],
    count: AtomicU32,
    hidden: [Slot; CAPACITY],
}

struct Guard {
    shared: &'static Shared,
    _mapping: OwnedHandle,
    exit: Option<OwnedHandle>,
    seq: u32,
}

unsafe impl Send for Guard {}

static GUARD: Mutex<Option<Guard>> = Mutex::new(None);

fn mapping_name(pid: u32) -> Vec<u16> {
    wide(&format!("Local\\202020-guard-{pid}"))
}

fn event_name(kind: &str, pid: u32, seq: u32) -> Vec<u16> {
    wide(&format!("Local\\202020-{kind}-{pid}-{seq}"))
}

fn map(mapping: HANDLE, access: FILE_MAP) -> Option<&'static Shared> {
    let view = unsafe { MapViewOfFile(mapping, access, 0, 0, size_of::<Shared>()) };
    (!view.Value.is_null()).then(|| unsafe { &*(view.Value as *const Shared) })
}

impl Guard {
    fn create() -> Option<Guard> {
        let name = mapping_name(unsafe { GetCurrentProcessId() });
        let size = size_of::<Shared>() as u32;
        let mapping =
            unsafe { CreateFileMappingW(INVALID_HANDLE_VALUE, None, PAGE_READWRITE, 0, size, PCWSTR(name.as_ptr())) }
                .map_err(|e| log::error!("watchdog mapping: {e}"))
                .ok()?;
        let mapping = OwnedHandle(mapping);
        let shared = map(mapping.0, FILE_MAP_ALL_ACCESS)?;
        Some(Guard { shared, _mapping: mapping, exit: None, seq: 0 })
    }

    fn sync(&mut self) -> bool {
        let s = self.shared;
        let outstanding = s.takeover.load(Relaxed) != 0 || s.count.load(Relaxed) != 0;
        match (&self.exit, outstanding) {
            (None, true) => {
                self.exit = self.spawn();
                self.exit.is_some()
            }
            (Some(exit), false) => {
                unsafe {
                    let _ = SetEvent(exit.0);
                }
                self.exit = None;
                true
            }
            _ => true,
        }
    }

    fn spawn(&mut self) -> Option<OwnedHandle> {
        let pid = unsafe { GetCurrentProcessId() };
        let seq = self.seq;
        self.seq += 1;
        let event = |kind| unsafe { CreateEventW(None, true, false, PCWSTR(event_name(kind, pid, seq).as_ptr())) };
        let exit = OwnedHandle(event("exit").ok()?);
        let ready = OwnedHandle(event("ready").ok()?);
        let exe = std::env::current_exe().ok()?;
        let child = std::process::Command::new(exe)
            .arg("--watchdog")
            .args([pid, seq].map(|v| v.to_string()))
            .spawn()
            .map_err(|e| log::error!("watchdog: {e}"))
            .ok()?;
        let process = HANDLE(child.as_raw_handle());
        if unsafe { WaitForMultipleObjects(&[ready.0, process], false, READY_TIMEOUT_MS) } != WAIT_OBJECT_0 {
            log::error!("watchdog did not start");
            unsafe {
                let _ = SetEvent(exit.0);
            }
            return None;
        }
        Some(exit)
    }
}

/// Writes to the shared state, then starts or ends the watchdog to match.
/// Returns false if the watchdog is needed but could not start.
fn update(needed: bool, write: impl FnOnce(&Shared)) -> bool {
    let mut guard = GUARD.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        if !needed {
            return true;
        }
        *guard = Guard::create();
    }
    let Some(guard) = guard.as_mut() else { return false };
    write(guard.shared);
    guard.sync()
}

pub fn set_takeover(saved: Option<Saved>) -> bool {
    update(saved.is_some(), |s| {
        if let Some(saved) = saved {
            s.ab_state.store(saved.ab_state, Relaxed);
            let r = saved.work_area;
            for (slot, v) in s.work_area.iter().zip([r.left, r.top, r.right, r.bottom]) {
                slot.store(v, Relaxed);
            }
        }
        s.takeover.store(u32::from(saved.is_some()), Relaxed);
    })
}

pub fn set_hidden(windows: &[(HWND, u32)]) -> bool {
    if windows.len() > CAPACITY {
        log::warn!("{} windows are more than can be hidden", windows.len());
        return false;
    }
    update(!windows.is_empty(), |s| {
        for (slot, &(h, pid)) in s.hidden.iter().zip(windows) {
            slot.hwnd.store(h.0 as isize, Relaxed);
            slot.pid.store(pid, Relaxed);
        }
        s.count.store(windows.len() as u32, Relaxed);
    })
}

fn restore(s: &Shared, pid: u32) {
    let n = (s.count.load(Relaxed) as usize).min(CAPACITY);
    for slot in &s.hidden[..n] {
        let h = hwnd(slot.hwnd.load(Relaxed));
        if window_ids(h).1 == slot.pid.load(Relaxed) {
            unsafe {
                let _ = ShowWindowAsync(h, SW_SHOWNA);
            }
        }
    }
    if n > 0 {
        log::info!("{n} hidden windows shown");
    }
    if s.takeover.load(Relaxed) != 0 {
        if let Some(tray) = shell::find_real_tray() {
            hook::detach(tray, pid);
        }
        let [left, top, right, bottom] = [0, 1, 2, 3].map(|i| s.work_area[i].load(Relaxed));
        shell::restore(&Saved { ab_state: s.ab_state.load(Relaxed), work_area: RECT { left, top, right, bottom } });
    }
}

/// `--watchdog <pid> <seq>`
pub fn run(args: &[String]) -> i32 {
    let nums: Vec<u32> = args.iter().filter_map(|a| a.parse().ok()).collect();
    let [pid, seq] = nums[..] else {
        return 2;
    };
    let Some(process) = open_process(PROCESS_SYNCHRONIZE, pid) else {
        return 1;
    };
    let name = mapping_name(pid);
    let Ok(mapping) = (unsafe { OpenFileMappingW(FILE_MAP_READ.0, false, PCWSTR(name.as_ptr())) }) else {
        return 1;
    };
    let mapping = OwnedHandle(mapping);
    let Some(shared) = map(mapping.0, FILE_MAP_READ) else {
        return 1;
    };
    let open = |kind, access| {
        let name = event_name(kind, pid, seq);
        unsafe { OpenEventW(access, false, PCWSTR(name.as_ptr())) }.ok().map(OwnedHandle)
    };
    let (Some(exit), Some(ready)) = (open("exit", SYNCHRONIZATION_SYNCHRONIZE), open("ready", EVENT_MODIFY_STATE))
    else {
        return 1;
    };
    unsafe {
        let _ = SetEvent(ready.0);
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
    let r = unsafe { WaitForMultipleObjects(&[process.0, exit.0], false, INFINITE) };
    if r == WAIT_OBJECT_0 && unsafe { WaitForSingleObject(exit.0, 0) } != WAIT_OBJECT_0 {
        log::warn!("202020 {pid} exited without cleanup");
        restore(shared, pid);
    }
    0
}
