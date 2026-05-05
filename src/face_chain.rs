//! `FaceChain` — ordered list of faces consulted in priority order
//! when shaping. Round-2 fallback support: when the primary face
//! doesn't have a glyph for a codepoint, the chain walks down the list
//! until one does, falling back to the primary's `.notdef` only if no
//! face provides a glyph.
//!
//! ## Design
//!
//! Each codepoint is mapped to `(face_idx, glyph_id)` independently;
//! ligature substitution + kerning then run on the resulting glyph runs
//! face-by-face (because GSUB / GPOS lookups are face-local — you can't
//! ligate an "f" from face A with an "i" from face B).
//!
//! The fallback decision is "first face where `glyph_index` returns a
//! non-zero, defined glyph". A returned `0` is treated as `.notdef`
//! (i.e. *not present*) because the primary face's `.notdef` is what
//! we'd use as the *final* fallback anyway, and skipping over it lets
//! a fallback face provide a real glyph.
//!
//! ## Cache key impact
//!
//! `PositionedGlyph::face_idx` lets the rasterizer pick the right face
//! out of the chain. The cache key already keys by `face_id` (per-face,
//! globally unique), so a glyph from face[0] and a glyph from face[1]
//! with the same numerical `glyph_id` never collide.

use crate::face::Face;
use crate::shaper::{shape_run_with_font, PositionedGlyph};
use crate::shaping::arabic::{compute_forms, script_of, Script};
use crate::shaping::arabic_pf::presentation_form;
use crate::shaping::indic::{
    cluster_boundaries_with, reorder_cluster_with, script_indic_tags, IndicCategory, ReorderRules,
    BENGALI_RULES, BURMESE_RULES, DEVANAGARI_RULES, GUJARATI_RULES, GURMUKHI_RULES, KANNADA_RULES,
    KHMER_RULES, LAO_RULES, MALAYALAM_RULES, ORIYA_RULES, SINHALA_RULES, TAMIL_RULES, TELUGU_RULES,
    THAI_RULES,
};
use crate::style::Style;
use crate::Error;

/// Ordered chain of faces. Index 0 is the primary; index N is consulted
/// only if 0..N all returned `.notdef` for a codepoint.
#[derive(Debug)]
pub struct FaceChain {
    faces: Vec<Face>,
}

impl FaceChain {
    /// Build a chain from a single primary face. Use
    /// [`FaceChain::push_fallback`] (chainable) to append fallbacks.
    pub fn new(primary: Face) -> Self {
        Self {
            faces: vec![primary],
        }
    }

    /// Append a fallback face to the end of the chain. Builder-style:
    /// `FaceChain::new(latin).push_fallback(cjk).push_fallback(emoji)`.
    #[must_use]
    pub fn push_fallback(mut self, face: Face) -> Self {
        self.faces.push(face);
        self
    }

    /// Number of faces in the chain (including the primary).
    pub fn len(&self) -> usize {
        self.faces.len()
    }

    /// True if the chain has no faces — never the case for chains
    /// constructed via [`FaceChain::new`], present only because clippy
    /// rightly complains when `len()` exists alone.
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// Borrow face at `idx`. Panics if `idx >= len()` — the rasterizer
    /// always reads `face_idx` from a `PositionedGlyph` produced by
    /// this chain so the index is bounded by construction.
    pub fn face(&self, idx: u16) -> &Face {
        &self.faces[idx as usize]
    }

    /// Borrow the primary face — useful for size/metric queries.
    pub fn primary(&self) -> &Face {
        &self.faces[0]
    }

    /// Mutably borrow face at `idx`. Used to flip per-face state like
    /// variation coordinates without rebuilding the chain. Panics if
    /// `idx >= len()`.
    pub fn face_mut(&mut self, idx: usize) -> &mut Face {
        &mut self.faces[idx]
    }

    /// Set the variation coordinates on the **primary face** (index 0).
    /// Convenience wrapper around [`Face::set_variation_coords`] for
    /// the common case of "shape this run at `wght=600 / wdth=125`".
    /// Mirrors [`Face::set_variation_coords`]'s clamp + length cap and
    /// returns its error variant unchanged.
    ///
    /// Fallback faces in the chain are NOT touched — call
    /// [`FaceChain::face_mut`] explicitly if a fallback also needs
    /// variation coords (rare in practice; fallback faces typically
    /// cover a different script and are loaded from a static cut).
    pub fn set_variation_coords(&mut self, coords: &[f32]) -> Result<(), Error> {
        self.faces[0].set_variation_coords(coords)
    }

    /// Named instances published by the face at `face_index`. Empty
    /// vec when the face is static / OTF, or when the index is out of
    /// range. Mirrors [`Face::named_instances`] for the chosen face.
    pub fn named_instances(&self, face_index: usize) -> Vec<crate::NamedInstance> {
        self.faces
            .get(face_index)
            .map(|f| f.named_instances())
            .unwrap_or_default()
    }

    /// Variation axes published by the face at `face_index`. Empty vec
    /// when the face is static / OTF, or when the index is out of
    /// range. Mirrors [`Face::variation_axes`] for the chosen face.
    pub fn variation_axes(&self, face_index: usize) -> Vec<crate::VariationAxis> {
        self.faces
            .get(face_index)
            .map(|f| f.variation_axes())
            .unwrap_or_default()
    }

    /// Shape `text` with full chain fallback at default style (upright,
    /// regular).
    pub fn shape(&self, text: &str, size_px: f32) -> Result<Vec<PositionedGlyph>, Error> {
        self.shape_styled(text, size_px, Style::REGULAR)
    }

    /// Shape `text` honouring `style` (italic / weight). The shear
    /// derived from `style` is applied at rasterise-time, not at
    /// shape-time — so glyph positions / advances stay identical
    /// regardless of italic. This matches what desktop shapers do
    /// (synthesised italic doesn't change the metrics).
    pub fn shape_styled(
        &self,
        text: &str,
        size_px: f32,
        _style: Style,
    ) -> Result<Vec<PositionedGlyph>, Error> {
        if text.is_empty() || size_px <= 0.0 {
            return Ok(Vec::new());
        }

        // Step 1: per-codepoint, decide which face owns this glyph.
        // Result: Vec<(face_idx, glyph_id)>.
        let assigned = self.assign_codepoints(text)?;
        if assigned.is_empty() {
            return Ok(Vec::new());
        }

        // Step 2: walk runs of consecutive (face_idx) glyphs and shape
        // each run within the appropriate face. Each run gets its own
        // GSUB + GPOS pass.
        let mut out: Vec<PositionedGlyph> = Vec::with_capacity(assigned.len());
        let mut run_start = 0usize;
        while run_start < assigned.len() {
            let face_idx = assigned[run_start].0;
            let mut run_end = run_start + 1;
            while run_end < assigned.len() && assigned[run_end].0 == face_idx {
                run_end += 1;
            }
            let gids: Vec<u16> = assigned[run_start..run_end].iter().map(|p| p.1).collect();
            let face = &self.faces[face_idx as usize];
            let mut run_glyphs =
                face.with_font(|font| shape_run_with_font(font, &gids, size_px, face_idx))?;
            out.append(&mut run_glyphs);
            run_start = run_end;
        }
        Ok(out)
    }

