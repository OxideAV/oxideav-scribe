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
use crate::shaping::indic::{cluster_boundaries, reorder_cluster};
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
        // Two pre-cmap shaping passes: Arabic joining → Devanagari
        // cluster reorder. The two scripts are disjoint so the order
        // doesn't matter for either one, but we run Arabic first to
        // match the round-7 behaviour exactly when no Devanagari is
        // present.
        let arabic_shaped = apply_arabic_joining(&chars);
        let shaped_chars = apply_devanagari_reorder(&arabic_shaped);
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
            // describes. The Devanagari pass only reorders (no
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
        Ok(out)
    }
}

/// Pre-cmap Devanagari shaping pass: walk `chars`, segment into Indic
/// clusters via [`cluster_boundaries`], and apply
/// [`reorder_cluster`] to each Devanagari cluster (pre-base matra
/// reorder + reph flagging). Non-Devanagari characters pass through
/// untouched.
///
/// Round 8 only handles the reorder; the reph glyph substitution (the
/// `rphf` GSUB feature) is deferred until `oxideav-ttf` exposes
/// feature-tagged single substitution.
fn apply_devanagari_reorder(chars: &[char]) -> Vec<char> {
    if chars.is_empty() {
        return Vec::new();
    }
    let bounds = cluster_boundaries(chars);
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    for (s, e) in bounds {
        let cluster = &chars[s..e];
        // Only rewrite clusters that contain at least one Devanagari
        // codepoint. Mixed Latin / non-Indic clusters fall through
        // identical (the cluster machine treats them as single-char
        // clusters anyway, so this is just a fast-path check).
        let has_devanagari = cluster
            .iter()
            .any(|&c| (0x0900..=0x097F).contains(&(c as u32)));
        if has_devanagari {
            let (reordered, _flags) = reorder_cluster(cluster);
            out.extend_from_slice(&reordered);
        } else {
            out.extend_from_slice(cluster);
        }
    }
    out
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
mod tests {
    use super::{apply_arabic_joining, apply_devanagari_reorder};

    #[test]
    fn ascii_passes_through_unchanged() {
        let chars: Vec<char> = "Hello".chars().collect();
        assert_eq!(apply_arabic_joining(&chars), chars);
    }

    #[test]
    fn devanagari_pre_base_matra_moves_to_front_of_cluster() {
        // "कि" = KA + sign-i → sign-i + KA after Devanagari reorder.
        let chars = vec!['\u{0915}', '\u{093F}'];
        let out = apply_devanagari_reorder(&chars);
        assert_eq!(out, vec!['\u{093F}', '\u{0915}']);
    }

    #[test]
    fn devanagari_two_clusters_each_reorder_independently() {
        // "किकि" → two clusters; each reorders its matra to the front.
        let chars = vec!['\u{0915}', '\u{093F}', '\u{0915}', '\u{093F}'];
        let out = apply_devanagari_reorder(&chars);
        assert_eq!(out, vec!['\u{093F}', '\u{0915}', '\u{093F}', '\u{0915}']);
    }

    #[test]
    fn devanagari_conjunct_reorder_keeps_halant_chain_intact() {
        // "क्षि" = KA + halant + SSA + sign-i. Conjunct stays in
        // logical order; matra moves to front.
        let chars = vec!['\u{0915}', '\u{094D}', '\u{0937}', '\u{093F}'];
        let out = apply_devanagari_reorder(&chars);
        assert_eq!(out, vec!['\u{093F}', '\u{0915}', '\u{094D}', '\u{0937}']);
    }

    #[test]
    fn ascii_passes_through_devanagari_reorder_unchanged() {
        // Sanity: non-Indic input must not be touched.
        let chars: Vec<char> = "Hello".chars().collect();
        assert_eq!(apply_devanagari_reorder(&chars), chars);
    }

    #[test]
    fn mixed_latin_and_devanagari_reorders_only_devanagari_clusters() {
        // "Aकि" → Latin A passes through; Devanagari cluster reorders.
        let chars = vec!['A', '\u{0915}', '\u{093F}'];
        let out = apply_devanagari_reorder(&chars);
        assert_eq!(out, vec!['A', '\u{093F}', '\u{0915}']);
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
