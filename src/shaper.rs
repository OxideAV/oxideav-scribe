//! Text shaper: cmap → ligature substitution → pair kerning →
//! mark-to-base attachment → mark-to-mark stacking → cursive
//! attachment → contextual / chained-contextual positioning.
//!
//! This covers the core of an OpenType positioning pass — enough to
//! render Latin (incl. extended diacritics) / Cyrillic / Greek (incl.
//! polytonic) / basic CJK with the ligatures, kerning, diacritic
//! attachment, and context-sensitive position adjustments that
//! production fonts ship. Bidi (UAX #9), Arabic joining, and Indic
//! conjunct formation are layered above this run-level pass by
//! [`crate::FaceChain`] and [`crate::shaping`].
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
//! 3. **Kerning (GPOS type 2 PairPos + legacy `kern`)**: for each
//!    adjacent pair, call `Font::lookup_kerning(left, right)` and apply
//!    the result to the **left glyph's `x_advance`**. Per OFF §6.4 a
//!    pair adjustment changes the xAdvance of the *first* glyph in the
//!    pair, so the pen accumulates the kern and every glyph downstream
//!    shifts with it (applying it as an `x_offset` on the right glyph
//!    would move only that glyph and leak the adjustment).
//! 4. **Mark-to-base attachment (GPOS type 4)**: for each (base, mark)
//!    pair where `mark` is classified as a mark by GDEF, call
//!    `Font::lookup_mark_to_base(base, mark)`. The returned anchor
//!    delta is applied to the mark's `x_offset` / `y_offset` (with
//!    the mark's own advance subtracted on X so the mark stacks on
//!    top of the base instead of being placed after it). When the
//!    walked-back base is a ligature glyph (formed in step 2 from two
//!    or more components), **mark-to-ligature attachment (GPOS type
//!    5)** is tried first via `Font::lookup_mark_to_ligature(lig,
//!    component, mark)`: the mark is associated with a ligature
//!    component by its trailing ordinal and lands on that component's
//!    per-class anchor, falling back to mark-to-base if the ligature
//!    publishes no LookupType-5 anchor for the mark.
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
//! 6. **Cursive attachment (GPOS type 3)** (round 276): for each
//!    consecutive pair of non-mark glyphs where the first publishes
//!    an exit anchor and the second an entry anchor, the first
//!    glyph's advance is rewritten so the anchors align in the
//!    line-layout direction, and the second glyph's `y_offset` is
//!    adjusted so they align in the cross-stream direction (the
//!    RIGHT_TO_LEFT-flag-clear semantics; see the in-pass comment).
//!    Marks attached to the second glyph follow it vertically.
//!    Cross-stream adjustments accumulate down a connected chain —
//!    the cascading-baseline behaviour cursive scripts need.
//! 7. **Contextual + chained-contextual positioning (GPOS types 7 +
//!    8)**: recognise an input context (a glyph sequence, optionally
//!    bracketed by backtrack / lookahead in the chained variant) and
//!    apply the nested per-glyph positioning adjustments the rule
//!    dispatches. The dependency resolves the sub-table match and the
//!    `SequenceLookupRecord` recursion into absolute per-glyph
//!    `PosRecord` deltas; this pass accumulates them onto the already-
//!    positioned run (see [`crate::shaping::contextual_pos`]). Runs last
//!    so the contextual rules see the post-kern / post-mark / post-
//!    cursive geometry.
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
    /// advance — a mark-attachment / SinglePos placement delta, or 0.
    /// Pair kerning is *not* carried here: per OFF §6.4 it adjusts the
    /// first glyph's [`Self::x_advance`], so the pen accumulates it.
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
    /// ```no_run
    /// # use oxideav_scribe::{Face, FaceChain, Shaper};
    /// # fn demo(face: Face) {
    /// let mut chain = FaceChain::new(face);
    /// let placed = Shaper::with_variation_coords(vec![900.0, 14.0])
    ///     .shape_to_paths(&mut chain, "Hello", 32.0);
    /// # let _ = placed;
    /// # }
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
/// ```no_run
/// # use oxideav_scribe::{Face, FaceChain, Shaper};
/// # fn demo(face: Face) {
/// let mut chain = FaceChain::new(face);
/// let builder = Shaper::with_variation_coords(vec![700.0]);
/// let placed  = builder.shape_to_paths(&mut chain, "Bold!", 24.0);
/// # let _ = placed;
/// # }
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

    // Step 1.5 (round 15): `ccmp` — Glyph Composition / Decomposition.
    // The OpenType required-feature for every script. Production fonts
    // (Inter has 7 such lookups, DejaVu has 2, Noto Sans Arabic has 1)
    // rely on `ccmp` decomposing precomposed codepoints into base +
    // combining sequences (typically via GSUB LookupType 2 multiple
    // substitution) so the round-3/4 mark-to-base attachment pass can
    // place the diacritic correctly. Skipping `ccmp` is what made the
    // round-1..14 pipeline output drifted away from spec for marked-up
    // Latin / Cyrillic / Greek runs. The general-script dispatcher in
    // [`crate::shaping::general`] probes the `latn` / `cyrl` / `grek` /
    // `DFLT` script tags in order and applies every `ccmp` lookup the
    // chosen script publishes. Coverage tables decide per-glyph
    // whether each lookup fires; fonts without `ccmp` are a no-op.
    let ccmp_gids: Vec<u16> = crate::shaping::general::apply_ccmp(font, raw_glyphs);
    let raw_glyphs: &[u16] = &ccmp_gids;

    // Step 2: ligature substitution. Walk through and let the font
    // collapse runs of input glyphs into single output glyphs.
    //
    // `component_counts` runs parallel to `shaped_gids`: it records how
    // many input glyphs collapsed into each output glyph (1 for an
    // un-ligated glyph, N for an N-component ligature). The
    // mark-to-ligature pass (GPOS LookupType 5, step 4) consumes this
    // to learn each ligature glyph's component count — the spec's
    // "multiple components (in a virtual sense — not actual glyphs)"
    // that a following mark must be associated with.
    let mut shaped_gids: Vec<u16> = Vec::with_capacity(raw_glyphs.len());
    let mut component_counts: Vec<u16> = Vec::with_capacity(raw_glyphs.len());
    let mut i = 0;
    while i < raw_glyphs.len() {
        if let Some((replacement, count)) = font.lookup_ligature(&raw_glyphs[i..]) {
            if count >= 2 {
                shaped_gids.push(replacement);
                component_counts.push(count.min(u16::MAX as usize) as u16);
                i += count;
                continue;
            }
        }
        shaped_gids.push(raw_glyphs[i]);
        component_counts.push(1);
        i += 1;
    }

    // Step 2.5 (round 15): `calt` — Contextual Alternates. Refines the
    // post-ligature glyph run with context-driven substitutions
    // (historical "ct" / "st" ligatures gated on word-boundary
    // context, swash variants in display faces, etc.). Inter ships 2
    // `calt` lookups; DejaVu Sans has none under `latn`. Dispatches
    // GSUB LookupTypes 1 / 2 / 3 / 4 / 5 / 6 / 8 per declared type via
    // the general-script feature dispatcher. Coverage misses are
    // silent no-ops.
    let pre_calt_len = shaped_gids.len();
    shaped_gids = crate::shaping::general::apply_calt(font, &shaped_gids);
    // `calt` may itself reshape the run (rare contextual ligatures /
    // splits). When it changes the glyph count the per-glyph component
    // tally no longer lines up positionally, so fall back to treating
    // every glyph as single-component — the mark-to-ligature pass then
    // simply never fires on a calt-mutated run rather than mis-indexing.
    if shaped_gids.len() != pre_calt_len {
        component_counts = vec![1u16; shaped_gids.len()];
    }

    // Steps 3..7: GPOS positioning on the now-substituted glyph run.
    position_run_with_font(font, &shaped_gids, &component_counts, scale, face_idx)
}

