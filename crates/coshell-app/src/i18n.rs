//! UI language. Korean source strings are the keys: `tr("서버")` returns the
//! English text when the language is English, the string itself otherwise.
//! Templates use numbered placeholders (`{0}`, `{1}`) so word order can differ.

use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};

mod en;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Ko,
    En,
}

static CURRENT: AtomicU8 = AtomicU8::new(Lang::Ko as u8);

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
    // The other crates word their own errors and prompts.
    coshell_ssh::set_english(lang == Lang::En);
    coshell_ai::set_english(lang == Lang::En);
}

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == Lang::En as u8 { Lang::En } else { Lang::Ko }
}

/// Language for a system locale such as `ko-KR` or `en_US.UTF-8`.
pub fn from_locale(locale: Option<&str>) -> Lang {
    match locale {
        Some(l) if l.to_ascii_lowercase().starts_with("ko") => Lang::Ko,
        _ => Lang::En,
    }
}

/// The language a setting stands for.
pub fn resolve(language: crate::settings::Language) -> Lang {
    use crate::settings::Language;
    match language {
        Language::Ko => Lang::Ko,
        Language::En => Lang::En,
        Language::Auto => from_locale(sys_locale::get_locale().as_deref()),
    }
}

fn translate(s: &str, lang: Lang) -> &str {
    match lang {
        Lang::Ko => s,
        // Untranslated strings stay Korean rather than disappear.
        Lang::En => en::en(s).unwrap_or(s),
    }
}

/// `s` in the current language.
pub fn tr(s: &str) -> &str {
    translate(s, current())
}

fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = template.to_string();
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), &a.to_string());
    }
    out
}

/// Template `tpl` in the current language with `{0}`, `{1}`, … filled in.
pub fn tf(tpl: &str, args: &[&dyn Display]) -> String {
    fill(tr(tpl), args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn korean_is_the_source_and_english_is_looked_up() {
        assert_eq!(translate("서버", Lang::Ko), "서버");
        assert_eq!(translate("서버", Lang::En), "Servers");
        assert_eq!(translate("번역 없는 문자열", Lang::En), "번역 없는 문자열");
    }

    #[test]
    fn templates_can_reorder_arguments() {
        assert_eq!(fill("{1} then {0}{0}", &[&"a", &2]), "2 then aa");
        assert_eq!(fill(translate("{0}에 연결 중", Lang::En), &[&"web"]), "Connecting to web");
    }

    /// Korean string literals in the app's non-test code (the translation keys).
    fn source_strings() -> Vec<(String, String)> {
        fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    rust_files(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let literal = regex::Regex::new(r#""((?:[^"\\]|\\.)*)""#).unwrap();
        let hangul = regex::Regex::new("[가-힣]").unwrap();
        let mut files = Vec::new();
        rust_files(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        let mut found = Vec::new();
        for path in files {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name == "tests.rs" || name == "en.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let code = text.split("\n#[cfg(test)]\nmod tests {").next().unwrap();
            for line in code.lines().filter(|l| !l.trim_start().starts_with("//")) {
                for m in literal.captures_iter(line) {
                    if hangul.is_match(&m[1]) {
                        // Compare as the compiler sees it.
                        found.push((name.clone(), m[1].replace("\\\"", "\"").replace("\\n", "\n")));
                    }
                }
            }
        }
        found
    }

    /// Strings that are not shown translated: a language's own name, and search keywords.
    const NOT_TRANSLATED: [&str; 6] = [
        "한국어",
        "새 로컬 셸 new local shell",
        "새 서버 추가 new server host",
        "서버 관리 servers",
        "설정 환경설정 settings preferences",
        "coshell 정보 버전 about version",
    ];

    #[test]
    fn every_source_string_has_english_with_the_same_placeholders() {
        let placeholders = |s: &str| {
            let mut p: Vec<String> = regex::Regex::new(r"\{\d\}").unwrap().find_iter(s).map(|m| m.as_str().to_string()).collect();
            p.sort();
            p.dedup();
            p
        };
        let strings = source_strings();
        assert!(strings.len() > 300, "scan found only {} strings", strings.len());
        let mut problems = Vec::new();
        for (file, s) in strings {
            if NOT_TRANSLATED.contains(&s.as_str()) {
                continue;
            }
            match en::en(&s) {
                None => problems.push(format!("{file}: no English for {s:?}")),
                Some(e) if placeholders(e) != placeholders(&s) => problems.push(format!("{file}: placeholders differ in {s:?}")),
                Some(_) => {}
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn locale_picks_the_language() {
        assert_eq!(from_locale(Some("ko-KR")), Lang::Ko);
        assert_eq!(from_locale(Some("KO_kr.UTF-8")), Lang::Ko);
        assert_eq!(from_locale(Some("en-US")), Lang::En);
        assert_eq!(from_locale(Some("ja-JP")), Lang::En);
        assert_eq!(from_locale(None), Lang::En);
    }
}
