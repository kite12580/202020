use std::cell::RefCell;

use capsule_core::shell_proto::Request;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::Shell::{
    BHID_SFUIObject, CMF_EXTENDEDVERBS, CMF_NORMAL, CMIC_MASK_CONTROL_DOWN, CMIC_MASK_PTINVOKE, CMIC_MASK_SHIFT_DOWN,
    CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, IContextMenu, IContextMenu2, IContextMenu3, IShellItem,
    SEE_MASK_UNICODE, SHCreateItemFromParsingName, SHEmptyRecycleBinW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, CreatePopupMenu, DestroyMenu, PostMessageW, SW_SHOWNORMAL, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR, WM_NULL,
};
use windows::core::{Interface, PCSTR, PCWSTR};

use crate::capsule::hook;
use crate::capsule::util::{env, hwnd, wide};

const FIRST_ID: u32 = 1;

/// Has explorer open a path, URI or shell parsing name like the Run dialog
/// does. False when the hook in explorer did not take it.
pub fn open(from: HWND, target: &str, args: &str) -> bool {
    let file = env(target);
    let dir = fs_parent(&file).unwrap_or_default().to_string();
    let req = Request::Open { params: env(args), dir, file };
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }
    let taken = hook::request(from, &req);
    if !taken {
        log::warn!("launch {target}: hook not attached");
    }
    taken
}

fn fs_parent(path: &str) -> Option<&str> {
    let b = path.as_bytes();
    let absolute = (b.len() > 2 && b[1] == b':' && b[2] == b'\\') || path.starts_with(r"\\");
    absolute.then(|| path.rsplit_once('\\').map(|(d, _)| d)).flatten().filter(|d| d.len() > 2)
}

pub fn empty_recycle_bin(owner: HWND) {
    unsafe {
        let _ = SHEmptyRecycleBinW(Some(owner), PCWSTR::null(), 0);
    }
}

pub fn post(owner: isize, msg: u32, id: u32) {
    unsafe {
        let _ = PostMessageW(Some(hwnd(owner)), msg, WPARAM(id as usize), LPARAM(0));
    }
}

enum MenuHandler {
    V2(IContextMenu2),
    V3(IContextMenu3),
}

thread_local! {
    static ACTIVE: RefCell<Option<MenuHandler>> = const { RefCell::new(None) };
}

/// Lets an open shell context menu draw owner-drawn items and fill submenus.
pub fn forward_menu_message(msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if !matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        return None;
    }
    ACTIVE.with(|a| {
        let a = a.try_borrow().ok()?;
        let mut result = LRESULT(0);
        let ok = match a.as_ref()? {
            MenuHandler::V3(m) => unsafe { m.HandleMenuMsg2(msg, wparam, lparam, Some(&mut result)) }.is_ok(),
            MenuHandler::V2(m) => unsafe { m.HandleMenuMsg(msg, wparam, lparam) }.is_ok(),
        };
        ok.then_some(result)
    })
}

/// Shows the Explorer context menu of `target` at screen point `(x, y)` and
/// runs the chosen command. Returns whether one was chosen.
pub fn context_menu(owner: HWND, target: &str, x: i32, y: i32) -> bool {
    let target = wide(&env(target));
    unsafe {
        let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(target.as_ptr()), None) else {
            return false;
        };
        let Ok(menu_handler) = item.BindToHandler::<_, IContextMenu>(None, &BHID_SFUIObject) else { return false };
        let Ok(menu) = CreatePopupMenu() else { return false };
        let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
        let flags = if shift { CMF_NORMAL | CMF_EXTENDEDVERBS } else { CMF_NORMAL };
        if menu_handler.QueryContextMenu(menu, 0, FIRST_ID, 0x7FFF, flags).is_err() {
            let _ = DestroyMenu(menu);
            return false;
        }
        let handler = menu_handler
            .cast::<IContextMenu3>()
            .map(MenuHandler::V3)
            .or_else(|_| menu_handler.cast::<IContextMenu2>().map(MenuHandler::V2))
            .ok();
        ACTIVE.with(|a| *a.borrow_mut() = handler);
        let id = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, x, y, owner, None).0 as u32;
        ACTIVE.with(|a| a.borrow_mut().take());
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let chosen = id >= FIRST_ID;
        if chosen {
            let mut mask = SEE_MASK_UNICODE | CMIC_MASK_PTINVOKE;
            if shift {
                mask |= CMIC_MASK_SHIFT_DOWN;
            }
            if GetKeyState(VK_CONTROL.0 as i32) < 0 {
                mask |= CMIC_MASK_CONTROL_DOWN;
            }
            let verb = (id - FIRST_ID) as usize;
            let info = CMINVOKECOMMANDINFOEX {
                cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                fMask: mask,
                hwnd: owner,
                lpVerb: PCSTR(verb as *const u8),
                lpVerbW: PCWSTR(verb as *const u16),
                nShow: SW_SHOWNORMAL.0,
                ptInvoke: POINT { x, y },
                ..Default::default()
            };
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            if let Err(e) = menu_handler.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO) {
                log::debug!("context menu command {verb}: {e}");
            }
        }
        let _ = DestroyMenu(menu);
        chosen
    }
}
