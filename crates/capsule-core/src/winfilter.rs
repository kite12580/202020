#[derive(Debug, Clone, Copy, Default)]
pub struct WindowFacts<'a> {
    pub visible: bool,
    pub cloaked: bool,
    pub own_process: bool,
    pub tool_window: bool,
    pub app_window: bool,
    pub no_activate: bool,
    pub owned: bool,
    pub class: &'a str,
}

const SHELL_CLASSES: [&str; 5] =
    ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "Windows.UI.Core.CoreWindow"];

/// Applies the taskbar's rules for which windows get a button. `Err` names
/// the rule that excluded the window.
pub fn taskbar_window(f: &WindowFacts) -> Result<(), &'static str> {
    if !f.visible {
        return Err("hidden");
    }
    if f.cloaked {
        return Err("cloaked");
    }
    if f.own_process {
        return Err("own");
    }
    if SHELL_CLASSES.contains(&f.class) {
        return Err("shell");
    }
    if f.app_window {
        return Ok(());
    }
    if f.tool_window {
        return Err("toolwindow");
    }
    if f.no_activate {
        return Err("noactivate");
    }
    if f.owned {
        return Err("owned");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal() -> WindowFacts<'static> {
        WindowFacts { visible: true, class: "Chrome_WidgetWin_1", ..Default::default() }
    }

    #[test]
    fn rules() {
        assert_eq!(taskbar_window(&normal()), Ok(()));
        assert_eq!(taskbar_window(&WindowFacts { visible: false, ..normal() }), Err("hidden"));
        assert_eq!(taskbar_window(&WindowFacts { cloaked: true, ..normal() }), Err("cloaked"));
        assert_eq!(taskbar_window(&WindowFacts { class: "Progman", ..normal() }), Err("shell"));
        assert_eq!(taskbar_window(&WindowFacts { tool_window: true, ..normal() }), Err("toolwindow"));
        assert_eq!(taskbar_window(&WindowFacts { owned: true, ..normal() }), Err("owned"));
        assert_eq!(taskbar_window(&WindowFacts { owned: true, app_window: true, ..normal() }), Ok(()));
        assert_eq!(taskbar_window(&WindowFacts { tool_window: true, app_window: true, ..normal() }), Ok(()));
        assert_eq!(taskbar_window(&WindowFacts { app_window: true, cloaked: true, ..normal() }), Err("cloaked"));
    }
}
