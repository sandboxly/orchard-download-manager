//! Links in pasted or copied text, and batch templates
//! ("https://x.org/img[001-120].jpg").

/// The http(s) links in `text`, in order, without duplicates;
/// templates expanded.
pub fn find(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`')) {
        let w = word.trim_matches(|c: char| matches!(c, '(' | ')' | ',' | ';' | '.' | '!' | '?'));
        let lower = w.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            continue;
        }
        for link in expand(w) {
            if url::Url::parse(&link).is_ok_and(|u| u.host_str().is_some()) && !out.contains(&link) {
                out.push(link);
            }
        }
    }
    out
}

/// Most links a template may make.
pub const MAX_EXPANSION: usize = 10_000;

/// "a[1-3]b" → a1b a2b a3b; "[01-10]" keeps the width; "[a-e]" letters.
/// A `[` that isn't a range stays as it is.
pub fn expand(template: &str) -> Vec<String> {
    let Some(open) = template.find('[') else { return vec![template.to_string()] };
    let Some(close) = template[open..].find(']').map(|c| open + c) else { return vec![template.to_string()] };
    let (head, inner, tail) = (&template[..open], &template[open + 1..close], &template[close + 1..]);
    let Some(items) = range_items(inner) else {
        return expand(tail).into_iter().map(|t| format!("{head}[{inner}]{t}")).collect();
    };
    let rest = expand(tail);
    let mut out = Vec::new();
    for i in &items {
        for r in &rest {
            if out.len() >= MAX_EXPANSION {
                return out;
            }
            out.push(format!("{head}{i}{r}"));
        }
    }
    out
}

fn range_items(inner: &str) -> Option<Vec<String>> {
    let (a, b) = inner.split_once('-')?;
    if let (Ok(x), Ok(y)) = (a.parse::<u64>(), b.parse::<u64>()) {
        if x > y || y - x >= MAX_EXPANSION as u64 {
            return None;
        }
        let width = if a.starts_with('0') && a.len() > 1 { a.len() } else { 0 };
        return Some((x..=y).map(|n| format!("{n:0width$}")).collect());
    }
    let (mut ca, mut cb) = (a.chars(), b.chars());
    match (ca.next(), ca.next(), cb.next(), cb.next()) {
        (Some(x), None, Some(y), None) if x.is_ascii_alphabetic() && y.is_ascii_alphabetic() && x <= y => {
            Some((x..=y).map(|c| c.to_string()).collect())
        }
        _ => None,
    }
}

/// The links in `text` the clipboard monitor offers: those whose file name
/// has one of the extensions it watches.
pub fn catchable(text: &str, catches: impl Fn(&str) -> bool) -> Vec<String> {
    if text.len() > 100_000 {
        return Vec::new();
    }
    find(text)
        .into_iter()
        .filter(|l| {
            let name = crate::engine::http::url_filename(l);
            std::path::Path::new(&name).extension().is_some_and(|e| catches(&e.to_string_lossy()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links_in_text() {
        let t = "Get it at <https://x.org/a.zip>, or (http://y.org/b.iso). Again: https://x.org/a.zip ftp://no";
        assert_eq!(find(t), ["https://x.org/a.zip", "http://y.org/b.iso"]);
        assert!(find("https://").is_empty());
    }

    #[test]
    fn templates() {
        assert_eq!(expand("x/[1-3].jpg"), ["x/1.jpg", "x/2.jpg", "x/3.jpg"]);
        assert_eq!(expand("x/[08-10].jpg"), ["x/08.jpg", "x/09.jpg", "x/10.jpg"]);
        assert_eq!(expand("[a-b][1-2]"), ["a1", "a2", "b1", "b2"]);
        assert_eq!(expand("x/[id].jpg"), ["x/[id].jpg"]);
        assert_eq!(expand("x/[0-99999999]").len(), 1, "too many: left as it is");
    }

    #[test]
    fn catches_by_extension() {
        let t = "https://x.org/page.html https://x.org/f.ZIP https://x.org/dl?id=3";
        assert_eq!(catchable(t, |e| e.eq_ignore_ascii_case("zip")), ["https://x.org/f.ZIP"]);
    }
}
