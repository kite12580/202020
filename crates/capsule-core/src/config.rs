use serde::Deserialize;
use std::collections::BTreeMap;
use toml_edit::{Array, DocumentMut, value};

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub capsule_bar: CapsuleBar,
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

pub const DEFAULT_TEXT: &str = r#"# 202020 settings. Capsule bar changes apply on save while it is running.

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

[logging]
# Write logs to the logs folder next to the exe: "off", "error", "warn", "info", "debug" or "trace". Use "debug" when
# reporting a problem; "trace" also logs why each window was shown on or left off the capsule bar. Read at startup.
level = "off"
"#;

pub fn parse(text: &str) -> Result<Config, String> {
    let mut config: Config = toml::from_str(text).map_err(|e| e.to_string())?;
    let c = &mut config.capsule_bar;
    c.position.center_x = c.position.center_x.clamp(0.0, 1.0);
    c.position.bottom_gap = c.position.bottom_gap.max(0.0);
    c.appearance.opacity = c.appearance.opacity.clamp(0.0, 1.0);
    c.volume.step = c.volume.step.clamp(1, 20);
    c.keyboard.labels =
        std::mem::take(&mut c.keyboard.labels).into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect();
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
    fn edits_create_missing_tables() {
        let t = set_enabled(&set_pinned(&set_position("", 0.25, 3.0), &["x|0".into()]), true);
        let c = parse(&t).unwrap().capsule_bar;
        assert!(c.enabled);
        assert_eq!(c.position.center_x, 0.25);
        assert_eq!(c.tray.pinned, ["x|0"]);
    }
}
