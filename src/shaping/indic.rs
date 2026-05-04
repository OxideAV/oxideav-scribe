//! Indic Devanagari complex-script shaping (round 8).
//!
//! Devanagari (used by Hindi, Marathi, Sanskrit, Nepali, etc.) is the
//! first Indic script we support. Unlike Arabic — which is purely
//! contextual joining over a left-to-right glyph stream — Indic
//! shaping is **cluster-based**: input characters are grouped into
//! orthographic syllables, then re-ordered + re-shaped within each
//! cluster according to script-specific rules.
//!
//! Round-8 covers two cluster transformations that work without
//! feature-tagged GSUB lookups (i.e. without the producer-side font
//! support that lookup type 1 single-substitution provides — see the
//! `oxideav-ttf` followup #430a).
//!
//! **Pre-base matra reordering** — the vowel sign U+093F (Devanagari
//! "i") is encoded *after* its base consonant in logical order, but
//! must render *before* it visually. We swap it back to its visual
//! position within the cluster so simple cmap-only fonts still draw
//! the cluster in the right order.
//!
//! **Reph identification** — when a cluster begins with the sequence
//! RA (U+0930) followed by halant (U+094D) followed by a base
//! consonant, the RA forms a "reph" — a superscript mark that's
//! positioned over the END of the cluster. Round 8 *identifies* the
//! reph via [`ClusterFlags::has_reph`] but does not yet emit a
//! different glyph for it (that requires the `rphf` GSUB feature,
//! deferred to a later round once `oxideav-ttf` exposes
//! feature-tagged single substitution).
//!
//! ## References
//!
//! - Unicode 15.1 Standard Annex #15 (Indic syllabic categories).
//! - Unicode 15.1 Standard Annex #29 (text segmentation; grapheme
//!   cluster baseline).
//! - Microsoft OpenType Layout — *Creating and supporting OpenType
//!   fonts for the Devanagari script* (the canonical description of
//!   the cluster reorder rules + GSUB feature application order).
//!
//! No HarfBuzz / FreeType / pango / ICU layout source consulted. The
//! algorithm is a clean-room implementation derived from the Unicode +
//! OpenType specs above plus the `DevanagariShaping` informative
//! examples in the OpenType layout doc.
//!
//! ## Cluster boundaries
//!
//! A Devanagari cluster is, informally, "one base consonant + its
//! halant chains + its matra + its modifiers". Concretely the round-8
//! boundary detector starts a new cluster when it sees a base-class
//! character (consonant, independent vowel, or any non-Devanagari
//! codepoint) **unless** the previous character is a halant — a halant
//! glues the next consonant into the same cluster (forming a conjunct).
//! Vowel signs, nuktas, and halants always extend the current cluster.
//!
//! This is conservative compared to a full Indic2 cluster machine
//! (which also handles dotted-circle insertion + ZWJ/ZWNJ overrides)
//! but covers the visually-correct cases we care about for round 8.

#![allow(clippy::manual_range_contains)]

/// Devanagari syllabic category. Names are short for readability;
/// see the per-variant docs for the full Unicode classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicCategory {
    /// Independent consonant (base) — U+0915..U+0939, U+0958..U+095F,
    /// U+0978..U+097F. Drives cluster start / base selection.
    Consonant,
    /// Independent vowel — U+0904..U+0914, U+0960..U+0961. Acts as
    /// a base for the cluster but does not chain via halant.
    Vowel,
    /// Halant / virama — U+094D. Suppresses the inherent vowel of the
    /// preceding consonant; when followed by another consonant it
    /// forms a conjunct (both stay in the same cluster).
    Halant,
    /// Pre-base reordering matra — U+093F (Devanagari sign "i"). In
    /// logical order it appears AFTER the base consonant but renders
    /// VISUALLY BEFORE it. The reorderer in [`reorder_cluster`] swaps
    /// it to the front of the cluster.
    PreBaseMatra,
    /// Vowel sign / matra (other than pre-base) — U+093A, U+093B,
    /// U+0940..U+094C, U+094E..U+094F, U+0955..U+0957, U+0962..U+0963.
    /// Stays in its logical position within the cluster.
    Matra,
    /// Nukta — U+093C. Combining dot-below; binds tightly to the
    /// preceding consonant (forms a "nukta'd" consonant).
    Nukta,
    /// Anusvara / candrabindu / visarga — U+0900..U+0903. Bindu marks
    /// that attach to the cluster end.
    Bindu,
    /// Devanagari sign avagraha + danda + double-danda + the digit
    /// block + miscellaneous symbols. Treated as cluster-breaking
    /// (each is its own cluster).
    Symbol,
    /// Anything outside U+0900..U+097F. Treated as a cluster boundary
    /// — a Devanagari cluster never crosses the script boundary.
    Other,
}

