use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering::Relaxed};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_CAPITAL, VK_CONTROL, VK_LCONTROL, VK_LSHIFT, VK_LWIN, VK_RCONTROL, VK_RSHIFT, VK_RWIN, VK_SHIFT,
    VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE, PeekMessageW, PostMessageW,
    PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
    WM_USER,
};

use crate::capsule::hyper;
use crate::capsule::input::{OWN_INPUT, held, win_held};
use crate::capsule::util::hwnd;

static TARGET: AtomicIsize = AtomicIsize::new(0);
static DIGIT_MSG: AtomicU32 = AtomicU32::new(0);
static INPUT_MSG: AtomicU32 = AtomicU32::new(0);
static DIGITS: AtomicBool = AtomicBool::new(false);
static SWALLOWED: AtomicU32 = AtomicU32::new(0);
static INPUT_PENDING: AtomicBool = AtomicBool::new(false);
static BAR: AtomicBool = AtomicBool::new(false);
static HYPER: AtomicBool = AtomicBool::new(false);
static THREAD: Mutex<Option<(u32, JoinHandle<()>)>> = Mutex::new(None);

const WM_HYPER_RESET: u32 = WM_USER + 1;

/// The capsule bar's use of the shared low-level keyboard hook. While held,
/// the hook swallows Win+0..9 when digits are on and posts `digit_msg(vk)` so
/// the bar can replay it once explorer's hidden taskbar is able to handle it.
/// It posts `input_msg` after keys that may switch the input language or IME
/// mode.
pub struct KeyHook;

impl KeyHook {
    pub fn start(target: HWND, digit_msg: u32, input_msg: u32) -> Option<KeyHook> {
        TARGET.store(target.0 as isize, Relaxed);
        DIGIT_MSG.store(digit_msg, Relaxed);
        INPUT_MSG.store(input_msg, Relaxed);
        SWALLOWED.store(0, Relaxed);
        INPUT_PENDING.store(false, Relaxed);
        BAR.store(true, Relaxed);
        if sync().is_some() {
            Some(KeyHook)
        } else {
            BAR.store(false, Relaxed);
            None
        }
    }

    pub fn set_digits(&self, on: bool) {
        DIGITS.store(on, Relaxed);
    }
}

impl Drop for KeyHook {
    fn drop(&mut self) {
        BAR.store(false, Relaxed);
        DIGITS.store(false, Relaxed);
        sync();
    }
}

pub fn set_hyper(on: bool) {
    HYPER.store(on, Relaxed);
    match sync() {
        None => HYPER.store(false, Relaxed),
        Some(tid) if !on => unsafe {
            let _ = PostThreadMessageW(tid, WM_HYPER_RESET, WPARAM(0), LPARAM(0));
        },
        _ => {}
    }
}

pub fn hyper_on() -> bool {
    HYPER.load(Relaxed)
}

/// Starts or ends the hook thread to match its users. Returns its thread id
/// while it runs.
fn sync() -> Option<u32> {
    let mut thread = THREAD.lock().unwrap_or_else(|e| e.into_inner());
    let wanted = BAR.load(Relaxed) || HYPER.load(Relaxed);
    match (thread.is_some(), wanted) {
        (false, true) => *thread = spawn(),
        (true, false) => {
            if let Some((tid, join)) = thread.take() {
                unsafe {
                    let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
                }
                let _ = join.join();
            }
        }
        _ => {}
    }
    thread.as_ref().map(|(tid, _)| *tid)
}

fn spawn() -> Option<(u32, JoinHandle<()>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let join = std::thread::Builder::new()
        .name("keyhook".into())
        .stack_size(256 << 10)
        .spawn(move || unsafe {
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
            let module = GetModuleHandleW(None).ok().map(Into::into);
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0);
            let _ = tx.send(hook.is_ok().then(|| GetCurrentThreadId()));
            let Ok(hook) = hook else { return };
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                if msg.message == WM_HYPER_RESET {
                    hyper::reset();
                }
            }
            hyper::reset();
            let _ = UnhookWindowsHookEx(hook);
        })
        .ok()?;
    match rx.recv().ok().flatten() {
        Some(tid) => Some((tid, join)),
        None => {
            let _ = join.join();
            None
        }
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        if k.dwExtraInfo != OWN_INPUT
            && ((HYPER.load(Relaxed) && hyper::on_key(k.vkCode, down, k.time))
                || (BAR.load(Relaxed) && on_key(k.vkCode, down)))
        {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn post(msg: &AtomicU32, wparam: usize, lparam: isize) {
    unsafe {
        let _ = PostMessageW(Some(hwnd(TARGET.load(Relaxed))), msg.load(Relaxed), WPARAM(wparam), LPARAM(lparam));
    }
}

/// Returns whether to swallow the key.
fn on_key(vk: u32, down: bool) -> bool {
    if let Some(n) = vk.checked_sub(u32::from(b'0')).filter(|&n| n <= 9) {
        let bit = 1 << n;
        if !down {
            return SWALLOWED.fetch_and(!bit, Relaxed) & bit != 0;
        }
        let win = win_held();
        if SWALLOWED.load(Relaxed) & bit != 0 {
            if win {
                return true;
            }
            SWALLOWED.fetch_and(!bit, Relaxed);
        }
        if !DIGITS.load(Relaxed) || !win {
            return false;
        }
        SWALLOWED.fetch_or(bit, Relaxed);
        post(&DIGIT_MSG, vk as usize, 0);
        return true;
    }
    let vk = VIRTUAL_KEY(vk as u16);
    let switches = match vk {
        VK_LSHIFT | VK_RSHIFT | VK_LCONTROL | VK_RCONTROL | VK_LWIN | VK_RWIN | VK_CAPITAL => true,
        VK_SPACE => win_held() || held(VK_CONTROL) || held(VK_SHIFT),
        _ => false,
    };
    if !down && switches && !INPUT_PENDING.swap(true, Relaxed) {
        post(&INPUT_MSG, 0, 0);
    }
    false
}

pub fn input_handled() {
    INPUT_PENDING.store(false, Relaxed);
}
