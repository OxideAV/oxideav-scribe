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
//! - **Shaper** — [`FaceChain::shape`] / [`Shaper::shape`]: script
//!   itemisation (UAX #24), grapheme-cluster font fallback, per-font
//!   normalisation, then each script's default `GSUB` / `GPOS`
//!   features under its language system. Latin-type scripts, Hebrew,
//!   Thai, Hangul, CJK and emoji go through the font parser's lookup
//!   engine; cursive-joining scripts (Arabic, Syriac, …) and nine
//!   Indic scripts go through the staged engine in [`shaping`], with
//!   per-glyph feature masks (joining forms; syllable positions and
//!   reordering per the OpenType script-development specifications).
//!   [`ShapeOptions`] switches features, picks a language system and
//!   selects vertical layout. See the README's "Shaping" and "Shaping
//!   conformance" sections.
//! - **Script helpers** — [`shaping::arabic`] (joining classes and
//!   forms, Arabic presentation-form mapping for fonts without form
//!   lookups) and [`shaping::indic`] (the character-level cluster
//!   model: categories, cluster boundaries, pre-base reordering for
//!   fourteen Brahmic scripts — used by the shaper for Sinhala, Khmer
//!   and Myanmar).
//! - **Paragraph layout** — [`layout_text`]: UAX #14 line breaking,
//!   alignment and justification, tab stops, per-paragraph direction,
//!   vertical columns.
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
//! - **BiDi (rounds 186 + 191)** — `bidi::bidi_class(c)` returns the
//!   UAX #9 §3.2 normative bidirectional class for every code point
//!   scribe needs today (the 12 explicit-format controls, ASCII /
//!   Latin-1, Hebrew, four core Arabic blocks plus Syriac, Thaana,
//!   N'Ko, the two Arabic Presentation Forms blocks, and combining-
//!   mark NSMs); `bidi::paragraph_level(text)` implements UAX #9
//!   rules P1 / P2 / P3, returning the paragraph embedding level
//!   (0 = LTR, 1 = RTL). `bidi::split_paragraphs(text)` is P1's
//!   split (every type-`B` character is kept with the previous
//!   paragraph). Round 191 lands `bidi::resolve_weak_types(classes,
//!   sos, eos)`, the §3.3.4 weak-type resolution pass (rules
//!   W1..W7) operating on one isolating run sequence in place: NSM
//!   inheritance (W1), EN-after-AL → AN (W2), AL → R (W3),
//!   single-separator-between-two-numbers collapse (W4),
//!   ET-adjacent-to-EN collapse (W5), leftover-separator
//!   neutralisation (W6), and EN-after-L → L (W7). Round 198 lands
//!   `bidi::resolve_neutral_types(classes, embedding_level, sos,
//!   eos)`, the §3.3.5 neutral / isolate-formatting resolution
//!   pass (rules N1 + N2): every maximal NI run (B / S / WS / ON /
//!   LRI / RLI / FSI / PDI) collapses to a strong direction —
//!   matching neighbours via N1 (with `EN` / `AN` counting as `R`)
//!   or the embedding direction via N2. Round 204 lands
//!   `bidi::resolve_implicit_levels(classes, embedding_level)`, the
//!   §3.3.6 implicit-level pass (rules I1 + I2): every character's
//!   resolved type drives a per-character level adjustment per
//!   UAX #9 Table 5 (L stays at even, R goes +1; AN / EN go +2;
//!   under odd, L / EN / AN all go +1). Round 210 lands the §3.4
//!   line-level pair: `bidi::reset_trailing_levels(orig_classes,
//!   levels, paragraph_level)` is rule **L1** (separator + trailing
//!   whitespace + isolate-formatting characters reset back to the
//!   paragraph embedding level using the *original* class slice
//!   per §3.4) and `bidi::reorder_line(levels) -> Vec<usize>` is
//!   rule **L2** (the progressive top-down reversal that produces
//!   a logical-to-visual permutation a renderer walks to emit
//!   glyphs in display order). Round 217 lands
//!   `bidi::resolve_explicit_levels(classes, paragraph_level)`, the
//!   §3.3.2 explicit-level / override / isolate stack pass (rules
//!   X1..X9): produces a per-character embedding level + an
//!   override-rewritten effective-class slice + the X9-removed-
//!   character flag set, ready for X10's isolating-run-sequence
//!   partition to feed the W / N / I phases. Round 227 lands
//!   `bidi::process_paragraph(text, base_level)` and the
//!   class-driven mirror `bidi::process_paragraph_classes(classes,
//!   base_level)`, the §3 whole-paragraph driver that composes
//!   P → X → W → N → I in one call and returns a
//!   `ParagraphBidi` carrier carrying the paragraph level + the
//!   original classes + the X9-removed flag set + the resolved
//!   per-character level vector. The carrier exposes
//!   `reorder_paragraph()` (whole-paragraph L1 + L2 convenience)
//!   and `reorder_line_range(start..end)` (per-line variant for
//!   callers that have a line-breaker). Round 247 lands
//!   `bidi::reorder_combining_marks(orig_classes, levels,
//!   &mut visual)`, the §3.4 rule **L3** in-place permutation
//!   adjuster: every L2-reversed `[NSM, …, NSM, base]` block in
//!   the visual stream (identified by its strictly-decreasing
//!   logical indices) is reversed back to `[base, NSM, …, NSM]`
//!   so callers running a non-scribe mark-attachment policy get
//!   the spec's "expects them to follow" alternative shape; the
//!   function is idempotent and ignores even-level (LTR) runs.
//!   Round 257 lands the §3.3.5 **N0 bracket-pair pass**:
//!   `bidi::paired_bracket(c)` is the BD14 / BD15 lookup for the
//!   six ASCII brackets (`(` ↔ `)`, `[` ↔ `]`, `{` ↔ `}`);
//!   `bidi::bracket_pairs(chars, classes)` runs the BD16 stack
//!   walk (63-deep, overflow → empty list, sort by opener);
//!   `bidi::resolve_bracket_pairs(classes, pairs, embedding,
//!   sos)` applies the N0 a / b / c / d cases in place (EN/AN
//!   counted as R for the inside-strong + preceding-strong walks,
//!   sequential ordering so inner pairs see the rewrites of outer
//!   ones, trailing-NSM inheritance per the §3.3.5 note); and
//!   `bidi::process_paragraph_with_brackets(text, base_level)` /
//!   `process_paragraph_classes_with_brackets(classes, chars,
//!   base_level)` are the §3 paragraph driver with N0 wired in
//!   between W7 and N1. The full Unicode `BidiBrackets.txt`
//!   table (the ~60 paired-bracket entries across the
//!   Mathematical Operators / CJK Symbols / Ornamental Brackets
//!   blocks) is vendored under `docs/text/unicode-bidi/` and covers
//!   every paired-bracket character.
//!   Round 268 lands the §3.4 **L4 mirroring rule**:
//!   `bidi::mirrored_glyph(c)` is the full `Bidi_Mirroring_Glyph`
//!   acceptable-mirror-pair lookup (an involution, `None` for
//!   characters without a mirror pair including the §3.4-excluded
//!   ornate parentheses U+FD3E / U+FD3F), and
//!   `bidi::apply_mirroring(chars, levels)` rewrites every
//!   odd-resolved-level (directionality R) position that has a
//!   mirror pair in place over the line's logical character
//!   sequence. As of round 319 the `Bidi_Mirroring_Glyph` table
//!   (paired brackets, mathematical operators, angle brackets, CJK
//!   bracket blocks) is provided by the `intl` crate's compiled
//!   UCD tables.
//!
//! See `README.md` for a tour and the deferral list.

