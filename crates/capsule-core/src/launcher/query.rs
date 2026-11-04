use super::Source;
use crate::config::Prefixes;

/// Splits off the longest matching source prefix.
pub fn split<'a>(query: &'a str, p: &Prefixes) -> (Option<Source>, &'a str) {
    let prefixes = [
        (&p.apps, Source::App),
        (&p.commands, Source::Command),
        (&p.folders, Source::Folder),
        (&p.windows, Source::Window),
    ];
    prefixes
        .into_iter()
        .filter(|(pre, _)| !pre.is_empty() && query.starts_with(pre.as_str()))
        .max_by_key(|(pre, _)| pre.len())
        .map_or((None, query.trim()), |(pre, s)| (Some(s), query[pre.len()..].trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes() {
        let mut p = Prefixes::default();
        assert_eq!(split(" wx ", &p), (None, "wx"));
        assert_eq!(split(">pause", &p), (Some(Source::Command), "pause"));
        assert_eq!(split("/ doc", &p), (Some(Source::Folder), "doc"));
        assert_eq!(split("@", &p), (Some(Source::Window), ""));
        p.apps = "//".into();
        assert_eq!(split("//x", &p), (Some(Source::App), "x"));
        p.windows.clear();
        assert_eq!(split("@x", &p), (None, "@x"));
    }
}
