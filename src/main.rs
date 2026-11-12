#![windows_subsystem = "windows"]

mod app;
mod capsule;
mod gdi;
mod render;
mod tray_menu;

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::RegisterSuspendResumeNotification;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use app::App;
use capsule::launcher::{self, AppCommand};
use gdi::paint_overlay;
use tray_menu::{run_command, show_menu};

const MAIN_CLASS: PCWSTR = w!("202020");
const OVERLAY_CLASS: PCWSTR = w!("202020Overlay");
const WM_TRAY: u32 = WM_APP + 1;
const WM_DISMISS: u32 = WM_APP + 2;
const WM_BAR_EXITED: u32 = WM_APP + 3;
const WM_BAR_DISABLE: u32 = WM_APP + 4;
const WM_BAR_RESTART: u32 = WM_APP + 5;
const WM_LAUNCHER_CMD: u32 = WM_APP + 6;
const WM_DESKTOP: u32 = WM_APP + 7;
const WM_CROP: u32 = WM_APP + 8;
const TRAY_ID: u32 = 1;
const HOTKEY_TOGGLE: i32 = 1;
const HOTKEY_LAUNCHER: i32 = 2;
const TIMER_TICK: usize = 1;
const TIMER_FRAME: usize = 2;
const TIMER_BREAK_END: usize = 3;
const CMD_TOGGLE: i32 = 1;
const CMD_BREAK: i32 = 2;
const CMD_RESET: i32 = 3;
const CMD_QUIT: i32 = 4;
const CMD_BAR: i32 = 5;
const CMD_EDIT_CONFIG: i32 = 6;
const CMD_RELOAD_CONFIG: i32 = 7;
const CMD_LAUNCHER: i32 = 8;
const CMD_HYPER: i32 = 9;

const LAUNCHER_COMMANDS: &[AppCommand] = &[
    AppCommand { id: CMD_TOGGLE as u32, name: "Pause or resume timer  暂停 继续计时" },
    AppCommand { id: CMD_BREAK as u32, name: "Break now  立即休息" },
    AppCommand { id: CMD_RESET as u32, name: "Reset timer  重置计时" },
    AppCommand { id: CMD_BAR as u32, name: "Toggle capsule bar  切换胶囊栏" },
    AppCommand { id: CMD_HYPER as u32, name: "Toggle CapsLock hyper  切换 CapsLock 超级键" },
    AppCommand { id: CMD_EDIT_CONFIG as u32, name: "Edit config  编辑配置" },
    AppCommand { id: CMD_RELOAD_CONFIG as u32, name: "Reload config  重新加载配置" },
];

