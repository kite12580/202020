//! Launch counts kept in memory. Saved as TSV lines of
//! `count, last launch (Unix seconds), key, name`.

use std::collections::HashMap;

use super::Source;

const MAX_ENTRIES: usize = 500;
const HALF_LIFE_DAYS: f64 = 30.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub count: u32,
    pub last: u64,
    pub name: String,
}

#[derive(Default)]
pub struct History {
    entries: HashMap<String, Entry>,
    dirty: bool,
}

pub fn key(source: Source, id: &str) -> String {
    format!("{}:{id}", source.tag())
}

/// The source and id `key` was made from.
pub fn split_key(key: &str) -> Option<(Source, &str)> {
    let (tag, id) = key.split_once(':')?;
    let mut tag = tag.chars();
    let source = Source::from_tag(tag.next()?)?;
    tag.next().is_none().then_some((source, id))
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\r', '\n'], " ")
}

fn frecency(e: &Entry, now: u64) -> f64 {
    let age_days = now.saturating_sub(e.last) as f64 / 86_400.0;
    f64::from(e.count).ln_1p() * (0.5 + 0.5 * 0.5f64.powf(age_days / HALF_LIFE_DAYS))
}

impl History {
    pub fn parse(text: &str) -> History {
        let mut entries = HashMap::new();
        for line in text.lines() {
            let mut f = line.splitn(4, '\t');
            let (Some(count), Some(last), Some(key), Some(name)) = (f.next(), f.next(), f.next(), f.next()) else {
                continue;
            };
            let (Ok(count), Ok(last)) = (count.parse(), last.parse()) else { continue };
            if split_key(key).is_some() {
                entries.insert(key.to_string(), Entry { count, last, name: name.to_string() });
            }
        }
        let mut h = History { entries, dirty: false };
        h.trim(u64::MAX);
        h
    }

    pub fn serialize(&self) -> String {
        let mut keys: Vec<&String> = self.entries.keys().collect();
        keys.sort();
        keys.into_iter()
            .map(|k| {
                let e = &self.entries[k];
                format!("{}\t{}\t{}\t{}\n", e.count, e.last, k, e.name)
            })
            .collect()
    }

    pub fn record(&mut self, key: &str, name: &str, now: u64) {
        let e = self.entries.entry(clean(key)).or_insert(Entry { count: 0, last: now, name: String::new() });
        e.count = e.count.saturating_add(1);
        e.last = now;
        e.name = clean(name);
        self.dirty = true;
        self.trim(now);
    }

    fn trim(&mut self, now: u64) {
        while self.entries.len() > MAX_ENTRIES {
            let Some(worst) = self
                .entries
                .iter()
                .min_by(|a, b| frecency(a.1, now).total_cmp(&frecency(b.1, now)))
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.entries.remove(&worst);
        }
    }

    /// Ranking multiplier, 1.0 for keys never launched.
    pub fn boost(&self, key: &str, now: u64) -> f64 {
        self.entries.get(key).map_or(1.0, |e| 1.0 + 0.25 * frecency(e, now))
    }

    /// Entries by descending frecency.
    pub fn top(&self, now: u64) -> Vec<(&str, &Entry)> {
        let mut v: Vec<(&str, &Entry, f64)> =
            self.entries.iter().map(|(k, e)| (k.as_str(), e, frecency(e, now))).collect();
        v.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(b.0)));
        v.into_iter().map(|(k, e, _)| (k, e)).collect()
    }

    /// Whether there are changes since the last call.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;

    #[test]
    fn keys_round_trip() {
        let k = key(Source::App, r"C:\a:b.exe");
        assert_eq!(k, r"a:C:\a:b.exe");
        assert_eq!(split_key(&k), Some((Source::App, r"C:\a:b.exe")));
        assert_eq!(split_key("x:1"), None);
        assert_eq!(split_key("ab:1"), None);
    }

    #[test]
    fn tsv_round_trip() {
        let mut h = History::default();
        h.record("a:chrome", "Google\tChrome", 10);
        h.record("a:chrome", "Google Chrome", 20);
        h.record("f:C:\\x.txt", "x.txt", 30);
        assert!(h.take_dirty());
        assert!(!h.take_dirty());
        let text = h.serialize();
        let back = History::parse(&format!("{text}garbage\nbad\tline\tz:q\tn\n"));
        assert_eq!(back.serialize(), text);
        assert_eq!(back.entries["a:chrome"], Entry { count: 2, last: 20, name: "Google Chrome".into() });
    }

    #[test]
    fn boost_grows_with_use_and_decays() {
        let mut h = History::default();
        assert_eq!(h.boost("a:x", 0), 1.0);
        h.record("a:x", "x", 0);
        let once = h.boost("a:x", 0);
        h.record("a:x", "x", 0);
        assert!(h.boost("a:x", 0) > once && once > 1.0);
        assert!(h.boost("a:x", 90 * DAY) < h.boost("a:x", 0));
        assert!(h.boost("a:x", 3650 * DAY) > 1.0);
    }

    #[test]
    fn top_and_cap() {
        let mut h = History::default();
        for i in 0..MAX_ENTRIES + 10 {
            h.record(&format!("c:{i}"), "n", i as u64 * DAY);
        }
        h.record("c:hot", "hot", 600 * DAY);
        h.record("c:hot", "hot", 600 * DAY);
        assert_eq!(h.entries.len(), MAX_ENTRIES);
        assert!(!h.entries.contains_key("c:0"));
        assert_eq!(h.top(600 * DAY)[0].0, "c:hot");
    }
}
