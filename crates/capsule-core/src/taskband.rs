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

/// A button in the task area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    Window(usize),
    Pin(usize),
}

/// Task buttons for windows sorted by pin slot (`usize::MAX` for none):
/// each pin in order with its windows, or the pin itself when none are
/// open, then the unpinned windows.
pub fn tasks(slots: &[usize], pins: usize) -> Vec<Task> {
    let mut out = Vec::with_capacity(slots.len() + pins);
    let mut windows = slots.iter().copied().enumerate().peekable();
    for p in 0..pins {
        let start = out.len();
        while let Some((i, _)) = windows.next_if(|&(_, s)| s == p) {
            out.push(Task::Window(i));
        }
        if out.len() == start {
            out.push(Task::Pin(p));
        }
    }
    out.extend(windows.map(|(i, _)| Task::Window(i)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_buttons() {
        use Task::{Pin, Window};
        assert_eq!(tasks(&[0, 0, 2, usize::MAX], 3), [Window(0), Window(1), Pin(1), Window(2), Window(3)]);
        assert_eq!(tasks(&[], 2), [Pin(0), Pin(1)]);
        assert_eq!(tasks(&[usize::MAX], 0), [Window(0)]);
    }

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
