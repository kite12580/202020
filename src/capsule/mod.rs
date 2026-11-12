mod app;
mod clipboard;
mod config_store;
pub mod crop;
pub mod desktops;
mod gfx;
mod hook;
pub mod hyper;
mod input;
mod keyboard;
mod keyhook;
pub mod launcher;
mod logger;
mod menu;
mod pinned;
mod popup;
mod shell;
mod theme;
mod tooltip;
mod tray;
mod util;
mod volume;
mod watchdog;
mod windows_list;

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

pub use app::Options;
pub use config_store::{config, launcher as launcher_config, set_enabled, set_hyper_enabled, set_launcher_enabled};
pub use util::register_class;

/// The capsule bar running on its own thread.
pub struct Bar {
    hwnd: HWND,
    thread: JoinHandle<()>,
}

impl Bar {
    /// Starts the bar. Once it has started, `opts.exited_msg` is posted to
    /// `opts.owner` when the bar thread is about to finish, after which `join`
    /// returns promptly.
    pub fn start(opts: Options) -> Option<Bar> {
        app::STOP.store(false, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut started = false;
            app::run(opts, |h| {
                started = h.is_some();
                let _ = tx.send(h.map(|h| h.0 as isize));
            });
            if started {
                opts.post_owner(opts.exited_msg);
            }
        });
        match rx.recv().ok().flatten() {
            Some(h) => Some(Bar { hwnd: util::hwnd(h), thread }),
            None => {
                let _ = thread.join();
                None
            }
        }
    }

    pub fn stop(&self) {
        app::STOP.store(true, Ordering::Relaxed);
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), app::WM_APP_STOP, WPARAM(0), LPARAM(0));
        }
    }

    pub fn reload_config(&self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), app::WM_APP_RELOAD_CONFIG, WPARAM(0), LPARAM(0));
        }
    }

    pub fn join(self) {
        let _ = self.thread.join();
    }
}

/// Hides the bar while a break overlay is up.
pub fn set_break(on: bool, bar: Option<&Bar>) {
    app::BREAK.store(on, Ordering::Relaxed);
    if let Some(bar) = bar {
        unsafe {
            let _ = PostMessageW(Some(bar.hwnd), app::WM_APP_BREAK, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn edit_config() {
    config_store::open_in_editor(&config_store::ensure_file());
}

pub fn init_logging(name: &str) {
    let level = config_store::log_level();
    if level != log::LevelFilter::Off {
        logger::init(&config_store::log_dir(), name, level);
    }
}

pub fn run_watchdog(args: &[String]) -> i32 {
    watchdog::run(args)
}

pub fn restore_taskbar() {
    shell::force_restore();
}
