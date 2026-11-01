/// An app pinned to the native taskbar, with lowercased identifiers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pin {
    pub aumid: Option<String>,
    pub target: Option<String>,
}

impl Pin {
    pub fn new(aumid: Option<&str>, target: Option<&str>) -> Pin {
        let norm = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(str::to_lowercase);
        Pin { aumid: norm(aumid), target: norm(target) }
    }
}

/// The pin a window groups under, mirroring explorer: a window with an
/// AppUserModelID only joins the pin with that ID, so a PWA does not land
/// on its browser's pin; a window without one joins by executable path.
pub fn slot(pins: &[Pin], aumid: Option<&str>, exe: Option<&str>) -> Option<usize> {
    if let Some(id) = aumid.filter(|s| !s.is_empty()) {
        let id = id.to_lowercase();
        return pins.iter().position(|p| p.aumid.as_deref() == Some(id.as_str()));
    }
    let exe = exe?.to_lowercase();
    pins.iter().position(|p| p.target.as_deref() == Some(exe.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching() {
        let pins = [
            Pin::new(Some("Microsoft.Windows.Explorer"), Some(r"C:\Windows\explorer.exe")),
            Pin::new(Some("Chrome"), Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe")),
            Pin::new(None, Some(r"C:\Tools\Editor.exe")),
            Pin::new(Some("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"), None),
        ];
        assert_eq!(slot(&pins, Some("chrome"), None), Some(1));
        assert_eq!(
            slot(&pins, Some("Chrome._crx_abc"), Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe")),
            None
        );
        assert_eq!(slot(&pins, None, Some(r"c:\tools\editor.EXE")), Some(2));
        assert_eq!(slot(&pins, None, Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe")), Some(1));
        assert_eq!(slot(&pins, Some("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"), None), Some(3));
        assert_eq!(slot(&pins, None, Some(r"C:\Other.exe")), None);
        assert_eq!(slot(&pins, None, None), None);
    }
}
