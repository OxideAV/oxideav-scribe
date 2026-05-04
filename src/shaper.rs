//! Text shaper: cmap → ligature substitution → pair kerning →
//! mark-to-base attachment → mark-to-mark stacking.
//!
//! This is a deliberately small subset of an OpenType shaper — enough
//! to render Latin (incl. extended diacritics) / Cyrillic / Greek
//! (incl. polytonic) / basic CJK with the ligatures, kerning and
//! diacritic attachment that production fonts ship. Bidi (UAX #9),
//! Arabic joining, Indic conjunct formation, and the more elaborate
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
//! 4. **Mark-to-base attachment (GPOS type 4)**: for each (base, mark)
//!    pair where `mark` is classified as a mark by GDEF, call
//!    `Font::lookup_mark_to_base(base, mark)`. The returned anchor
//!    delta is applied to the mark's `x_offset` / `y_offset` (with
//!    the mark's own advance subtracted on X so the mark stacks on
//!    top of the base instead of being placed after it).
//! 5. **Mark-to-mark stacking (GPOS type 6)**: for each consecutive
//!    `(mark_prev, mark_new)` pair where both are GDEF marks, call
//!    `Font::lookup_mark_to_mark(mark_prev, mark_new)`. If the font
//!    provides an anchor for the pair, the new mark is positioned
//!    relative to the previous mark's *post-attachment* position
//!    (which already sits on the base). This handles double-diacritic
//!    stacks like polytonic Greek and Vietnamese tonal vowels. The
//!    mark-to-base path remains the fallback: if no mark-to-mark
//!    anchor exists, the new mark falls back to attaching to the
//!    walked-back base (round-3 behaviour).
//!
//! Output is a `Vec<PositionedGlyph>` ready for [`crate::compose`].

use crate::face::Face;
use crate::face_chain::FaceChain;
use crate::Error;
use oxideav_core::{Group, Node, Transform2D};

