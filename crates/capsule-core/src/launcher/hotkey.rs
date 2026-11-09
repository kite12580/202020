//! Parses hotkeys like `Alt+Space` into `RegisterHotKey` modifiers and a
//! virtual-key code.

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

const NAMED: &[(&str, u32)] = &[
    ("space", 0x20),
    ("enter", 0x0D),
    ("return", 0x0D),
    ("tab", 0x09),
    ("esc", 0x1B),
    ("escape", 0x1B),
    ("backspace", 0x08),
    ("insert", 0x2D),
    ("delete", 0x2E),
    ("home", 0x24),
    ("end", 0x23),
    ("pageup", 0x21),
    ("pagedown", 0x22),
    ("left", 0x25),
    ("up", 0x26),
    ("right", 0x27),
    ("down", 0x28),
    ("apps", 0x5D),
    ("browserback", 0xA6),
    ("browserforward", 0xA7),
    ("medianext", 0xB0),
    ("mediaprev", 0xB1),
    ("mediaplaypause", 0xB3),
    ("`", 0xC0),
    ("backquote", 0xC0),
    ("-", 0xBD),
    ("=", 0xBB),
    ("[", 0xDB),
    ("]", 0xDD),
    ("\\", 0xDC),
    (";", 0xBA),
    ("'", 0xDE),
    (",", 0xBC),
    (".", 0xBE),
    ("/", 0xBF),
];

pub fn key(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    if let [c] = lower.as_bytes()
        && c.is_ascii_alphanumeric()
    {
        return Some(u32::from(c.to_ascii_uppercase()));
    }
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u32>().ok())
        && (1..=24).contains(&n)
    {
        return Some(0x70 + n - 1);
    }
    NAMED.iter().find(|(n, _)| *n == lower).map(|&(_, vk)| vk)
}

/// `(modifiers, virtual key)`.
pub fn parse(s: &str) -> Option<(u32, u32)> {
    let mut mods = 0;
    let mut vk = None;
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    for part in parts {
        let m = match part.to_ascii_lowercase().as_str() {
            "alt" => MOD_ALT,
            "ctrl" | "control" => MOD_CONTROL,
            "shift" => MOD_SHIFT,
            "win" | "windows" => MOD_WIN,
            _ => {
                if vk.replace(key(part)?).is_some() {
                    return None;
                }
                continue;
            }
        };
        mods |= m;
    }
    Some((mods, vk?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!(parse("Alt+Space"), Some((MOD_ALT, 0x20)));
        assert_eq!(parse(" ctrl + shift + k "), Some((MOD_CONTROL | MOD_SHIFT, 'K' as u32)));
        assert_eq!(parse("Win+F12"), Some((MOD_WIN, 0x7B)));
        assert_eq!(parse("Alt+`"), Some((MOD_ALT, 0xC0)));
        assert_eq!(parse("F1"), Some((0, 0x70)));
        assert_eq!(parse("Alt+1"), Some((MOD_ALT, '1' as u32)));
    }

    #[test]
    fn rejects() {
        for s in ["", "Alt", "Alt+", "Alt+A+B", "Alt+Bogus", "F25", "Hyper+A"] {
            assert_eq!(parse(s), None, "{s}");
        }
    }
}
