//! Toneless pinyin readings, most common first. The table is built by
//! `scripts/gen_pinyin.py`, which documents its layout.

static DATA: &[u8] = include_bytes!("../data/pinyin.bin");

const RANGES: [(u32, u32); 2] = [(0x3400, 0x4DBF), (0x4E00, 0x9FFF)];
const SYLLABLE: usize = 6;

fn u16_at(byte: usize) -> usize {
    usize::from(u16::from_le_bytes([DATA[byte], DATA[byte + 1]]))
}

struct Layout {
    syllables: usize,
    offsets: usize,
    lists: usize,
    chars: usize,
}

fn layout() -> Layout {
    let (n_syl, n_lists) = (u16_at(0), u16_at(2));
    let syllables = 4;
    let offsets = syllables + n_syl * SYLLABLE;
    let lists = offsets + (n_lists + 1) * 2;
    let chars = lists + u16_at(offsets + n_lists * 2) * 2;
    Layout { syllables, offsets, lists, chars }
}

fn syllable(l: &Layout, id: usize) -> &'static str {
    let s = &DATA[l.syllables + id * SYLLABLE..][..SYLLABLE];
    let n = s.iter().position(|&b| b == 0).unwrap_or(SYLLABLE);
    std::str::from_utf8(&s[..n]).unwrap_or("")
}

fn char_slot(c: char) -> Option<usize> {
    let c = c as u32;
    let mut base = 0;
    for (lo, hi) in RANGES {
        if (lo..=hi).contains(&c) {
            return Some(base + (c - lo) as usize);
        }
        base += (hi - lo + 1) as usize;
    }
    None
}

pub fn readings(c: char) -> impl Iterator<Item = &'static str> {
    let l = layout();
    let list = char_slot(c).map_or(0, |slot| u16_at(l.chars + slot * 2));
    let ids = match list {
        0 => 0..0,
        n => u16_at(l.offsets + (n - 1) * 2)..u16_at(l.offsets + n * 2),
    };
    ids.map(move |i| syllable(&l, u16_at(l.lists + i * 2)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(c: char) -> Vec<&'static str> {
        readings(c).collect()
    }

    #[test]
    fn common_readings() {
        assert_eq!(all('微'), ["wei"]);
        assert_eq!(all('信'), ["xin", "shen"]);
        assert_eq!(all('中'), ["zhong"]);
        assert_eq!(all('行')[..2], ["xing", "hang"]);
        assert_eq!(all('绿')[0], "lv");
        assert_eq!(all('㐀'), ["qiu"]);
    }

    #[test]
    fn non_han_has_none() {
        assert!(all('a').is_empty());
        assert!(all('あ').is_empty());
        assert!(all('〇').is_empty());
        assert!(!all('龙').is_empty());
    }
}
