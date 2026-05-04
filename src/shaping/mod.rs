//! Script-aware shaping helpers layered on top of the cmap → GSUB →
//! GPOS pipeline in [`crate::shaper`].
//!
//! Round 7 introduces [`arabic`] — Unicode joining-class lookup + the
//! adjacency state machine that picks `Isol` / `Init` / `Medi` / `Fina`
//! for every character in a run. Future rounds will sit Indic /
//! complex-script modules alongside it.

pub mod arabic;
pub mod arabic_pf;

pub use arabic::{
    compute_forms, feature_tags_for_run, joining_class, script_of, JoiningClass, JoiningForm,
    Script,
};
pub use arabic_pf::presentation_form;
