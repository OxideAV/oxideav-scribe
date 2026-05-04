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
    fn assign_codepoints(&self, text: &str) -> Result<Vec<(u16, u16)>, Error> {
        let mut out: Vec<(u16, u16)> = Vec::with_capacity(text.len());
        for ch in text.chars() {
            let mut found: Option<(u16, u16)> = None;
            for (idx, face) in self.faces.iter().enumerate() {
                let g = face.with_font(|font| font.glyph_index(ch))?;
                match g {
                    Some(gid) if gid != 0 => {
                        found = Some((idx as u16, gid));
                        break;
                    }
                    _ => continue,
                }
            }
            // No face had it — render as primary's .notdef.
            out.push(found.unwrap_or((0, 0)));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    // Mock-style tests are covered in the integration test file, where
    // we build a 2-face chain and verify face_idx routing. Unit-level
    // testing here is awkward because Face requires real TTF bytes.
}
