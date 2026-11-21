//! Repairs Hangul that macOS's Korean input method splits apart.
//!
//! Right after switching to Korean (or focusing a field), the system IME hands the first
//! key over as a bare letter (`ㅎ`) instead of starting a syllable, and then composes from
//! the following vowel on its own: `한` arrives as `ㅎ`, `ㅏ`, `ㄴ`. Apple's own reports call
//! it a system quirk (Qt and others see it too). [`join_split`] puts such pieces back
//! together as they are typed.

/// Initial consonants in syllable order.
const INITIAL: [char; 19] = ['ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ'];
/// Final consonants in syllable order (index 0 is "none").
const FINAL: [char; 28] = [
    '\0', 'ㄱ', 'ㄲ', 'ㄳ', 'ㄴ', 'ㄵ', 'ㄶ', 'ㄷ', 'ㄹ', 'ㄺ', 'ㄻ', 'ㄼ', 'ㄽ', 'ㄾ', 'ㄿ', 'ㅀ', 'ㅁ', 'ㅂ', 'ㅄ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
];
const SYLLABLE_FIRST: u32 = 0xAC00;
const SYLLABLE_LAST: u32 = 0xD7A3;
const VOWELS: std::ops::RangeInclusive<char> = 'ㅏ'..='ㅣ';

fn vowel(c: char) -> Option<u32> {
    VOWELS.contains(&c).then(|| c as u32 - 'ㅏ' as u32)
}

fn syllable(initial: usize, vowel: u32, fin: usize) -> char {
    char::from_u32(SYLLABLE_FIRST + (initial as u32 * 21 + vowel) * 28 + fin as u32).unwrap_or('\u{FFFD}')
}

/// A syllable with no final consonant yet.
fn open_syllable(c: char) -> bool {
    (SYLLABLE_FIRST..=SYLLABLE_LAST).contains(&(c as u32)) && (c as u32 - SYLLABLE_FIRST).is_multiple_of(28)
}

/// `new` is `old` with text typed at the end. When that text continues a split syllable,
/// returns the repaired value. `joined` carries whether the last syllable was made here (only
/// such a syllable takes a lone consonant as its final: typed normally, `하ㅋㅋ` stays as is).
pub fn join_split(old: &str, new: &str, joined: &mut bool) -> Option<String> {
    let was_joined = std::mem::take(joined);
    let added = new.strip_prefix(old)?;
    let mut rest = added.chars();
    let first = rest.next()?;
    let last = old.chars().next_back()?;
    let head = &old[..old.len() - last.len_utf8()];
    // A bare initial, then the vowel the IME composed on its own.
    if let (Some(i), Some(v)) = (INITIAL.iter().position(|&c| c == last), vowel(first)) {
        *joined = true;
        return Some(format!("{head}{}{}", syllable(i, v, 0), rest.as_str()));
    }
    // That syllable's final, committed on its own because the next key began a new syllable.
    if was_joined && open_syllable(last) {
        let f = FINAL.iter().skip(1).position(|&c| c == first)? + 1;
        let base = last as u32 - SYLLABLE_FIRST;
        let (i, v) = ((base / 28 / 21) as usize, base / 28 % 21);
        return Some(format!("{head}{}{}", syllable(i, v, f), rest.as_str()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds the pieces as the IME delivers them and returns the field's value.
    fn typed(pieces: &[&str]) -> String {
        let (mut value, mut joined) = (String::from("#"), false);
        for p in pieces {
            let new = format!("{value}{p}");
            value = join_split(&value, &new, &mut joined).unwrap_or(new);
        }
        value
    }

    #[test]
    fn split_first_syllable_is_joined() {
        // What macOS sent for "한국어 " right after switching to Korean.
        assert_eq!(typed(&["ㅎ", "ㅏ", "ㄴ", "국", "어 "]), "#한국어 ");
        assert_eq!(typed(&["ㅅ", "ㅓ", "버"]), "#서버");
        assert_eq!(typed(&["ㄷ", "ㅏ", "ㄺ"]), "#닭");
        assert_eq!(typed(&["ㅎ", "ㅘ"]), "#화");
        assert_eq!(typed(&["ㅎ", "ㅏ", "나"]), "#하나");
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        assert_eq!(typed(&["좋", "아", "ㅋ", "ㅋ"]), "#좋아ㅋㅋ");
        assert_eq!(typed(&["ㅠ", "ㅠ"]), "#ㅠㅠ");
        assert_eq!(typed(&["d", "f", " -h"]), "#df -h");
        let mut joined = true;
        assert_eq!(join_split("#하", "#", &mut joined), None, "deleting is not typing");
        assert!(!joined);
    }
}
