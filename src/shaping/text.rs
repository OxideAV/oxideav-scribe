//! Character-level facts the shaper needs before any font table is
//! consulted: combining classes, canonical (de)composition, default
//! ignorables. All data comes from the `intl` crate's Unicode tables.

use intl::unicode::{canonical_combining_class, general_category, nfc, nfd};

/// Canonical combining class.
#[inline]
pub(crate) fn ccc(c: char) -> u8 {
    if (c as u32) < 0x300 {
        return 0;
    }
    canonical_combining_class(c)
}

/// General category `Mn` / `Mc` / `Me`.
#[inline]
pub(crate) fn is_mark(c: char) -> bool {
    (c as u32) >= 0x300 && general_category(c).is_mark()
}

/// Variation selectors: U+FE00..U+FE0F, U+E0100..U+E01EF and the
/// Mongolian free variation selectors.
#[inline]
pub(crate) fn is_variation_selector(c: char) -> bool {
    matches!(c as u32, 0xFE00..=0xFE0F | 0xE0100..=0xE01EF | 0x180B..=0x180D | 0x180F)
}

/// `Default_Ignorable_Code_Point`: characters that are invisible
/// unless a font deliberately draws them.
#[inline]
pub(crate) fn is_default_ignorable(c: char) -> bool {
    (c as u32) >= 0xAD && intl::unicode::is_default_ignorable(c)
}

/// The primary composite of `a` followed by `b`, if the pair composes
/// canonically (composition exclusions respected).
pub(crate) fn compose_pair(a: char, b: char) -> Option<char> {
    if (a as u32) < 0x80 && (b as u32) < 0x300 {
        return None;
    }
    let mut it = nfc([a, b].into_iter());
    let first = it.next()?;
    if it.next().is_some() {
        return None;
    }
    Some(first)
}

/// Full canonical decomposition of `c` appended to `out`; returns
/// `false` (appending nothing) when `c` does not decompose.
pub(crate) fn decompose(c: char, out: &mut Vec<char>) -> bool {
    if (c as u32) < 0xC0 {
        return false;
    }
    let start = out.len();
    out.extend(nfd(std::iter::once(c)));
    if out.len() == start + 1 && out[start] == c {
        out.truncate(start);
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_compose_and_decompose() {
        assert_eq!(compose_pair('e', '\u{301}'), Some('\u{e9}'));
        assert_eq!(compose_pair('e', 'x'), None);
        assert_eq!(compose_pair('\u{1112}', '\u{1161}'), Some('\u{d558}'));
        assert_eq!(compose_pair('\u{d558}', '\u{11ab}'), Some('\u{d55c}'));
        // Composition exclusion: U+0958 never recomposes.
        assert_eq!(compose_pair('\u{915}', '\u{93c}'), None);
        let mut v = Vec::new();
        assert!(decompose('\u{1ea5}', &mut v));
        assert_eq!(v, ['a', '\u{302}', '\u{301}']);
        v.clear();
        assert!(!decompose('a', &mut v));
        assert!(!decompose('\u{4e00}', &mut v));
        assert!(v.is_empty());
    }

    #[test]
    fn classes() {
        assert_eq!(ccc('a'), 0);
        assert_eq!(ccc('\u{301}'), 230);
        assert_eq!(ccc('\u{323}'), 220);
        assert!(is_mark('\u{301}') && is_mark('\u{93e}') && !is_mark('a'));
        assert!(is_default_ignorable('\u{200d}') && is_default_ignorable('\u{fe0f}'));
        assert!(!is_default_ignorable(' '));
        assert!(is_variation_selector('\u{e0100}') && !is_variation_selector('\u{200d}'));
    }
}
