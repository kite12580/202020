use capsule_core::config::{self, CapsuleBar, Config, Launcher};
use std::path::PathBuf;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FindCloseChangeNotification,
    FindFirstChangeNotificationW, FindNextChangeNotification,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, w};

use crate::capsule::util::wide;

pub fn app_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)).unwrap_or_default()
}

pub fn log_dir() -> PathBuf {
    app_dir().join("logs")
}

fn config_path() -> PathBuf {
    app_dir().join("config.toml")
}

/// Path of `config.toml`, written with the defaults if missing.
pub fn ensure_file() -> PathBuf {
    let path = config_path();
    if !path.exists() {
        let _ = std::fs::write(&path, config::DEFAULT_TEXT);
    }
    path
}

fn read() -> (PathBuf, String, Config) {
    let path = ensure_file();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let config = config::parse(&text).unwrap_or_else(|e| {
        log::error!("config: {e}");
        Config::default()
    });
    (path, text, config)
}

pub fn config() -> Config {
    read().2
}

pub fn log_level() -> log::LevelFilter {
    read().2.logging.level.into()
}

fn edit_file(edit: impl FnOnce(&str) -> String) {
    let (path, text, _) = read();
    if let Err(e) = std::fs::write(&path, edit(&text)) {
        log::error!("config write: {e}");
    }
}

pub fn set_enabled(enabled: bool) {
    edit_file(|t| config::set_enabled(t, enabled));
}

pub fn launcher() -> Launcher {
    read().2.launcher
}

pub fn set_launcher_enabled(enabled: bool) {
    edit_file(|t| config::set_launcher_enabled(t, enabled));
}

pub fn set_hyper_enabled(enabled: bool) {
    edit_file(|t| config::set_hyper_enabled(t, enabled));
}

pub fn modified() -> Option<std::time::SystemTime> {
    std::fs::metadata(config_path()).and_then(|m| m.modified()).ok()
}

pub fn open_in_editor(path: &std::path::Path) {
    let path = wide(&path.to_string_lossy());
    let r = unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(path.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
    };
    if r.0 as isize <= 32 {
        unsafe {
            ShellExecuteW(None, w!("open"), w!("notepad.exe"), PCWSTR(path.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL);
        }
    }
}

pub struct ConfigStore {
    pub path: PathBuf,
    pub config: CapsuleBar,
    last_text: String,
    watch: HANDLE,
}

impl ConfigStore {
    pub fn open() -> Self {
        let (path, text, config) = read();
        let dir = app_dir();
        let watch = unsafe {
            FindFirstChangeNotificationW(
                PCWSTR(wide(&dir.to_string_lossy()).as_ptr()),
                false,
                FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME,
            )
        }
        .unwrap_or_default();
        Self { path, config: config.capsule_bar, last_text: text, watch }
    }

    pub fn watch_handle(&self) -> HANDLE {
        self.watch
    }

    pub fn rearm_watch(&self) {
        unsafe {
            let _ = FindNextChangeNotification(self.watch);
        }
    }

    /// Re-reads the file. Returns whether the effective config changed.
    pub fn reload(&mut self) -> bool {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return false;
        };
        if text == self.last_text {
            return false;
        }
        self.last_text = text;
        match config::parse(&self.last_text) {
            Ok(c) if c.capsule_bar != self.config => {
                self.config = c.capsule_bar;
                true
            }
            Ok(_) => false,
            Err(e) => {
                log::error!("config: {e}");
                false
            }
        }
    }

    fn write(&mut self, text: String) {
        if text == self.last_text {
            return;
        }
        if let Err(e) = std::fs::write(&self.path, &text) {
            log::error!("config write: {e}");
        }
        self.last_text = text;
    }

    pub fn save_position(&mut self, center_x: f32, bottom_gap: f32) {
        self.config.position.center_x = center_x;
        self.config.position.bottom_gap = bottom_gap;
        let text = config::set_position(&self.last_text, center_x, bottom_gap);
        self.write(text);
    }

    pub fn save_pinned(&mut self) {
        let text = config::set_pinned(&self.last_text, &self.config.tray.pinned);
        self.write(text);
    }
}

impl Drop for ConfigStore {
    fn drop(&mut self) {
        if !self.watch.is_invalid() {
            unsafe {
                let _ = FindCloseChangeNotification(self.watch);
            }
        }
    }
}
