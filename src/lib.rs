//! Pure-Rust font shaper + layout for the
//! [oxideav](https://github.com/OxideAV) framework.
//!
//! Scribe is a **vector-only shaper**: parse TTF / OTF tables → emit
//! positioned vector glyphs as [`oxideav_core::Node`]s. All pixel
//! work — outline flattening, scanline anti-aliasing, alpha
//! compositing — happens downstream in
//! [`oxideav-raster`](https://github.com/OxideAV/oxideav-raster).
//!
//! Scope:
//! - **Shaper** — `cmap` + GSUB type 4 (ligatures) + GPOS type 2
//!   (pair kerning) + mark-to-base / mark-to-mark, enough for Latin /
//!   Cyrillic / Greek / basic CJK / Vietnamese / polytonic Greek.
//! - **Arabic contextual joining (round 7)** — `shaping::arabic`
//!   computes the joining form per character using the Unicode joining
//!   classes + an adjacency state machine; `FaceChain::shape` then
//!   translates Arabic letters into their Arabic Presentation Forms-B
//!   equivalents (U+FE70..U+FEFF) before cmap, so a font that ships
//!   the PF-B block (DejaVuSans, Noto Sans Arabic, Amiri) renders
//!   visually-correct contextual shapes — including LAM-ALEF
//!   ligatures via the existing GSUB pass.
//! - **Indic + Brahmic complex-script shaping (rounds 8 + 10 + 11 +
//!   12 + 13)** — `shaping::indic` classifies Devanagari
//!   (U+0900..U+097F), Bengali (U+0980..U+09FF), Tamil
//!   (U+0B80..U+0BFF), Gurmukhi (U+0A00..U+0A7F), Gujarati
//!   (U+0A80..U+0AFF), Telugu (U+0C00..U+0C7F), Kannada
//!   (U+0C80..U+0CFF), Malayalam (U+0D00..U+0D7F), Oriya
//!   (U+0B00..U+0B7F), Sinhala (U+0D80..U+0DFF), Khmer
//!   (U+1780..U+17FF), Thai (U+0E00..U+0E7F), Lao (U+0E80..U+0EFF),
//!   and Myanmar / Burmese (U+1000..U+109F) codepoints into syllabic
//!   categories, segments runs into orthographic clusters, and
//!   applies per-script cluster transformations: pre-base matra
//!   reorder (a uniform mechanism across all scripts that have one)
//!   plus reph identification (the Indic core scripts; Tamil +
//!   Malayalam + Sinhala + Khmer + Thai + Lao are reph-disabled;
//!   Burmese identifies a kinzi NGA+Asat+Virama+Cons instead via
//!   `RephKind::BurmeseKinzi`). Khmer's halant role is played by
//!   U+17D2 COENG which stacks subjoined consonants underneath the
//!   base; Thai + Lao have no halant and their pre-base vowels are
//!   already in storage order before their consonant. The
//!   `FaceChain::shape` pipeline applies the reorder before cmap so
//!   cmap-only fonts render simple clusters with the matra in the
//!   correct visual position. When the active face publishes a
//!   `rphf` GSUB lookup for the script, identified reph clusters
//!   get the leading RA glyph substituted to its reph-form and the
//!   halant glyph dropped via `Font::gsub_apply_lookup_type_1`.
//!   Round 11 wires cluster-position GSUB features (`half` / `pref`
//!   / `blwf` / `abvf` / `pstf` / `pres` / `psts` / `abvs` / `blws`).
//!   Round 13 adds a multi-glyph context-aware GSUB pass dispatching
//!   `locl` / `nukt` / `akhn` / `cjct` / `init` / `haln` via
//!   `Font::gsub_apply_lookup_type_5` (Contextual) +
//!   `gsub_apply_lookup_type_6` (Chained Context).
//! - **Variable fonts (rounds 9 + 14)** — `Face::set_variation_coords`
//!   / `variation_axes` / `named_instances` / `is_variable` surface
//!   the font's `fvar` declarations; outlines flow through
//!   `oxideav-ttf`'s gvar interpolator. `Shaper::with_variation_coords`
//!   is the per-call override. Round 14 closes the metric-variation
//!   gap: `Face::mvar()` / `metric_delta(tag)`, `Face::hvar()` /
//!   `h_advance_delta(gid)`, `Face::vvar()` / `v_advance_delta(gid)`,
//!   `Face::stat()` / `stat_axes()` / `stat_axis_values()` (with
//!   `name_id(nid)` resolving the human-readable label).
//!   `Face::cff2()` parses the CFF2 INDEX walker for presence + axis
//!   count + glyph count; full Type 2 v3 charstring evaluation with
//!   the `blend` operator is deferred.
//! - **`Face::glyph_path` / `glyph_node`** — TrueType + OTF (CFF)
//!   outlines as `oxideav_core::Path`; CBDT/sbix colour bitmaps as
//!   `Node::Image` carrying a `VideoFrame`.
//! - **`Shaper::shape_to_paths`** — vector text API: positioned
//!   `(face_idx, Node, Transform2D)` triples ready to compose into a
//!   `VectorFrame`. Each glyph is wrapped in a cache-keyed `Group` so
//!   the downstream rasterizer's bitmap cache reuses the same memoised
//!   glyph across renders.
//! - **Face chain** — multi-face fallback (primary → fallback chain),
//!   per-codepoint resolution.
//! - **Layout** — line measurement + word-wrap (no bidi).
//!
//! See `README.md` for a tour and the deferral list.

#![deny(missing_debug_implementations)]
#![warn(rust_2018_idioms)]

pub mod color;
pub mod color_glyph;
pub mod face;
pub mod face_chain;
pub mod layout;
pub mod shaper;
pub mod shaping;
pub mod style;
pub mod variations;

