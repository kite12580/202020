//! The strings an entry is matched by: its name, the name with Han characters
//! spelled out in pinyin, and hidden keywords.

use nucleo_matcher::Utf32String;

use crate::pinyin;

const MAX_PINYIN_VARIANTS: usize = 8;
const GAP: u32 = u32::MAX;

pub struct Key {
    pub text: Utf32String,
    kind: Kind,
}

enum Kind {
    Name,
    /// Name character index per text character, `GAP` for inserted spaces.
    Pinyin(Box<[u32]>),
    Hidden,
}

pub struct Keys(pub(crate) Box<[Key]>);

impl Keys {
    pub fn new(name: &str, hidden: &[&str]) -> Keys {
        let mut keys = vec![Key { text: name.into(), kind: Kind::Name }];
        for (text, map) in pinyin_variants(name) {
            keys.push(Key { text: text.into(), kind: Kind::Pinyin(map.into()) });
        }
        keys.extend(hidden.iter().filter(|h| !h.is_empty()).map(|&h| Key { text: h.into(), kind: Kind::Hidden }));
        Keys(keys.into())
    }
}

impl Key {
    /// Maps match indices in this key to sorted, unique name character indices.
    pub fn name_indices(&self, indices: &[u32]) -> Vec<u32> {
        let mut out: Vec<u32> = match &self.kind {
            Kind::Name => indices.to_vec(),
            Kind::Pinyin(map) => indices.iter().map(|&i| map[i as usize]).filter(|&i| i != GAP).collect(),
            Kind::Hidden => Vec::new(),
        };
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// A name to also match an Apps folder entry by, from its parsing name:
/// `{GUID}\Tools\calc.exe` → `calc`, `Microsoft.WindowsCalculator_8wekyb3d8bbwe!App` → `WindowsCalculator`.
pub fn app_keyword(id: &str) -> &str {
    if let Some((family, _)) = id.split_once('!') {
        let family = family.split('_').next().unwrap_or(family);
        return family.rsplit('.').next().unwrap_or(family);
    }
    let file = id.rsplit(['\\', '/']).next().unwrap_or(id);
    file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

/// `微信 Beta` → `wei xin Beta`, one string per reading combination: first the
/// most common reading everywhere, then each alternative reading on its own.
fn pinyin_variants(name: &str) -> Vec<(String, Vec<u32>)> {
    let chars: Vec<char> = name.chars().collect();
    let readings: Vec<Vec<&str>> = chars.iter().map(|&c| pinyin::readings(c).collect()).collect();
    if readings.iter().all(Vec::is_empty) {
        return Vec::new();
    }
    let mut out = vec![spell(&chars, &readings, None)];
    'outer: for (i, r) in readings.iter().enumerate() {
        for alt in 1..r.len() {
            if out.len() >= MAX_PINYIN_VARIANTS {
                break 'outer;
            }
            out.push(spell(&chars, &readings, Some((i, alt))));
        }
    }
    out
}

fn spell(chars: &[char], readings: &[Vec<&str>], alt: Option<(usize, usize)>) -> (String, Vec<u32>) {
    let mut text = String::new();
    let mut map = Vec::new();
    let mut gap = false;
    let mut push = |text: &mut String, c: char, i: u32| {
        text.push(c);
        map.push(i);
    };
    for (i, (&c, r)) in chars.iter().zip(readings).enumerate() {
        let after_word = !text.is_empty() && !text.ends_with(char::is_whitespace);
        if r.is_empty() {
            if gap && !c.is_whitespace() && after_word {
                push(&mut text, ' ', GAP);
            }
            gap = false;
            push(&mut text, c, i as u32);
            continue;
        }
        if after_word {
            push(&mut text, ' ', GAP);
        }
        let pick = match alt {
            Some((at, n)) if at == i => n,
            _ => 0,
        };
        for s in r[pick].chars() {
            push(&mut text, s, i as u32);
        }
        gap = true;
    }
    (text, map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(name: &str) -> Vec<String> {
        pinyin_variants(name).into_iter().map(|(t, _)| t).collect()
    }

    #[test]
    fn spells_han_with_word_gaps() {
        assert_eq!(texts("微信")[0], "wei xin");
        assert_eq!(texts("QQ音乐")[..2], ["QQ yin le", "QQ yin yue"]);
        assert_eq!(texts("微信 Beta")[0], "wei xin Beta");
        assert_eq!(texts("微信Beta")[0], "wei xin Beta");
        assert!(texts("Chrome").is_empty());
    }

    #[test]
    fn app_keywords() {
        assert_eq!(app_keyword(r"{6D809377-6AF0-444B-8957-A3773F02200E}\Tools\calc.exe"), "calc");
        assert_eq!(app_keyword("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"), "WindowsCalculator");
        assert_eq!(app_keyword("Chrome"), "Chrome");
    }

    #[test]
    fn alternatives_are_capped() {
        let v = texts("行行行行行");
        assert_eq!(v.len(), MAX_PINYIN_VARIANTS);
        assert_eq!(v[0], "xing xing xing xing xing");
        assert_eq!(v[1], "hang xing xing xing xing");
    }

    #[test]
    fn maps_back_to_name_chars() {
        let (text, map) = &pinyin_variants("QQ音乐")[0];
        let key = Key { text: text.as_str().into(), kind: Kind::Pinyin(map.clone().into()) };
        assert_eq!(key.name_indices(&[0, 2, 3, 6, 7]), [0, 2, 3]);
        let hidden = Key { text: "x".into(), kind: Kind::Hidden };
        assert!(hidden.name_indices(&[0]).is_empty());
    }
}
