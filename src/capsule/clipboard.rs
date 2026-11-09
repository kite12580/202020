use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::core::w;

use crate::capsule::util::wide;

const CF_UNICODETEXT: u32 = 13;
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

pub fn get(owner: HWND) -> Option<String> {
    if !open(owner) {
        return None;
    }
    let text = read(CF_UNICODETEXT).map(|bytes| {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
        let len = units.iter().position(|&c| c == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..len])
    });
    unsafe {
        let _ = CloseClipboard();
    }
    text
}

pub fn set(owner: HWND, s: &str) {
    if !s.is_empty() {
        write(owner, &[(CF_UNICODETEXT, text_bytes(s))], false);
    }
}

/// Puts `s` on the clipboard without it entering clipboard history.
pub fn set_private(owner: HWND, s: &str) {
    write(owner, &[(CF_UNICODETEXT, text_bytes(s))], true);
}

fn text_bytes(s: &str) -> Vec<u8> {
    wide(s).iter().flat_map(|u| u.to_le_bytes()).collect()
}

/// Must be called with the clipboard open.
fn read(format: u32) -> Option<Vec<u8>> {
    unsafe {
        let mem = HGLOBAL(GetClipboardData(format).ok()?.0);
        let size = GlobalSize(mem);
        let p = GlobalLock(mem) as *const u8;
        if p.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(p, size).to_vec();
        let _ = GlobalUnlock(mem);
        Some(bytes)
    }
}

fn write(owner: HWND, formats: &[(u32, Vec<u8>)], private: bool) {
    if !open(owner) {
        return;
    }
    unsafe {
        let _ = EmptyClipboard();
        for (format, bytes) in formats {
            put(*format, bytes);
        }
        if private {
            for name in [
                w!("ExcludeClipboardContentFromMonitorProcessing"),
                w!("Clipboard Viewer Ignore"),
                w!("CanIncludeInClipboardHistory"),
                w!("CanUploadToCloudClipboard"),
            ] {
                put(RegisterClipboardFormatW(name), &0u32.to_le_bytes());
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

/// A copy of the clipboard's memory-backed formats.
pub struct Saved {
    sequence: u32,
    formats: Vec<(u32, Vec<u8>)>,
}

pub fn save(owner: HWND) -> Saved {
    let mut saved = Saved { sequence: sequence(), formats: Vec::new() };
    if !open(owner) {
        return saved;
    }
    let mut total = 0;
    let mut format = 0;
    loop {
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }
        if skip_format(format) {
            continue;
        }
        if let Some(bytes) = read(format).filter(|b| total + b.len() <= MAX_SAVED) {
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
        write(owner, &saved.formats, true);
    }
}