/// Look up the Indic category for `ch` within the Devanagari block.
/// Codepoints outside U+0900..U+097F return [`IndicCategory::Other`].
///
/// The classification follows the Unicode `IndicSyllabicCategory.txt`
/// and `IndicPositionalCategory.txt` properties — but condensed to the
/// six categories the round-8 cluster machine actually distinguishes
/// (so e.g. "Consonant_Dead", "Consonant", "Consonant_Subjoined"
/// all collapse to `Consonant`).
pub fn devanagari_category(ch: char) -> IndicCategory {
    let cp = ch as u32;
    // Fast-path: outside the Devanagari block entirely.
    if cp < 0x0900 || cp > 0x097F {
        return IndicCategory::Other;
    }
    match cp {
        // Bindu marks (anusvara / candrabindu / visarga / inverted
        // candrabindu).
        0x0900..=0x0903 => IndicCategory::Bindu,
        // Independent vowels (A through AU).
        0x0904..=0x0914 => IndicCategory::Vowel,
        // Consonants KA..HA + extended consonants (NNNA, RRA, etc.).
        0x0915..=0x0939 => IndicCategory::Consonant,
        // Nukta — combining dot below.
        0x093C => IndicCategory::Nukta,
        // Avagraha (sign) — symbol; treated as cluster break.
        0x093D => IndicCategory::Symbol,
        // Vowel signs (matras) other than pre-base "i". The matras
        // U+093A and U+093B sit visually above the base; U+0940..U+094C
        // are the standard post-base / above / below matras; U+094E
        // is "PRISHTHAMATRA E" (rendered before the base in some
        // sequences but not pre-base reordered like U+093F);
        // U+094F is "AW".
        0x093A | 0x093B => IndicCategory::Matra,
        // Post-base matra AA (U+093E) — the most common matra in
        // running Hindi text. Sits between the nukta (U+093C, classed
        // above) and the pre-base matra "i" (U+093F).
        0x093E => IndicCategory::Matra,
        // Pre-base matra "i" — the only matra that needs reordering
        // in modern Devanagari.
        0x093F => IndicCategory::PreBaseMatra,
        // The post-base, above-base, and below-base matras.
        0x0940..=0x094C => IndicCategory::Matra,
        // Halant / virama.
        0x094D => IndicCategory::Halant,
        0x094E..=0x094F => IndicCategory::Matra,
        // Stress signs / udatta + anudatta + grave + acute + Vedic
        // marks. Treated as cluster-extending bindi-style modifiers.
        0x0951..=0x0954 => IndicCategory::Bindu,
        // Vowel signs UE / UUE / SHORT_E (Marathi extensions).
        0x0955..=0x0957 => IndicCategory::Matra,
        // Additional nukta'd consonants (QA..YYA).
        0x0958..=0x095F => IndicCategory::Consonant,
        // Vocalic L + LL (independent vowels).
        0x0960..=0x0961 => IndicCategory::Vowel,
        // Vowel signs vocalic L + LL.
        0x0962..=0x0963 => IndicCategory::Matra,
        // Danda + double-danda + abbreviation sign + Devanagari ".".
        // All cluster-breaking symbols.
        0x0964..=0x096F => IndicCategory::Symbol, // includes digits 0966..096F
        0x0970..=0x0977 => IndicCategory::Symbol,
        // Extended consonants (Sindhi / Marathi etc.).
        0x0978..=0x097F => IndicCategory::Consonant,
        // Already enumerated everything in the block — exhaustive
        // match for clarity. Anything we missed defaults to Symbol
        // (cluster-breaking) which is the conservative choice.
        _ => IndicCategory::Symbol,
    }
}

