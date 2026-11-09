//! CapsLock hyper layer. Keys are decided on the shared keyboard hook's thread
//! (`keyhook`); anything that waits on the clipboard or another window runs on
//! a short-lived job thread.

mod jobs;

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};

use capsule_core::config::Hyper;
use capsule_core::hyper::{Effect, Event, Layer, Target};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, HC_ACTION, HHOOK, MSLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_MOUSE_LL, WM_MOUSEWHEEL,
};

use crate::capsule::util::{class_name, foreground, hwnd, process_path, window_ids};
use crate::capsule::{input, keyhook};
use jobs::AlphaMsg;

pub struct Options {
    pub owner: isize,
    pub command_msg: u32,
    pub reload_command: u32,
}

static OPTIONS: OnceLock<Options> = OnceLock::new();
static CONFIG: Mutex<Option<Arc<Hyper>>> = Mutex::new(None);
static GENERATION: AtomicU32 = AtomicU32::new(0);

struct Alpha {
    hook: HHOOK,
    tx: Sender<AlphaMsg>,
}

thread_local! {
    static LAYER: RefCell<Layer> = const { RefCell::new(Layer::new()) };
    static TARGET: Cell<(isize, u32, Target)> = const { Cell::new((0, u32::MAX, Target::Other)) };
    static SAVED_SPEED: Cell<Option<u32>> = const { Cell::new(None) };
    static ALPHA: RefCell<Option<Alpha>> = const { RefCell::new(None) };
    static KEYMAPS: RefCell<Option<(u32, Arc<Hyper>)>> = const { RefCell::new(None) };
}

pub fn init(opts: Options) {
    let _ = OPTIONS.set(opts);
}

pub fn apply(cfg: Hyper) {
    let on = cfg.enabled;
    *CONFIG.lock().unwrap_or_else(|e| e.into_inner()) = on.then(|| Arc::new(cfg));
    GENERATION.fetch_add(1, Relaxed);
    if on {
        input::caps_lock_off();
    }
    keyhook::set_hyper(on);
}

pub fn on() -> bool {
    keyhook::hyper_on()
}

pub fn stop() {
    keyhook::set_hyper(false);
}

fn config() -> Arc<Hyper> {
    CONFIG.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// Returns whether to swallow the key.
pub(super) fn on_key(vk_code: u32, down: bool, time: u32) -> bool {
    LAYER.with(|layer| {
        let Ok(mut layer) = layer.try_borrow_mut() else { return false };
        KEYMAPS.with(|cached| {
            let mut cached = cached.borrow_mut();
            let generation = GENERATION.load(Relaxed);
            if cached.as_ref().is_none_or(|(g, _)| *g != generation) {
                *cached = Some((generation, config()));
            }
            let Some((_, cfg)) = cached.as_ref() else { return false };
            let ev = Event { vk: vk_code as u16, down, time };
            layer.on_key(ev, &cfg.keys, &cfg.alt_keys, input::held_mods, target, &mut |e| run(e, cfg))
        })
    })
}

pub(super) fn reset() {
    LAYER.with(|l| *l.borrow_mut() = Layer::new());
    KEYMAPS.with(|k| k.borrow_mut().take());
    slow_mouse(false);
    end_alpha(false);
}

fn run(effect: Effect, cfg: &Arc<Hyper>) {
    match effect {
        Effect::Keys(keys) => input::keys(keys),
        Effect::Wheel(n) => input::wheel(n),
        Effect::Job(job) => jobs::spawn(job.clone(), cfg.clone()),
        Effect::Reload => {
            if let Some(o) = OPTIONS.get() {
                unsafe {
                    let _ =
                        PostMessageW(Some(hwnd(o.owner)), o.command_msg, WPARAM(o.reload_command as usize), LPARAM(0));
                }
            }
        }
        Effect::Button(b, down) => input::button(b, down),
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
    let t = classify(fg);
    TARGET.set((fg.0 as isize, generation, t));
    t
}

fn classify(h: HWND) -> Target {
    if class_name(h) == "Vim" {
        return Target::Vim;
    }
    let path = process_path(window_ids(h).1).unwrap_or_default();
    let exe = path.rsplit('\\').next().unwrap_or_default();
    if exe.eq_ignore_ascii_case("nvim-qt.exe") {
        return Target::Vim;
    }
    if config().terminals.iter().any(|t| t.eq_ignore_ascii_case(exe)) {
        return Target::Terminal;
    }
    Target::Other
}

fn slow_mouse(on: bool) {
    if on {
        if SAVED_SPEED.get().is_none() {
            SAVED_SPEED.set(Some(input::mouse_speed()));
            input::set_mouse_speed(config().slow_mouse_speed);
        }
    } else if let Some(speed) = SAVED_SPEED.take() {
        input::set_mouse_speed(speed);
    }
}

fn begin_alpha() {
    end_alpha(false);
    let Some(fg) = foreground() else { return };
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