/// Compute the producer-side `Group::cache_key` for one positioned
/// glyph. The hash inputs cover every dimension that can change the
/// rasterised glyph bitmap **before** placement (the per-glyph
/// `Transform2D` carries the placement and is mixed in by the
/// downstream rasterizer's composite key).
///
/// Inputs:
/// - `face_stable_id` — `Face::stable_id()` (content-derived; stable
///   across program runs).
/// - `glyph_id` — the gid emitted by the shaper.
/// - `size_q8` — `(size_px * 256.0).round() as u32`, matching the
///   raster glyph cache's size quantisation (see [`crate::cache`]).
/// - `style_bits` — opaque producer-side style flags (italic shear /
///   bold dilation / fill colour). Pass `0` for the default
///   `shape_to_paths` output (upright, regular, default black fill).
fn glyph_cache_key(face_stable_id: u64, glyph_id: u16, size_q8: u32, style_bits: u64) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    // A short version tag lets us evolve the layout without colliding
    // against historical cache entries (consumers persisting the
    // composite key across processes shouldn't be common, but this is
    // cheap insurance).
    b"oxideav-scribe.glyph_cache_key.v1".hash(&mut h);
    face_stable_id.hash(&mut h);
    glyph_id.hash(&mut h);
    size_q8.hash(&mut h);
    style_bits.hash(&mut h);
    h.finish()
}

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
///
/// Round 9 (variable fonts): the unit-struct stateless API is preserved
/// for the static / default-coords case. Callers that want to shape
/// against a specific variation-coord vector (e.g. `wght=600 / wdth=125`
/// on Inter) construct a [`ShaperBuilder`] via
/// [`Shaper::with_variation_coords`] and call its
/// [`ShaperBuilder::shape_to_paths`] / [`ShaperBuilder::shape`] methods,
/// which apply the coords to the primary face for the duration of the
/// call and restore them on drop. See also
/// [`Shaper::named_instances`] for picking a coord vector by name.
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

    /// Shape a text run and return positioned **vector glyphs** —
    /// the primary vector text API. Each tuple is
    /// `(face_idx, glyph Node, transform)` where:
    ///
    /// - `face_idx` is the index into `face_chain` that owns the glyph
    ///   (matches [`PositionedGlyph::face_idx`]).
    /// - `glyph` is a self-contained [`Node`] from
    ///   [`Face::glyph_node`] — outline glyphs become `Node::Path`
    ///   (Y-down, baked-in size scale, black solid fill); bitmap glyphs
    ///   (CBDT/sbix) become `Node::Image`. The node's local origin
    ///   `(0, 0)` is the glyph's pen origin (baseline-left).
    /// - `transform` is the per-glyph placement: a translation by
    ///   `(x_int + x_frac, y_baseline + y_offset)` in raster pixels,
    ///   honouring the round-3 sub-pixel positioning and the
    ///   round-3/4 mark `(x_offset, y_offset)`. The Y-baseline is
    ///   chosen so the run sits on `y = 0` — callers wanting a
    ///   different baseline pre-translate or wrap in a `Group` of
    ///   their own.
    ///
    /// Glyphs that produce no rendering output (whitespace, empty
    /// outlines) are skipped so the returned `Vec` length is
    /// `<= shaped.len()`.
    ///
    /// Empty strings, zero-glyph runs, and `size_px <= 0` all return
    /// an empty `Vec`.  The full-fidelity `Shaper::shape` path runs
    /// underneath so GSUB ligatures + GPOS kerning + mark attachment
    /// + face-chain fallback all apply.
    ///
    /// **Cache identity (round 8 / #357).** Each emitted glyph is
    /// wrapped in a [`Group`] carrying a deterministic
    /// [`Group::cache_key`] computed from
    /// `(face_stable_id, glyph_id, size_q8)` (see [`glyph_cache_key`]).
    /// The downstream [`oxideav-raster`] renderer combines that key
    /// with the per-glyph `Transform2D` to memoise the rasterised
    /// glyph, so the same glyph rendered at the same effective
    /// resolution reuses its bitmap across calls (and across renderer
    /// instances when bitmap caches are shared). The `Group` carries
    /// the unmodified [`Node`] from [`Face::glyph_node`] and an
    /// identity transform — placement stays on the outer
    /// `Transform2D` so two adjacent glyphs hit the cache regardless
    /// of pen position.
    pub fn shape_to_paths(
        face_chain: &FaceChain,
        text: &str,
        size_px: f32,
    ) -> Vec<(usize, Node, Transform2D)> {
        if text.is_empty() || size_px <= 0.0 || !size_px.is_finite() {
            return Vec::new();
        }
        let glyphs = match face_chain.shape(text, size_px) {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let mut out: Vec<(usize, Node, Transform2D)> = Vec::with_capacity(glyphs.len());
        // Pen advances along X; the baseline is at y = 0 (caller can
        // translate the whole run via a wrapping Group if needed).
        let mut pen_x = 0.0_f32;
        // Match the raster glyph-cache size quantisation (cache::GlyphKey
        // uses the same `(size_px * 256.0).round() as u32` formula).
        let size_q8 = (size_px * 256.0).round().max(0.0) as u32;
        for g in &glyphs {
            let face_idx = g.face_idx as usize;
            let face = face_chain.face(g.face_idx);
            // Vector text is resolution-independent, so the full
            // floating-point target X is the right per-glyph
            // translation — no need for the round-3 raster
            // sub-pixel-slot quantisation. The pen sums advances even
            // for non-rendering glyphs (SPACE, etc) so the next
            // glyph lands at the correct position.
            let target_x = pen_x + g.x_offset;
            pen_x += g.x_advance;
            let node = match face.glyph_node(g.glyph_id, size_px) {
                Some(n) => n,
                None => continue,
            };
            let ty = g.y_offset;
            // Wrap each glyph in a `Group` carrying a stable cache_key
            // so oxideav-raster's bitmap cache can memoise the glyph
            // bitmap across renders. style_bits = 0 because
            // `shape_to_paths` always uses the upright/regular/black
            // default fill from `Face::glyph_node`; `render_text_styled`
            // (which adds shear / bold) goes through a separate
            // rasterised path and doesn't share this cache key.
            let cache_key = glyph_cache_key(face.stable_id(), g.glyph_id, size_q8, 0);
            let group_node = Node::Group(Group {
                cache_key: Some(cache_key),
                children: vec![node],
                ..Group::default()
            });
            out.push((face_idx, group_node, Transform2D::translate(target_x, ty)));
        }
        out
    }

    /// Build a [`ShaperBuilder`] that will apply `coords` to the primary
    /// face on each shape call, then restore the previous coords. The
    /// vector is in user-space units and is silently length-capped /
    /// clamped per [`crate::Face::set_variation_coords`].
    ///
    /// ```ignore
    /// let placed = Shaper::with_variation_coords(vec![900.0, 14.0])
    ///     .shape_to_paths(&mut chain, "Hello", 32.0)?;
    /// ```
    pub fn with_variation_coords(coords: Vec<f32>) -> ShaperBuilder {
        ShaperBuilder { coords }
    }

    /// Named instances published by the face at `face_index` of
    /// `face_chain`. Convenience entry point so callers can pick a
    /// pre-defined coordinate vector (e.g. "Light" / "Regular" / "Bold")
    /// without first walking the chain.
    ///
    /// Each [`crate::NamedInstance`] carries a `coords` vector that
    /// matches [`crate::Face::variation_axes`] one-to-one. Callers
    /// pass the chosen `coords` to either [`crate::Face::set_variation_coords`]
    /// directly or to [`Shaper::with_variation_coords`] for the
    /// per-call override path.
    ///
    /// Returns an empty vec for static / OTF faces or when `face_index`
    /// is out of range.
    pub fn named_instances(
        face_chain: &crate::FaceChain,
        face_index: usize,
    ) -> Vec<crate::NamedInstance> {
        face_chain.named_instances(face_index)
    }
}

