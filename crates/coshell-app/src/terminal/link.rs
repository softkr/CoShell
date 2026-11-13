//! Web links in terminal output (⌘+click).

use std::sync::LazyLock;

use regex::Regex;

static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"https?://[^\s<>"'`]+"#).expect("static regex"));

/// Punctuation that usually belongs to the sentence, not the link.
const TRAILING: [char; 9] = ['.', ',', ';', ':', '!', '?', '\'', ']', '}'];

/// The http(s) URL covering byte offset `at` of `line`, if any.
pub fn url_at(line: &str, at: usize) -> Option<&str> {
    URL.find_iter(line).find_map(|m| {
        let mut url = m.as_str().trim_end_matches(TRAILING);
        // "(see https://x.y/z)" — a closing bracket without its opener is not part of the link.
        if url.ends_with(')') && !url.contains('(') {
            url = url.trim_end_matches(')').trim_end_matches(TRAILING);
        }
        (m.start()..m.start() + url.len()).contains(&at).then_some(url)
    })
}

#[cfg(test)]
mod tests {
    use super::url_at;

    #[test]
    fn finds_the_link_under_the_offset() {
        let line = "see https://example.com/a?b=1&c=2, then http://x.test.";
        assert_eq!(url_at(line, 4), Some("https://example.com/a?b=1&c=2"));
        assert_eq!(url_at(line, 20), Some("https://example.com/a?b=1&c=2"));
        assert_eq!(url_at(line, line.len() - 2), Some("http://x.test"));
        assert_eq!(url_at(line, 0), None);
        assert_eq!(url_at(line, 33), None, "trailing comma is not part of the link");
    }

    #[test]
    fn brackets_and_other_schemes() {
        assert_eq!(url_at("(https://example.com/x)", 5), Some("https://example.com/x"));
        assert_eq!(url_at("https://en.wikipedia.org/wiki/A_(b)", 3), Some("https://en.wikipedia.org/wiki/A_(b)"));
        assert_eq!(url_at("file:///etc/passwd javascript:alert(1)", 3), None);
    }
}
