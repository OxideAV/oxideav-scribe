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
    cluster_boundaries_with, reorder_cluster_with, script_indic_tags, ReorderRules, BENGALI_RULES,
    DEVANAGARI_RULES, TAMIL_RULES,
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
        let (shaped_chars, reph_marks) = apply_indic_reorder(&arabic_shaped);
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
        let out = self.apply_reph_substitutions(out, &reph_marks)?;
        Ok(out)
    }

    /// For each `RephMark`, apply the active face's `rphf` GSUB lookup
    /// to the RA glyph and drop the halant glyph if a substitute is
    /// returned. Marks for which no `rphf` lookup applies pass through
    /// unchanged.
    fn apply_reph_substitutions(
        &self,
        glyphs: Vec<(u16, u16)>,
        marks: &[RephMark],
    ) -> Result<Vec<(u16, u16)>, Error> {
        if marks.is_empty() {
            return Ok(glyphs);
        }
        // Process marks back-to-front so earlier RA / halant indices
        // stay stable while we splice out the halant slots.
        let mut out = glyphs;
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
                }
            }
        }
        Ok(out)
    }
}

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

/// Pre-cmap Indic shaping pass: walk `chars`, find contiguous runs of
/// Indic codepoints (Devanagari / Bengali / Tamil), segment each run
/// into orthographic clusters, and apply per-script
/// [`reorder_cluster_with`] to each (pre-base matra reorder + reph
/// flagging). Non-Indic characters pass through untouched.
///
/// Returns the reordered char stream plus a list of [`RephMark`]
/// sidecar entries — one per cluster whose `ClusterFlags::has_reph`
/// was set. The marks are then consumed by
/// [`FaceChain::apply_reph_substitutions`] which wires the actual
/// `rphf` GSUB substitution after cmap.
///
/// Indices in the returned [`RephMark`]s are into the returned
/// character stream (the post-reorder one).
fn apply_indic_reorder(chars: &[char]) -> (Vec<char>, Vec<RephMark>) {
    if chars.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut reph_marks: Vec<RephMark> = Vec::new();
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
                let ra_idx = out.len() + ra_offset;
                let halant_idx = ra_idx + 1;
                reph_marks.push(RephMark {
                    ra_idx,
                    halant_idx,
                    script: run_script,
                });
            }
            out.extend_from_slice(&reordered);
        }
    }
    (out, reph_marks)
}

/// Map a [`Script`] to its Indic [`ReorderRules`], if any. Used by
/// the per-codepoint Indic dispatch in [`apply_indic_reorder`].
fn indic_rules_for_script(script: Script) -> Option<&'static ReorderRules> {
    match script {
        Script::Devanagari => Some(&DEVANAGARI_RULES),
        Script::Bengali => Some(&BENGALI_RULES),
        Script::Tamil => Some(&TAMIL_RULES),
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
    use super::{apply_arabic_joining, apply_indic_reorder};
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
        let (out, marks) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{093F}', '\u{0915}']);
        assert!(marks.is_empty(), "no reph in this cluster");
    }

    #[test]
    fn devanagari_two_clusters_each_reorder_independently() {
        // "किकि" → two clusters; each reorders its matra to the front.
        let chars = vec!['\u{0915}', '\u{093F}', '\u{0915}', '\u{093F}'];
        let (out, _) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{093F}', '\u{0915}', '\u{093F}', '\u{0915}']);
    }

    #[test]
    fn devanagari_conjunct_reorder_keeps_halant_chain_intact() {
        // "क्षि" = KA + halant + SSA + sign-i. Conjunct stays in
        // logical order; matra moves to front.
        let chars = vec!['\u{0915}', '\u{094D}', '\u{0937}', '\u{093F}'];
        let (out, _) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{093F}', '\u{0915}', '\u{094D}', '\u{0937}']);
    }

    #[test]
    fn ascii_passes_through_indic_reorder_unchanged() {
        // Sanity: non-Indic input must not be touched.
        let chars: Vec<char> = "Hello".chars().collect();
        let (out, marks) = apply_indic_reorder(&chars);
        assert_eq!(out, chars);
        assert!(marks.is_empty());
    }

    #[test]
    fn mixed_latin_and_devanagari_reorders_only_devanagari_clusters() {
        // "Aकि" → Latin A passes through; Devanagari cluster reorders.
        let chars = vec!['A', '\u{0915}', '\u{093F}'];
        let (out, _) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['A', '\u{093F}', '\u{0915}']);
    }

    #[test]
    fn devanagari_reph_emits_reph_mark_at_correct_index() {
        // RA + halant + KA → reph cluster. The mark records ra_idx=0
        // and halant_idx=1 (no pre-base matra reorder shifted them).
        let chars = vec!['\u{0930}', '\u{094D}', '\u{0915}'];
        let (out, marks) = apply_indic_reorder(&chars);
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
        let (out, marks) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{093F}', '\u{0930}', '\u{094D}', '\u{0915}']);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].ra_idx, 1);
        assert_eq!(marks[0].halant_idx, 2);
    }

    #[test]
    fn bengali_pre_base_matra_e_moves_to_front_of_cluster() {
        // BENGALI KA + sign-e → sign-e + KA.
        let chars = vec!['\u{0995}', '\u{09C7}'];
        let (out, marks) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{09C7}', '\u{0995}']);
        assert!(marks.is_empty());
    }

    #[test]
    fn bengali_reph_emits_reph_mark_with_bengali_script_tag() {
        // BENGALI RA + halant + KA → reph cluster.
        let chars = vec!['\u{09B0}', '\u{09CD}', '\u{0995}'];
        let (out, marks) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{09B0}', '\u{09CD}', '\u{0995}']);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].script, Script::Bengali);
    }

    #[test]
    fn tamil_pre_base_matra_e_moves_to_front_of_cluster() {
        // TAMIL KA + sign-e → sign-e + KA.
        let chars = vec!['\u{0B95}', '\u{0BC6}'];
        let (out, marks) = apply_indic_reorder(&chars);
        assert_eq!(out, vec!['\u{0BC6}', '\u{0B95}']);
        assert!(marks.is_empty());
    }

    #[test]
    fn tamil_RA_plus_halant_does_NOT_emit_reph_mark() {
        // TAMIL RA + pulli + KA — Tamil never forms a reph.
        let chars = vec!['\u{0BB0}', '\u{0BCD}', '\u{0B95}'];
        let (_out, marks) = apply_indic_reorder(&chars);
        assert!(marks.is_empty(), "Tamil never sets the reph flag");
    }

    #[test]
    fn mixed_devanagari_and_bengali_runs_segment_independently() {
        // Devanagari KA + sign-i + Bengali KA + sign-i.
        let chars = vec!['\u{0915}', '\u{093F}', '\u{0995}', '\u{09BF}'];
        let (out, _) = apply_indic_reorder(&chars);
        // Each script's pre-base matra moves to the front of its OWN
        // cluster (cluster boundary at the script switch).
        assert_eq!(out, vec!['\u{093F}', '\u{0915}', '\u{09BF}', '\u{0995}']);
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
