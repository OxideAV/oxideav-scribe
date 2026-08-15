//! Universal (script-agnostic) cluster segmentation and mark handling.
//!
//! The per-script machines in [`crate::shaping::indic`] and
//! [`crate::shaping::arabic`] know their scripts' syllable structure.
//! For every *other* script the shaper still needs a cluster notion —
//! the **combining character sequence** of UAX #24 §5.2: a base
//! character plus its combining marks (General_Category `Mn` / `Mc` /
//! `Me`) and any interleaved join controls (U+200C ZERO WIDTH
//! NON-JOINER / U+200D ZERO WIDTH JOINER, which UAX #24 §2.1 lists as
//! `Script = Inherited`, i.e. cluster-bound to the preceding base).
//!
//! §5.2's rule is blunt and universal: **never break between a
//! combining mark and its base**. This module provides that
//! segmentation ([`universal_cluster_boundaries`]) together with the
//! §5.2 script-resolution refinement for a segmented cluster
//! ([`cluster_script`]: the script of the first non-Inherited,
//! non-Common character, else Common).
//!
//! The primary in-crate consumer is
//! [`crate::FaceChain`]'s font-fallback assignment: a combining mark
//! must be sourced from the **same face as its base** whenever that
//! face covers it, because `GPOS` mark-to-base attachment (and the
//! mark's design-coordinate fit in general) cannot cross faces.
//!
//! Sources: UAX #24 §2.1 / §5.2
//! (`docs/text/unicode-script/uax24-script-extensions.md`); the
//! General_Category values come from the `intl` crate's compiled UCD
//! tables.

use intl::unicode::script::{script, Script};
use intl::unicode::{general_category, GeneralCategory};

/// The role a character plays inside a universal cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterCategory {
    /// A base character — starts a new cluster.
    Base,
    /// A combining mark (`gc` = `Mn`, `Mc`, or `Me`) — extends the
    /// cluster of the preceding base (UAX #24 §5.2).
    Mark,
    /// U+200C ZWNJ / U+200D ZWJ — join controls carried with the
    /// cluster (they are `Script = Inherited` per UAX #24 §2.1 and
    /// modify the joining/ligation behaviour of their neighbours).
    Joiner,
}

/// Classify `c` for universal cluster segmentation.
#[must_use]
pub fn cluster_category(c: char) -> ClusterCategory {
    match c {
        '\u{200C}' | '\u{200D}' => ClusterCategory::Joiner,
        _ => match general_category(c) {
            GeneralCategory::NonspacingMark
            | GeneralCategory::SpacingMark
            | GeneralCategory::EnclosingMark => ClusterCategory::Mark,
            _ => ClusterCategory::Base,
        },
    }
}

/// Segment `chars` into universal clusters (combining character
/// sequences, UAX #24 §5.2): each cluster is one base character plus
/// every following `Mark` / `Joiner` character.
///
/// Returns half-open `(start, end)` char-index spans, in order,
/// tiling `0..chars.len()` completely (same convention as
/// [`crate::shaping::indic::cluster_boundaries`]). A defective leading
/// sequence (text starting with marks/joiners, no base) forms one
/// cluster of its own ending at the first base character. The empty
/// input yields no spans.
#[must_use]
pub fn universal_cluster_boundaries(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    if chars.is_empty() {
        return out;
    }
    let mut start = 0usize;
    for (i, &c) in chars.iter().enumerate().skip(1) {
        if cluster_category(c) == ClusterCategory::Base {
            out.push((start, i));
            start = i;
        }
    }
    out.push((start, chars.len()));
    out
}

/// Resolve the script of one segmented cluster per the UAX #24 §5.2
/// refinement: the `Script` of the first character that is neither
/// `Inherited` nor `Common`; `Common` when no such character exists.
///
/// (`Unknown` is treated like `Common` here — an unassigned base with
/// inherited marks gives the cluster no usable script identity.)
#[must_use]
pub fn cluster_script(cluster: &[char]) -> Script {
    cluster
        .iter()
        .map(|&c| script(c))
        .find(|s| !matches!(s, Script::Inherited | Script::Common | Script::Unknown))
        .unwrap_or(Script::Common)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_one_cluster_per_char() {
        let chars: Vec<char> = "abc".chars().collect();
        assert_eq!(
            universal_cluster_boundaries(&chars),
            vec![(0, 1), (1, 2), (2, 3)]
        );
    }

    #[test]
    fn marks_extend_the_preceding_cluster() {
        // e + COMBINING ACUTE, then x.
        let chars: Vec<char> = "e\u{0301}x".chars().collect();
        assert_eq!(universal_cluster_boundaries(&chars), vec![(0, 2), (2, 3)]);
        // Stacked marks: base + two nonspacing marks.
        let chars: Vec<char> = "o\u{0302}\u{0323}".chars().collect();
        assert_eq!(universal_cluster_boundaries(&chars), vec![(0, 3)]);
    }

    #[test]
    fn spacing_and_enclosing_marks_cluster_too() {
        // Devanagari KA + AA matra (gc = Mc, SpacingMark).
        let chars: Vec<char> = "\u{0915}\u{093E}".chars().collect();
        assert_eq!(universal_cluster_boundaries(&chars), vec![(0, 2)]);
        // a + U+20DD COMBINING ENCLOSING CIRCLE (gc = Me).
        let chars: Vec<char> = "a\u{20DD}".chars().collect();
        assert_eq!(universal_cluster_boundaries(&chars), vec![(0, 2)]);
    }

    #[test]
    fn joiners_stay_with_the_cluster() {
        // base + ZWJ + base: the ZWJ belongs to the first cluster.
        let chars: Vec<char> = "a\u{200D}b".chars().collect();
        assert_eq!(universal_cluster_boundaries(&chars), vec![(0, 2), (2, 3)]);
        assert_eq!(cluster_category('\u{200C}'), ClusterCategory::Joiner);
        assert_eq!(cluster_category('\u{200D}'), ClusterCategory::Joiner);
    }

    #[test]
    fn defective_leading_marks_form_their_own_cluster() {
        let chars: Vec<char> = "\u{0301}\u{0302}a".chars().collect();
        assert_eq!(universal_cluster_boundaries(&chars), vec![(0, 2), (2, 3)]);
    }

    #[test]
    fn empty_input_yields_no_spans() {
        assert!(universal_cluster_boundaries(&[]).is_empty());
    }

    #[test]
    fn spans_tile_the_input() {
        let chars: Vec<char> = "a\u{0301}\u{05D0}\u{05B4} \u{0915}\u{093E}\u{200D}"
            .chars()
            .collect();
        let spans = universal_cluster_boundaries(&chars);
        assert_eq!(spans.first().unwrap().0, 0);
        assert_eq!(spans.last().unwrap().1, chars.len());
        for w in spans.windows(2) {
            assert_eq!(w[0].1, w[1].0, "gap/overlap: {spans:?}");
        }
    }

    #[test]
    fn cluster_script_uses_first_real_script() {
        // Latin base + Inherited mark → Latin.
        let chars: Vec<char> = "e\u{0301}".chars().collect();
        assert_eq!(cluster_script(&chars), Script::Latin);
        // Hebrew point (explicit sc = Hebrew) after a defective start:
        // the mark itself carries the script.
        let chars: Vec<char> = "\u{05B4}".chars().collect();
        assert_eq!(cluster_script(&chars), Script::Hebrew);
        // Pure neutrals → Common.
        let chars: Vec<char> = "1\u{0301}".chars().collect();
        assert_eq!(cluster_script(&chars), Script::Common);
        assert_eq!(cluster_script(&[]), Script::Common);
    }
}