/// Run the full GPOS positioning pass (steps 3..7 of the shaping
/// pipeline) over an **already-GSUB-substituted** glyph run.
///
/// `shaped_gids` is the post-substitution glyph buffer; `component_counts`
/// runs parallel to it (1 for an un-ligated glyph, N for an N-component
/// ligature glyph formed by GSUB LookupType 4) so the mark-to-ligature
/// pass (GPOS LookupType 5) can associate a trailing mark with the right
/// ligature component. `scale = size_px / units_per_em` converts the
/// font's design units to raster pixels. `face_idx` tags every output
/// glyph with the [`crate::FaceChain`] face it came from.
///
/// This is the positioning half of [`shape_run_with_font`], split out so
/// both the always-on pipeline and the caller-driven feature surface
/// ([`crate::shaping::feature_subst`]) can run the same kerning →
/// SinglePos → mark-to-base / mark-to-mark / mark-to-ligature → cursive →
/// contextual-positioning sequence on whatever glyph run their GSUB pass
/// produced. The ordering matches the module-level pipeline docs: each
/// later pass sees the geometry the earlier ones established.
///
/// When `component_counts` is shorter than `shaped_gids` (e.g. a caller
/// that doesn't track ligature components), the missing entries default
/// to single-component — the mark-to-ligature path simply never fires for
/// those slots, falling back to mark-to-base, which is the correct
/// graceful degradation.
pub fn position_run_with_font(
    font: &oxideav_ttf::Font<'_>,
    shaped_gids: &[u16],
    component_counts: &[u16],
    scale: f32,
    face_idx: u16,
) -> Vec<PositionedGlyph> {
    // Step 3: kerning. A GPOS PairPos (LookupType 2) / legacy `kern`
    // pair adjustment is, per OFF §6.4 (ISO/IEC 14496-22:2019), a
    // change to the **xAdvance of the first glyph in the pair** — the
    // spec's worked Format 2 example states the pair is "kerned by
    // reducing the XAdvance of the first glyph by 50 design units."
    //
    // The dependency's `lookup_kerning` already resolves a `(left,
    // right)` pair to that single xAdvance delta (it tries GPOS PairPos
    // — both Format 1 and Format 2 — first, then falls back to the
    // legacy `kern` table). We apply the delta to the **left** glyph's
    // `x_advance` so the pen accumulates correctly and every glyph
    // downstream of the kerned pair shifts with it. Applying the kern
    // as an `x_offset` on the right glyph instead would move only that
    // one glyph and leak the adjustment — the next glyph would be
    // placed from the unkerned advance, so the kern never propagates
    // past the immediate pair and the run width (`layout::run_width`)
    // would be wrong.
    let mut out: Vec<PositionedGlyph> = Vec::with_capacity(shaped_gids.len());
    for &gid in shaped_gids.iter() {
        let advance_units = font.glyph_advance(gid) as f32;
        let x_advance = advance_units * scale;
        out.push(PositionedGlyph {
            glyph_id: gid,
            x_offset: 0.0,
            y_offset: 0.0,
            x_advance,
            face_idx,
        });
    }
    for idx in 1..shaped_gids.len() {
        let prev = shaped_gids[idx - 1];
        let gid = shaped_gids[idx];
        let kern_units = font.lookup_kerning(prev, gid) as f32;
        if kern_units != 0.0 {
            // xAdvance of the first (left) glyph of the pair.
            out[idx - 1].x_advance += kern_units * scale;
        }
    }

    // Step 3.5 (round 298): single adjustment positioning (GPOS
    // LookupType 1, SinglePos).
    //
    // Per the GPOS spec a SinglePos subtable "is used to adjust the
    // placement or advance of a single glyph, such as a subscript or
    // superscript. In addition, a SinglePos subtable is commonly used
    // to implement lookup data for contextual positioning." Two
    // sub-table formats exist — Format 1 applies one shared
    // `ValueRecord` to every glyph the coverage lists, Format 2 a
    // per-glyph `ValueRecord` array — both already decoded by the
    // dependency's `gpos_apply_lookup_type_1`, which returns the four
    // geometric fields (xPlacement / yPlacement / xAdvance / yAdvance)
    // in TT font units (Y-up), zeroing whichever the on-disk
    // `valueFormat` mask omits.
    //
    // The four fields map onto a positioned glyph as: `xPlacement`
    // shifts the drawn position right (added to `x_offset`),
    // `yPlacement` shifts it up in TT Y-up space (subtracted from
    // `y_offset`, which is raster Y-down), and `xAdvance` widens or
    // narrows the horizontal advance. `yAdvance` only affects
    // vertical-layout runs and is ignored on this horizontal pen.
    //
    // The pass runs after kerning and before mark attachment so the
    // mark-to-base advance accumulation in step 4 sees the
    // SinglePos-adjusted base advances. It is gated on the font
    // actually publishing a LookupType-1 GPOS lookup (mirroring the
    // cursive gate in step 6) so plain fonts pay only one
    // lookup-list scan. Every type-1 lookup is applied to every glyph
    // in lookup-list order; coverage misses return `None` and leave
    // the glyph untouched.
    let single_pos_lookups: Vec<u16> = font
        .gpos_lookup_list()
        .iter()
        .filter(|&&(_, ty, _)| ty == 1)
        .map(|&(idx, _, _)| idx)
        .collect();
    if !single_pos_lookups.is_empty() {
        for g in out.iter_mut() {
            for &lookup_index in &single_pos_lookups {
                if let Some(v) = font.gpos_apply_lookup_type_1(lookup_index, g.glyph_id) {
                    g.x_offset += f32::from(v.x_placement) * scale;
                    g.y_offset -= f32::from(v.y_placement) * scale;
                    g.x_advance += f32::from(v.x_advance) * scale;
                }
            }
        }
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

        // Resolve the anchor delta. A ligature base (component count > 1)
        // gets the mark-to-ligature path (GPOS LookupType 5) first; a
        // plain base falls through to mark-to-base (LookupType 4).
        //
        // GPOS §MarkLigPos: a ligature glyph carries one anchor *per
        // component per mark class* — "the appropriate base attachment
        // point is determined by which ligature component the mark is
        // associated with." The component association is normally
        // recovered from the original character string; this shaper's
        // ligature collapse (step 2) keeps every component's marks
        // trailing the ligature glyph in source order, so the k-th mark
        // attached to the ligature targets a component by its ordinal,
        // with the last component absorbing any overflow. Because a
        // component may publish a NULL anchor for the mark's class
        // (`lookup_mark_to_ligature` returns `None`), we probe the
        // preferred component first and then walk the remaining
        // components so a mark still lands on whichever component does
        // define an anchor for its class.
        let comp_count = component_counts.get(j).copied().unwrap_or(1);
        let anchor = if comp_count > 1 {
            // Component association. The spec ties each mark to the
            // ligature component it followed in the original character
            // string. This shaper's step-2 ligature collapse keeps the
            // marks trailing the whole ligature glyph (it never
            // interleaves a mark into a ligature's component run), so a
            // trailing mark's source component is the *last* component
            // by default — the "fi + dot-above" case, where the dot
            // followed the 'i' (component 1). The probe order therefore
            // starts at the last component and walks down toward 0, so
            // a mark still lands on whichever component publishes a
            // non-NULL anchor for its class (`lookup_mark_to_ligature`
            // returns `None` for a NULL or out-of-range component
            // anchor).
            let last = comp_count - 1;
            let mut found = None;
            let mut c = last;
            loop {
                if let Some(d) = font.lookup_mark_to_ligature(base_gid, c, mark_gid) {
                    found = Some(d);
                    break;
                }
                if c == 0 {
                    break;
                }
                c -= 1;
            }
            // Mark-to-base fallback if the ligature publishes no LookupType-5
            // anchor for this mark on any component (some fonts ship a
            // single mark-to-base anchor covering the ligature glyph).
            found.or_else(|| font.lookup_mark_to_base(base_gid, mark_gid))
        } else {
            font.lookup_mark_to_base(base_gid, mark_gid)
        };

        if let Some((dx_units, dy_units)) = anchor {
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

    // Step 6 (round 276): cursive attachment (GPOS LookupType 3).
    //
    // Per the CursivePos spec text, adjacent glyphs join by aligning
    // two anchor points: "the designated exit point of a glyph, and
    // the designated entry point of the following glyph". The two
    // axes work differently:
    //
    //   - **Line-layout direction (X here)** — "the layout engine
    //     adjusts the advance of the first glyph (in logical order)",
    //     which moves the second glyph so the anchors align in that
    //     direction.
    //   - **Cross-stream direction (Y here)** — "placement of one
    //     glyph is adjusted to make the anchors align". With the
    //     parent lookup's RIGHT_TO_LEFT flag clear, "the second glyph
    //     is adjusted to align anchors with the first glyph" — the
    //     semantics implemented below. The flag-set variant (first
    //     glyph adjusted, chain anchored to the *last* glyph's
    //     baseline position) needs the lookup flag, which the
    //     dependency's public GPOS API does not currently expose —
    //     deferred.
    //
    // The pass runs after mark attachment so intervening marks have
    // already had their advances zeroed: the chain walks consecutive
    // **non-mark** glyphs (cursive coverage tables list base forms;
    // marks ride along with whichever base they attached to). "If no
    // corresponding anchor point exists, the offset for either the
    // entry or exit Anchor table may be NULL, in which case no
    // positioning adjustment is applied" — pairs missing either side
    // are skipped. Cross-stream adjustments accumulate naturally down
    // the chain because each second glyph is placed relative to the
    // (already-adjusted) first.
    let has_cursive = font.gpos_lookup_list().iter().any(|&(_, ty, _)| ty == 3);
    if has_cursive {
        let mut prev_base: Option<usize> = None;
        for i in 0..out.len() {
            if font.is_mark_glyph(out[i].glyph_id) {
                continue;
            }
            if let Some(p) = prev_base {
                let pair = (
                    font.lookup_cursive_attachment(out[p].glyph_id)
                        .and_then(|a| a.exit),
                    font.lookup_cursive_attachment(out[i].glyph_id)
                        .and_then(|a| a.entry),
                );
                if let (Some((exit_x, exit_y)), Some((entry_x, entry_y))) = pair {
                    // X (line-layout direction): rewrite the FIRST
                    // glyph's advance so the second glyph's drawn
                    // position puts its entry anchor on the first
                    // glyph's exit anchor. With pen_second = pen_first
                    // + first.advance + intervening-mark advances, and
                    // drawn = pen + x_offset, exact alignment solves
                    // to:
                    let mut intervening = 0.0_f32;
                    for e in &out[p + 1..i] {
                        intervening += e.x_advance;
                    }
                    out[p].x_advance = out[p].x_offset - out[i].x_offset - intervening
                        + (f32::from(exit_x) - f32::from(entry_x)) * scale;
                    // Y (cross-stream): move the SECOND glyph so its
                    // entry anchor sits at the first glyph's exit
                    // height. Anchors are font units (TT Y-up);
                    // y_offset is raster Y-down, hence the negation.
                    let new_y = out[p].y_offset - (f32::from(exit_y) - f32::from(entry_y)) * scale;
                    let dy = new_y - out[i].y_offset;
                    out[i].y_offset = new_y;
                    // Marks attached to the second glyph follow it
                    // vertically. (No X fix-up needed: changing the
                    // first glyph's advance shifts the pen of the
                    // second glyph and its trailing marks equally.)
                    let mut j = i + 1;
                    while j < out.len() && font.is_mark_glyph(out[j].glyph_id) {
                        out[j].y_offset += dy;
                        j += 1;
                    }
                }
            }
            prev_base = Some(i);
        }
    }

    // Step 7: contextual (GPOS LookupType 7) + chained-contextual (GPOS
    // LookupType 8) positioning.
    //
    // These recognise an input context — a glyph sequence, optionally
    // bracketed by backtrack / lookahead windows in the chained variant
    // — and dispatch nested per-glyph positioning adjustments. The
    // dependency resolves the sub-table match + the nested
    // `SequenceLookupRecord` recursion into absolute per-glyph
    // `PosRecord` deltas; this pass accumulates those deltas onto the
    // run. It runs last so the contextual rules see the post-kern,
    // post-SinglePos, post-mark, post-cursive geometry — matching the
    // §6 rule that the contextual lookups' nested actions are ordinary
    // positioning adjustments layered on the already-positioned run.
    // Gated internally on the font publishing at least one type-7/8
    // GPOS lookup, so plain Latin faces pay a single lookup-list scan.
    crate::shaping::contextual_pos::apply_contextual_pos(font, &mut out, scale);

    out
}
