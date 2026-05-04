//! Script-aware shaping helpers layered on top of the cmap → GSUB →
//! GPOS pipeline in [`crate::shaper`].
//!
//! - Round 7: [`arabic`] — Unicode joining-class lookup + the
//!   adjacency state machine that picks `Isol` / `Init` / `Medi` /
//!   `Fina` for every character in a run, plus [`arabic_pf`] for the
//!   Arabic Presentation Forms-B translation table.
//! - Round 8: [`indic`] — Devanagari complex-script cluster machine
//!   (cluster boundaries + pre-base matra reorder + reph
//!   identification). Other Indic scripts (Bengali, Tamil, etc.) follow
//!   the same broad pattern with distinct categories and feature lists
//!   and will land in future rounds.

pub mod arabic;
pub mod arabic_pf;
pub mod indic;

pub use arabic::{
    compute_forms, feature_tags_for_run, joining_class, script_of, JoiningClass, JoiningForm,
    Script,
};
pub use arabic_pf::presentation_form;
pub use indic::{
    cluster_boundaries, devanagari_category, devanagari_feature_tags, reorder_cluster,
    ClusterFlags, IndicCategory,
};
