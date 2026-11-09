use serde::Deserialize;
use std::collections::BTreeMap;
use toml_edit::{Array, DocumentMut, value};

mod defaults;
mod hyper;

pub use defaults::DEFAULT_TEXT;
pub use hyper::Hyper;

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub capsule_bar: CapsuleBar,
    #[serde(default)]
    pub launcher: Launcher,
    #[serde(default)]
    pub hyper: Hyper,
    #[serde(default)]
    pub logging: Logging,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    #[default]
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<LogLevel> for log::LevelFilter {
    fn from(l: LogLevel) -> Self {
        match l {
            LogLevel::Off => Self::Off,
            LogLevel::Error => Self::Error,
            LogLevel::Warn => Self::Warn,
            LogLevel::Info => Self::Info,
            LogLevel::Debug => Self::Debug,
            LogLevel::Trace => Self::Trace,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Logging {
    pub level: LogLevel,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct CapsuleBar {
    pub enabled: bool,
    pub position: Position,
    pub appearance: Appearance,
    pub volume: Volume,
    pub keyboard: Keyboard,
    pub tray: Tray,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Position {
    /// Fraction of the monitor width, 0.0 – 1.0.
    pub center_x: f32,
    /// DIPs above the monitor bottom.
    pub bottom_gap: f32,
}

impl Default for Position {
    fn default() -> Self {
        Self { center_x: 0.5, bottom_gap: 0.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Backdrop {
    Acrylic,
    Accent,
    #[default]
    None,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub backdrop: Backdrop,
    pub opacity: f32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { backdrop: Backdrop::None, opacity: 0.8 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Volume {
    /// Percent per wheel notch.
    pub step: u32,
}

impl Default for Volume {
    fn default() -> Self {
        Self { step: 2 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Keyboard {
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Tray {
    pub pinned: Vec<String>,
}

impl Default for Tray {
    fn default() -> Self {
        Self { pinned: vec!["202020.exe|1".into()] }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Launcher {
    pub enabled: bool,
    pub hotkey: String,
    pub max_results: usize,
    pub idle_unload_minutes: u32,
    pub prefixes: Prefixes,
    pub apps: AppsSource,
    pub windows: WindowsSource,
    pub commands: CommandsSource,
    pub folders: FoldersSource,
}

impl Default for Launcher {
    fn default() -> Self {
        Self {
            enabled: true,
            hotkey: "Alt+Space".into(),
            max_results: 8,
            idle_unload_minutes: 10,
            prefixes: Prefixes::default(),
            apps: AppsSource::default(),
            windows: WindowsSource::default(),
            commands: CommandsSource::default(),
            folders: FoldersSource::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Prefixes {
    pub apps: String,
    pub commands: String,
    pub folders: String,
    pub windows: String,
}

impl Default for Prefixes {
    fn default() -> Self {
        Self { apps: String::new(), commands: ">".into(), folders: "/".into(), windows: "@".into() }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct AppsSource {
    pub enabled: bool,
    pub weight: f32,
}

impl Default for AppsSource {
    fn default() -> Self {
        Self { enabled: false, weight: 1.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct WindowsSource {
    pub enabled: bool,
    pub weight: f32,
}

impl Default for WindowsSource {
    fn default() -> Self {
        Self { enabled: false, weight: 0.9 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct CommandsSource {
    pub enabled: bool,
    pub weight: f32,
    pub custom: Vec<CustomCommand>,
}

impl Default for CommandsSource {
    fn default() -> Self {
        Self { enabled: true, weight: 0.8, custom: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct CustomCommand {
    pub name: String,
    pub command: String,
    pub args: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct FoldersSource {
    pub enabled: bool,
    pub weight: f32,
    pub roots: Vec<FolderRoot>,
}

impl Default for FoldersSource {
    fn default() -> Self {
        Self {
            enabled: true,
            weight: 1.0,
            roots: vec![FolderRoot {
                path: r"C:\Games".into(),
                depth: 1,
                extensions: vec!["exe".into(), "lnk".into()],
                exclude: Vec::new(),
                include_dirs: false,
                max_items: 20_000,
            }],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct FolderRoot {
    pub path: String,
    pub depth: u32,
    pub extensions: Vec<String>,
    pub exclude: Vec<String>,
    pub include_dirs: bool,
    pub max_items: usize,
}

impl Default for FolderRoot {
    fn default() -> Self {
        Self {
            path: r"%USERPROFILE%\Documents".into(),
            depth: 3,
            extensions: Vec::new(),
            exclude: vec!["node_modules".into(), ".git".into(), "target".into()],
            include_dirs: true,
            max_items: 20_000,
        }
    }
}

pub fn parse(text: &str) -> Result<Config, String> {
    let mut config: Config = toml::from_str(text).map_err(|e| e.to_string())?;
    let c = &mut config.capsule_bar;
    c.position.center_x = c.position.center_x.clamp(0.0, 1.0);
    c.position.bottom_gap = c.position.bottom_gap.max(0.0);
    c.appearance.opacity = c.appearance.opacity.clamp(0.0, 1.0);
    c.volume.step = c.volume.step.clamp(1, 20);
    c.keyboard.labels =
        std::mem::take(&mut c.keyboard.labels).into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect();
    let l = &mut config.launcher;
    l.max_results = l.max_results.clamp(1, 20);
    l.idle_unload_minutes = l.idle_unload_minutes.clamp(1, 24 * 60);
    config.hyper.slow_mouse_speed = config.hyper.slow_mouse_speed.clamp(1, 20);
    for w in [&mut l.apps.weight, &mut l.windows.weight, &mut l.commands.weight, &mut l.folders.weight] {
        *w = w.clamp(0.0, 2.0);
    }
    Ok(config)
}

fn edit(text: &str, f: impl FnOnce(&mut DocumentMut)) -> String {
    let mut doc = text.parse::<DocumentMut>().unwrap_or_default();
    f(&mut doc);
    doc.to_string()
}

/// Rewrites the position keys, preserving everything else in the file.
pub fn set_position(text: &str, center_x: f32, bottom_gap: f32) -> String {
    edit(text, |doc| {
        doc["capsule_bar"]["position"]["center_x"] = value((f64::from(center_x) * 10_000.0).round() / 10_000.0);
        doc["capsule_bar"]["position"]["bottom_gap"] = value((f64::from(bottom_gap) * 10.0).round() / 10.0);
    })
}

/// Rewrites the pinned tray list, preserving everything else in the file.
pub fn set_pinned(text: &str, pinned: &[String]) -> String {
    edit(text, |doc| {
        let mut arr: Array = pinned.iter().map(String::as_str).collect();
        arr.set_trailing_comma(false);
        doc["capsule_bar"]["tray"]["pinned"] = value(arr);
    })
}

pub fn set_enabled(text: &str, enabled: bool) -> String {
    edit(text, |doc| doc["capsule_bar"]["enabled"] = value(enabled))
}

pub fn set_launcher_enabled(text: &str, enabled: bool) -> String {
    edit(text, |doc| doc["launcher"]["enabled"] = value(enabled))
}

pub fn set_hyper_enabled(text: &str, enabled: bool) -> String {
    edit(text, |doc| doc["hyper"]["enabled"] = value(enabled))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_text_matches_defaults() {
        assert_eq!(parse(DEFAULT_TEXT).unwrap(), Config::default());
    }

    #[test]
    fn empty_and_partial() {
        assert_eq!(parse("").unwrap(), Config::default());
        let c = parse(
            "[capsule_bar.appearance]\nbackdrop = \"accent\"\nopacity = 3\n[capsule_bar.keyboard.labels]\nEN = \"英\"",
        )
        .unwrap()
        .capsule_bar;
        assert!(!c.enabled);
        assert_eq!(c.appearance.backdrop, Backdrop::Accent);
        assert_eq!(c.appearance.opacity, 1.0);
        assert_eq!(c.keyboard.labels["en"], "英");
        assert!(parse("[capsule_bar.appearance]\nbackdrop = \"bogus\"").is_err());
        assert_eq!(parse("[logging]\nlevel = \"debug\"").unwrap().logging.level, LogLevel::Debug);
    }

    #[test]
    fn edits_preserve_comments() {
        let t = set_position(DEFAULT_TEXT, 0.123456, 12.34);
        assert!(t.contains("# Horizontal center"));
        let c = parse(&t).unwrap().capsule_bar;
        assert_eq!(c.position.center_x, 0.1235);
        assert_eq!(c.position.bottom_gap, 12.3);

        let t = set_pinned(&t, &["a.exe|1".into(), "guid:{X}".into()]);
        assert!(t.contains("# Tray icons shown"));
        assert_eq!(parse(&t).unwrap().capsule_bar.tray.pinned, ["a.exe|1", "guid:{X}"]);

        let t = set_enabled(&t, true);
        assert!(t.contains("# Replace the Windows taskbar"));
        assert!(parse(&t).unwrap().capsule_bar.enabled);
        assert!(!parse(&set_enabled(&t, false)).unwrap().capsule_bar.enabled);
    }

    #[test]
    fn launcher_section() {
        let l = parse("[launcher.apps]\nenabled = true\n[launcher.folders]\nweight = 9").unwrap().launcher;
        assert!(l.enabled && l.apps.enabled && !l.windows.enabled);
        assert_eq!((l.windows.weight, l.folders.weight), (0.9, 2.0));
        assert_eq!(l.folders.roots, FoldersSource::default().roots);
        let l = parse("[[launcher.folders.roots]]\npath = 'D:\\x'\n[launcher]\nmax_results = 0").unwrap().launcher;
        assert_eq!(l.folders.roots.len(), 1);
        assert_eq!((l.folders.roots[0].path.as_str(), l.folders.roots[0].depth), (r"D:\x", 3));
        assert_eq!(l.max_results, 1);
        let t = set_launcher_enabled(DEFAULT_TEXT, false);
        assert!(t.contains("# Spotlight-style search"));
        assert!(!parse(&t).unwrap().launcher.enabled);
    }

    #[test]
    fn hyper_section() {
        let h = parse("[hyper]\nslow_mouse_speed = 50\n[hyper.hotstrings]\nsig = 'a\\nb'").unwrap().hyper;
        assert!(h.enabled);
        assert_eq!(h.slow_mouse_speed, 20);
        assert_eq!(h.hotstrings["sig"], "a\\nb");
        assert_eq!(parse("[hyper.hotstrings]\nsig = \"a\\nb\"").unwrap().hyper.hotstrings["sig"], "a\nb");
        let t = set_hyper_enabled(DEFAULT_TEXT, false);
        assert!(t.contains("# Hold CapsLock"));
        assert!(!parse(&t).unwrap().hyper.enabled);
    }

    #[test]
    fn edits_create_missing_tables() {
        let t = set_enabled(&set_pinned(&set_position("", 0.25, 3.0), &["x|0".into()]), true);
        let c = parse(&t).unwrap().capsule_bar;
        assert!(c.enabled);
        assert_eq!(c.position.center_x, 0.25);
        assert_eq!(c.tray.pinned, ["x|0"]);
    }
}