#![deny(missing_debug_implementations)]
#![warn(rust_2018_idioms)]

pub mod bidi;
pub mod color;
pub mod color_glyph;
pub mod color_layers;
pub mod face;
pub mod face_chain;
pub mod layout;
pub mod outline_cache;
pub mod paragraph;
mod parsed;
pub mod post;
pub mod script;
pub mod shaper;
pub mod shaping;
pub mod style;
pub mod variations;

pub use bidi::{
    BidiClass, BracketKind, ExplicitLevels, IsolatingRunSequence, LevelRun, MAX_DEPTH,
    ParagraphBidi, ParagraphSlice, TextBidi, apply_mirroring, bidi_class, bracket_pairs,
    isolating_run_sequences, level_runs, mirrored_glyph, paired_bracket, paragraph_level,
    process_paragraph, process_paragraph_classes, process_paragraph_classes_with_brackets,
    process_paragraph_with_brackets, process_text, reorder_combining_marks, reorder_line,
    reset_trailing_levels, resolve_bracket_pairs, resolve_explicit_levels, resolve_implicit_levels,
    resolve_neutral_types, resolve_weak_types, split_paragraphs,
};
pub use color::{Rgba, TRANSPARENT, WHITE};
pub use color_glyph::ColorGlyphBitmap;
pub use color_layers::{ColorGlyphLayer, LayerColor};
pub use face::{BaselineAxis, Face, FaceKind};
pub use face_chain::FaceChain;
pub use layout::{
    ShapedParagraph, ShapedVisualLine, VisualLine, reorder_line_visual, run_width,
    shape_paragraphs, shape_visual_line, shape_visual_line_with, wrap_and_shape_lines, wrap_lines,
};
pub use outline_cache::{DEFAULT_BUDGET_BYTES, OutlineCache, OutlineCacheStats};
pub use oxideav_ttf::{NamedInstance, VariationAxis};
pub use paragraph::{Align, LaidOutLine, LayoutOptions, layout_text};
pub use post::{PostTable, STANDARD_MAC_GLYPH_NAMES, standard_mac_glyph_name};
pub use script::{
    ScriptRun, ot_script_tag, ot_script_tags, resolve_scripts, script_runs, script_runs_str,
};
pub use shaper::{PositionedGlyph, ShapeOptions, Shaper, ShaperBuilder};
pub use shaping::{
    BENGALI_RULES, BURMESE_RULES, ClusterCategory, ClusterFlags, DEVANAGARI_RULES, GUJARATI_RULES,
    GURMUKHI_RULES, IndicCategory, JoiningClass, JoiningForm, KANNADA_RULES, KHMER_RULES,
    LAO_RULES, MALAYALAM_RULES, ORIYA_RULES, ReorderRules, RephKind, SINHALA_RULES, Script,
    TAMIL_RULES, TELUGU_RULES, THAI_RULES, bengali_category, bengali_feature_tags,
    burmese_category, burmese_feature_tags, cluster_boundaries, cluster_boundaries_with,
    cluster_category, cluster_script, compute_forms, devanagari_category, devanagari_feature_tags,
    feature_tags_for_run, gujarati_category, gujarati_feature_tags, gurmukhi_category,
    gurmukhi_feature_tags, joining_class, kannada_category, kannada_feature_tags, khmer_category,
    khmer_feature_tags, lao_category, lao_feature_tags, malayalam_category, malayalam_feature_tags,
    oriya_category, oriya_feature_tags, position_gids_with_features_with_font,
    position_text_with_alternates_with_font, position_text_with_features_with_font,
    position_text_with_script_and_features_with_font, presentation_form, reorder_cluster,
    reorder_cluster_with, script_indic_tags, shape_text_with_font,
    shape_text_with_script_with_font, sinhala_category, sinhala_feature_tags, tamil_category,
    tamil_feature_tags, telugu_category, telugu_feature_tags, thai_category, thai_feature_tags,
    universal_cluster_boundaries,
};
pub use style::{
    DEFAULT_SYNTHETIC_ITALIC_DEG, ITALIC_ANGLE_EPSILON_DEG, Style, synthetic_italic_shear,
};

/// Errors emitted by the scribe pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
