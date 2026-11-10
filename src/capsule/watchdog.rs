//! A helper process that undoes 202020's changes outside itself (the taken
//! over taskbar, windows hidden on the other desktop) if 202020 dies without
//! cleaning up. It runs only while one of them is in effect and reads them
//! from memory shared with 202020.

use std::os::windows::io::AsRawHandle;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, Ordering::Relaxed};

use windows::Win32::Foundation::{HANDLE, HWND, INVALID_HANDLE_VALUE, RECT, WAIT_OBJECT_0};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP, FILE_MAP_ALL_ACCESS, FILE_MAP_READ, MapViewOfFile, OpenFileMappingW, PAGE_READWRITE,
};
use windows::Win32::System::Threading::{
    CreateEventW, EVENT_MODIFY_STATE, GetCurrentProcess, GetCurrentProcessId, INFINITE, OpenEventW,
    PROCESS_SYNCHRONIZE, SYNCHRONIZATION_SYNCHRONIZE, SetEvent, SetProcessWorkingSetSize, WaitForMultipleObjects,
    WaitForSingleObject,
};
use windows::core::PCWSTR;

use crate::capsule::shell::{self, Saved};
use crate::capsule::util::{OwnedHandle, View, hwnd, open_process, wide};
use crate::capsule::{desktops, hook};

const CAPACITY: usize = 1024;
const READY_TIMEOUT_MS: u32 = 1000;

#[repr(C)]
struct Slot {
    hwnd: AtomicIsize,
    pid: AtomicU32,
}

#[repr(C)]
struct Shared {
    takeover: AtomicBool,
    ab_state: AtomicU32,
    work_area: [AtomicI32; 4],
    count: AtomicU32,
    hidden: [Slot; CAPACITY],
}

impl Shared {
    fn outstanding(&self) -> bool {
        self.takeover.load(Relaxed) || self.count.load(Relaxed) != 0
    }
}

struct Guard {
    shared: &'static Shared,
    _view: View,
    _mapping: OwnedHandle,
    exit: OwnedHandle,
}

unsafe impl Send for Guard {}

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe {
            let _ = SetEvent(self.exit.0);
        }
    }
}

static GUARD: Mutex<Option<Guard>> = Mutex::new(None);
/// Numbers each watchdog so a new one never opens a previous one's objects.
static SPAWNS: AtomicU32 = AtomicU32::new(0);

fn object_name(kind: &str, pid: u32, seq: u32) -> Vec<u16> {
    wide(&format!("Local\\202020-{kind}-{pid}-{seq}"))
}

fn map(mapping: HANDLE, access: FILE_MAP) -> Option<(View, &'static Shared)> {
    let view = View(unsafe { MapViewOfFile(mapping, access, 0, 0, size_of::<Shared>()) });
    let shared = view.0.Value as *const Shared;
    (!shared.is_null()).then(|| (view, unsafe { &*shared }))
}

impl Guard {
    fn spawn() -> Option<Guard> {
        let pid = unsafe { GetCurrentProcessId() };
        let seq = SPAWNS.fetch_add(1, Relaxed);
        let name = object_name("guard", pid, seq);
        let size = size_of::<Shared>() as u32;
        let mapping =
            unsafe { CreateFileMappingW(INVALID_HANDLE_VALUE, None, PAGE_READWRITE, 0, size, PCWSTR(name.as_ptr())) }
                .map_err(|e| log::error!("watchdog mapping: {e}"))
                .ok()?;
        let mapping = OwnedHandle(mapping);
        let (view, shared) = map(mapping.0, FILE_MAP_ALL_ACCESS)?;
        let event = |kind| unsafe { CreateEventW(None, true, false, PCWSTR(object_name(kind, pid, seq).as_ptr())) };
        let exit = OwnedHandle(event("exit").ok()?);
        let guard = Guard { shared, _view: view, _mapping: mapping, exit };
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
            return None;
        }
        Some(guard)
    }
}

/// Writes to the shared state, then starts or ends the watchdog to match.
/// Returns false if the watchdog is needed but could not start.
fn update(needed: bool, write: impl FnOnce(&Shared)) -> bool {
    let mut slot = GUARD.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        if !needed {
            return true;
        }
        *slot = Guard::spawn();
    }
    let Some(guard) = slot.as_ref() else { return false };
    write(guard.shared);
    if !guard.shared.outstanding() {
        *slot = None;
    }
    true
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
        s.takeover.store(saved.is_some(), Relaxed);
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
    desktops::show(
        s.hidden[..n]
            .iter()
            .map(|slot| (hwnd(slot.hwnd.load(Relaxed)), slot.pid.load(Relaxed)))
            .filter(desktops::alive),
    );
    if n > 0 {
        log::info!("{n} hidden windows shown");
    }
    if s.takeover.load(Relaxed) {
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
    let name = object_name("guard", pid, seq);
    let Ok(mapping) = (unsafe { OpenFileMappingW(FILE_MAP_READ.0, false, PCWSTR(name.as_ptr())) }) else {
        return 1;
    };
    let mapping = OwnedHandle(mapping);
    let Some((_view, shared)) = map(mapping.0, FILE_MAP_READ) else {
        return 1;
    };
    let open = |kind, access| {
        let name = object_name(kind, pid, seq);
        unsafe { OpenEventW(access, false, PCWSTR(name.as_ptr())) }.ok().map(OwnedHandle)
    };
    let (Some(exit), Some(ready)) = (open("exit", SYNCHRONIZATION_SYNCHRONIZE), open("ready", EVENT_MODIFY_STATE))
    else {
        return 1;
    };
    unsafe {
        let _ = SetEvent(ready.0);
    }
    super::init_logging("watchdog.txt");
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
    let r = unsafe { WaitForMultipleObjects(&[process.0, exit.0], false, INFINITE) };
    if r == WAIT_OBJECT_0 && unsafe { WaitForSingleObject(exit.0, 0) } != WAIT_OBJECT_0 {
        log::warn!("202020 {pid} exited without cleanup");
        restore(shared, pid);
    }
    0
}
