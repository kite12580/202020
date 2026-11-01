use capsule_core::config::{self, Config};
use std::path::PathBuf;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FindCloseChangeNotification,
    FindFirstChangeNotificationW, FindNextChangeNotification,
};
use windows::core::PCWSTR;

use crate::util::wide;

pub fn app_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)).unwrap_or_default()
}

pub fn log_dir() -> PathBuf {
    app_dir().join("logs")
}

pub struct ConfigStore {
    pub path: PathBuf,
    pub config: Config,
    last_text: String,
    watch: HANDLE,
}

impl ConfigStore {
    pub fn open() -> Self {
        let dir = app_dir();
        let path = dir.join("config.toml");
        if !path.exists() {
            let _ = std::fs::write(&path, config::DEFAULT_TEXT);
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let config = config::parse(&text).unwrap_or_else(|e| {
            log::error!("config: {e}");
            Config::default()
        });
        let watch = unsafe {
            FindFirstChangeNotificationW(
                PCWSTR(wide(&dir.to_string_lossy()).as_ptr()),
                false,
                FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME,
            )
        }
        .unwrap_or_default();
        Self { path, config, last_text: text, watch }
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
            Ok(c) if c != self.config => {
                self.config = c;
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
