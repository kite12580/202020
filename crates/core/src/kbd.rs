use std::collections::BTreeMap;

pub fn label(iso639: &str, overrides: &BTreeMap<String, String>) -> String {
    let iso = iso639.to_ascii_lowercase();
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
        assert_eq!(label("zh", &overrides), "中");
        assert_eq!(label("en", &overrides), "EN");
        assert_eq!(label("", &overrides), "?");
        overrides.insert("en".into(), "英".into());
        assert_eq!(label("EN", &overrides), "英");
    }
}