/// Variation-coord-aware shaper handle returned by
/// [`Shaper::with_variation_coords`]. Carries the coord vector and
/// delegates to the existing [`Shaper`] entry points after temporarily
/// installing the coords on the primary face.
///
/// Construction:
///
/// ```ignore
/// let builder = Shaper::with_variation_coords(vec![700.0]);
/// let placed  = builder.shape_to_paths(&mut chain, "Bold!", 24.0)?;
/// ```
#[derive(Debug, Clone)]
pub struct ShaperBuilder {
    coords: Vec<f32>,
}

impl ShaperBuilder {
    /// Replace the stored coord vector. Builder-style.
    #[must_use]
    pub fn with_variation_coords(mut self, coords: Vec<f32>) -> Self {
        self.coords = coords;
        self
    }

    /// The coord vector this builder will install before each shape
    /// call. Unmodified copy of what was passed in — clamping happens
    /// inside [`crate::Face::set_variation_coords`] at install time.
    pub fn variation_coords(&self) -> &[f32] {
        &self.coords
    }

    /// Variation-coords-aware mirror of [`Shaper::shape`]. Installs the
    /// builder's coords on the primary face, runs the shape, then
    /// restores the face's previous coords. Returns the shape result
    /// (or any error from coord installation).
    pub fn shape(
        &self,
        face_chain: &mut crate::FaceChain,
        text: &str,
        size_px: f32,
    ) -> Result<Vec<PositionedGlyph>, Error> {
        let prev = face_chain.primary().variation_coords().to_vec();
        face_chain.set_variation_coords(&self.coords)?;
        let result = face_chain.shape(text, size_px);
        // Restore — even on error — so the chain is left untouched.
        // An empty `prev` means the caller never installed coords on
        // the chain directly; re-clearing keeps the post-call state
        // observably indistinguishable from the pre-call state.
        let restore_err = if prev.is_empty() {
            face_chain.face_mut(0).clear_variation_coords();
            None
        } else {
            face_chain.face_mut(0).set_variation_coords(&prev).err()
        };
        match (result, restore_err) {
            (Ok(v), None) => Ok(v),
            (Ok(_), Some(e)) => Err(e),
            (Err(e), _) => Err(e),
        }
    }

