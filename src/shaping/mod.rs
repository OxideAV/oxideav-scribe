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

pub mod arabic;
pub mod arabic_pf;
pub mod indic;

pub use arabic::{
    compute_forms, feature_tags_for_run, joining_class, script_of, JoiningClass, JoiningForm,
    Script,
};
pub use arabic_pf::presentation_form;
pub use indic::{
    bengali_category, bengali_feature_tags, cluster_boundaries, cluster_boundaries_with,
    devanagari_category, devanagari_feature_tags, reorder_cluster, reorder_cluster_with,
    script_indic_tags, tamil_category, tamil_feature_tags, ClusterFlags, IndicCategory,
    ReorderRules, BENGALI_RULES, DEVANAGARI_RULES, TAMIL_RULES,
};
