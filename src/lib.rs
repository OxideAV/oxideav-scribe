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
//! - **Indic complex-script shaping (rounds 8 + 10)** —
//!   `shaping::indic` classifies Devanagari (U+0900..U+097F), Bengali
//!   (U+0980..U+09FF), and Tamil (U+0B80..U+0BFF) codepoints into
//!   syllabic categories, segments runs into orthographic clusters,
//!   and applies per-script cluster transformations: pre-base matra
//!   reorder (Devanagari U+093F; Bengali U+09BF / U+09C7 / U+09C8;
//!   Tamil U+0BC6 / U+0BC7 / U+0BC8) plus reph identification
//!   (Devanagari + Bengali only — Tamil RA does not form a reph).
//!   The `FaceChain::shape` pipeline applies the reorder before cmap
//!   so cmap-only Indic fonts render simple clusters like "कি" /
//!   "কি" / "கெ" with the matra in the correct visual position.
//!   When the active face publishes a `rphf` GSUB lookup for the
//!   script, identified reph clusters get the leading RA glyph
//!   substituted to its reph-form and the halant glyph dropped via
//!   `Font::gsub_apply_lookup_type_1`.
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

pub use color::{Rgba, TRANSPARENT, WHITE};
pub use color_glyph::ColorGlyphBitmap;
pub use face::{Face, FaceKind};
pub use face_chain::FaceChain;
pub use layout::{run_width, wrap_lines};
pub use oxideav_ttf::{NamedInstance, VariationAxis};
pub use shaper::{PositionedGlyph, Shaper, ShaperBuilder};
pub use shaping::{
    bengali_category, bengali_feature_tags, cluster_boundaries, cluster_boundaries_with,
    compute_forms, devanagari_category, devanagari_feature_tags, feature_tags_for_run,
    joining_class, presentation_form, reorder_cluster, reorder_cluster_with, script_indic_tags,
    tamil_category, tamil_feature_tags, ClusterFlags, IndicCategory, JoiningClass, JoiningForm,
    ReorderRules, Script, BENGALI_RULES, DEVANAGARI_RULES, TAMIL_RULES,
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

impl std::error::Error for Error {}