    /// Variation-coords-aware mirror of [`Shaper::shape_to_paths`].
    /// Installs the builder's coords on the primary face, runs the
    /// vector-text pipeline, then restores the previous coords. Returns
    /// an empty vec on any error (matching the static
    /// `Shaper::shape_to_paths` contract — errors from the variation
    /// install are swallowed because the static path also swallows
    /// shape errors).
    pub fn shape_to_paths(
        &self,
        face_chain: &mut crate::FaceChain,
        text: &str,
        size_px: f32,
    ) -> Vec<(usize, Node, Transform2D)> {
        let prev = face_chain.primary().variation_coords().to_vec();
        if face_chain.set_variation_coords(&self.coords).is_err() {
            return Vec::new();
        }
        let result = Shaper::shape_to_paths(face_chain, text, size_px);
        // Mirror the `shape` method's restore logic: an empty `prev`
        // means the chain had never been touched, so re-clearing keeps
        // the post-call state byte-equal to the pre-call state.
        if prev.is_empty() {
            face_chain.face_mut(0).clear_variation_coords();
        } else {
            let _ = face_chain.face_mut(0).set_variation_coords(&prev);
        }
        result
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

    // Step 4 + 5: mark-to-base attachment (GPOS LookupType 4) plus
    // mark-to-mark stacking (GPOS LookupType 6).
    //
    // For each `i` whose glyph is a GDEF mark, we choose ONE of two
    // anchor sources:
    //
    //   a) **Mark-to-mark (preferred when applicable)** — if the
    //      immediately-previous glyph (i-1) is also a mark AND the
    //      font ships a MarkMarkPos anchor for the pair, stack on
    //      that mark. The previous mark has already been positioned
    //      relative to the base (we processed marks left-to-right, so
    //      its `x_offset` / `y_offset` already encode the offset
    //      from base to mark). Stacking on top of it just adds the
    //      mark-to-mark delta.
    //
    //   b) **Mark-to-base (fallback)** — walk back to the nearest
    //      non-mark base and apply MarkBasePos. This is what round 3
    //      did and is still correct for "single mark on base" plus
    //      for any case where the font lacks a mark-to-mark anchor
    //      (most fonts ship a sparse set of mark-to-mark anchors
    //      compared to mark-to-base).
    //
    // In both cases we zero the new mark's x_advance so subsequent
    // base glyphs start where the pen would be without this mark.
    for i in 1..out.len() {
        let mark_gid = out[i].glyph_id;
        if !font.is_mark_glyph(mark_gid) {
            continue;
        }

        // Try mark-to-mark first against the immediately-previous mark.
        let prev = i - 1;
        let prev_gid = out[prev].glyph_id;
        if font.is_mark_glyph(prev_gid) {
            if let Some((dx_units, dy_units)) = font.lookup_mark_to_mark(prev_gid, mark_gid) {
                let dx = dx_units as f32 * scale;
                let dy = dy_units as f32 * scale;
                // The previous mark's effective pen position is its
                // own (x_offset, y_offset) plus the cumulative
                // advances up to it. The new mark is one slot later,
                // so its un-attached pen X is `prev.advance` further
                // right (typically 0 since the previous mark already
                // had its advance zeroed). We want the new mark to
                // land at `prev_pen + (dx, dy)` — which is
                // `prev.x_offset + dx`, with the new mark's own pen
                // delta already accounted for in its current
                // `x_offset` (zero kerning since marks don't kern).
                //
                // Concretely: shift the new mark by
                //   (prev.x_offset - new_mark.current_x_offset) + dx
                // on X, and equivalently on Y. Simplest expression:
                // overwrite both offsets and zero the advance.
                out[i].x_offset = out[prev].x_offset + dx - out[i].x_advance;
                // TT Y-up → raster Y-down (negate dy on Y). The
                // previous mark's y_offset is already in raster space.
                out[i].y_offset = out[prev].y_offset - dy;
                out[i].x_advance = 0.0;
                continue;
            }
        }

        // Fallback: mark-to-base. Walk back to the nearest non-mark.
        let mut j = i;
        while j > 0 {
            j -= 1;
            if !font.is_mark_glyph(out[j].glyph_id) {
                break;
            }
        }
        if font.is_mark_glyph(out[j].glyph_id) {
            // No base found in this run — leave the mark unattached.
            continue;
        }
        let base_gid = out[j].glyph_id;
        if let Some((dx_units, dy_units)) = font.lookup_mark_to_base(base_gid, mark_gid) {
            let dx = dx_units as f32 * scale;
            let dy = dy_units as f32 * scale;
            // The mark's pen lands AFTER the base + any intervening
            // marks (whose advance is typically also 0 — and we zero
            // it below for that reason). Total advance from base to
            // mark = sum of advances in (j..i].
            let mut intervening_advance = 0.0_f32;
            for entry in out.iter().take(i + 1).skip(j + 1) {
                intervening_advance += entry.x_advance;
            }
            // x_offset already encodes any kern; we add the anchor
            // delta minus the cumulative advance.
            out[i].x_offset += dx - intervening_advance;
            // TT Y-up → raster Y-down: a positive base_anchor.y means
            // the base anchor is above the baseline; the mark should
            // sit higher in raster (smaller Y). Negate here.
            out[i].y_offset -= dy;
            // Zero the mark's own advance so subsequent base glyphs
            // start where the pen would be without this mark.
            out[i].x_advance = 0.0;
        }
    }

    out
}