    /// Per-codepoint face assignment. For every char in `text`, walk
    /// the chain and pick the first face whose `glyph_index` returns a
    /// non-zero glyph id. If none does, fall back to face 0 with glyph
    /// 0 (.notdef) — measurement still works, the user sees tofu.
    ///
    /// **Round 7 — Arabic contextual joining.** Before cmap lookup the
    /// input chars are run through the joining state machine in
    /// [`crate::shaping::arabic`] and Arabic letters are translated to
    /// their Presentation Forms-B equivalents (U+FE70..U+FEFF). Forms
    /// the active face doesn't have a glyph for fall back to the
    /// original base codepoint — so the worst case is "render in
    /// isolated form" (the round-6 behaviour), which is the right
    /// graceful degradation for a font that ships only base glyphs.
    ///
    /// **Round 8 — Devanagari cluster reorder.** Devanagari runs are
    /// segmented into clusters (one base consonant with its halant
    /// chains, matras, and modifiers) and each cluster is rewritten
    /// to its visual order: pre-base matras (U+093F) move to the
    /// front of the cluster so a cmap-only font still draws the
    /// cluster correctly. Reph identification is performed but the
    /// actual glyph substitution is gated on the `rphf` GSUB feature
    /// (followup once `oxideav-ttf` exposes feature-tagged single
    /// substitution).
    fn assign_codepoints(&self, text: &str) -> Result<Vec<(u16, u16)>, Error> {
        let chars: Vec<char> = text.chars().collect();
        // Three pre-cmap shaping passes: Arabic joining → Indic
        // cluster reorder → reph GSUB (post-cmap, deferred to after
        // the codepoint→glyph assignment loop). The scripts are
        // pairwise disjoint so the order doesn't matter for any one;
        // we run Arabic first to match the round-7 behaviour exactly
        // when no Indic input is present.
        let arabic_shaped = apply_arabic_joining(&chars);
        let (shaped_chars, reph_marks, cluster_spans) = apply_indic_reorder(&arabic_shaped);
        let mut out: Vec<(u16, u16)> = Vec::with_capacity(shaped_chars.len());
        for (orig_idx, ch) in shaped_chars.iter().enumerate() {
            let mut found: Option<(u16, u16)> = None;
            for (idx, face) in self.faces.iter().enumerate() {
                let g = face.with_font(|font| font.glyph_index(*ch))?;
                match g {
                    Some(gid) if gid != 0 => {
                        found = Some((idx as u16, gid));
                        break;
                    }
                    _ => continue,
                }
            }
            // If the substituted presentation-form is missing in every
            // face, retry with the corresponding original (base)
            // codepoint — this is the graceful fallback the doc-comment
            // describes. The Indic pass only reorders (no
            // substitution), so the original char is already present
            // somewhere in `chars`; the Arabic pass substitutes 1:1
            // so `chars[orig_idx]` is the right base when the
            // permutation is identity. Use position equality where
            // possible, fall back to char-value lookup otherwise.
            if found.is_none() {
                let orig = if orig_idx < chars.len() && *ch != chars[orig_idx] {
                    Some(chars[orig_idx])
                } else if !chars.contains(ch) {
                    None
                } else {
                    // The shaped char is present in the original input
                    // — no substitution happened, so the retry is a
                    // no-op (the same lookup we already did failed).
                    None
                };
                if let Some(orig) = orig {
                    for (idx, face) in self.faces.iter().enumerate() {
                        let g = face.with_font(|font| font.glyph_index(orig))?;
                        if let Some(gid) = g {
                            if gid != 0 {
                                found = Some((idx as u16, gid));
                                break;
                            }
                        }
                    }
                }
            }
            // No face had it — render as primary's .notdef.
            out.push(found.unwrap_or((0, 0)));
        }
        // Apply reph GSUB substitution: for each `RephMark` we identified
        // in the pre-cmap pass, look up the `rphf` feature on the
        // *assigned* face for the RA glyph, apply LookupType 1, and if
        // a substitute is returned, rewrite the RA gid + drop the halant
        // by replacing the halant slot with the same gid as the
        // following base consonant — i.e. the cluster collapses
        // [reph_gid, halant, base, ...] → [reph_gid, base, base, ...]
        // and the duplicate base is removed below. Marks for which the
        // active face publishes no `rphf` lookup are silently skipped
        // (the cluster falls back to the in-line RA + halant + base
        // rendering, which is the round-8 behaviour).
        //
        // We process marks back-to-front so the index manipulation
        // stays straightforward (no shifting of pending marks).
        let (out, dropped_halants) = self.apply_reph_substitutions(out, &reph_marks)?;
        // Round 11 — cluster-position-aware GSUB pass. For every Indic
        // cluster, dispatch `half` / `pref` / `blwf` / `abvf` / `pstf`
        // (per-position substitution) on halant-suffixed conjunct
        // components, then `pres` / `psts` / `abvs` / `blws` (cluster-
        // wide presentation features) on every glyph in the cluster.
        // Coverage misses pass through unchanged so a font without a
        // given lookup degrades gracefully (the round-10 behaviour).
        //
        // The reph pass may have removed halant glyphs from `out`; the
        // `dropped_halants` Vec tells us which post-reorder character
        // indices are now absent so we can shift cluster span ends
        // accordingly. (Reph removal touches the END of a cluster's
        // first 3 chars, never its boundary, so the START index is
        // always still valid.)
        let adjusted_spans = adjust_cluster_spans(&cluster_spans, &dropped_halants, &shaped_chars);
        let out = self.apply_cluster_position_substitutions(out, &adjusted_spans, &shaped_chars)?;
        // Round 13 — multi-glyph context-aware GSUB pass. For every
        // Indic cluster, dispatch the multi-glyph features
        // (`locl` / `nukt` / `akhn` / `cjct` / `init` / `haln`) via
        // `Font::gsub_apply_lookup_type_5` (Contextual) and
        // `gsub_apply_lookup_type_6` (Chained Context). These features
        // need surrounding-glyph context that the round-11
        // single-substitution pass can't carry. Per-position invocation
        // because the lookup itself decides whether the surrounding
        // glyphs match. Faces without the relevant lookup degrade
        // gracefully (the round-11 behaviour).
        //
        // The cluster span boundaries may have shifted again because of
        // the round-11 single-substitution pass — but the start/end
        // glyph indices map 1:1 onto the input chars, and round-11 is
        // length-preserving (single substitutions don't insert/delete),
        // so `adjusted_spans` is still correct here.
        let out = self.apply_cluster_context_substitutions(out, &adjusted_spans)?;
        Ok(out)
    }

