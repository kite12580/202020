//! CapsLock hyper layer. Keys are decided on the shared keyboard hook's thread
//! (`keyhook`); anything that waits on the clipboard or another window runs on
//! a short-lived job thread.

mod jobs;

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};

use capsule_core::config::Hyper;
use capsule_core::hyper::{Action, CTRL, Chord, Effect, Event, Job, Layer, Target, WIN, vk};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::RRF_RT_REG_BINARY;
use windows::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_CONTROL};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, HC_ACTION, HHOOK, MSLLHOOKSTRUCT, SetWindowsHookExW, UnhookWindowsHookEx,
    WH_MOUSE_LL, WM_MOUSEWHEEL,
};
use windows::core::w;

use crate::capsule::util::{self, foreground, process_path, reg_value, window_ids};
use crate::capsule::{crop, desktops, input, keyhook};
use jobs::AlphaMsg;

pub struct Options {
    pub owner: isize,
    pub command_msg: u32,
    pub reload_command: u32,
    pub desktop_msg: u32,
    pub crop_msg: u32,
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
    static WHEEL_HOOK: Cell<Option<HHOOK>> = const { Cell::new(None) };
    static KEYMAPS: RefCell<Option<(u32, Arc<Hyper>)>> = const { RefCell::new(None) };
}

pub fn init(opts: Options) {
    let _ = OPTIONS.set(opts);
}

pub fn apply(cfg: Hyper) {
    let on = cfg.enabled;
    if !(on && cfg.binds(&Action::VirgoDesktop)) {
        desktops::restore_all();
    }
    if !(on && cfg.binds(&Action::Crop)) {
        crop::restore_all();
    }
    {
        let mut current = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
        if on == keyhook::hyper_on() && current.as_deref() == on.then_some(&cfg) {
            return;
        }
        *current = on.then(|| Arc::new(cfg));
    }
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

/// Returns whether to swallow the key.
pub(super) fn on_key(vk_code: u32, down: bool, time: u32) -> bool {
    LAYER.with(|layer| {
        let Ok(mut layer) = layer.try_borrow_mut() else { return false };
        KEYMAPS.with(|cached| {
            let mut cached = cached.borrow_mut();
            let generation = GENERATION.load(Relaxed);
            if cached.as_ref().is_none_or(|(g, _)| *g != generation) {
                *cached = CONFIG.lock().unwrap_or_else(|e| e.into_inner()).clone().map(|cfg| (generation, cfg));
            }
            let Some((_, cfg)) = cached.as_ref() else { return false };
            let ev = Event { vk: vk_code as u16, down, time };
            if matches!(ev.vk, vk::LCONTROL | vk::RCONTROL) {
                block_wheel(cfg.block_ctrl_wheel && ctrl_held(ev));
            }
            layer.on_key(ev, cfg, input::held_mods, || target(cfg), &mut |e| run(e, cfg))
        })
    })
}

pub(super) fn reset() {
    LAYER.with(|l| *l.borrow_mut() = Layer::new());
    KEYMAPS.with(|k| k.borrow_mut().take());
    slow_mouse(None);
    end_alpha(false);
    block_wheel(false);
}

fn run(effect: Effect, cfg: &Arc<Hyper>) {
    match effect {
        Effect::Keys(keys) => input::keys(keys),
        Effect::WinKeys(keys) => match *keys {
            [Chord { mods: WIN, vk: digit @ vk::D0..=vk::D9 }] if keyhook::win_digit(digit) => {}
            _ => input::keys_holding_win(keys),
        },
        Effect::Wheel(n) => input::wheel(n),
        Effect::Job(Job::Hotstring) if cfg.hotstrings.is_empty() => {}
        Effect::Job(job) => jobs::spawn(job.clone(), cfg.clone()),
        Effect::Reload => post(|o| (o.command_msg, o.reload_command as usize)),
        Effect::SwitchDesktop => {
            let vk = if on_first_desktop() { vk::RIGHT } else { vk::LEFT };
            input::keys(&[Chord { mods: WIN | CTRL, vk }]);
        }
        Effect::MaskWin => input::mask_win(),
        Effect::VirgoToggle => post(|o| (o.desktop_msg, desktops::TOGGLE)),
        Effect::VirgoMove => post(|o| (o.desktop_msg, desktops::MOVE_ACTIVE)),
        Effect::Crop => post(|o| (o.crop_msg, crop::TOGGLE)),
        Effect::Button(b, down) => input::button(b, down),
        Effect::SlowMouse(on) => slow_mouse(on.then_some(cfg.slow_mouse_speed)),
        Effect::AlphaBegin => begin_alpha(),
        Effect::AlphaEnd { tap } => end_alpha(tap),
    }
}

fn post(msg: impl FnOnce(&Options) -> (u32, usize)) {
    if let Some(o) = OPTIONS.get() {
        let (msg, wparam) = msg(o);
        util::post(o.owner, msg, wparam);
    }
}

fn on_first_desktop() -> bool {
    let key = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops");
    let mut current = [0u8; 16];
    let mut ids = [0u8; 16 * 64];
    reg_value(key, w!("CurrentVirtualDesktop"), RRF_RT_REG_BINARY, &mut current) != Some(16)
        || reg_value(key, w!("VirtualDesktopIDs"), RRF_RT_REG_BINARY, &mut ids).is_none_or(|n| n < 16)
        || current == ids[..16]
}

fn target(cfg: &Hyper) -> Target {
    let fg = unsafe { GetForegroundWindow() };
    let generation = GENERATION.load(Relaxed);
    let (cached, cached_gen, t) = TARGET.get();
    if cached == fg.0 as isize && cached_gen == generation {
        return t;
    }
    let t = classify(fg, cfg);
    TARGET.set((fg.0 as isize, generation, t));
    t
}

fn classify(h: HWND, cfg: &Hyper) -> Target {
    let path = process_path(window_ids(h).1).unwrap_or_default();
    let exe = path.rsplit('\\').next().unwrap_or_default();
    if cfg.terminals.iter().any(|t| t.eq_ignore_ascii_case(exe)) {
        return Target::Terminal;
    }
    Target::Other
}

fn slow_mouse(speed: Option<u32>) {
    match speed {
        Some(speed) => {
            if SAVED_SPEED.get().is_none() {
                SAVED_SPEED.set(Some(input::mouse_speed()));
                input::set_mouse_speed(speed);
            }
        }
        None => {
            if let Some(saved) = SAVED_SPEED.take() {
                input::set_mouse_speed(saved);
            }
        }
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

fn ctrl_held(ev: Event) -> bool {
    let other = if ev.vk == vk::LCONTROL { vk::RCONTROL } else { vk::LCONTROL };
    ev.down || input::held(VIRTUAL_KEY(other))
}

fn block_wheel(on: bool) {
    match WHEEL_HOOK.get() {
        None if on => {
            let module = unsafe { GetModuleHandleW(None) }.ok().map(Into::into);
            WHEEL_HOOK.set(unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(wheel_proc), module, 0) }.ok());
        }
        Some(hook) if !on => unsafe {
            WHEEL_HOOK.set(None);
            let _ = UnhookWindowsHookEx(hook);
        },
        _ => {}
    }
}

unsafe extern "system" fn wheel_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        if !input::held(VK_CONTROL) {
            block_wheel(false);
        } else if wparam.0 as u32 == WM_MOUSEWHEEL {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