pub use color::{Rgba, TRANSPARENT, WHITE};
pub use color_glyph::ColorGlyphBitmap;
pub use face::{Face, FaceKind};
pub use face_chain::FaceChain;
pub use layout::{run_width, wrap_lines};
pub use oxideav_ttf::{NamedInstance, VariationAxis};
pub use shaper::{PositionedGlyph, Shaper, ShaperBuilder};
pub use shaping::{
    bengali_category, bengali_feature_tags, burmese_category, burmese_feature_tags,
    cluster_boundaries, cluster_boundaries_with, compute_forms, devanagari_category,
    devanagari_feature_tags, feature_tags_for_run, gujarati_category, gujarati_feature_tags,
    gurmukhi_category, gurmukhi_feature_tags, joining_class, kannada_category,
    kannada_feature_tags, khmer_category, khmer_feature_tags, lao_category, lao_feature_tags,
    malayalam_category, malayalam_feature_tags, oriya_category, oriya_feature_tags,
    presentation_form, reorder_cluster, reorder_cluster_with, script_indic_tags,
    shape_text_with_font, sinhala_category, sinhala_feature_tags, tamil_category,
    tamil_feature_tags, telugu_category, telugu_feature_tags, thai_category, thai_feature_tags,
    ClusterFlags, IndicCategory, JoiningClass, JoiningForm, ReorderRules, RephKind, Script,
    BENGALI_RULES, BURMESE_RULES, DEVANAGARI_RULES, GUJARATI_RULES, GURMUKHI_RULES, KANNADA_RULES,
    KHMER_RULES, LAO_RULES, MALAYALAM_RULES, ORIYA_RULES, SINHALA_RULES, TAMIL_RULES, TELUGU_RULES,
    THAI_RULES,
};
pub use style::{
    synthetic_italic_shear, Style, DEFAULT_SYNTHETIC_ITALIC_DEG, ITALIC_ANGLE_EPSILON_DEG,
};

/// Errors emitted by the scribe pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The underlying TTF parser rejected the bytes.
    Ttf(oxideav_ttf::Error),
    /// The underlying OTF (CFF) parser rejected the bytes.
    Otf(oxideav_otf::Error),
    /// `size_px` was non-positive (negative or NaN).
    InvalidSize,
    /// A `with_font` / `with_otf_font` call was made on a face of
    /// the wrong flavour.
    WrongFaceKind {
        expected: FaceKind,
        actual: FaceKind,
    },
}

impl From<oxideav_ttf::Error> for Error {
    fn from(e: oxideav_ttf::Error) -> Self {
        Self::Ttf(e)
    }
}

impl From<oxideav_otf::Error> for Error {
    fn from(e: oxideav_otf::Error) -> Self {
        Self::Otf(e)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ttf(e) => write!(f, "ttf error: {e}"),
            Self::Otf(e) => write!(f, "otf error: {e}"),
            Self::InvalidSize => f.write_str("non-positive font size"),
            Self::WrongFaceKind { expected, actual } => {
                write!(f, "wrong face kind: expected {expected:?}, got {actual:?}")
            }
        }
    }
}

impl std::error::Error for Error {
    /// Expose the underlying parser error for the `Ttf` / `Otf`
    /// variants so callers consuming `dyn Error` (or printing the
    /// chain via `anyhow` / `thiserror`'s `source()` walker) see the
    /// real diagnostic rather than just the wrapper. The leaf
    /// variants (`InvalidSize`, `WrongFaceKind`) have no inner
    /// source.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Ttf(e) => Some(e),
            Self::Otf(e) => Some(e),
            Self::InvalidSize | Self::WrongFaceKind { .. } => None,
        }
    }
}

#[cfg(test)]
mod error_tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn leaf_variants_have_no_source() {
        let e = Error::InvalidSize;
        assert!(e.source().is_none());
        let e = Error::WrongFaceKind {
            expected: FaceKind::Ttf,
            actual: FaceKind::Otf,
        };
        assert!(e.source().is_none());
    }

    #[test]
    fn display_emits_human_readable_text() {
        let e = Error::InvalidSize;
        assert_eq!(format!("{e}"), "non-positive font size");
        let e = Error::WrongFaceKind {
            expected: FaceKind::Ttf,
            actual: FaceKind::Otf,
        };
        let s = format!("{e}");
        assert!(s.starts_with("wrong face kind"), "got {s}");
    }

    #[test]
    fn ttf_wrap_round_trips_via_from() {
        // Constructing a real `oxideav_ttf::Error` requires triggering
        // the upstream parser, which we exercise indirectly through
        // `Face::from_ttf_bytes` in the face module. Here we only
        // assert the `From` arm matches the variant shape — feeding
        // an obviously-invalid byte slice through a Face load is what
        // the integration tests already cover.
        let err: Result<Face, Error> = Face::from_ttf_bytes(vec![0u8, 1, 2, 3]);
        match err {
            Err(Error::Ttf(inner)) => {
                let _ = inner; // confirms variant + that it carries the inner
            }
            Ok(_) => panic!("4-byte garbage should not parse as TTF"),
            Err(other) => panic!("expected Error::Ttf, got {other:?}"),
        }
    }

    #[test]
    fn otf_wrap_round_trips_via_from() {
        let err: Result<Face, Error> = Face::from_otf_bytes(vec![0u8, 1, 2, 3]);
        match err {
            Err(Error::Otf(inner)) => {
                let _ = inner;
            }
            Ok(_) => panic!("4-byte garbage should not parse as OTF"),
            Err(other) => panic!("expected Error::Otf, got {other:?}"),
        }
    }
}