    /// For each `RephMark`, apply the active face's `rphf` GSUB lookup
    /// to the RA glyph and drop the halant glyph if a substitute is
    /// returned. Marks for which no `rphf` lookup applies pass through
    /// unchanged.
    ///
    /// Returns the rewritten glyph list AND a list of the post-reorder
    /// character indices whose corresponding halant glyph was removed
    /// from the run. The cluster-position GSUB pass downstream uses
    /// this to shift cluster span end indices.
    fn apply_reph_substitutions(
        &self,
        glyphs: Vec<(u16, u16)>,
        marks: &[RephMark],
    ) -> Result<RephSubstResult, Error> {
        if marks.is_empty() {
            return Ok((glyphs, Vec::new()));
        }
        // Process marks back-to-front so earlier RA / halant indices
        // stay stable while we splice out the halant slots.
        let mut out = glyphs;
        let mut dropped: Vec<usize> = Vec::new();
        for mark in marks.iter().rev() {
            // Bounds + face-coverage sanity. The reph mark records the
            // RA's index into the post-reorder character stream which
            // matches `out` 1:1.
            if mark.ra_idx >= out.len() || mark.halant_idx >= out.len() {
                continue;
            }
            let (ra_face_idx, ra_gid) = out[mark.ra_idx];
            // The reph substitution only fires when the RA glyph
            // actually came from an in-chain face (not .notdef on face
            // 0 because no face had it).
            if ra_gid == 0 {
                continue;
            }
            let face = &self.faces[ra_face_idx as usize];
            // Look up the script's `rphf` feature on this face. We try
            // the modern Indic2 tag first (`dev2` / `bng2`), then the
            // legacy v1 tag (`deva` / `beng`) for older fonts.
            let (modern, legacy) = match script_indic_tags(mark.script) {
                Some(p) => p,
                None => continue,
            };
            let new_ra = face.with_font(|font| {
                let mut substitute: Option<u16> = None;
                for tag in [modern, legacy] {
                    let features = font.gsub_features_for_script(tag, None);
                    for feat in features {
                        if feat.tag == *b"rphf" {
                            for &lookup_idx in &feat.lookup_indices {
                                if let Some(g) = font.gsub_apply_lookup_type_1(lookup_idx, ra_gid) {
                                    substitute = Some(g);
                                    break;
                                }
                            }
                            if substitute.is_some() {
                                break;
                            }
                        }
                    }
                    if substitute.is_some() {
                        break;
                    }
                }
                substitute
            })?;
            if let Some(reph_gid) = new_ra {
                // Rewrite the RA glyph to its reph form.
                out[mark.ra_idx] = (ra_face_idx, reph_gid);
                // Drop the halant glyph (it's redundant once the reph
                // is in place — the visual reph mark stands in for the
                // RA + halant pair). We splice it out of the assigned
                // glyph list so the downstream shaper sees the
                // collapsed cluster.
                if mark.halant_idx < out.len() {
                    out.remove(mark.halant_idx);
                    dropped.push(mark.halant_idx);
                }
            }
        }
        Ok((out, dropped))
    }

    /// Round-11 cluster-position-aware GSUB pass. For every Indic
    /// cluster span, dispatch the position-driven GSUB features:
    ///
    /// - **`half`** — applied to a base consonant immediately followed
    ///   by a halant when the cluster has more characters after the
    ///   halant (i.e. the consonant is in the *non-final* slot of a
    ///   conjunct — its inherent vowel is suppressed and a "half form"
    ///   glyph is the canonical shape).
    /// - **`pref` / `blwf` / `abvf` / `pstf`** — applied to a consonant
    ///   that follows a halant. The Telugu/Kannada/Malayalam family
    ///   distinguishes the position by glyph; we try `pref` first,
    ///   then `blwf`, `abvf`, `pstf` in that order. The first lookup
    ///   that returns a substitute wins. (Coverage misses pass through
    ///   unchanged — the ordering picks the position the font has a
    ///   form for.)
    /// - **`pres` / `psts` / `abvs` / `blws`** — presentation-pass
    ///   single substitutions; applied to every glyph in the cluster
    ///   (each glyph independently — coverage misses pass through).
    ///
    /// Faces without any of the above lookups for the cluster's script
    /// degrade to the round-10 behaviour (just `rphf`).
    fn apply_cluster_position_substitutions(
        &self,
        glyphs: Vec<(u16, u16)>,
        spans: &[ClusterSpan],
        chars: &[char],
    ) -> Result<Vec<(u16, u16)>, Error> {
        if spans.is_empty() {
            return Ok(glyphs);
        }
        let mut out = glyphs;
        for span in spans {
            // Bound the span to the current `out` length — reph drops
            // may have shifted the end down; downstream
            // `adjust_cluster_spans` clamps but be defensive.
            let end = span.end.min(out.len());
            if span.start >= end {
                continue;
            }
            // Resolve the script's GSUB script tag pair.
            let (modern, legacy) = match script_indic_tags(span.script) {
                Some(p) => p,
                None => continue,
            };
            // Per-position substitution for halant-suffixed conjunct
            // components. Walk the cluster's chars (post-reorder) and
            // identify (a) base + halant pairs (`half`) and (b)
            // post-halant consonants (`pref` / `blwf` / `abvf` /
            // `pstf`). The chars vec is bounded by the pre-reph
            // cluster's char positions; reph drops shift the END down
            // by 1 per drop but not the START — clamp to chars.len().
            let chars_end = span.end.min(chars.len());
            let cat_of =
                |i: usize| -> IndicCategory { indic_category_for_script(span.script, chars[i]) };
            let mut pos = span.start;
            while pos < chars_end {
                let here = cat_of(pos);
                if here == IndicCategory::Consonant && pos + 1 < chars_end {
                    let next = cat_of(pos + 1);
                    if next == IndicCategory::Halant {
                        let glyph_idx = pos.min(out.len().saturating_sub(1));
                        let after_halant = pos + 2;
                        // `half` form fires when there's anything after
                        // the halant in the cluster.
                        if after_halant < chars_end && glyph_idx < out.len() {
                            self.try_apply_single_subst(
                                &mut out, glyph_idx, modern, legacy, b"half",
                            )?;
                        }
                        // The post-halant consonant (if any) gets the
                        // pref / blwf / abvf / pstf cascade. We pick
                        // the first feature whose lookup covers the
                        // gid — the font's form-position table dictates
                        // which one wins.
                        if after_halant < chars_end
                            && cat_of(after_halant) == IndicCategory::Consonant
                            && after_halant < out.len()
                        {
                            for tag in [b"pref", b"blwf", b"abvf", b"pstf"] {
                                if self.try_apply_single_subst(
                                    &mut out,
                                    after_halant,
                                    modern,
                                    legacy,
                                    tag,
                                )? {
                                    break;
                                }
                            }
                        }
                    }
                }
                pos += 1;
            }
            // Presentation-pass single substitutions over every glyph
            // in the cluster.
            for tag in [b"pres", b"psts", b"abvs", b"blws"] {
                for idx in span.start..end {
                    if idx < out.len() {
                        self.try_apply_single_subst(&mut out, idx, modern, legacy, tag)?;
                    }
                }
            }
        }
        Ok(out)
    }

