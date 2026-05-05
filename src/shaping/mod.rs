//! Script-aware shaping helpers layered on top of the cmap → GSUB →
//! GPOS pipeline in [`crate::shaper`].
//!
//! - Round 7: [`arabic`] — Unicode joining-class lookup + the
//!   adjacency state machine that picks `Isol` / `Init` / `Medi` /
//!   `Fina` for every character in a run, plus [`arabic_pf`] for the
//!   Arabic Presentation Forms-B translation table.
//! - Round 8: [`indic`] — Devanagari complex-script cluster machine
//!   (cluster boundaries + pre-base matra reorder + reph
//!   identification).
//! - Round 10: [`indic`] now also covers Bengali (U+0980..U+09FF) and
//!   Tamil (U+0B80..U+0BFF). Bengali shares Devanagari's reph rule and
//!   adds two more pre-base matras (U+09C7 / U+09C8); Tamil's cluster
//!   machine omits reph (no superscript RA) and conjunct formation,
//!   keeping only pre-base matra reorder for U+0BC6 / U+0BC7 / U+0BC8.
//! - Round 12: [`indic`] adds three Brahmic non-Indic scripts —
//!   Sinhala (U+0D80..U+0DFF, halant-driven like Indic), Khmer
//!   (U+1780..U+17FF, coeng-based subjoined consonants via U+17D2),
//!   and Thai (U+0E00..U+0E7F, no halant — pre-base vowels are already
//!   in the visual position). The cluster machine still drives all of
//!   them via [`indic::ReorderRules`] (Sinhala / Khmer reorder the
//!   pre-base matras; Thai is a no-op reorder, only segmentation).
//! - Round 13: [`indic`] adds the remaining two Brahmic non-Indic
//!   scripts — Lao (U+0E80..U+0EFF, structural twin of Thai) and
//!   Myanmar / Burmese (U+1000..U+109F, Asat killer + medials +
//!   kinzi reph-equivalent). The [`indic::RephKind`] enum carries the
//!   kinzi pattern (NGA + Asat + Virama + Cons) as a separate variant
//!   so the cluster reorderer dispatches the right reph detector.

pub mod arabic;
pub mod arabic_pf;
pub mod indic;

pub use arabic::{
    compute_forms, feature_tags_for_run, joining_class, script_of, JoiningClass, JoiningForm,
    Script,
};
pub use arabic_pf::presentation_form;
pub use indic::{
    bengali_category, bengali_feature_tags, burmese_category, burmese_feature_tags,
    cluster_boundaries, cluster_boundaries_with, devanagari_category, devanagari_feature_tags,
    gujarati_category, gujarati_feature_tags, gurmukhi_category, gurmukhi_feature_tags,
    kannada_category, kannada_feature_tags, khmer_category, khmer_feature_tags, lao_category,
    lao_feature_tags, malayalam_category, malayalam_feature_tags, oriya_category,
    oriya_feature_tags, reorder_cluster, reorder_cluster_with, script_indic_tags, sinhala_category,
    sinhala_feature_tags, tamil_category, tamil_feature_tags, telugu_category, telugu_feature_tags,
    thai_category, thai_feature_tags, ClusterFlags, IndicCategory, ReorderRules, RephKind,
    BENGALI_RULES, BURMESE_RULES, DEVANAGARI_RULES, GUJARATI_RULES, GURMUKHI_RULES, KANNADA_RULES,
    KHMER_RULES, LAO_RULES, MALAYALAM_RULES, ORIYA_RULES, SINHALA_RULES, TAMIL_RULES, TELUGU_RULES,
    THAI_RULES,
};
