use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;
use toml_edit::{Array, DocumentMut, value};

use crate::hyper::{self, Action, Binding, Job, Keymap};

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub capsule_bar: CapsuleBar,
    pub launcher: Launcher,
    pub hyper: Hyper,
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

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Tray {
    pub pinned: Vec<String>,
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Hyper {
    pub enabled: bool,
    pub slow_mouse_speed: u32,
    pub terminals: Vec<String>,
    pub hotstrings: BTreeMap<String, String>,
    pub keys: Keymap,
    pub alt_keys: Keymap,
}

impl Default for Hyper {
    fn default() -> Self {
        #[derive(Deserialize)]
        struct Doc {
            hyper: Keys,
        }
        #[derive(Deserialize)]
        struct Keys {
            keys: Keymap,
            alt_keys: Keymap,
        }
        let Doc { hyper: Keys { keys, alt_keys } } = toml::from_str(DEFAULT_KEYS).expect("DEFAULT_KEYS");
        Self {
            enabled: true,
            slow_mouse_speed: 1,
            terminals: vec!["WindowsTerminal.exe".into()],
            hotstrings: BTreeMap::new(),
            keys,
            alt_keys,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingTable {
    #[serde(rename = "do")]
    action: Option<String>,
    wrap: Option<[String; 2]>,
    #[serde(default)]
    at_end: bool,
    text: Option<String>,
    wheel: Option<i32>,
    terminal: Option<String>,
}

fn action(s: &str) -> Result<Action, String> {
    hyper::action(s).ok_or_else(|| format!("{s:?} is neither keys nor an action"))
}

fn binding(v: toml::Value) -> Result<Binding, String> {
    if let toml::Value::String(s) = &v {
        return Ok(Binding { action: action(s)?, terminal: None });
    }
    let t: BindingTable = v.try_into().map_err(|e: toml::de::Error| e.message().to_owned())?;
    let actions = [
        t.action.as_deref().map(action).transpose()?,
        t.wrap.map(|[open, close]| Action::Job(Job::Wrap { open, close, at_end: t.at_end })),
        t.text.map(|text| Action::Job(Job::Paste(text))),
        t.wheel.map(Action::Wheel),
    ];
    let mut actions = actions.into_iter().flatten();
    let (Some(main), None) = (actions.next(), actions.next()) else {
        return Err("needs exactly one of do, wrap, text and wheel".into());
    };
    Ok(Binding { action: main, terminal: t.terminal.as_deref().map(action).transpose()? })
}

fn keymap(table: BTreeMap<String, toml::Value>, mut skipped: impl FnMut(String)) -> Keymap {
    table
        .into_iter()
        .filter_map(|(name, v)| {
            let Some(key) = crate::launcher::hotkey::key(&name) else {
                skipped(format!("{name:?}: unknown key"));
                return None;
            };
            binding(v).map(|b| (key as u16, b)).map_err(|e| skipped(format!("{name:?}: {e}"))).ok()
        })
        .collect()
}

impl<'de> Deserialize<'de> for Keymap {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(keymap(BTreeMap::deserialize(d)?, |e| log::warn!("hyper keys: {e}")))
    }
}

macro_rules! default_keys {
    () => {
        r#"[hyper.keys]
# CapsLock + key. Without this table these defaults apply; with it, keys left out of it do nothing.
# A value is either keys to press, separated by spaces ("End Enter", "Ctrl+Shift+Left"), or one of the actions
# copy_or_line, paste_plain, delete_line, hotstring, escape_or_move, power_options, topmost, caption,
# reload, transparency, switch_desktop, left_button, middle_button and right_button (held while the key is).
# A table instead holds one of:
#   do = keys or an action
#   wrap = [open, close]   wraps the selection, or inserts both; with at_end = true, at the end of the line, then Enter
#   text = "..."           types the text
#   wheel = n              turns the wheel n notches, negative is down
# plus terminal = ... for the programs in terminals.
h = "Left"
j = "Down"
k = "Up"
l = "Right"
e = "Ctrl+Right"
b = "Ctrl+Left"
";" = "Home"
"'" = "End"
enter = "End"
space = "Right"
g = "Ctrl+End"
r = "PageUp"
q = "PageDown"
n = "Ctrl+Shift+Tab"
m = "Ctrl+Tab"
o = "End Enter"
u = { do = "Ctrl+Z", terminal = "Ctrl+/" }
z = "Ctrl+S"
c = "Alt+Tab"
v = "Ctrl+Shift+V"
"/" = "Ctrl+F"
1 = "left_button"
2 = "middle_button"
3 = "right_button"
"," = "BrowserBack"
"." = "BrowserForward"
"`" = "Apps"
w = { do = "Ctrl+Shift+Left Backspace", terminal = "Ctrl+W" }
backspace = "Ctrl+Left Ctrl+Shift+Right Backspace"
d = { do = "delete_line", terminal = "Ctrl+U" }
x = "Ctrl+Insert Delete"
y = "copy_or_line"
p = "Shift+Insert"
9 = { wrap = ["(", ")"] }
0 = { text = ")" }
"[" = { wrap = [" {", "}"], at_end = true }
"=" = { wrap = [' + "', '" + '] }
tab = "hotstring"
f = "escape_or_move"
f1 = "topmost"
f3 = "MediaNext"
f4 = "transparency"
f5 = "reload"
f9 = "power_options"

[hyper.alt_keys]
# CapsLock + left Alt + key, written as above.
h = "Shift+Left"
j = "Shift+Down"
k = "Shift+Up"
l = "Shift+Right"
e = "Ctrl+Shift+Right"
b = "Ctrl+Shift+Left"
";" = "Shift+Home"
"'" = "Shift+End"
g = "Ctrl+Home"
r = { wheel = 2 }
q = { wheel = -2 }
o = "Home Enter Up"
u = "Ctrl+Y"
z = "Ctrl+Shift+S"
v = "switch_desktop"
w = "End Shift+Down Shift+Home Backspace"
d = "End Shift+Home Delete"
x = "Ctrl+Insert Backspace"
y = "copy_or_line"
p = "paste_plain"
9 = { wrap = ['"', '"'] }
0 = { wrap = ["'", "'"] }
"[" = { wrap = ["[", "]"] }
f1 = "caption"
f2 = "MediaPlayPause"
f3 = "MediaPrev"
"#
    };
}

const DEFAULT_KEYS: &str = default_keys!();

pub const DEFAULT_TEXT: &str = concat!(
    r#"# 202020 settings. Capsule bar changes apply on save while it is running.

[capsule_bar]
# Replace the Windows taskbar with the capsule bar. Toggle it from the tray menu.
enabled = false

[capsule_bar.position]
# Horizontal center of the capsule as a fraction of the screen width (0.0 – 1.0). Updated when dragged.
# 0.0 and 1.0 keep the capsule against the left or right edge as it grows and shrinks.
center_x = 0.5
# Distance from the capsule to the bottom of the screen (DIPs). Updated when dragged.
bottom_gap = 0.0

[capsule_bar.appearance]
# "acrylic" (system acrylic), "accent" (legacy blur, for when acrylic doesn't work) or "none" (plain transparent).
backdrop = "none"
# Opacity of the background tint (0.0 – 1.0).
opacity = 0.8

[capsule_bar.volume]
# Volume change per wheel notch, in percent.
step = 2

[capsule_bar.keyboard.labels]
# Override keyboard layout labels by ISO 639 language code. Built in: zh = "中", ja = "あ", ko = "한"; others show the
# uppercase code. An IME in English mode always shows "英" (Chinese) or "A" (Japanese, Korean).
# en = "EN"

[capsule_bar.tray]
# Tray icons shown on the capsule. Right-click an icon in the ^ popup to pin or unpin it.
pinned = []

[launcher]
# Spotlight-style search, opened with the hotkey. Toggle it from the tray menu.
enabled = true
# Modifiers (Alt, Ctrl, Shift, Win) and a key, e.g. "Alt+Space" or "Ctrl+Shift+K". Applied by Reload config.
hotkey = "Alt+Space"
# Rows shown at most (1 – 20).
max_results = 8
# Free the search index after this many minutes without opening the launcher.
idle_unload_minutes = 10

[launcher.prefixes]
# Typing a prefix first searches only that source. "" turns a prefix off.
apps = ""
commands = ">"
folders = "/"
windows = "@"

# Each source can be turned off; its weight scales its results' ranking (0.0 – 2.0).
[launcher.apps]
enabled = false
weight = 1.0

[launcher.windows]
enabled = false
weight = 0.9

[launcher.commands]
enabled = true
weight = 0.8
custom = []
# Your own entries:
# [[launcher.commands.custom]]
# name = "Calculator 计算器"
# command = "calc.exe"
# args = ""

[launcher.folders]
enabled = true
weight = 1.0

# Folders whose files and subfolders are searchable by name. Repeat the table for more roots.
[[launcher.folders.roots]]
# %VARIABLES% are expanded.
path = "C:\\Games"
# Subfolder levels to descend; 0 searches only the folder itself.
depth = 1
# Only files with these extensions, e.g. ["pdf", "docx"]; [] means all.
extensions = ["exe", "lnk"]
# File or folder names to skip; * and ? are wildcards.
exclude = []
# Also list folders themselves.
include_dirs = false
# Stop scanning this root after this many entries.
max_items = 20000

[hyper]
# Hold CapsLock for the hyper layer, tap it for Esc, press Shift+CapsLock for CapsLock. Toggle it from the tray menu. Applied by Reload config.
enabled = true
# Mouse speed (1 – 20) while CapsLock and left Alt are held.
slow_mouse_speed = 1
# Programs that get the terminal = ... variant of a key below.
terminals = ["WindowsTerminal.exe"]

[hyper.hotstrings]
# Type a key, then CapsLock+Tab replaces it with the text. Keys match the end of the line, ignoring case.
# sig = "Best regards,\nMe"

"#,
    default_keys!(),
    r#"
[logging]
# Write logs to the logs folder next to the exe: "off", "error", "warn", "info", "debug" or "trace". Use "debug" when
# reporting a problem; "trace" also logs why each window was shown on or left off the capsule bar. Read at startup.
level = "off"
"#
);

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
    fn keymaps() {
        #[derive(Deserialize)]
        struct Doc {
            hyper: BTreeMap<String, BTreeMap<String, toml::Value>>,
        }
        let mut doc: Doc = toml::from_str(DEFAULT_KEYS).unwrap();
        for name in ["keys", "alt_keys"] {
            let table = doc.hyper.remove(name).unwrap();
            let n = table.len();
            assert_eq!(keymap(table, |e| panic!("{e}")).len(), n);
        }

        let text = "[hyper.keys]\nh = 'Home'\nbogus = 'End'\nj = { do = 'End', wheel = 1 }\nk = { wrap = ['a'] }\n\
                    l = { do = 'Left', terminal = 'nope' }\nm = { tetx = 'x' }\nn = 'Ctrl+Bogus'";
        let table: Doc = toml::from_str(text).unwrap();
        let mut skipped = Vec::new();
        let keys = keymap(table.hyper["keys"].clone(), |e| skipped.push(e));
        assert_eq!(keys.len(), 1);
        assert_eq!(skipped.len(), 6, "{skipped:?}");
        let h = parse(text).unwrap().hyper;
        assert_eq!(h.keys, keys);
        assert_eq!(h.alt_keys, Hyper::default().alt_keys);
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