static MAIN_HWND: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|app| app.try_borrow_mut().ok().and_then(|mut app| app.as_mut().map(f)))
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 && wparam.0 == WM_KEYDOWN as usize {
            let vk = (*(lparam.0 as *const KBDLLHOOKSTRUCT)).vkCode;
            if vk == u32::from(VK_ESCAPE.0) || vk == u32::from(VK_SPACE.0) {
                let _ = PostMessageW(Some(HWND(MAIN_HWND.load(Ordering::Relaxed))), WM_DISMISS, WPARAM(0), LPARAM(0));
                return LRESULT(1);
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            with_app(|app| match wparam.0 {
                TIMER_TICK => app.tick(),
                TIMER_FRAME => app.advance_overlay(),
                TIMER_BREAK_END => app.dismiss_break(),
                _ => {}
            });
        }
        WM_HOTKEY if wparam.0 as i32 == HOTKEY_LAUNCHER => {
            if with_app(|app| app.brk.is_none()) == Some(true) {
                launcher::toggle();
            }
        }
        WM_HOTKEY => {
            with_app(App::toggle_pause);
        }
        WM_LAUNCHER_CMD => run_command(wparam.0 as i32),
        WM_DESKTOP => capsule::desktops::on_request(wparam.0),
        WM_CROP => capsule::crop::on_toggle(),
        WM_QUERYENDSESSION => {
            capsule::desktops::restore_all();
            capsule::crop::restore_all();
            return LRESULT(1);
        }
        WM_ENDSESSION if wparam.0 != 0 => launcher::flush_history(),
        WM_DISMISS => {
            with_app(App::dismiss_break);
        }
        WM_BAR_EXITED => {
            with_app(App::on_bar_exited);
        }
        WM_BAR_DISABLE => {
            with_app(App::disable_bar);
        }
        WM_BAR_RESTART => {
            with_app(App::restart_bar);
        }
        WM_POWERBROADCAST if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize => {
            with_app(App::reset_timer);
        }
        WM_TRAY => {
            if matches!(lparam.0 as u32, WM_LBUTTONUP | WM_RBUTTONUP) {
                show_menu(hwnd);
            }
        }
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        _ if msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            if with_app(|app| app.notify(NIM_ADD)).is_none() {
                unsafe {
                    let _ = PostMessageW(Some(hwnd), msg, wparam, lparam);
                }
            }
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_PAINT {
            let painted = APP.with(|app| {
                let Ok(app) = app.try_borrow() else { return false };
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if let Some(brk) = app.as_ref().and_then(|app| app.brk.as_ref()) {
                    paint_overlay(hdc, &brk.overlay);
                }
                let _ = EndPaint(hwnd, &ps);
                true
            });
            if painted {
                return LRESULT(0);
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

fn register_classes() {
    capsule::register_class(WNDCLASSEXW {
        lpfnWndProc: Some(main_proc),
        lpszClassName: MAIN_CLASS,
        ..Default::default()
    });
    capsule::register_class(WNDCLASSEXW {
        lpfnWndProc: Some(overlay_proc),
        hbrBackground: HBRUSH(unsafe { GetStockObject(BLACK_BRUSH) }.0),
        lpszClassName: OVERLAY_CLASS,
        ..Default::default()
    });
}

fn create_main_window() -> Option<HWND> {
    let instance = unsafe { GetModuleHandleW(None) }.ok()?;
    unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            MAIN_CLASS,
            w!("202020"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .ok()
}

fn start_features(hwnd: HWND) {
    let cfg = capsule::config();
    if cfg.capsule_bar.enabled {
        with_app(App::start_bar);
    }
    launcher::init(launcher::Options {
        owner: hwnd.0 as isize,
        command_msg: WM_LAUNCHER_CMD,
        commands: LAUNCHER_COMMANDS,
    });
    with_app(|app| app.apply_launcher(cfg.launcher));
    capsule::crop::init(capsule::crop::Options { owner: hwnd.0 as isize, msg: WM_CROP });
    capsule::hyper::init(capsule::hyper::Options {
        owner: hwnd.0 as isize,
        command_msg: WM_LAUNCHER_CMD,
        reload_command: CMD_RELOAD_CONFIG as u32,
        desktop_msg: WM_DESKTOP,
    });
    capsule::hyper::apply(cfg.hyper);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    match args.first().map(String::as_str) {
        Some("--watchdog") => std::process::exit(capsule::run_watchdog(&args[1..])),
        Some("--restore") => {
            capsule::restore_taskbar();
            return;
        }
        _ => {}
    }
    capsule::init_logging("log.txt");
    unsafe {
        let _ = CreateMutexW(None, false, w!("Local\\202020"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
    }
    register_classes();
    let Some(hwnd) = create_main_window() else { return };
    MAIN_HWND.store(hwnd.0, Ordering::Relaxed);
    TASKBAR_CREATED.store(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }, Ordering::Relaxed);

    let mut app = App::new(hwnd, flag("--safe"));
    app.refresh_icon();
    app.notify(NIM_ADD);
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    start_features(hwnd);

    unsafe {
        let _ = RegisterHotKey(Some(hwnd), HOTKEY_TOGGLE, MOD_WIN | MOD_ALT | MOD_NOREPEAT, 'C' as u32);
        let _ = RegisterSuspendResumeNotification(HANDLE(hwnd.0), DEVICE_NOTIFY_WINDOW_HANDLE);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
