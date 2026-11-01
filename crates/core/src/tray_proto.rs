//! Wire format of the WM_COPYDATA messages `Shell_NotifyIcon` sends to
//! `Shell_TrayWnd`. Handles are 32-bit on the wire even in 64-bit processes.

pub const SIGNATURE: u32 = 0x3475_3423;

pub const COPYDATA_APPBAR: usize = 0;
pub const COPYDATA_NOTIFYICON: usize = 1;
pub const COPYDATA_ICON_RECT: usize = 3;

pub const NIM_ADD: u32 = 0;
pub const NIM_MODIFY: u32 = 1;
pub const NIM_DELETE: u32 = 2;
pub const NIM_SETFOCUS: u32 = 3;
pub const NIM_SETVERSION: u32 = 4;

pub const NIF_MESSAGE: u32 = 0x01;
pub const NIF_ICON: u32 = 0x02;
pub const NIF_TIP: u32 = 0x04;
pub const NIF_STATE: u32 = 0x08;
pub const NIF_GUID: u32 = 0x20;
pub const NIF_SHOWTIP: u32 = 0x80;

pub const NIS_HIDDEN: u32 = 0x01;

/// Byte offsets within NOTIFYICONDATAW32.
mod off {
    pub const CB_SIZE: usize = 0;
    pub const HWND: usize = 4;
    pub const UID: usize = 8;
    pub const FLAGS: usize = 12;
    pub const CALLBACK: usize = 16;
    pub const ICON: usize = 20;
    pub const TIP: usize = 24;
    pub const STATE: usize = 280;
    pub const STATE_MASK: usize = 284;
    pub const VERSION: usize = 800;
    pub const GUID: usize = 936;
    pub const END: usize = 956;
}

