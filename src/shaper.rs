//! Round-1 text shaper: cmap → ligature substitution → pair kerning.
//!
//! This is a deliberately small subset of an OpenType shaper — enough
//! to render Latin / Cyrillic / Greek / basic CJK with the ligatures
//! and kerning that production fonts ship. Bidi (UAX #9), Arabic
//! joining, Indic conjunct formation, and the more elaborate
//! contextual GSUB/GPOS lookups are explicitly deferred.
//!
//! ## Pipeline
//!
//! 1. **cmap**: walk the input string codepoint-by-codepoint, looking
//!    each up via `Font::glyph_index`. Unmapped codepoints fall back
//!    to glyph 0 (the `.notdef` "tofu" glyph) — preserving
//!    measurement and visual feedback that the font was missing.
//! 2. **Ligatures (GSUB type 4)**: for every position in the glyph
//!    array, ask `Font::lookup_ligature(&glyphs[i..])`. If it returns
//!    `Some((replacement, n))`, replace `n` glyphs starting at `i`
//!    with the single replacement and advance the cursor.
//! 3. **Kerning (GPOS type 2 + legacy `kern`)**: for each adjacent
//!    pair, call `Font::lookup_kerning(left, right)` and apply the
//!    result as an additional `x_offset` on the right-hand glyph.
//!
//! Output is a `Vec<PositionedGlyph>` ready for [`crate::compose`].

use crate::face::Face;
use crate::Error;

/// A single shaped glyph with its position relative to the run's pen
/// origin. Coordinates are in raster pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedGlyph {
    /// Glyph id within the face (after cmap mapping + ligature
    /// substitution).
    pub glyph_id: u16,
    /// Horizontal offset to apply on top of the cumulative pen
    /// advance — typically a kerning adjustment, otherwise 0.
    pub x_offset: f32,
    /// Vertical offset (round-1 always 0; reserved for future
    /// mark-to-base attachment).
    pub y_offset: f32,
    /// Per-glyph horizontal advance, in raster pixels. The pen moves
    /// `x_advance` after this glyph is drawn.
    pub x_advance: f32,
    /// Index into the [`crate::FaceChain`] that owns the face this
    /// glyph was sourced from. 0 for the primary face. Round-1 callers
    /// using the single-face `Shaper::shape` get 0 for every glyph.
    pub face_idx: u16,
}

/// Round-1 shaper. Stateless — every call starts from scratch.
#[derive(Debug)]
pub struct Shaper;

impl Shaper {
    /// Shape `text` against `face` at `size_px`. See module docs for
    /// the lookup pipeline.
    pub fn shape(face: &Face, text: &str, size_px: f32) -> Result<Vec<PositionedGlyph>, Error> {
        if text.is_empty() || size_px <= 0.0 {
            return Ok(Vec::new());
        }
        face.with_font(|font| shape_with_font(font, text, size_px))
    }
}

fn shape_with_font(font: &oxideav_ttf::Font<'_>, text: &str, size_px: f32) -> Vec<PositionedGlyph> {
    // Step 1: cmap.
    let raw_glyphs: Vec<u16> = text
        .chars()
        .map(|ch| font.glyph_index(ch).unwrap_or(0))
        .collect();

    shape_run_with_font(font, &raw_glyphs, size_px, 0)
}

/// Shape a *pre-cmap'd* run of glyph ids through GSUB + GPOS using
/// `font`. Used by [`crate::FaceChain`] which performs cmap fallback
/// at chain-walk time, then hands a per-face run to this entry point.
///
/// Each output glyph is tagged with `face_idx` so the rasterizer knows
/// which face to fetch the outline from.
pub fn shape_run_with_font(
    font: &oxideav_ttf::Font<'_>,
    raw_glyphs: &[u16],
    size_px: f32,
    face_idx: u16,
) -> Vec<PositionedGlyph> {
    let upem = font.units_per_em().max(1) as f32;
    let scale = size_px / upem;

    // Step 2: ligature substitution. Walk through and let the font
    // collapse runs of input glyphs into single output glyphs.
    let mut shaped_gids: Vec<u16> = Vec::with_capacity(raw_glyphs.len());
    let mut i = 0;
    while i < raw_glyphs.len() {
        if let Some((replacement, count)) = font.lookup_ligature(&raw_glyphs[i..]) {
            if count >= 2 {
                shaped_gids.push(replacement);
                i += count;
                continue;
            }
        }
        shaped_gids.push(raw_glyphs[i]);
        i += 1;
    }

    // Step 3: kerning. Apply the kerning between each adjacent glyph
    // pair as an x_offset on the right-hand glyph.
    let mut out: Vec<PositionedGlyph> = Vec::with_capacity(shaped_gids.len());
    for (idx, &gid) in shaped_gids.iter().enumerate() {
        let advance_units = font.glyph_advance(gid) as f32;
        let x_advance = advance_units * scale;
        let mut x_offset = 0.0_f32;
        if idx > 0 {
            let prev = shaped_gids[idx - 1];
            let kern_units = font.lookup_kerning(prev, gid) as f32;
            x_offset = kern_units * scale;
        }
        out.push(PositionedGlyph {
            glyph_id: gid,
            x_offset,
            y_offset: 0.0,
            x_advance,
            face_idx,
        });
    }

    out
}
