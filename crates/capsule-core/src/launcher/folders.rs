//! Which folder entries get indexed.

use crate::config::FolderRoot;

/// Case-insensitive match with `*` (any run) and `?` (any one character).
pub fn glob(pattern: &str, name: &str) -> bool {
    if pattern.is_ascii() && name.is_ascii() {
        return wildcard(pattern.as_bytes(), name.as_bytes(), b'*', b'?', u8::eq_ignore_ascii_case);
    }
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let n: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    wildcard(&p, &n, '*', '?', char::eq)
}

fn wildcard<T: Copy + PartialEq>(p: &[T], n: &[T], any: T, one: T, eq: impl Fn(&T, &T) -> bool) -> bool {
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && (p[pi] == one || eq(&p[pi], &n[ni])) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == any {
            star = Some((pi, ni));
            pi += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == any)
}

pub fn excluded(root: &FolderRoot, name: &str) -> bool {
    root.exclude.iter().any(|p| glob(p, name))
}

pub fn wants_file(root: &FolderRoot, name: &str) -> bool {
    if root.extensions.is_empty() {
        return true;
    }
    let Some((_, ext)) = name.rsplit_once('.') else { return false };
    root.extensions.iter().any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(ext))
}

/// Expands `%NAME%` references; unknown ones are kept as written.
pub fn expand_env(path: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = path;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match lookup(name) {
                    Some(v) => out.push_str(&v),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob("node_modules", "Node_Modules"));
        assert!(glob("*.tmp", "a.TMP"));
        assert!(glob("~$*", "~$doc.docx"));
        assert!(glob("a?c", "abc"));
        assert!(glob("*", ""));
        assert!(glob("a*b*c", "axxbyyc"));
        assert!(!glob("a*b*c", "axxbyy"));
        assert!(!glob(".git", ".github"));
    }

    #[test]
    fn root_filters() {
        let mut r = FolderRoot::default();
        assert!(excluded(&r, ".git") && !excluded(&r, "src"));
        assert!(wants_file(&r, "Makefile"));
        r.extensions = vec!["pdf".into(), ".DOCX".into()];
        assert!(wants_file(&r, "a.PDF") && wants_file(&r, "b.docx"));
        assert!(!wants_file(&r, "c.txt") && !wants_file(&r, "pdf"));
    }

    #[test]
    fn env_expansion() {
        let env = |n: &str| (n.eq_ignore_ascii_case("userprofile")).then(|| r"C:\Users\me".to_string());
        assert_eq!(expand_env(r"%USERPROFILE%\Documents", env), r"C:\Users\me\Documents");
        assert_eq!(expand_env(r"%NOPE%\x", env), r"%NOPE%\x");
        assert_eq!(expand_env("100%", env), "100%");
        assert_eq!(expand_env("a%%b", env), "a%%b");
    }
}
