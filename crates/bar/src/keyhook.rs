use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering::Relaxed};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_CAPITAL, VK_CONTROL, VK_LCONTROL, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RSHIFT, VK_RWIN,
    VK_SHIFT, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, PM_NOREMOVE, PeekMessageW,
    PostMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT,
    WM_SYSKEYDOWN, WM_USER,
};

use crate::util::hwnd;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Launch {
    Activate,
    New,
    Admin,
}

impl Launch {
    pub fn from_raw(v: isize) -> Launch {
        match v {
            1 => Launch::New,
            2 => Launch::Admin,
            _ => Launch::Activate,
        }
    }
}

static TARGET: AtomicIsize = AtomicIsize::new(0);
static DIGIT_MSG: AtomicU32 = AtomicU32::new(0);
static INPUT_MSG: AtomicU32 = AtomicU32::new(0);
static DIGITS: AtomicBool = AtomicBool::new(false);
static SWALLOWED: AtomicU32 = AtomicU32::new(0);
static INPUT_PENDING: AtomicBool = AtomicBool::new(false);

/// Unassigned virtual key, injected so releasing Win after a swallowed
/// Win+digit does not open Start.
const VK_MASK: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);

/// Low-level keyboard hook on its own thread. Posts `digit_msg(index, Launch)`
/// for Win+1..0 when enabled and `input_msg` after keys that may switch the
/// input language or IME mode.
pub struct KeyHook {
    thread: u32,
    join: Option<JoinHandle<()>>,
}

impl KeyHook {
    pub fn start(target: HWND, digit_msg: u32, input_msg: u32) -> Option<KeyHook> {
        TARGET.store(target.0 as isize, Relaxed);
        DIGIT_MSG.store(digit_msg, Relaxed);
        INPUT_MSG.store(input_msg, Relaxed);
        let (tx, rx) = std::sync::mpsc::channel();
        let join = std::thread::spawn(move || unsafe {
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
            let module = GetModuleHandleW(None).ok().map(Into::into);
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0);
            let _ = tx.send(hook.is_ok().then(|| GetCurrentThreadId()));
            let Ok(hook) = hook else { return };
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {}
            let _ = UnhookWindowsHookEx(hook);
        });
        let thread = rx.recv().ok().flatten()?;
        Some(KeyHook { thread, join: Some(join) })
    }

    pub fn set_digits(&self, on: bool) {
        DIGITS.store(on, Relaxed);
    }
}

impl Drop for KeyHook {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        if k.flags.0 & LLKHF_INJECTED.0 == 0 && on_key(k.vkCode, down) {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
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
        let win = held(VK_LWIN) || held(VK_RWIN);
        if SWALLOWED.load(Relaxed) & bit != 0 {
            if win {
                return true;
            }
            SWALLOWED.fetch_and(!bit, Relaxed);
        }
        if !DIGITS.load(Relaxed) || !win || held(VK_MENU) {
            return false;
        }
        SWALLOWED.fetch_or(bit, Relaxed);
        let launch = match (held(VK_SHIFT), held(VK_CONTROL)) {
            (false, _) => Launch::Activate,
            (true, false) => Launch::New,
            (true, true) => Launch::Admin,
        };
        post(&DIGIT_MSG, (n as usize + 9) % 10, launch as isize);
        mask_start_menu();
        return true;
    }
    let vk = VIRTUAL_KEY(vk as u16);
    let switches = match vk {
        VK_LSHIFT | VK_RSHIFT | VK_LCONTROL | VK_RCONTROL | VK_LWIN | VK_RWIN | VK_CAPITAL => true,
        VK_SPACE => held(VK_LWIN) || held(VK_RWIN) || held(VK_CONTROL) || held(VK_SHIFT),
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

fn mask_start_menu() {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_MASK, dwFlags: flags, ..Default::default() } },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}
