use capsule_core::taskband::Pin;
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, PROPERTYKEY};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET, RegCloseKey, RegNotifyChangeKeyValue,
    RegOpenKeyExW,
};
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    IEnumFullIDList, IShellItem2, SHCreateItemFromIDList, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
};
use windows::core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, interface, w};

use crate::capsule::gfx::Image;
use crate::capsule::util::{OwnedHandle, PKEY_APP_USER_MODEL_ID, apps_folder, take_string};

const CLSID_TASKBAND_PIN: GUID = GUID::from_u128(0x90aa3a4e_1cba_4233_b8bb_535773d48449);
const PKEY_LINK_TARGET_PARSING_PATH: PROPERTYKEY =
    PROPERTYKEY { fmtid: GUID::from_u128(0xb9b4b3fc_2b51_4a42_b5d8_324146afcf25), pid: 2 };

/// Undocumented `IPinnedList3`; only its first method is declared.
#[interface("0dd79ae2-d156-45d4-9eeb-3b549769e940")]
unsafe trait IPinnedList3: IUnknown {
    unsafe fn enum_objects(&self, list: *mut Option<IEnumFullIDList>) -> HRESULT;
}

/// A program pinned to the native taskbar.
pub struct PinnedApp {
    pub pin: Pin,
    pub parsing_name: String,
    pub name: String,
    pub image: Option<Image>,
}

fn string(item: &IShellItem2, key: &PROPERTYKEY) -> Option<String> {
    take_string(unsafe { item.GetString(key) }.ok()?)
}

/// The native taskbar's pinned apps, in order.
pub fn read() -> Vec<PinnedApp> {
    let mut pins = Vec::new();
    let Ok(list) = (unsafe { CoCreateInstance::<_, IPinnedList3>(&CLSID_TASKBAND_PIN, None, CLSCTX_INPROC_SERVER) })
    else {
        log::warn!("taskbar pin list unavailable");
        return pins;
    };
    let mut items = None;
    if unsafe { list.enum_objects(&mut items) }.is_err() {
        return pins;
    }
    let Some(items) = items else { return pins };
    loop {
        let mut pidl = [std::ptr::null_mut::<ITEMIDLIST>()];
        if unsafe { items.Next(&mut pidl, None) } != HRESULT(0) {
            break;
        }
        if let Ok(item) = unsafe { SHCreateItemFromIDList::<IShellItem2>(pidl[0]) } {
            let aumid = string(&item, &PKEY_APP_USER_MODEL_ID);
            let target = string(&item, &PKEY_LINK_TARGET_PARSING_PATH);
            let display = |sigdn| unsafe { item.GetDisplayName(sigdn) }.ok().and_then(take_string).unwrap_or_default();
            pins.push(PinnedApp {
                pin: Pin::new(aumid.as_deref(), target.as_deref()),
                // A pinned packaged app's own parsing name does not parse back.
                parsing_name: match (&aumid, &target) {
                    (Some(id), None) => apps_folder(id),
                    _ => display(SIGDN_DESKTOPABSOLUTEPARSING),
                },
                name: display(SIGDN_NORMALDISPLAY),
                image: None,
            });
        }
        unsafe { CoTaskMemFree(Some(pidl[0] as *const _)) };
    }
    log::debug!("taskbar pins {:?}", pins.iter().map(|p| (&p.pin, &p.parsing_name)).collect::<Vec<_>>());
    pins
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
