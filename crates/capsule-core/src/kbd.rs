use std::collections::BTreeMap;

pub fn has_ime(iso639: &str) -> bool {
    matches!(iso639.to_ascii_lowercase().as_str(), "zh" | "ja" | "ko")
}

/// `native` is the IME conversion state when the layout has one: `false`
/// means the IME is typing Latin letters.
pub fn label(iso639: &str, overrides: &BTreeMap<String, String>, native: Option<bool>) -> String {
    let iso = iso639.to_ascii_lowercase();
    if native == Some(false) {
        return if iso == "zh" { "英".into() } else { "A".into() };
    }
    if let Some(label) = overrides.get(&iso) {
        return label.clone();
    }
    match iso.as_str() {
        "zh" => "中".into(),
        "ja" => "あ".into(),
        "ko" => "한".into(),
        "" => "?".into(),
        _ => iso.to_ascii_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        let mut overrides = BTreeMap::new();
        assert_eq!(label("zh", &overrides, None), "中");
        assert_eq!(label("zh", &overrides, Some(true)), "中");
        assert_eq!(label("zh", &overrides, Some(false)), "英");
        assert_eq!(label("ja", &overrides, Some(false)), "A");
        assert_eq!(label("en", &overrides, None), "EN");
        assert_eq!(label("", &overrides, None), "?");
        overrides.insert("en".into(), "英".into());
        assert_eq!(label("EN", &overrides, None), "英");
        assert!(has_ime("ZH") && !has_ime("en"));
    }
}
