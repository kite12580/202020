//! Loaded into explorer's tray thread, where it subclasses `Shell_TrayWnd`
//! and relays tray icon messages to the capsule bar. A panic here aborts
//! explorer, so nothing may panic.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering::Relaxed};

use capsule_core::tray_proto::{COPYDATA_ICON_RECT, COPYDATA_NOTIFYICON, HOOK_ATTACH_MSG, HOOK_DETACH_MSG};
use windows::Win32::Foundation::{CloseHandle, FreeLibrary, HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::{
    FreeLibraryAndExitThread, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleHandleExW,
};
use windows::Win32::System::Threading::{CreateThread, ExitThread, GetCurrentThreadId, THREAD_CREATION_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CallWindowProcW, GWLP_WNDPROC, GetWindowLongPtrW, GetWindowThreadProcessId, HC_ACTION, IsWindow,
    MSG, PM_REMOVE, PostMessageW, RegisterWindowMessageW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, SendMessageW,
    SetWindowLongPtrW, WM_COPYDATA, WM_NCDESTROY, WM_NULL, WNDPROC,
};
use windows::core::PCWSTR;

static ATTACH_MSG: AtomicU32 = AtomicU32::new(0);
static DETACH_MSG: AtomicU32 = AtomicU32::new(0);
/// The tray's window procedure while we subclass it, else 0.
static ORIGINAL: AtomicIsize = AtomicIsize::new(0);
static RECEIVER: AtomicIsize = AtomicIsize::new(0);
/// Guards against the relay window's handle being reused by another process
/// after the bar exits without detaching.
static RECEIVER_PID: AtomicU32 = AtomicU32::new(0);
/// Our own reference to this DLL, which keeps it loaded after the bar
/// removes the hook, else 0.
static PIN: AtomicIsize = AtomicIsize::new(0);
static DEPTH: AtomicU32 = AtomicU32::new(0);
/// Set by `detach`. Unloading while a `subclass` frame is still on the stack
/// would return into unmapped code, so the last frame to return unloads.
static UNLOAD: AtomicBool = AtomicBool::new(false);

const RELAY_TIMEOUT_MS: u32 = 500;

fn register(name: &str) -> u32 {
    let mut buf = [0u16; 64];
    for (slot, unit) in buf.iter_mut().take(63).zip(name.encode_utf16()) {
        *slot = unit;
    }
    unsafe { RegisterWindowMessageW(PCWSTR(buf.as_ptr())) }
}

fn hwnd(v: isize) -> HWND {
    HWND(v as *mut _)
}

fn pid_of(h: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

fn subclass_addr() -> isize {
    subclass as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT as usize as isize
}

/// # Safety
/// Called by user32 as a `WH_GETMESSAGE` hook procedure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetMsgProc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam.0 == PM_REMOVE.0 as usize && lparam.0 != 0 {
        if ATTACH_MSG.load(Relaxed) == 0 {
            DETACH_MSG.store(register(HOOK_DETACH_MSG), Relaxed);
            ATTACH_MSG.store(register(HOOK_ATTACH_MSG), Relaxed);
        }
        let msg = unsafe { &mut *(lparam.0 as *mut MSG) };
        if msg.hwnd.is_invalid() && msg.message != 0 && msg.message == ATTACH_MSG.load(Relaxed) {
            attach(hwnd(msg.lParam.0), hwnd(msg.wParam.0 as isize));
            msg.message = WM_NULL;
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn attach(tray: HWND, receiver: HWND) {
    let receiver_pid = pid_of(receiver);
    if receiver_pid == 0 || unsafe { GetWindowThreadProcessId(tray, None) } != unsafe { GetCurrentThreadId() } {
        return;
    }
    let pinned_here = PIN.load(Relaxed) == 0;
    if pinned_here {
        let mut module = HMODULE::default();
        let addr = PCWSTR(subclass_addr() as *const u16);
        if unsafe { GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, addr, &mut module) }.is_err() {
            return;
        }
        PIN.store(module.0 as isize, Relaxed);
    }
    if ORIGINAL.load(Relaxed) == 0 {
        let original = unsafe { SetWindowLongPtrW(tray, GWLP_WNDPROC, subclass_addr()) };
        if original == 0 {
            if pinned_here {
                // The hook's own reference keeps this code mapped until we return.
                let _ = unsafe { FreeLibrary(HMODULE(PIN.swap(0, Relaxed) as *mut _)) };
            }
            return;
        }
        ORIGINAL.store(original, Relaxed);
    }
    UNLOAD.store(false, Relaxed);
    RECEIVER_PID.store(receiver_pid, Relaxed);
    RECEIVER.store(receiver.0 as isize, Relaxed);
    unsafe {
        let _ = PostMessageW(Some(receiver), ATTACH_MSG.load(Relaxed), WPARAM(0), LPARAM(0));
    }
}

fn original(h: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let proc: WNDPROC = unsafe { std::mem::transmute(ORIGINAL.load(Relaxed)) };
    unsafe { CallWindowProcW(proc, h, msg, wparam, lparam) }
}

unsafe extern "system" fn subclass(h: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    DEPTH.fetch_add(1, Relaxed);
    let r = match msg {
        WM_COPYDATA => relay(h, wparam, lparam),
        WM_NCDESTROY => {
            let r = original(h, msg, wparam, lparam);
            ORIGINAL.store(0, Relaxed);
            RECEIVER.store(0, Relaxed);
            r
        }
        m if m != 0 && m == DETACH_MSG.load(Relaxed) => {
            detach(h, wparam);
            LRESULT(0)
        }
        _ => original(h, msg, wparam, lparam),
    };
    if DEPTH.fetch_sub(1, Relaxed) == 1 && UNLOAD.load(Relaxed) {
        unload(h);
    }
    r
}

fn relay(h: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let cds = lparam.0 as *const COPYDATASTRUCT;
    match (!cds.is_null()).then(|| unsafe { (*cds).dwData }) {
        Some(COPYDATA_NOTIFYICON) => {
            let r = original(h, WM_COPYDATA, wparam, lparam);
            send(wparam, lparam);
            r
        }
        Some(COPYDATA_ICON_RECT) => send(wparam, lparam).unwrap_or_else(|| original(h, WM_COPYDATA, wparam, lparam)),
        _ => original(h, WM_COPYDATA, wparam, lparam),
    }
}

fn send(wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let receiver = hwnd(RECEIVER.load(Relaxed));
    if receiver.is_invalid() {
        return None;
    }
    if pid_of(receiver) != RECEIVER_PID.load(Relaxed) {
        RECEIVER.store(0, Relaxed);
        return None;
    }
    let mut result = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            receiver,
            WM_COPYDATA,
            wparam,
            lparam,
            SMTO_ABORTIFHUNG,
            RELAY_TIMEOUT_MS,
            Some(&mut result),
        )
    };
    (ok.0 != 0).then_some(LRESULT(result as isize))
}