/// Per-cluster shaping flags computed by [`reorder_cluster`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClusterFlags {
    /// True when the cluster started with `RA + halant + consonant`
    /// — the RA at position 0 should ultimately render as a "reph"
    /// superscript mark over the cluster end. Round 8 only flags it;
    /// the actual glyph substitution is gated on the `rphf` GSUB
    /// feature which lands once `oxideav-ttf` exposes feature-tagged
    /// single substitution.
    pub has_reph: bool,
    /// True when the cluster contained a pre-base matra (U+093F)
    /// that was moved to the front of the cluster.
    pub pre_base_reordered: bool,
}

/// Walk `chars` and emit `(cluster_start, cluster_end_exclusive)` byte
/// indices into `chars` for every Devanagari cluster. Non-Devanagari
/// characters become single-character clusters whose category is
/// [`IndicCategory::Other`].
///
/// A cluster boundary starts a new cluster when:
/// - the current character is `Other` (non-Devanagari);
/// - the current character is `Consonant` or `Vowel` AND the previous
///   character is NOT `Halant` (a halant glues the next consonant
///   into the same cluster, forming a conjunct);
/// - the previous character was `Symbol` (danda etc. always end a
///   cluster).
///
/// Otherwise the current character extends the cluster.
pub fn cluster_boundaries(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    if chars.is_empty() {
        return out;
    }
    let n = chars.len();
    let mut start = 0usize;
    for i in 1..n {
        let prev = devanagari_category(chars[i - 1]);
        let cur = devanagari_category(chars[i]);
        let boundary = matches!(cur, IndicCategory::Other | IndicCategory::Symbol)
            || matches!(prev, IndicCategory::Other | IndicCategory::Symbol)
            || (matches!(cur, IndicCategory::Consonant | IndicCategory::Vowel)
                && !matches!(prev, IndicCategory::Halant));
        if boundary {
            out.push((start, i));
            start = i;
        }
    }
    out.push((start, n));
    out
}

/// Apply Devanagari cluster reordering to a single cluster.
///
/// Returns the reordered character slice plus [`ClusterFlags`]
/// describing what was done.
///
/// Round-8 scope:
/// - **Pre-base matra (U+093F)** — if the cluster contains a pre-base
///   matra anywhere after the first consonant, move it to position 0
///   (immediately before the base consonant). This is the visual
///   position the matra occupies in rendered output.
/// - **Reph detection** — if the cluster begins with `RA + halant +
///   consonant` (any consonant), set [`ClusterFlags::has_reph`]. The
///   characters themselves are NOT removed yet (the substitution to
///   the reph glyph requires `rphf` GSUB; for now the cmap path will
///   render RA + halant + base which is visually wrong but
///   syllable-correct — once `oxideav-ttf` lookup type 1 lands we'll
///   come back and rewrite RA → reph + drop the halant).
pub fn reorder_cluster(cluster: &[char]) -> (Vec<char>, ClusterFlags) {
    let mut flags = ClusterFlags::default();
    if cluster.is_empty() {
        return (Vec::new(), flags);
    }
    let mut out: Vec<char> = cluster.to_vec();

    // Pre-base matra reorder. Find the FIRST pre-base matra in the
    // cluster (there's normally at most one) and move it to position
    // 0. The base consonant is whichever consonant precedes the matra
    // — for round 8 we just put the matra at the very front of the
    // cluster, which is visually correct for the canonical
    // single-consonant case (e.g. "कि" = KA + sign-i → sign-i + KA).
    // Multi-consonant conjuncts (e.g. "क्षि" = KA + halant + SSA +
    // sign-i) also reorder the matra to front; the conjunct itself
    // stays intact.
    if let Some(matra_idx) = out
        .iter()
        .position(|&c| devanagari_category(c) == IndicCategory::PreBaseMatra)
    {
        if matra_idx > 0 {
            let matra = out.remove(matra_idx);
            out.insert(0, matra);
            flags.pre_base_reordered = true;
        }
    }

    // Reph detection — the cluster (post-reorder) might now have the
    // pre-base matra at position 0; we need to look at the original
    // cluster's start to detect RA + halant + consonant. Use the
    // input slice directly so a leading matra doesn't mask the reph.
    if cluster.len() >= 3
        && cluster[0] == '\u{0930}' // DEVANAGARI LETTER RA
        && devanagari_category(cluster[1]) == IndicCategory::Halant
        && devanagari_category(cluster[2]) == IndicCategory::Consonant
    {
        flags.has_reph = true;
    }

    (out, flags)
}

