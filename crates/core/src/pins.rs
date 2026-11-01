/// Persistent identity of a tray icon: its GUID when it has one, otherwise
/// the owning executable's file name plus the icon's uID.
pub fn pin_key(exe_path: &str, uid: u32, guid: Option<&[u8; 16]>) -> String {
    if let Some(g) = guid {
        return format!("guid:{}", guid_string(g));
    }
    let name = exe_path.rsplit(['\\', '/']).next().unwrap_or(exe_path);
    format!("{}|{uid}", name.to_lowercase())
}

/// Formats GUID bytes in their in-memory (little-endian) layout.
pub fn guid_string(g: &[u8; 16]) -> String {
    let d1 = u32::from_le_bytes([g[0], g[1], g[2], g[3]]);
    let d2 = u16::from_le_bytes([g[4], g[5]]);
    let d3 = u16::from_le_bytes([g[6], g[7]]);
    format!(
        "{{{d1:08X}-{d2:04X}-{d3:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(pin_key(r"C:\Program Files\App\App.EXE", 7, None), "app.exe|7");
        let g = [0x78, 0x56, 0x34, 0x12, 0x34, 0x12, 0x78, 0x56, 1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(pin_key("ignored.exe", 1, Some(&g)), "guid:{12345678-1234-5678-0102-030405060708}");
    }
}