const V1_TIP_CHARS: usize = 64;
const TIP_CHARS: usize = 128;
const V1_SIZE: usize = off::TIP + V1_TIP_CHARS * 2;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NotifyIconData {
    pub hwnd: u32,
    pub uid: u32,
    pub flags: u32,
    pub callback_message: u32,
    pub icon: u32,
    pub tip: String,
    pub state: u32,
    pub state_mask: u32,
    pub version: u32,
    pub guid: Option<[u8; 16]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyIconMessage {
    pub message: u32,
    pub data: NotifyIconData,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconRectQuery {
    /// 1 asks for the top-left corner, 2 for the bottom-right corner.
    pub message: u32,
    pub hwnd: u32,
    pub uid: u32,
    pub guid: Option<[u8; 16]>,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn guid_at(b: &[u8], at: usize) -> Option<[u8; 16]> {
    let g: [u8; 16] = b.get(at..at + 16)?.try_into().ok()?;
    (g != [0; 16]).then_some(g)
}

fn utf16_at(b: &[u8], at: usize, max_chars: usize) -> String {
    let end = (at + max_chars * 2).min(b.len());
    let units: Vec<u16> = b
        .get(at..end)
        .unwrap_or_default()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .take_while(|&c| c != 0)
        .collect();
    String::from_utf16_lossy(&units).replace('\r', "")
}

/// Parses a SHELLTRAYDATA payload (`dwData == 1`).
pub fn parse_notify_icon(bytes: &[u8]) -> Option<NotifyIconMessage> {
    if u32_at(bytes, 0)? != SIGNATURE {
        return None;
    }
    let message = u32_at(bytes, 4)?;
    let nid = bytes.get(8..)?;
    let cb_size = u32_at(nid, off::CB_SIZE)? as usize;
    let nid = &nid[..cb_size.min(nid.len()).min(off::END)];
    let has = |end: usize| nid.len() >= end;
    if !has(off::TIP) {
        return None;
    }
    let flags = u32_at(nid, off::FLAGS)?;
    let tip_chars = if nid.len() > V1_SIZE { TIP_CHARS } else { V1_TIP_CHARS };
    let data = NotifyIconData {
        hwnd: u32_at(nid, off::HWND)?,
        uid: u32_at(nid, off::UID)?,
        flags,
        callback_message: u32_at(nid, off::CALLBACK)?,
        icon: u32_at(nid, off::ICON)?,
        tip: if flags & NIF_TIP != 0 { utf16_at(nid, off::TIP, tip_chars) } else { String::new() },
        state: u32_at(nid, off::STATE).unwrap_or(0),
        state_mask: u32_at(nid, off::STATE_MASK).unwrap_or(0),
        version: u32_at(nid, off::VERSION).unwrap_or(0),
        guid: if flags & NIF_GUID != 0 && has(off::GUID + 16) { guid_at(nid, off::GUID) } else { None },
    };
    Some(NotifyIconMessage { message, data })
}

/// Parses a NOTIFYICONIDENTIFIER payload (`dwData == 3`).
pub fn parse_icon_rect_query(bytes: &[u8]) -> Option<IconRectQuery> {
    if u32_at(bytes, 0)? != SIGNATURE {
        return None;
    }
    Some(IconRectQuery {
        message: u32_at(bytes, 4)?,
        hwnd: u32_at(bytes, 16)?,
        uid: u32_at(bytes, 20)?,
        guid: guid_at(bytes, 24),
    })
}

/// Sign-extends a 32-bit wire handle to a native handle value.
pub fn handle(h: u32) -> isize {
    h as i32 as isize
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn shelltraydata(message: u32, cb_size: u32, f: impl FnOnce(&mut [u8])) -> Vec<u8> {
        let mut b = vec![0u8; 8 + off::END];
        b[0..4].copy_from_slice(&SIGNATURE.to_le_bytes());
        b[4..8].copy_from_slice(&message.to_le_bytes());
        b[8..12].copy_from_slice(&cb_size.to_le_bytes());
        f(&mut b[8..]);
        b
    }

    fn put(b: &mut [u8], at: usize, v: u32) {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn put_str(b: &mut [u8], at: usize, s: &str) {
        for (i, u) in s.encode_utf16().enumerate() {
            b[at + i * 2..at + i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
    }

    #[test]
    fn full_size() {
        let b = shelltraydata(NIM_ADD, off::END as u32, |n| {
            put(n, off::HWND, 0xFFFF_FFF0);
            put(n, off::UID, 7);
            put(n, off::FLAGS, NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_GUID);
            put(n, off::CALLBACK, 0x8001);
            put(n, off::ICON, 0x1234);
            put_str(n, off::TIP, "Hello\r\nWorld");
            put(n, off::STATE, NIS_HIDDEN);
            put(n, off::VERSION, 4);
            n[off::GUID] = 0xAB;
        });
        let m = parse_notify_icon(&b).unwrap();
        assert_eq!(m.message, NIM_ADD);
        assert_eq!(handle(m.data.hwnd), -16);
        assert_eq!(m.data.uid, 7);
        assert_eq!(m.data.icon, 0x1234);
        assert_eq!(m.data.tip, "Hello\nWorld");
        assert_eq!(m.data.state, NIS_HIDDEN);
        assert_eq!(m.data.version, 4);
        assert_eq!(m.data.guid.unwrap()[0], 0xAB);
    }

    #[test]
    fn v1_size_ignores_later_fields() {
        let b = shelltraydata(NIM_MODIFY, V1_SIZE as u32, |n| {
            put(n, off::FLAGS, NIF_TIP | NIF_GUID);
            put_str(n, off::TIP, &"x".repeat(100));
            put(n, off::STATE, 1);
            put(n, off::VERSION, 4);
            n[off::GUID] = 1;
        });
        let m = parse_notify_icon(&b).unwrap();
        assert_eq!(m.data.tip.len(), V1_TIP_CHARS);
        assert_eq!(m.data.state, 0);
        assert_eq!(m.data.version, 0);
        assert_eq!(m.data.guid, None);
    }

    #[test]
    fn truncated_payloads() {
        let b = shelltraydata(NIM_ADD, off::END as u32, |_| {});
        assert!(parse_notify_icon(&b[..20]).is_none());
        let m = parse_notify_icon(&b[..8 + 300]).unwrap();
        assert_eq!(m.data.version, 0);
        let mut bad = b.clone();
        bad[0] = 0;
        assert!(parse_notify_icon(&bad).is_none());
    }

    #[test]
    fn rect_query() {
        let mut b = vec![0u8; 40];
        b[0..4].copy_from_slice(&SIGNATURE.to_le_bytes());
        put(&mut b, 4, 2);
        put(&mut b, 16, 0x42);
        put(&mut b, 20, 9);
        let q = parse_icon_rect_query(&b).unwrap();
        assert_eq!((q.message, q.hwnd, q.uid, q.guid), (2, 0x42, 9, None));
    }
}