/// Devanagari OpenType GSUB feature tags, in the spec-mandated
/// application order. The first 8 tags (`locl`..`cjct`) are
/// "substitution" features that reshape clusters into conjuncts and
/// half-forms; the last 6 (`init`..`haln`) are "presentation"
/// features that pick contextual variants.
///
/// Round 8 returns the tag list but does NOT yet resolve the
/// substitutions — that requires `oxideav-ttf` to expose feature-tagged
/// GSUB lookup type 1 (followup #430a). The shaper exposes the tags
/// so future rounds can iterate on the same vector once the lookup
/// path is in place.
pub fn devanagari_feature_tags() -> Vec<[u8; 4]> {
    vec![
        *b"locl", // language-form substitutions
        *b"ccmp", // glyph composition / decomposition
        *b"nukt", // nukta forms
        *b"akhn", // akhand ligatures (e.g. ksha, jnya)
        *b"rphf", // reph form (RA + halant → superscript)
        *b"blwf", // below-base forms
        *b"half", // half forms (consonant + halant in non-final position)
        *b"vatu", // vattu variants
        *b"cjct", // conjunct forms
        *b"init", // initial contextual variants
        *b"pres", // pre-base substitutions
        *b"abvs", // above-base substitutions
        *b"blws", // below-base substitutions
        *b"psts", // post-base substitutions
        *b"haln", // halant forms
    ]
}

#[cfg(test)]
#[allow(non_snake_case)] // tests reference Unicode codepoint literals
mod tests {
    use super::*;

    #[test]
    fn devanagari_category_lookup_returns_consonant_for_ka_U_0915() {
        // U+0915 DEVANAGARI LETTER KA — the canonical first consonant.
        assert_eq!(devanagari_category('\u{0915}'), IndicCategory::Consonant);
    }

    #[test]
    fn devanagari_category_lookup_returns_halant_for_U_094D() {
        // U+094D DEVANAGARI SIGN VIRAMA (halant).
        assert_eq!(devanagari_category('\u{094D}'), IndicCategory::Halant);
    }

    #[test]
    fn devanagari_category_lookup_returns_pre_base_matra_for_U_093F() {
        // U+093F DEVANAGARI VOWEL SIGN I — the pre-base reordering matra.
        assert_eq!(devanagari_category('\u{093F}'), IndicCategory::PreBaseMatra);
    }

    #[test]
    fn devanagari_category_classifies_vowel_a_as_vowel() {
        // U+0905 DEVANAGARI LETTER A.
        assert_eq!(devanagari_category('\u{0905}'), IndicCategory::Vowel);
    }

    #[test]
    fn devanagari_category_classifies_anusvara_as_bindu() {
        // U+0902 DEVANAGARI SIGN ANUSVARA.
        assert_eq!(devanagari_category('\u{0902}'), IndicCategory::Bindu);
    }

    #[test]
    fn devanagari_category_classifies_nukta_as_nukta() {
        // U+093C DEVANAGARI SIGN NUKTA.
        assert_eq!(devanagari_category('\u{093C}'), IndicCategory::Nukta);
    }