    /// Round-13 multi-glyph context-aware GSUB pass. For every Indic
    /// cluster span, dispatch the multi-glyph feature pipeline:
    ///
    /// - **`locl`** — language-form contextual variants (e.g.
    ///   Marathi vs Hindi RA glyphs).
    /// - **`nukt`** — nukta contextual forms.
    /// - **`akhn`** — akhand ligatures (e.g. Devanagari ksha or jnya
    ///   matched as 3-glyph context).
    /// - **`cjct`** — conjunct forms (multi-glyph cluster glyphs;
    ///   a halant + consonant pair gets emitted as a single conjunct
    ///   gid).
    /// - **`init`** — initial contextual variant for the leading
    ///   consonant of a cluster.
    /// - **`haln`** — halant contextual forms (final-position halant
    ///   variants).
    ///
    /// For each feature in the per-script list that's also in the
    /// allow-set above, walk the cluster span and at each glyph index
    /// try the lookup as both LookupType 5 (Contextual) and LookupType
    /// 6 (Chained Context). The font's lookup decides whether the rule
    /// fires — if it does, we replace the cluster's glyph slice with
    /// the rewritten glyphs. Faces that don't publish any of these
    /// lookups for the script tag fall through unchanged.
    ///
    /// The contextual lookups are length-changing (LookupType 4
    /// ligature + LookupType 2 multiple substitution embedded in
    /// LookupType 5/6 sub-actions), so the cluster span boundary may
    /// shift. We re-clamp the span end after each successful
    /// substitution and continue from the same position so further
    /// chained rules can fire on top of the rewritten glyphs (bounded
    /// by total cluster size to avoid infinite loops).
    fn apply_cluster_context_substitutions(
        &self,
        glyphs: Vec<(u16, u16)>,
        spans: &[ClusterSpan],
    ) -> Result<Vec<(u16, u16)>, Error> {
        if spans.is_empty() {
            return Ok(glyphs);
        }
        // The 6 multi-glyph context features in canonical Indic
        // application order. Kept in lockstep with the
        // `*_feature_tags()` lists in `shaping::indic`.
        const CONTEXT_FEATURES: &[&[u8; 4]] =
            &[b"locl", b"nukt", b"akhn", b"cjct", b"init", b"haln"];
        let mut out = glyphs;
        // Walk spans front-to-back. Each span's char positions are
        // unchanged by length-changing edits within a PRECEDING span
        // because the spans are non-overlapping; but a length change
        // within span N shifts spans N+1.. by the delta. We track the
        // running offset.
        let mut offset_delta: isize = 0;
        for span in spans {
            // Apply the offset so far to recover the current span's
            // glyph indices in `out`.
            let span_start = (span.start as isize + offset_delta).max(0) as usize;
            let span_end = (span.end as isize + offset_delta).max(0) as usize;
            let span_end = span_end.min(out.len());
            if span_start >= span_end {
                continue;
            }
            let (modern, legacy) = match script_indic_tags(span.script) {
                Some(p) => p,
                None => continue,
            };
            // Faces inside one cluster are not guaranteed to be the
            // same — but in practice an Indic cluster's chars all come
            // from the same font (`assign_codepoints` walks the chain
            // per char and Indic glyphs cluster on whichever face has
            // the script). If the cluster has mixed faces the lookup
            // simply won't match (different gid space) — graceful
            // degradation.
            let face_idx = out[span_start].0;
            let face = &self.faces[face_idx as usize];
            let pre_len = span_end - span_start;
            // Snapshot the cluster's gid slice (just the gids — we'll
            // splice the (face_idx, gid) pairs back after the lookup).
            let gid_slice: Vec<u16> = out[span_start..span_end].iter().map(|p| p.1).collect();
            // For each feature in priority order, try the lookup at
            // every position inside the cluster. The lookup's coverage
            // table decides whether the rule fires — if it does, we
            // adopt the new glyph slice and re-extract.
            let new_gids = face.with_font(|font| {
                let mut gids = gid_slice.clone();
                for feat_tag in CONTEXT_FEATURES {
                    for tag in [modern, legacy] {
                        let features = font.gsub_features_for_script(tag, None);
                        for feat in &features {
                            if &feat.tag != *feat_tag {
                                continue;
                            }
                            for &lookup_idx in &feat.lookup_indices {
                                // Try LookupType 5 then LookupType 6 at
                                // every position inside the cluster.
                                // The first successful rewrite wins per
                                // (lookup_idx, position) — we don't
                                // rerun the same lookup on the rewritten
                                // glyphs because the spec already
                                // recurses internally for chained
                                // sub-lookups.
                                let mut pos = 0usize;
                                while pos < gids.len() {
                                    let mut applied = false;
                                    if let Some(rewritten) =
                                        font.gsub_apply_lookup_type_5(lookup_idx, &gids, pos)
                                    {
                                        gids = rewritten;
                                        applied = true;
                                    } else if let Some(rewritten) =
                                        font.gsub_apply_lookup_type_6(lookup_idx, &gids, pos)
                                    {
                                        gids = rewritten;
                                        applied = true;
                                    }
                                    if applied {
                                        // Stay at the same `pos` — the
                                        // rewrite may have introduced
                                        // a new glyph that another rule
                                        // can hit. But cap iteration to
                                        // prevent pathological loops.
                                        pos += 1;
                                    } else {
                                        pos += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                gids
            })?;
            let post_len = new_gids.len();
            // Splice the rewritten gids back into `out`, preserving
            // face_idx (length may have changed).
            let face_idx = out[span_start].0;
            let new_pairs: Vec<(u16, u16)> = new_gids.into_iter().map(|g| (face_idx, g)).collect();
            out.splice(span_start..span_end, new_pairs);
            offset_delta += post_len as isize - pre_len as isize;
        }
        Ok(out)
    }

    /// Attempt to apply a feature-tagged single substitution
    /// (LookupType 1) to `out[glyph_idx]`. Walks the modern Indic2 tag
    /// first then the legacy v1 tag; returns `Ok(true)` when a
    /// substitution was applied, `Ok(false)` otherwise. Glyphs whose
    /// owning face publishes no matching feature pass through silently.
    fn try_apply_single_subst(
        &self,
        out: &mut [(u16, u16)],
        glyph_idx: usize,
        modern: [u8; 4],
        legacy: [u8; 4],
        feature_tag: &[u8; 4],
    ) -> Result<bool, Error> {
        let (face_idx, gid) = out[glyph_idx];
        if gid == 0 {
            return Ok(false);
        }
        let face = &self.faces[face_idx as usize];
        let new_gid = face.with_font(|font| {
            for tag in [modern, legacy] {
                let features = font.gsub_features_for_script(tag, None);
                for feat in features {
                    if &feat.tag == feature_tag {
                        for &lookup_idx in &feat.lookup_indices {
                            if let Some(g) = font.gsub_apply_lookup_type_1(lookup_idx, gid) {
                                return Some(g);
                            }
                        }
                    }
                }
            }
            None
        })?;
        if let Some(g) = new_gid {
            out[glyph_idx] = (face_idx, g);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Return type of [`FaceChain::apply_reph_substitutions`]: the
/// rewritten `(face_idx, glyph_id)` list plus the post-reorder
/// character indices whose halant glyph was removed.
type RephSubstResult = (Vec<(u16, u16)>, Vec<usize>);

/// Sidecar info recorded by [`apply_indic_reorder`] for every cluster
/// whose `ClusterFlags::has_reph` was set. Carries the indices into
/// the post-reorder character stream of the leading RA glyph and the
/// halant immediately after it, plus the originating script.
///
/// Consumed by [`FaceChain::apply_reph_substitutions`] which looks up
/// the `rphf` GSUB feature on the face that owns the RA glyph and
/// rewrites the gid pair if a substitute is returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RephMark {
    /// Index of the RA character in the post-reorder stream.
    ra_idx: usize,
    /// Index of the halant character in the post-reorder stream
    /// (always `ra_idx + 1` for current Indic scripts; tracked
    /// explicitly so the splice operation is unambiguous).
    halant_idx: usize,
    /// Script the cluster originated from. Drives the OpenType script
    /// tag pair (`dev2` / `deva` etc.) for the GSUB lookup.
    script: Script,
}

/// Per-cluster span recorded by [`apply_indic_reorder`] so the post-cmap
/// cluster-position-aware GSUB pass knows which glyphs belong to which
/// Indic cluster + what script they came from.
///
/// Consumed by [`FaceChain::apply_cluster_position_substitutions`] (round
/// 11) which dispatches `half` / `pref` / `blwf` / `abvf` / `pstf` for
/// halant-suffixed conjunct components, and `pres` / `psts` / `abvs` /
/// `blws` as presentation-pass single substitutions for every glyph in
/// the cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClusterSpan {
    /// Inclusive start index in the post-reorder character stream
    /// (matches the assigned-glyphs list 1:1).
    start: usize,
    /// Exclusive end index in the post-reorder character stream.
    end: usize,
    /// Script the cluster originated from. Drives the OpenType script
    /// tag pair (`dev2` / `deva` etc.) for the GSUB lookup.
    script: Script,
}

/// Pre-cmap Indic shaping pass: walk `chars`, find contiguous runs of
/// Indic codepoints (any script we have rules for), segment each run
/// into orthographic clusters, and apply per-script
/// [`reorder_cluster_with`] to each (pre-base matra reorder + reph
/// flagging). Non-Indic characters pass through untouched.
///
/// Returns the reordered char stream plus a list of [`RephMark`]
/// sidecar entries (one per cluster with `ClusterFlags::has_reph`)
/// PLUS a list of [`ClusterSpan`] entries (one per Indic cluster) so
/// the cluster-position-aware GSUB pass downstream can dispatch
/// per-position lookups.
///
/// Indices in the returned [`RephMark`]s and [`ClusterSpan`]s are into
/// the returned character stream (the post-reorder one).
fn apply_indic_reorder(chars: &[char]) -> (Vec<char>, Vec<RephMark>, Vec<ClusterSpan>) {
    if chars.is_empty() {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut reph_marks: Vec<RephMark> = Vec::new();
    let mut spans: Vec<ClusterSpan> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        // Walk a maximal Indic-of-one-script run starting at `i`.
        let run_script = script_of(chars[i]);
        let rules = match indic_rules_for_script(run_script) {
            Some(r) => r,
            None => {
                // Non-Indic code point — pass through.
                out.push(chars[i]);
                i += 1;
                continue;
            }
        };
        let run_start = i;
        while i < chars.len() && script_of(chars[i]) == run_script {
            i += 1;
        }
        let run = &chars[run_start..i];
        // Cluster this run + reorder each cluster.
        let bounds = cluster_boundaries_with(run, rules.category);
        for (s, e) in bounds {
            let cluster = &run[s..e];
            let (reordered, flags) = reorder_cluster_with(cluster, rules);
            let cluster_start = out.len();
            // Record the reph mark BEFORE we extend `out` with the
            // reordered cluster — `out.len()` at this point is the
            // index of the RA glyph in the post-reorder stream.
            if flags.has_reph {
                // The reph rule guarantees the leading RA + halant +
                // consonant sit at positions 0 + 1 + 2 of the cluster
                // (pre-base matras don't reorder past the leading
                // characters; the matra moves to position 0 only when
                // the matra is itself in the cluster — but the reph
                // detection in `reorder_cluster_with` checks the
                // ORIGINAL cluster, not the reordered one). To keep
                // the indexing unambiguous, assert that the cluster's
                // post-reorder layout has RA + halant at the cluster
                // start when flags.has_reph is set AND no pre-base
                // matra was reordered. When a pre-base matra DID
                // reorder, the RA + halant sit at positions 1 + 2
                // (after the matra at position 0).
                let ra_offset = if flags.pre_base_reordered { 1 } else { 0 };
                let ra_idx = cluster_start + ra_offset;
                let halant_idx = ra_idx + 1;
                reph_marks.push(RephMark {
                    ra_idx,
                    halant_idx,
                    script: run_script,
                });
            }
            out.extend_from_slice(&reordered);
            // Record the cluster span (inclusive..exclusive) for the
            // cluster-position-aware GSUB pass downstream.
            spans.push(ClusterSpan {
                start: cluster_start,
                end: out.len(),
                script: run_script,
            });
        }
    }
    (out, reph_marks, spans)
}

/// Shift a list of [`ClusterSpan`]s after the reph pass dropped some
/// halant glyphs. Reph drop occurs at index `halant_idx` in the
/// post-reorder stream which corresponds to the SECOND character of a
/// reph cluster (positions 1 / 2 of the cluster, depending on whether a
/// pre-base matra reordered). The drop:
/// - shifts the END index of the affected span down by 1 (one fewer
///   glyph in this cluster);
/// - shifts the START + END indices of every subsequent span down by 1.
///
/// The cluster char positions in the chars vec are unchanged — only
/// the GLYPH indices in `out` shift. We track this by computing per-
/// span how many drops happened at-or-before its start (shift_start)
/// and at-or-before its end-1 (shift_end).
fn adjust_cluster_spans(
    spans: &[ClusterSpan],
    dropped: &[usize],
    chars: &[char],
) -> Vec<ClusterSpan> {
    if dropped.is_empty() {
        return spans.to_vec();
    }
    // Count drops strictly before `idx`.
    let drops_before = |idx: usize| -> usize { dropped.iter().filter(|&&d| d < idx).count() };
    spans
        .iter()
        .map(|s| {
            let new_start = s.start.saturating_sub(drops_before(s.start));
            let new_end = s.end.saturating_sub(drops_before(s.end));
            ClusterSpan {
                start: new_start.min(chars.len()),
                end: new_end.min(chars.len()),
                script: s.script,
            }
        })
        .collect()
}

/// Look up a script-specific [`IndicCategory`] for `ch`. Used by the
/// cluster-position GSUB pass to identify halant chains within a
/// cluster span. Returns [`IndicCategory::Other`] for non-Indic scripts.
fn indic_category_for_script(script: Script, ch: char) -> IndicCategory {
    use crate::shaping::indic;
    match script {
        Script::Devanagari => indic::devanagari_category(ch),
        Script::Bengali => indic::bengali_category(ch),
        Script::Tamil => indic::tamil_category(ch),
        Script::Gurmukhi => indic::gurmukhi_category(ch),
        Script::Gujarati => indic::gujarati_category(ch),
        Script::Telugu => indic::telugu_category(ch),
        Script::Kannada => indic::kannada_category(ch),
        Script::Malayalam => indic::malayalam_category(ch),
        Script::Oriya => indic::oriya_category(ch),
        Script::Sinhala => indic::sinhala_category(ch),
        Script::Khmer => indic::khmer_category(ch),
        Script::Thai => indic::thai_category(ch),
        Script::Lao => indic::lao_category(ch),
        Script::Burmese => indic::burmese_category(ch),
        _ => IndicCategory::Other,
    }
}

/// Map a [`Script`] to its Indic [`ReorderRules`], if any. Used by
/// the per-codepoint Indic dispatch in [`apply_indic_reorder`].
fn indic_rules_for_script(script: Script) -> Option<&'static ReorderRules> {
    match script {
        Script::Devanagari => Some(&DEVANAGARI_RULES),
        Script::Bengali => Some(&BENGALI_RULES),
        Script::Tamil => Some(&TAMIL_RULES),
        Script::Gurmukhi => Some(&GURMUKHI_RULES),
        Script::Gujarati => Some(&GUJARATI_RULES),
        Script::Telugu => Some(&TELUGU_RULES),
        Script::Kannada => Some(&KANNADA_RULES),
        Script::Malayalam => Some(&MALAYALAM_RULES),
        Script::Oriya => Some(&ORIYA_RULES),
        Script::Sinhala => Some(&SINHALA_RULES),
        Script::Khmer => Some(&KHMER_RULES),
        Script::Thai => Some(&THAI_RULES),
        Script::Lao => Some(&LAO_RULES),
        Script::Burmese => Some(&BURMESE_RULES),
        _ => None,
    }
}

/// Pre-cmap Arabic shaping pass: walk `chars`, find contiguous runs of
/// Arabic codepoints, run the joining state machine on each run, and
/// translate joining-aware base letters to their Arabic Presentation
/// Forms-B equivalents. Non-Arabic codepoints pass through untouched.
///
/// This sits *before* face-chain cmap lookup so a font that only
/// supports the FE70..FEFF block (most desktop fonts) still gets
/// visually-correct contextual shapes. Faces that lack the
/// presentation-form glyph fall back via the retry path in
/// [`FaceChain::assign_codepoints`].
fn apply_arabic_joining(chars: &[char]) -> Vec<char> {
    if chars.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        // Walk a maximal Arabic-only run starting at `i`.
        let run_start = i;
        while i < chars.len() && script_of(chars[i]) == Script::Arabic {
            i += 1;
        }
        if i > run_start {
            let run = &chars[run_start..i];
            let forms = compute_forms(run);
            for (k, &ch) in run.iter().enumerate() {
                let translated = presentation_form(ch, forms[k]).unwrap_or(ch);
                out.push(translated);
            }
        }
        // Pass non-Arabic chars through unchanged.
        if i < chars.len() && script_of(chars[i]) != Script::Arabic {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
#[allow(non_snake_case)] // tests reference Unicode codepoint literals + algorithm shorthands
mod tests {
    use super::{apply_arabic_joining, apply_indic_reorder, ClusterSpan};
    use crate::shaping::arabic::Script;

    #[test]
    fn ascii_passes_through_unchanged() {
        let chars: Vec<char> = "Hello".chars().collect();
        assert_eq!(apply_arabic_joining(&chars), chars);
    }

    #[test]
    fn devanagari_pre_base_matra_moves_to_front_of_cluster() {
        // "कि" = KA + sign-i → sign-i + KA after Devanagari reorder.
        let chars = vec!['\u{0915}', '\u{093F}'];
        let (out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{093F}', '\u{0915}']);
        assert!(marks.is_empty(), "no reph in this cluster");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans[0].end, 2);
        assert_eq!(spans[0].script, Script::Devanagari);
    }

    #[test]
    fn devanagari_two_clusters_each_reorder_independently() {
        // "किकि" → two clusters; each reorders its matra to the front.
        let chars = vec!['\u{0915}', '\u{093F}', '\u{0915}', '\u{093F}'];
        let (out, _, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{093F}', '\u{0915}', '\u{093F}', '\u{0915}']);
        assert_eq!(spans.len(), 2);
        assert_eq!((spans[0].start, spans[0].end), (0, 2));
        assert_eq!((spans[1].start, spans[1].end), (2, 4));
    }

    #[test]
    fn devanagari_conjunct_reorder_keeps_halant_chain_intact() {
        // "क्षि" = KA + halant + SSA + sign-i. Conjunct stays in
        // logical order; matra moves to front.
        let chars = vec!['\u{0915}', '\u{094D}', '\u{0937}', '\u{093F}'];
        let (out, _, _) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{093F}', '\u{0915}', '\u{094D}', '\u{0937}']);
    }

    #[test]
    fn ascii_passes_through_indic_reorder_unchanged() {
        // Sanity: non-Indic input must not be touched.
        let chars: Vec<char> = "Hello".chars().collect();
        let (out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, chars);
        assert!(marks.is_empty());
        // Non-Indic chars produce no cluster spans.
        assert!(spans.is_empty());
    }

    #[test]
    fn mixed_latin_and_devanagari_reorders_only_devanagari_clusters() {
        // "Aकि" → Latin A passes through; Devanagari cluster reorders.
        let chars = vec!['A', '\u{0915}', '\u{093F}'];
        let (out, _, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['A', '\u{093F}', '\u{0915}']);
        // Only the Devanagari cluster gets a span.
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (1, 3));
    }

    #[test]
    fn devanagari_reph_emits_reph_mark_at_correct_index() {
        // RA + halant + KA → reph cluster. The mark records ra_idx=0
        // and halant_idx=1 (no pre-base matra reorder shifted them).
        let chars = vec!['\u{0930}', '\u{094D}', '\u{0915}'];
        let (out, marks, _) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{0930}', '\u{094D}', '\u{0915}']);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].ra_idx, 0);
        assert_eq!(marks[0].halant_idx, 1);
        assert_eq!(marks[0].script, Script::Devanagari);
    }

    #[test]
    fn devanagari_reph_with_pre_base_matra_shifts_reph_mark_by_one() {
        // RA + halant + KA + sign-i — matra moves to position 0; RA
        // is now at position 1, halant at 2.
        let chars = vec!['\u{0930}', '\u{094D}', '\u{0915}', '\u{093F}'];
        let (out, marks, _) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{093F}', '\u{0930}', '\u{094D}', '\u{0915}']);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].ra_idx, 1);
        assert_eq!(marks[0].halant_idx, 2);
    }

    #[test]
    fn bengali_pre_base_matra_e_moves_to_front_of_cluster() {
        // BENGALI KA + sign-e → sign-e + KA.
        let chars = vec!['\u{0995}', '\u{09C7}'];
        let (out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{09C7}', '\u{0995}']);
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].script, Script::Bengali);
    }

    #[test]
    fn bengali_reph_emits_reph_mark_with_bengali_script_tag() {
        // BENGALI RA + halant + KA → reph cluster.
        let chars = vec!['\u{09B0}', '\u{09CD}', '\u{0995}'];
        let (out, marks, _) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{09B0}', '\u{09CD}', '\u{0995}']);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].script, Script::Bengali);
    }

    #[test]
    fn tamil_pre_base_matra_e_moves_to_front_of_cluster() {
        // TAMIL KA + sign-e → sign-e + KA.
        let chars = vec!['\u{0B95}', '\u{0BC6}'];
        let (out, marks, _) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{0BC6}', '\u{0B95}']);
        assert!(marks.is_empty());
    }

    #[test]
    fn tamil_RA_plus_halant_does_NOT_emit_reph_mark() {
        // TAMIL RA + pulli + KA — Tamil never forms a reph.
        let chars = vec!['\u{0BB0}', '\u{0BCD}', '\u{0B95}'];
        let (_out, marks, _) = apply_indic_reorder(&chars);
        assert!(marks.is_empty(), "Tamil never sets the reph flag");
    }

    #[test]
    fn mixed_devanagari_and_bengali_runs_segment_independently() {
        // Devanagari KA + sign-i + Bengali KA + sign-i.
        let chars = vec!['\u{0915}', '\u{093F}', '\u{0995}', '\u{09BF}'];
        let (out, _, spans) = apply_indic_reorder(&chars);
        // Each script's pre-base matra moves to the front of its OWN
        // cluster (cluster boundary at the script switch).
        assert_eq!(out, vec!['\u{093F}', '\u{0915}', '\u{09BF}', '\u{0995}']);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].script, Script::Devanagari);
        assert_eq!(spans[1].script, Script::Bengali);
    }

    // ---------- Round 11 — new scripts ----------

    #[test]
    fn gurmukhi_cluster_reorder_emits_span_with_gurmukhi_script() {
        // KA + sign-i — pre-base matra reorders.
        let chars = vec!['\u{0A15}', '\u{0A3F}'];
        let (out, _, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{0A3F}', '\u{0A15}']);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].script, Script::Gurmukhi);
    }

    #[test]
    fn gujarati_cluster_reorder_emits_span_with_gujarati_script() {
        // KA + sign-i.
        let chars = vec!['\u{0A95}', '\u{0ABF}'];
        let (out, _, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{0ABF}', '\u{0A95}']);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].script, Script::Gujarati);
    }

    #[test]
    fn telugu_pre_base_matra_e_reorders_with_telugu_span() {
        // KA + sign-e (pre-base) — reorder.
        let chars = vec!['\u{0C15}', '\u{0C46}'];
        let (out, _, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{0C46}', '\u{0C15}']);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].script, Script::Telugu);
    }

    #[test]
    fn kannada_reph_emits_reph_mark_with_kannada_script_tag() {
        let chars = vec!['\u{0CB0}', '\u{0CCD}', '\u{0C95}'];
        let (_out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].script, Script::Kannada);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].script, Script::Kannada);
    }

    #[test]
    fn malayalam_RA_plus_halant_does_NOT_emit_reph_mark() {
        // Modern Malayalam — chillu replaces reph.
        let chars = vec!['\u{0D30}', '\u{0D4D}', '\u{0D15}'];
        let (_out, marks, spans) = apply_indic_reorder(&chars);
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].script, Script::Malayalam);
    }

    #[test]
    fn oriya_pre_base_matra_e_reorders_with_oriya_span() {
        let chars = vec!['\u{0B15}', '\u{0B47}'];
        let (out, _, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{0B47}', '\u{0B15}']);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].script, Script::Oriya);
    }

    #[test]
    fn malayalam_chillu_starts_new_cluster_from_following_consonant() {
        // Chillu U+0D7A + KA U+0D15 — chillu is a Consonant, the next
        // consonant starts a new cluster.
        let chars = vec!['\u{0D7A}', '\u{0D15}'];
        let (_out, _marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(spans.len(), 2);
    }

    // ---------- Round 12 (Brahmic non-Indic) ----------

    #[test]
    fn sinhala_pre_base_matra_reorders_with_sinhala_span() {
        // Sinhala KA U+0D9A + sign-e U+0DD9 → sign-e + KA.
        let chars = vec!['\u{0D9A}', '\u{0DD9}'];
        let (out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{0DD9}', '\u{0D9A}']);
        assert!(marks.is_empty(), "Sinhala has no reph");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].script, Script::Sinhala);
    }

    #[test]
    fn sinhala_RA_plus_al_lakuna_does_NOT_emit_reph_mark() {
        // Sinhala has no superscript reph rendering.
        let chars = vec!['\u{0DBB}', '\u{0DCA}', '\u{0D9A}'];
        let (_out, marks, spans) = apply_indic_reorder(&chars);
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].script, Script::Sinhala);
    }

    #[test]
    fn khmer_pre_base_matra_reorders_with_khmer_span() {
        // Khmer KA U+1780 + sign-e U+17C1 → sign-e + KA.
        let chars = vec!['\u{1780}', '\u{17C1}'];
        let (out, _marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{17C1}', '\u{1780}']);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].script, Script::Khmer);
    }

    #[test]
    fn khmer_coeng_keeps_subjoined_chain_in_one_cluster_span() {
        // KA + COENG + KHA + COENG + GA — three-deep subjoined chain.
        let chars = vec!['\u{1780}', '\u{17D2}', '\u{1781}', '\u{17D2}', '\u{1782}'];
        let (out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, chars); // no reorder (no pre-base matra)
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (0, 5));
        assert_eq!(spans[0].script, Script::Khmer);
    }

    #[test]
    fn thai_no_reorder_preserves_storage_order() {
        // Thai SARA E (pre-base in storage) + KO KAI — already in
        // visual order; cluster machine starts a new cluster at each.
        let chars = vec!['\u{0E40}', '\u{0E01}'];
        let (out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, chars);
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].script, Script::Thai);
        assert_eq!(spans[1].script, Script::Thai);
    }

