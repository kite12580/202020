use serde::Deserialize;
use std::collections::BTreeMap;
use toml_edit::{Array, DocumentMut, value};

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Config {
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
        Self { center_x: 0.5, bottom_gap: 6.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Backdrop {
    #[default]
    Acrylic,
    Accent,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Render {
    #[default]
    Hardware,
    Software,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub backdrop: Backdrop,
    pub opacity: f32,
    pub render: Render,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { backdrop: Backdrop::Acrylic, opacity: 0.25, render: Render::Hardware }
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

pub const DEFAULT_TEXT: &str = r#"# capsule-bar 配置。保存后自动生效。

[position]
# 胶囊中心的水平位置，占屏幕宽度的比例（0.0 – 1.0）。拖动后自动更新。
center_x = 0.5
# 胶囊到屏幕底边的距离（DIP）。拖动后自动更新。
bottom_gap = 6.0

[appearance]
# "acrylic"（系统亚克力）、"accent"（旧版模糊，亚克力失效时使用）或 "none"（纯透明）。
backdrop = "acrylic"
# 背景色叠加的不透明度（0.0 – 1.0）。
opacity = 0.25
# "hardware" 或 "software"。
render = "hardware"

[volume]
# 每格滚轮调整的音量百分比。
step = 2

[keyboard.labels]
# 按 ISO 639 语言代码覆盖键盘布局标签。内置：zh = "中"、ja = "あ"、ko = "한"，其余显示大写代码。
# 输入法处于英文模式时固定显示 "英"（中文）或 "A"（日文、韩文）。
# en = "EN"

[tray]
# 显示在胶囊上的托盘图标。在 ^ 弹窗里右键图标即可固定或取消固定。
pinned = []
"#;

pub fn parse(text: &str) -> Result<Config, String> {
    let mut c: Config = toml::from_str(text).map_err(|e| e.to_string())?;
    c.position.center_x = c.position.center_x.clamp(0.0, 1.0);
    c.position.bottom_gap = c.position.bottom_gap.max(0.0);
    c.appearance.opacity = c.appearance.opacity.clamp(0.0, 1.0);
    c.volume.step = c.volume.step.clamp(1, 20);
    c.keyboard.labels = c.keyboard.labels.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect();
    Ok(c)
}

fn edit(text: &str, f: impl FnOnce(&mut DocumentMut)) -> String {
    let mut doc = text.parse::<DocumentMut>().unwrap_or_default();
    f(&mut doc);
    doc.to_string()
}

/// Rewrites the position keys, preserving everything else in the file.
pub fn set_position(text: &str, center_x: f32, bottom_gap: f32) -> String {
    edit(text, |doc| {
        doc["position"]["center_x"] = value((f64::from(center_x) * 10_000.0).round() / 10_000.0);
        doc["position"]["bottom_gap"] = value((f64::from(bottom_gap) * 10.0).round() / 10.0);
    })
}

/// Rewrites the pinned tray list, preserving everything else in the file.
pub fn set_pinned(text: &str, pinned: &[String]) -> String {
    edit(text, |doc| {
        let mut arr: Array = pinned.iter().map(String::as_str).collect();
        arr.set_trailing_comma(false);
        doc["tray"]["pinned"] = value(arr);
    })
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
        let c = parse("[appearance]\nbackdrop = \"accent\"\nopacity = 3\n[keyboard.labels]\nEN = \"英\"").unwrap();
        assert_eq!(c.appearance.backdrop, Backdrop::Accent);
        assert_eq!(c.appearance.opacity, 1.0);
        assert_eq!(c.keyboard.labels["en"], "英");
        assert!(parse("[appearance]\nbackdrop = \"bogus\"").is_err());
    }

    #[test]
    fn edits_preserve_comments() {
        let t = set_position(DEFAULT_TEXT, 0.123456, 12.34);
        assert!(t.contains("# 胶囊中心"));
        let c = parse(&t).unwrap();
        assert_eq!(c.position.center_x, 0.1235);
        assert_eq!(c.position.bottom_gap, 12.3);

        let t = set_pinned(&t, &["a.exe|1".into(), "guid:{X}".into()]);
        assert!(t.contains("# 显示在胶囊上"));
        assert_eq!(parse(&t).unwrap().tray.pinned, ["a.exe|1", "guid:{X}"]);
    }

    #[test]
    fn edits_create_missing_tables() {
        let t = set_pinned(&set_position("", 0.25, 3.0), &["x|0".into()]);
        let c = parse(&t).unwrap();
        assert_eq!(c.position.center_x, 0.25);
        assert_eq!(c.tray.pinned, ["x|0"]);
    }
}