/// Ignores detaches from other bar processes than the attached one, such as
/// a watchdog's for a bar that was already restarted. Stays subclassed,
/// relaying nothing, if another subclass was installed on top of ours:
/// restoring the original procedure would cut it out.
fn detach(h: HWND, pid: WPARAM) {
    if pid.0 as u32 != RECEIVER_PID.load(Relaxed) {
        return;
    }
    RECEIVER.store(0, Relaxed);
    if unsafe { GetWindowLongPtrW(h, GWLP_WNDPROC) } == subclass_addr() {
        unsafe { SetWindowLongPtrW(h, GWLP_WNDPROC, ORIGINAL.swap(0, Relaxed)) };
        UNLOAD.store(true, Relaxed);
    }
}

/// Stays loaded when the tray is gone, since `unload_thread` could not wait
/// for the tray thread to leave this code.
fn unload(tray: HWND) {
    UNLOAD.store(false, Relaxed);
    let pin = PIN.swap(0, Relaxed);
    if pin == 0 || !unsafe { IsWindow(Some(tray)) }.as_bool() {
        PIN.store(pin, Relaxed);
        return;
    }
    let param = Some(tray.0 as *const c_void);
    match unsafe { CreateThread(None, 0, Some(unload_thread), param, THREAD_CREATION_FLAGS(0), None) } {
        Ok(t) => unsafe {
            let _ = CloseHandle(t);
        },
        Err(_) => PIN.store(pin, Relaxed),
    }
}

/// The tray thread handles the sent `WM_NULL` only from its message loop,
/// so by then it has returned out of `subclass`. Releases the pin `unload`
/// took from `PIN`.
unsafe extern "system" fn unload_thread(tray: *mut c_void) -> u32 {
    let tray = HWND(tray);
    unsafe {
        SendMessageW(tray, WM_NULL, None, None);
        let mut module = HMODULE::default();
        let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
        if !IsWindow(Some(tray)).as_bool()
            || GetModuleHandleExW(flags, PCWSTR(subclass_addr() as *const u16), &mut module).is_err()
        {
            ExitThread(0);
        }
        FreeLibraryAndExitThread(module, 0)
    }
}
