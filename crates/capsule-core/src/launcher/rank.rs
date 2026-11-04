use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

use super::haystack::Keys;

pub struct Searcher {
    matcher: Matcher,
    pattern: Pattern,
    indices: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Match {
    pub score: u32,
    pub key: usize,
}

impl Default for Searcher {
    fn default() -> Self {
        let mut config = Config::DEFAULT;
        config.prefer_prefix = true;
        Self { matcher: Matcher::new(config), pattern: Pattern::default(), indices: Vec::new() }
    }
}

impl Searcher {
    /// Space-separated words must all match, each fuzzily.
    pub fn set_query(&mut self, query: &str) {
        self.pattern = Pattern::new(query, CaseMatching::Ignore, Normalization::Smart, AtomKind::Fuzzy);
    }

    pub fn is_empty(&self) -> bool {
        self.pattern.atoms.is_empty()
    }

    pub fn score(&mut self, keys: &Keys) -> Option<Match> {
        let mut best: Option<Match> = None;
        for (key, k) in keys.0.iter().enumerate() {
            if let Some(score) = self.pattern.score(k.text.slice(..), &mut self.matcher)
                && best.is_none_or(|b| score > b.score)
            {
                best = Some(Match { score, key });
            }
        }
        best
    }

    /// Name character indices to highlight for `m`.
    pub fn highlight(&mut self, keys: &Keys, m: Match) -> Vec<u32> {
        let key = &keys.0[m.key];
        self.indices.clear();
        self.pattern.indices(key.text.slice(..), &mut self.matcher, &mut self.indices);
        key.name_indices(&self.indices)
    }
}

pub fn rank(score: u32, weight: f32, boost: f64) -> f64 {
    f64::from(score) * f64::from(weight) * boost
}

/// The `k` best of `(rank, tie-breaker, id)` candidates, best first; equal
/// ranks go to the lower tie-breaker. Returns their ids.
pub fn top(candidates: &mut [(f64, usize, usize)], k: usize) -> Vec<usize> {
    let order = |a: &(f64, usize, usize), b: &(f64, usize, usize)| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1));
    if candidates.len() > k && k > 0 {
        candidates.select_nth_unstable_by(k - 1, order);
    }
    let n = k.min(candidates.len());
    let best = &mut candidates[..n];
    best.sort_unstable_by(order);
    best.iter().map(|c| c.2).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(query: &str, names: &[&str]) -> Vec<String> {
        let keys: Vec<Keys> = names.iter().map(|n| Keys::new(n, &[])).collect();
        let mut s = Searcher::default();
        s.set_query(query);
        let mut c: Vec<(f64, usize, usize)> = keys
            .iter()
            .enumerate()
            .filter_map(|(i, k)| s.score(k).map(|m| (rank(m.score, 1.0, 1.0), names[i].chars().count(), i)))
            .collect();
        top(&mut c, 10).into_iter().map(|i| names[i].to_string()).collect()
    }

    fn matches(query: &str, name: &str) -> bool {
        !search(query, &[name]).is_empty()
    }

    #[test]
    fn pinyin_forms() {
        for q in ["wx", "weixin", "wxin", "weix", "WX", "微信"] {
            assert!(matches(q, "微信"), "{q}");
        }
        assert!(matches("yh", "银行") && matches("yx", "银行"));
        assert!(matches("qqyy", "QQ音乐"));
        assert!(matches("zh", "中"));
        assert!(!matches("xw", "微信"));
    }

    #[test]
    fn highlights_name_chars() {
        let keys = Keys::new("QQ音乐", &[]);
        let mut s = Searcher::default();
        s.set_query("qqyy");
        let m = s.score(&keys).unwrap();
        assert_eq!(s.highlight(&keys, m), [0, 1, 2, 3]);
        s.set_query("yy");
        let m = s.score(&keys).unwrap();
        assert_eq!(s.highlight(&keys, m), [2, 3]);
    }

    #[test]
    fn hidden_keys_match_without_highlight() {
        let keys = Keys::new("计算器", &["calc"]);
        let mut s = Searcher::default();
        s.set_query("calc");
        let m = s.score(&keys).unwrap();
        assert!(s.highlight(&keys, m).is_empty());
        s.set_query("jsq");
        assert!(s.score(&keys).is_some());
    }

    #[test]
    fn word_starts_beat_mid_word() {
        assert_eq!(search("code", &["Unicode Viewer", "Visual Studio Code"])[0], "Visual Studio Code");
        assert_eq!(search("term", &["Determinant", "Terminal"])[0], "Terminal");
        assert_eq!(search("wx", &["Twix", "微信"])[0], "微信");
    }

    #[test]
    fn weight_and_boost_order() {
        assert!(rank(100, 1.0, 1.5) > rank(100, 1.0, 1.0));
        assert!(rank(100, 0.7, 1.0) < rank(100, 1.0, 1.0));
        let mut c = vec![(1.0, 5, 0), (3.0, 5, 1), (3.0, 2, 2), (2.0, 0, 3)];
        assert_eq!(top(&mut c, 3), [2, 1, 3]);
        assert_eq!(top(&mut c, 0), Vec::<usize>::new());
    }
}
