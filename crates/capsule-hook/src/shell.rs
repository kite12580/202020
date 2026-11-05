//! Shell work for the bar and the launcher, on a worker thread of our own so
//! explorer's tray thread never waits for it. Explorer already has the
//! shell's COM servers and caches loaded, which 202020 would otherwise load
//! for itself.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use capsule_core::shell_proto::{Answer, AppInfo, Bitmap, PinInfo, Request};
use windows::Win32::Foundation::{CloseHandle, FreeLibrary, HMODULE, HWND, PROPERTYKEY, SIZE};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GetDIBits, GetObjectW, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::LibraryLoader::{
    FreeLibraryAndExitThread, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleHandleExW,
};
use windows::Win32::System::Threading::{CreateThread, THREAD_CREATION_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumFullIDList, IEnumShellItems, IShellItem, IShellItem2,
    IShellItemImageFactory, KF_FLAG_DEFAULT, SEE_MASK_NOASYNC, SEE_MASK_UNICODE, SHCreateItemFromIDList,
    SHCreateItemFromParsingName, SHELLEXECUTEINFOW, SHGetKnownFolderItem, SIGDN, SIGDN_DESKTOPABSOLUTEPARSING,
    SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows_core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR, PWSTR, interface};

const IDLE: Duration = Duration::from_secs(10);

const CLSID_TASKBAND_PIN: GUID = GUID::from_u128(0x90aa3a4e_1cba_4233_b8bb_535773d48449);
const PKEY_APP_USER_MODEL_ID: PROPERTYKEY =
    PROPERTYKEY { fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };
const PKEY_LINK_TARGET_PARSING_PATH: PROPERTYKEY =
    PROPERTYKEY { fmtid: GUID::from_u128(0xb9b4b3fc_2b51_4a42_b5d8_324146afcf25), pid: 2 };

/// Undocumented `IPinnedList3`; only its first method is declared.
#[interface("0dd79ae2-d156-45d4-9eeb-3b549769e940")]
unsafe trait IPinnedList3: IUnknown {
    unsafe fn enum_objects(&self, list: *mut Option<IEnumFullIDList>) -> HRESULT;
}

struct Queue {
    /// With the window that made each.
    requests: VecDeque<(HWND, Request)>,
    running: bool,
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue { requests: VecDeque::new(), running: false });
static WAKE: Condvar = Condvar::new();

fn queue() -> MutexGuard<'static, Queue> {
    QUEUE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Queues `req`, starting the worker if it is not running. The worker holds
/// its own reference to this DLL and exits after `IDLE` without requests.
pub fn submit(from: HWND, req: Request) -> bool {
    let mut q = queue();
    q.requests.push_back((from, req));
    if q.running {
        WAKE.notify_one();
        return true;
    }
    let mut module = HMODULE::default();
    let addr = PCWSTR(worker as unsafe extern "system" fn(*mut c_void) -> u32 as usize as *const u16);
    if unsafe { GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, addr, &mut module) }.is_err() {
        q.requests.clear();
        return false;
    }
    let param = Some(module.0 as *const c_void);
    match unsafe { CreateThread(None, 0, Some(worker), param, THREAD_CREATION_FLAGS(0), None) } {
        Ok(t) => unsafe {
            let _ = CloseHandle(t);
            q.running = true;
            true
        },
        Err(_) => {
            let _ = unsafe { FreeLibrary(module) };
            q.requests.clear();
            false
        }
    }
}

pub fn clear() {
    queue().requests.clear();
}

// SAFETY: window handles are plain values usable from any thread.
unsafe impl Send for Queue {}

fn next() -> Option<(HWND, Request)> {
    let q = queue();
    let (mut q, _) =
        WAKE.wait_timeout_while(q, IDLE, |q| q.requests.is_empty()).unwrap_or_else(PoisonError::into_inner);
    let req = q.requests.pop_front();
    q.running = req.is_some();
    req
}

unsafe extern "system" fn worker(module: *mut c_void) -> u32 {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    while let Some((from, req)) = next() {
        let answer = match req {
            Request::Pins => Answer::Pins(pins()),
            Request::Apps => Answer::Apps(apps()),
            Request::Icon { id, px, name } => Answer::Icon { id, bitmap: icon(&name, px) },
            Request::Open { file, params, dir } => {
                open(&file, &params, &dir);
                continue;
            }
        };
        crate::answer(from, &answer.encode());
    }
    unsafe {
        CoUninitialize();
        FreeLibraryAndExitThread(HMODULE(module), 0)
    }
}