    #[test]
    fn devanagari_category_classifies_post_base_matra_aa_as_matra() {
        // U+093E DEVANAGARI VOWEL SIGN AA.
        assert_eq!(devanagari_category('\u{093E}'), IndicCategory::Matra);
    }

    #[test]
    fn devanagari_category_classifies_danda_as_symbol() {
        // U+0964 DEVANAGARI DANDA.
        assert_eq!(devanagari_category('\u{0964}'), IndicCategory::Symbol);
    }

    #[test]
    fn devanagari_category_returns_other_for_latin_a() {
        assert_eq!(devanagari_category('A'), IndicCategory::Other);
    }

    #[test]
    fn script_of_recognises_devanagari_block() {
        use super::super::arabic::{script_of, Script};
        assert_eq!(script_of('\u{0915}'), Script::Devanagari);
        assert_eq!(script_of('\u{094D}'), Script::Devanagari);
        assert_eq!(script_of('\u{097F}'), Script::Devanagari);
    }

    #[test]
    fn script_of_still_classifies_arabic_and_latin_correctly() {
        use super::super::arabic::{script_of, Script};
        // Adding Devanagari must not regress Arabic / Latin classification.
        assert_eq!(script_of('\u{0627}'), Script::Arabic);
        assert_eq!(script_of('A'), Script::Other);
    }

    #[test]
    fn pre_base_matra_reorders_before_base_consonant() {
        // "कि" = KA U+0915 + sign-i U+093F. Logical order is KA then i;
        // visual order must be i then KA (the matra is rendered to the
        // LEFT of the base in Devanagari).
        let cluster = ['\u{0915}', '\u{093F}'];
        let (out, flags) = reorder_cluster(&cluster);
        assert_eq!(out, vec!['\u{093F}', '\u{0915}']);
        assert!(flags.pre_base_reordered);
        assert!(!flags.has_reph);
    }

    #[test]
    fn pre_base_matra_reorders_in_conjunct_cluster() {
        // "क्षि" = KA U+0915 + halant U+094D + SSA U+0937 + sign-i U+093F.
        // Conjunct stays intact; matra moves to front.
        let cluster = ['\u{0915}', '\u{094D}', '\u{0937}', '\u{093F}'];
        let (out, flags) = reorder_cluster(&cluster);
        assert_eq!(out, vec!['\u{093F}', '\u{0915}', '\u{094D}', '\u{0937}']);
        assert!(flags.pre_base_reordered);
    }

    #[test]
    fn reph_formation_at_cluster_start_marks_RA_for_superscript() {
        // RA + halant + KA = "र्क" — the leading RA forms a reph that
        // visually sits over the KA. Round 8 only flags it.
        let cluster = ['\u{0930}', '\u{094D}', '\u{0915}'];
        let (out, flags) = reorder_cluster(&cluster);
        // No pre-base matra to reorder, so the cluster itself is unchanged.
        assert_eq!(out, vec!['\u{0930}', '\u{094D}', '\u{0915}']);
        assert!(flags.has_reph);
        assert!(!flags.pre_base_reordered);
    }

    #[test]
    fn reph_with_pre_base_matra_combines_both_flags() {
        // RA + halant + KA + sign-i — both reph AND pre-base matra
        // reorder fire. The matra moves to front; the reph stays at
        // the cluster's logical start (which is now position 1 in the
        // output, after the moved matra).
        let cluster = ['\u{0930}', '\u{094D}', '\u{0915}', '\u{093F}'];
        let (out, flags) = reorder_cluster(&cluster);
        assert_eq!(out, vec!['\u{093F}', '\u{0930}', '\u{094D}', '\u{0915}']);
        assert!(flags.has_reph);
        assert!(flags.pre_base_reordered);
    }

    #[test]
    fn cluster_without_reph_consonant_does_not_set_flag() {
        // KA + halant + SSA = क्ष — conjunct, but RA is not the leading
        // consonant so no reph.
        let cluster = ['\u{0915}', '\u{094D}', '\u{0937}'];
        let (_out, flags) = reorder_cluster(&cluster);
        assert!(!flags.has_reph);
    }