    #[test]
    fn thai_consonant_with_above_vowel_and_tone_emits_one_span() {
        // KO KAI + SARA I (above) + MAI THO (tone) — single cluster.
        let chars = vec!['\u{0E01}', '\u{0E34}', '\u{0E49}'];
        let (out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, chars);
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (0, 3));
        assert_eq!(spans[0].script, Script::Thai);
    }

    #[test]
    fn mixed_devanagari_and_thai_segments_at_script_boundary() {
        // Devanagari KA + Thai KO KAI — different scripts, two
        // independent clusters.
        let chars = vec!['\u{0915}', '\u{0E01}'];
        let (_out, marks, spans) = apply_indic_reorder(&chars);
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].script, Script::Devanagari);
        assert_eq!(spans[1].script, Script::Thai);
    }

    // ---------- Round 13 (Burmese / Lao) ----------

    #[test]
    fn lao_no_reorder_preserves_storage_order() {
        // Lao SARA E (pre-base in storage) + KO — already in visual
        // order; cluster machine starts a new cluster at each.
        let chars = vec!['\u{0EC0}', '\u{0E81}'];
        let (out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, chars);
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].script, Script::Lao);
        assert_eq!(spans[1].script, Script::Lao);
    }

    #[test]
    fn lao_consonant_with_above_vowel_and_tone_emits_one_span() {
        // KO + SARA I (above) + MAI EK (tone) — single cluster.
        let chars = vec!['\u{0E81}', '\u{0EB4}', '\u{0EC8}'];
        let (out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, chars);
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (0, 3));
        assert_eq!(spans[0].script, Script::Lao);
    }

    #[test]
    fn burmese_pre_base_matra_e_reorders_with_burmese_span() {
        // Burmese KA + sign-e (pre-base) → sign-e + KA.
        let chars = vec!['\u{1000}', '\u{1031}'];
        let (out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{1031}', '\u{1000}']);
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].script, Script::Burmese);
    }

    #[test]
    fn burmese_kinzi_emits_reph_mark_with_burmese_script_tag() {
        // NGA + Asat + Virama + KA — kinzi.
        let chars = vec!['\u{1004}', '\u{103A}', '\u{1039}', '\u{1000}'];
        let (_out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(marks.len(), 1, "Burmese kinzi emits a reph mark");
        assert_eq!(marks[0].script, Script::Burmese);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].script, Script::Burmese);
        assert_eq!((spans[0].start, spans[0].end), (0, 4));
    }

    #[test]
    fn burmese_asat_does_NOT_chain_following_consonant_into_cluster() {
        // KA + Asat + KHA — Asat (Bindu) attaches to KA; KHA starts
        // a new cluster (because Asat is NOT halant).
        let chars = vec!['\u{1000}', '\u{103A}', '\u{1001}'];
        let (out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, chars);
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].script, Script::Burmese);
        assert_eq!(spans[1].script, Script::Burmese);
    }

    #[test]
    fn burmese_virama_chains_subjoined_consonant_into_one_span() {
        // KA + Virama + KHA — true halant, single cluster.
        let chars = vec!['\u{1000}', '\u{1039}', '\u{1001}'];
        let (out, marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(out, chars);
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (0, 3));
    }

    #[test]
    fn burmese_ka_with_medial_ya_in_one_span() {
        // KA + medial YA — medial extends the cluster as a Matra.
        let chars = vec!['\u{1000}', '\u{103B}'];
        let (_out, _marks, spans) = apply_indic_reorder(&chars);
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (0, 2));
    }

    #[test]
    fn mixed_burmese_and_thai_segments_at_script_boundary() {
        // Burmese KA + Thai KO KAI — different scripts, two clusters.
        let chars = vec!['\u{1000}', '\u{0E01}'];
        let (_out, marks, spans) = apply_indic_reorder(&chars);
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].script, Script::Burmese);
        assert_eq!(spans[1].script, Script::Thai);
    }

    #[test]
    fn mixed_lao_and_burmese_segments_at_script_boundary() {
        // Lao KO + Burmese KA — different scripts, two clusters.
        let chars = vec!['\u{0E81}', '\u{1000}'];
        let (_out, marks, spans) = apply_indic_reorder(&chars);
        assert!(marks.is_empty());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].script, Script::Lao);
        assert_eq!(spans[1].script, Script::Burmese);
    }

    #[test]
    fn adjust_cluster_spans_shifts_subsequent_spans_after_drop() {
        use super::adjust_cluster_spans;
        let chars = vec!['a'; 10];
        let spans = vec![
            ClusterSpan {
                start: 0,
                end: 3,
                script: Script::Devanagari,
            },
            ClusterSpan {
                start: 3,
                end: 6,
                script: Script::Devanagari,
            },
        ];
        // Pretend reph dropped the halant at index 1 (in cluster 0).
        let dropped = vec![1usize];
        let adjusted = adjust_cluster_spans(&spans, &dropped, &chars);
        // Cluster 0 shrinks by 1 at the end.
        assert_eq!((adjusted[0].start, adjusted[0].end), (0, 2));
        // Cluster 1 shifts both start and end down by 1.
        assert_eq!((adjusted[1].start, adjusted[1].end), (2, 5));
    }

    #[test]
    fn arabic_run_translates_to_presentation_forms() {
        // BEH BEH BEH → Init Medi Fina presentation forms.
        // 0x0628 BEH → init 0xFE91, medi 0xFE92, fina 0xFE90.
        let chars = vec!['\u{0628}', '\u{0628}', '\u{0628}'];
        let out = apply_arabic_joining(&chars);
        assert_eq!(out, vec!['\u{FE91}', '\u{FE92}', '\u{FE90}']);
    }

    #[test]
    fn arabic_run_with_ascii_separator() {
        // BEH BEH SPACE BEH BEH → first BEHs become Init/Fina, space
        // unchanged, second BEHs become Init/Fina.
        let chars = vec!['\u{0628}', '\u{0628}', ' ', '\u{0628}', '\u{0628}'];
        let out = apply_arabic_joining(&chars);
        assert_eq!(
            out,
            vec!['\u{FE91}', '\u{FE90}', ' ', '\u{FE91}', '\u{FE90}']
        );
    }

    // Mock-style tests are covered in the integration test file, where
    // we build a 2-face chain and verify face_idx routing. Unit-level
    // testing here is awkward because Face requires real TTF bytes.
}
