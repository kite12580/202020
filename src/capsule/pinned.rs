use capsule_core::shell_proto::PinInfo;
use capsule_core::taskband::Pin;
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET, RegCloseKey, RegNotifyChangeKeyValue,
    RegOpenKeyExW,
};
use windows::Win32::System::Threading::CreateEventW;
use windows::core::w;

use crate::capsule::gfx::Image;
use crate::capsule::util::OwnedHandle;

/// A program pinned to the native taskbar.
pub struct PinnedApp {
    pub pin: Pin,
    pub parsing_name: String,
    pub name: String,
    pub image: Option<Image>,
}

impl From<PinInfo> for PinnedApp {
    fn from(p: PinInfo) -> Self {
        PinnedApp {
            pin: Pin::new(p.aumid.as_deref(), p.target.as_deref()),
            parsing_name: p.parsing_name,
            name: p.name,
            image: None,
        }
    }
}

/// Signals when explorer rewrites the pin list.
pub struct Watch {
    key: HKEY,
    event: OwnedHandle,
}

impl Watch {
    pub fn new() -> Option<Watch> {
        let mut key = HKEY::default();
        let path = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Taskband");
        if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path, Some(0), KEY_NOTIFY, &mut key) } != ERROR_SUCCESS {
            return None;
        }
        let event = OwnedHandle(unsafe { CreateEventW(None, false, false, None) }.ok()?);
        let w = Watch { key, event };
        w.rearm();
        Some(w)
    }

    pub fn handle(&self) -> HANDLE {
        self.event.0
    }

    pub fn rearm(&self) {
        unsafe {
            let _ = RegNotifyChangeKeyValue(self.key, false, REG_NOTIFY_CHANGE_LAST_SET, Some(self.event.0), true);
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.key);
        }
    }
}