    #[test]
    fn cluster_boundary_starts_new_cluster_at_consonant_after_vowel() {
        // KA + sign-aa + KA → two clusters: ["KA", "sign-aa"] then
        // ["KA"]. The post-base matra extends the first cluster; the
        // second KA starts a new one because the previous char is NOT
        // a halant.
        let chars = ['\u{0915}', '\u{093E}', '\u{0915}'];
        let bounds = cluster_boundaries(&chars);
        assert_eq!(bounds, vec![(0, 2), (2, 3)]);
    }

    #[test]
    fn cluster_boundary_keeps_conjunct_in_one_cluster() {
        // KA + halant + SSA → single cluster (the halant glues SSA
        // into the same cluster as KA).
        let chars = ['\u{0915}', '\u{094D}', '\u{0937}'];
        let bounds = cluster_boundaries(&chars);
        assert_eq!(bounds, vec![(0, 3)]);
    }

    #[test]
    fn cluster_boundary_breaks_at_danda_symbol() {
        // KA + danda + KA → three single-char clusters because the
        // danda is a Symbol (always its own cluster) and the second
        // KA starts a fresh one.
        let chars = ['\u{0915}', '\u{0964}', '\u{0915}'];
        let bounds = cluster_boundaries(&chars);
        assert_eq!(bounds, vec![(0, 1), (1, 2), (2, 3)]);
    }

    #[test]
    fn cluster_boundary_breaks_at_non_indic_codepoint() {
        // KA + space + KA → three clusters: [KA] [space] [KA]. The
        // space is `Other` so it forms its own boundary on both sides.
        let chars = ['\u{0915}', ' ', '\u{0915}'];
        let bounds = cluster_boundaries(&chars);
        assert_eq!(bounds, vec![(0, 1), (1, 2), (2, 3)]);
    }

    #[test]
    fn cluster_boundary_handles_empty_input() {
        let bounds = cluster_boundaries(&[]);
        assert!(bounds.is_empty());
    }

    #[test]
    fn cluster_boundary_single_consonant_is_one_cluster() {
        let chars = ['\u{0915}'];
        let bounds = cluster_boundaries(&chars);
        assert_eq!(bounds, vec![(0, 1)]);
    }

    #[test]
    fn devanagari_feature_tags_are_in_canonical_order() {
        let tags = devanagari_feature_tags();
        // First three tags match the Devanagari OpenType layout spec
        // application order.
        assert_eq!(&tags[0], b"locl");
        assert_eq!(&tags[1], b"ccmp");
        assert_eq!(&tags[2], b"nukt");
        // rphf comes after akhn.
        assert_eq!(&tags[3], b"akhn");
        assert_eq!(&tags[4], b"rphf");
        // Final presentation features end with haln.
        assert_eq!(tags.last(), Some(b"haln"));
    }

    #[test]
    fn empty_cluster_reorder_returns_empty() {
        let (out, flags) = reorder_cluster(&[]);
        assert!(out.is_empty());
        assert_eq!(flags, ClusterFlags::default());
    }

    #[test]
    fn single_consonant_cluster_does_not_reorder() {
        let cluster = ['\u{0915}'];
        let (out, flags) = reorder_cluster(&cluster);
        assert_eq!(out, vec!['\u{0915}']);
        assert!(!flags.pre_base_reordered);
        assert!(!flags.has_reph);
    }

    #[test]
    fn two_clusters_with_pre_base_matras_each_reorder_independently() {
        // "किकि" → two clusters ["KA", "i"], ["KA", "i"]; each must
        // reorder its matra to the front independently.
        let chars = ['\u{0915}', '\u{093F}', '\u{0915}', '\u{093F}'];
        let bounds = cluster_boundaries(&chars);
        assert_eq!(bounds, vec![(0, 2), (2, 4)]);
        for (s, e) in bounds {
            let (out, flags) = reorder_cluster(&chars[s..e]);
            assert_eq!(out, vec!['\u{093F}', '\u{0915}']);
            assert!(flags.pre_base_reordered);
        }
    }
}