fn take_string(s: PWSTR) -> Option<String> {
    let out = unsafe { s.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(s.0 as *const _)) };
    out
}

fn pins() -> Vec<PinInfo> {
    let mut pins = Vec::new();
    let Ok(list) = (unsafe { CoCreateInstance::<_, IPinnedList3>(&CLSID_TASKBAND_PIN, None, CLSCTX_INPROC_SERVER) })
    else {
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
            let string = |key: &PROPERTYKEY| unsafe { item.GetString(key) }.ok().and_then(take_string);
            let display = |sigdn: SIGDN| unsafe { item.GetDisplayName(sigdn) }.ok().and_then(take_string);
            let aumid = string(&PKEY_APP_USER_MODEL_ID);
            let target = string(&PKEY_LINK_TARGET_PARSING_PATH);
            // A pinned packaged app's own parsing name does not parse back.
            let parsing_name = match (&aumid, &target) {
                (Some(id), None) => Some(format!(r"shell:AppsFolder\{id}")),
                _ => display(SIGDN_DESKTOPABSOLUTEPARSING),
            };
            pins.push(PinInfo {
                aumid,
                target,
                parsing_name: parsing_name.unwrap_or_default(),
                name: display(SIGDN_NORMALDISPLAY).unwrap_or_default(),
            });
        }
        unsafe { CoTaskMemFree(Some(pidl[0] as *const _)) };
    }
    pins
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn apps() -> Vec<AppInfo> {
    let mut out = Vec::new();
    let Ok(folder) = (unsafe { SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) })
    else {
        return out;
    };
    let Ok(items) = (unsafe { folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) }) else {
        return out;
    };
    loop {
        let mut batch: [Option<IShellItem>; 64] = std::array::from_fn(|_| None);
        let mut fetched = 0;
        if unsafe { items.Next(&mut batch, Some(&mut fetched)) }.is_err() || fetched == 0 {
            break;
        }
        for item in batch.iter().take(fetched as usize).flatten() {
            let display = |sigdn: SIGDN| unsafe { item.GetDisplayName(sigdn) }.ok().and_then(take_string);
            if let (Some(name), Some(id)) = (display(SIGDN_NORMALDISPLAY), display(SIGDN_PARENTRELATIVEPARSING)) {
                out.push(AppInfo { name, id });
            }
        }
    }
    out
}

fn open(file: &str, params: &str, dir: &str) {
    let (file, params, dir) = (wide(file), wide(params), wide(dir));
    let ptr = |s: &[u16]| if s.len() > 1 { PCWSTR(s.as_ptr()) } else { PCWSTR::null() };
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_UNICODE | SEE_MASK_NOASYNC,
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: ptr(&params),
        lpDirectory: ptr(&dir),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    let _ = unsafe { ShellExecuteExW(&mut info) };
}

fn icon(name: &str, px: u32) -> Option<Bitmap> {
    let name = wide(name);
    let px = px as i32;
    let bitmap = unsafe {
        SHCreateItemFromParsingName::<_, _, IShellItemImageFactory>(PCWSTR(name.as_ptr()), None)
            .and_then(|f| f.GetImage(SIZE { cx: px, cy: px }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK))
    }
    .ok()?;
    let out = read_bitmap(bitmap);
    unsafe {
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
    }
    out
}

fn read_bitmap(bitmap: HBITMAP) -> Option<Bitmap> {
    let mut bm = BITMAP::default();
    let size = size_of::<BITMAP>() as i32;
    if unsafe { GetObjectW(HGDIOBJ(bitmap.0), size, Some(&mut bm as *mut BITMAP as *mut c_void)) } == 0
        || bm.bmWidth <= 0
        || bm.bmHeight <= 0
    {
        return None;
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight);
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    let rows = unsafe {
        let dc = CreateCompatibleDC(None);
        let rows =
            GetDIBits(dc, bitmap, 0, h as u32, Some(pixels.as_mut_ptr() as *mut c_void), &mut info, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        rows
    };
    (rows == h).then_some(Bitmap { width: w as u32, height: h as u32, bpp: bm.bmBitsPixel, pixels })
}
