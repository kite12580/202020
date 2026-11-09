//! CapsLock hyper layer. Keys are decided on the shared keyboard hook's thread
//! (`keyhook`); anything that waits on the clipboard or another window runs on
//! a short-lived job thread.

mod jobs;
mod send;

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering::Relaxed};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use capsule_core::config::Hyper;
use capsule_core::hyper::{Action, Chord, Effect, Event, Job, Layer, Target, vk};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, HC_ACTION, HHOOK, MSLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_MOUSE_LL, WM_MOUSEWHEEL,
};

use crate::capsule::keyhook;
use crate::capsule::util::{class_name, hwnd, process_path, window_ids};
use jobs::AlphaMsg;

pub struct Options {
    pub owner: isize,
    pub command_msg: u32,
    pub reload_command: u32,
}

static OWNER: AtomicIsize = AtomicIsize::new(0);
static COMMAND_MSG: AtomicU32 = AtomicU32::new(0);
static RELOAD_COMMAND: AtomicU32 = AtomicU32::new(0);
static CONFIG: Mutex<Option<Arc<Hyper>>> = Mutex::new(None);
static GENERATION: AtomicU32 = AtomicU32::new(0);

struct Alpha {
    hook: HHOOK,
    tx: Sender<AlphaMsg>,
}

thread_local! {
    static LAYER: RefCell<Layer> = RefCell::default();
    static TARGET: Cell<(isize, u32, Target)> = const { Cell::new((0, u32::MAX, Target::Other)) };
    static SAVED_SPEED: Cell<Option<u32>> = const { Cell::new(None) };
    static ALPHA: RefCell<Option<Alpha>> = const { RefCell::new(None) };
}

pub fn init(opts: Options) {
    OWNER.store(opts.owner, Relaxed);
    COMMAND_MSG.store(opts.command_msg, Relaxed);
    RELOAD_COMMAND.store(opts.reload_command, Relaxed);
}

/// Applies `cfg`. Returns whether the layer is on.
pub fn apply(cfg: Hyper) -> bool {
    let on = cfg.enabled;
    *CONFIG.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(cfg));
    GENERATION.fetch_add(1, Relaxed);
    if on && !keyhook::hyper_on() {
        send::caps_lock_off();
    }
    keyhook::set_hyper(on) && on
}

pub fn stop() {
    keyhook::set_hyper(false);
}

fn config() -> Arc<Hyper> {
    CONFIG.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// Called by the keyboard hook for input not injected by 202020. Returns
/// whether to swallow the key.
pub(super) fn on_key(vk_code: u32, down: bool, time: u32) -> bool {
    let key = vk_code as u16;
    LAYER.with(|layer| {
        let Ok(mut layer) = layer.try_borrow_mut() else { return false };
        if key != vk::CAPITAL && !layer.engaged() {
            return false;
        }
        let mods = if layer.held() || key == vk::CAPITAL { send::held_mods() } else { 0 };
        layer.on_key(Event { vk: key, down, time, mods }, target, &mut run)
    })
}

/// Clears the layer when it is turned off or the hook thread ends.
pub(super) fn reset() {
    LAYER.with(|l| *l.borrow_mut() = Layer::default());
    slow_mouse(false);
    end_alpha(false);
}

fn run(effect: Effect) {
    match effect {
        Effect::Esc => send::keys(&[Chord { mods: 0, vk: vk::ESCAPE }]),
        Effect::Run(Action::Keys(keys)) => send::keys(keys),
        Effect::Run(Action::Wheel(n)) => send::wheel(n),
        Effect::Run(Action::Job(Job::Reload)) => unsafe {
            let _ = PostMessageW(
                Some(hwnd(OWNER.load(Relaxed))),
                COMMAND_MSG.load(Relaxed),
                WPARAM(RELOAD_COMMAND.load(Relaxed) as usize),
                LPARAM(0),
            );
        },
        Effect::Run(Action::Job(job)) => jobs::spawn(job, config()),
        Effect::Run(_) => {}
        Effect::Button(b, down) => send::button(b, down),
        Effect::SlowMouse(on) => slow_mouse(on),
        Effect::AlphaBegin => begin_alpha(),
        Effect::AlphaEnd { tap } => end_alpha(tap),
    }
}

fn target() -> Target {
    let fg = unsafe { GetForegroundWindow() };
    let generation = GENERATION.load(Relaxed);
    let (cached, cached_gen, t) = TARGET.get();
    if cached == fg.0 as isize && cached_gen == generation {
        return t;
    }
    let t = if class_name(fg) == "Vim" {
        Target::Vim
    } else {
        let path = process_path(window_ids(fg).1).unwrap_or_default();
        let exe = path.rsplit('\\').next().unwrap_or_default();
        if exe.eq_ignore_ascii_case("nvim-qt.exe") {
            Target::Vim
        } else if config().terminals.iter().any(|t| t.eq_ignore_ascii_case(exe)) {
            Target::Terminal
        } else {
            Target::Other
        }
    };
    TARGET.set((fg.0 as isize, generation, t));
    t
}

fn slow_mouse(on: bool) {
    if on {
        if SAVED_SPEED.get().is_none() {
            SAVED_SPEED.set(Some(send::mouse_speed()));
            send::set_mouse_speed(config().slow_mouse_speed);
        }
    } else if let Some(speed) = SAVED_SPEED.take() {
        send::set_mouse_speed(speed);
    }
}

fn begin_alpha() {
    end_alpha(false);
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_invalid() {
        return;
    }
    let Some(tx) = jobs::alpha_session(fg) else { return };
    let module = unsafe { GetModuleHandleW(None) }.ok().map(Into::into);
    let Ok(hook) = (unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0) }) else { return };
    ALPHA.with(|a| *a.borrow_mut() = Some(Alpha { hook, tx }));
}

fn end_alpha(tap: bool) {
    let Some(alpha) = ALPHA.with(|a| a.borrow_mut().take()) else { return };
    unsafe {
        let _ = UnhookWindowsHookEx(alpha.hook);
    }
    let _ = alpha.tx.send(AlphaMsg::End { tap });
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam.0 as u32 == WM_MOUSEWHEEL {
        let m = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let up = (m.mouseData >> 16) as i16 > 0;
        if ALPHA.with(|a| a.borrow().as_ref().is_some_and(|a| a.tx.send(AlphaMsg::Wheel(up)).is_ok())) {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
