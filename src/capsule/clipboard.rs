use std::sync::OnceLock;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::core::w;

const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
const CF_DIBV5: u32 = 17;
const MAX_SAVED: usize = 64 << 20;

/// Formats whose data isn't a plain HGLOBAL, or that Windows synthesizes
/// from another format.
fn skip_format(f: u32) -> bool {
    matches!(f, 1..=3 | 7 | 9 | 14 | 0x80..=0x8E | 0x200..=0x3FF)
}

fn open(owner: HWND) -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    false
}

pub fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

fn decode(bytes: &[u8]) -> String {
    let units = bytes.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).take_while(|&u| u != 0);
    char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
}

fn text_bytes(s: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(s.len() * 2 + 2);
    for u in s.encode_utf16().chain(Some(0)) {
        bytes.extend(u.to_le_bytes());
    }
    bytes
}

pub fn get(owner: HWND) -> Option<String> {
    if !open(owner) {
        return None;
    }
    let text = with_data(CF_UNICODETEXT, usize::MAX, decode);
    unsafe {
        let _ = CloseClipboard();
    }
    text
}

pub fn set(owner: HWND, s: &str) {
    if !s.is_empty() {
        write(owner, [(CF_UNICODETEXT, text_bytes(s).as_slice())], false);
    }
}

/// Puts `s` on the clipboard without it entering clipboard history.
pub fn set_private(owner: HWND, s: &str) {
    write(owner, [(CF_UNICODETEXT, text_bytes(s).as_slice())], true);
}

/// Must be called with the clipboard open.
fn with_data<T>(format: u32, max: usize, f: impl FnOnce(&[u8]) -> T) -> Option<T> {
    unsafe {
        let mem = HGLOBAL(GetClipboardData(format).ok()?.0);
        let size = GlobalSize(mem);
        if size > max {
            return None;
        }
        let p = GlobalLock(mem) as *const u8;
        if p.is_null() {
            return None;
        }
        let out = f(std::slice::from_raw_parts(p, size));
        let _ = GlobalUnlock(mem);
        Some(out)
    }
}

fn private_formats() -> &'static [u32; 4] {
    static IDS: OnceLock<[u32; 4]> = OnceLock::new();
    IDS.get_or_init(|| {
        [
            w!("ExcludeClipboardContentFromMonitorProcessing"),
            w!("Clipboard Viewer Ignore"),
            w!("CanIncludeInClipboardHistory"),
            w!("CanUploadToCloudClipboard"),
        ]
        .map(|name| unsafe { RegisterClipboardFormatW(name) })
    })
}

fn write<'a>(owner: HWND, formats: impl IntoIterator<Item = (u32, &'a [u8])>, private: bool) {
    if !open(owner) {
        return;
    }
    unsafe {
        let _ = EmptyClipboard();
        for (format, bytes) in formats {
            put(format, bytes);
        }
        if private {
            for &format in private_formats() {
                put(format, &0u32.to_le_bytes());
            }
        }
        let _ = CloseClipboard();
    }
}

fn put(format: u32, bytes: &[u8]) {
    unsafe {
        let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) else { return };
        let p = GlobalLock(mem) as *mut u8;
        if p.is_null() {
            let _ = GlobalFree(Some(mem));
            return;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(mem);
        if SetClipboardData(format, Some(HANDLE(mem.0))).is_err() {
            let _ = GlobalFree(Some(mem));
        }
    }
}

pub struct Saved {
    sequence: u32,
    formats: Vec<(u32, Vec<u8>)>,
}

impl Saved {
    pub fn text(&self) -> Option<String> {
        self.formats.iter().find(|(f, _)| *f == CF_UNICODETEXT).map(|(_, bytes)| decode(bytes))
    }
}

pub fn save(owner: HWND) -> Saved {
    let mut saved = Saved { sequence: sequence(), formats: Vec::new() };
    if !open(owner) {
        return saved;
    }
    let mut total = 0;
    let mut format = 0;
    let mut dib = false;
    loop {
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }
        if skip_format(format) {
            continue;
        }
        // CF_DIB and CF_DIBV5 are synthesized from each other; reading the
        // second would make Windows convert the image.
        if matches!(format, CF_DIB | CF_DIBV5) {
            if dib {
                continue;
            }
            dib = true;
        }
        if let Some(bytes) = with_data(format, MAX_SAVED - total, <[u8]>::to_vec) {
            total += bytes.len();
            saved.formats.push((format, bytes));
        }
    }
    unsafe {
        let _ = CloseClipboard();
    }
    saved
}

/// Puts `saved` back unless the clipboard changed since `expected`, or never
/// changed after the save.
pub fn restore(owner: HWND, saved: &Saved, expected: u32) {
    let now = sequence();
    if now == expected && now != saved.sequence {
        write(owner, saved.formats.iter().map(|(f, b)| (*f, b.as_slice())), true);
    }
}
