//! Round 15 — general-script (Latin / Cyrillic / Greek / DFLT) GSUB
//! feature pass. Wires `ccmp` (Glyph Composition / Decomposition) and
//! `calt` (Contextual Alternates) into the post-cmap path of
//! [`crate::shaper::shape_run_with_font`] for fonts whose `latn` /
//! `cyrl` / `grek` / `DFLT` script tables publish those features.
//!
//! ## Why this exists
//!
//! The OpenType spec (chapter 5 — "OpenType Layout common table
//! formats", "Required Features") lists `ccmp` as a feature that
//! shapers MUST apply at the start of the GSUB pass for every script.
//! `ccmp` is the canonical site for fonts to decompose a precomposed
//! codepoint into base + combining sequence (or compose the other way)
//! so the rest of the pipeline can attach marks correctly. The
//! round-1..14 pipeline ignored `ccmp` entirely, which meant any font
//! relying on its `ccmp` decomposition for diacritic placement (Inter
//! has 7 such lookups; DejaVu has 2; Noto Sans Arabic has 1) was
//! shaping incorrectly. Same for `calt`: Latin "ct" / "st" historical
//! ligatures, fraction collapsing, swash variants in display faces —
//! all live in `calt` and need contextual-substitution dispatch
//! (GSUB LookupType 5 / 6) which the existing `lookup_ligature` walker
//! cannot reach.
//!
//! ## Application order
//!
//! The OpenType spec doesn't enumerate a per-script feature application
//! order beyond "required features first" — most production shapers
//! apply features in the order they're declared by the font, but
//! `ccmp` is always first (required-feature semantics) and `calt`
//! always after `liga` / `clig` (contextual alternates often refine
//! the output of the ligature pass). We follow the same convention:
//!
//!   1. `ccmp` — pre-substitution decomposition / composition.
//!   2. `liga` / `clig` — handled by the existing `lookup_ligature`
//!      walker in [`crate::shaper::shape_run_with_font`].
//!   3. `calt` — post-ligature contextual refinement.
//!
//! Lookups are dispatched via the appropriate type entry point —
//! [`oxideav_ttf::Font::gsub_apply_lookup_type_1`] for single
//! substitution, type 2 for multiple (decomposition), type 4 for
//! ligature, type 5 / 6 for contextual / chained context. A lookup
//! whose declared type isn't one of those (e.g. GPOS-only) is silently
//! skipped — `oxideav-ttf` returns `None` for the wrong type and the
//! caller keeps the input glyph unchanged.
//!
//! ## Script auto-detection
//!
//! `shape_run_with_font` doesn't have access to the original codepoints
//! (the caller already cmap'd them), so script detection at the GSUB-
//! application stage isn't possible without a side channel. We instead
//! probe a fixed-priority list of script tags — `latn`, `cyrl`, `grek`,
//! `DFLT` — and apply every `ccmp` / `calt` feature the font publishes
//! under the FIRST tag that has any matching features. The lookup's own
//! coverage table decides per-glyph whether a substitution fires, so
//! probing a script tag the run doesn't actually use is benign: the
//! coverage table won't match the input glyph and the lookup returns
//! `None`. Indic and Arabic shaping uses different script tags
//! (`dev2` / `deva` / `arab`) and is dispatched from the per-script
//! pipelines in [`crate::face_chain`] before this pass runs.
//!
//! Source: the project-vendored OpenType GSUB chapter
//! (`docs/text/opentype/otspec-gsub.html`) and the common-layout-tables
//! chapter (`docs/text/opentype/otspec-chapter2-common-layout-tables.html`)
//! for the required-feature / feature-ordering semantics, plus the
//! registered-feature catalogue under `docs/text/opentype/registries/`.

use oxideav_ttf::Font;

/// Apply `ccmp` (Glyph Composition / Decomposition) substitutions to
/// `gids` for whichever of the probed general scripts (`latn`, `cyrl`,
/// `grek`, `DFLT`) the font has features under. Returns the rewritten
/// glyph run. Length may change — `ccmp` is the canonical
/// decomposition-and-composition site, so a `ç` decomposing into `c` +
/// combining-cedilla via a LookupType 2 multiple-substitution is exactly
/// what this pass enables.
///
/// **Idempotent.** Re-running this on already-shaped output is a no-op
/// because the lookups are designed to fire at most once per glyph
/// (coverage table excludes the post-substitution glyph).
pub fn apply_ccmp(font: &Font<'_>, gids: &[u16]) -> Vec<u16> {
    apply_feature(font, gids, b"ccmp")
}

/// Apply `calt` (Contextual Alternates) substitutions to `gids`.
/// Typically dispatched AFTER the round-1 ligature pass — `calt` rules
/// often refine the output of `liga` / `clig` (e.g. picking a wider
/// "ct" variant only when the input was the historical "ct" digraph).
///
/// See [`apply_ccmp`] for the script-tag probing strategy and the
/// no-op-when-absent contract.
pub fn apply_calt(font: &Font<'_>, gids: &[u16]) -> Vec<u16> {
    apply_feature(font, gids, b"calt")
}

/// Generic feature-tag dispatcher used by [`apply_ccmp`] / [`apply_calt`].
/// Walks the probed script tag list and applies every lookup the chosen
/// script publishes for `feature_tag`. Each lookup is dispatched
/// according to its declared GSUB LookupType — types 1, 2, 4, 5, 6
/// are all covered.
fn apply_feature(font: &Font<'_>, gids: &[u16], feature_tag: &[u8; 4]) -> Vec<u16> {
    if gids.is_empty() {
        return Vec::new();
    }
    // Probe the script tag list in priority order. The first tag whose
    // feature list contains a matching `feature_tag` wins — its lookups
    // are applied, the rest of the list is ignored. This matches
    // production shapers' "one script per run" semantics without
    // requiring scribe to know the codepoint script for each glyph.
    let script_tags: [[u8; 4]; 4] = [*b"latn", *b"cyrl", *b"grek", *b"DFLT"];
    let mut chosen_lookups: Vec<u16> = Vec::new();
    for tag in script_tags {
        let features = font.gsub_features_for_script(tag, None);
        for feat in features {
            if &feat.tag == feature_tag {
                chosen_lookups.extend_from_slice(&feat.lookup_indices);
            }
        }
        if !chosen_lookups.is_empty() {
            break;
        }
    }
    if chosen_lookups.is_empty() {
        return gids.to_vec();
    }

    // Resolve each chosen lookup's type once.
    let lookup_list = font.gsub_lookup_list();
    let lookup_type = |idx: u16| -> Option<u16> {
        lookup_list
            .iter()
            .find(|(i, _, _)| *i == idx)
            .map(|(_, ty, _)| *ty)
    };

    let mut current: Vec<u16> = gids.to_vec();
    for lookup_idx in chosen_lookups {
        let ty = match lookup_type(lookup_idx) {
            Some(t) => t,
            None => continue,
        };
        current = apply_one_lookup(font, lookup_idx, ty, &current);
    }
    current
}

/// Apply a single GSUB lookup of declared `ty` across every position in
/// `gids`. Returns the rewritten run.
///
/// Per-type semantics:
/// - **Type 1 (single)**: replace one glyph with one; length-preserving.
/// - **Type 2 (multiple)**: replace one glyph with N; length-changing.
/// - **Type 3 (alternate)**: pick alternate 0 (the default per spec —
///   user-driven indices live above this layer).
/// - **Type 4 (ligature)**: replace M glyphs with one; length-changing.
/// - **Type 5 (contextual)**: replace a window; length may change.
/// - **Type 6 (chained context)**: replace a window with
///   backtrack/lookahead; length may change.
///
/// The walker advances by one position when no substitution fires; when
/// a substitution fires it adopts the rewrite and re-examines the same
/// position so that a follow-on rule can match the new glyph. Iteration
/// is bounded by `4 * gids.len() + 8` to prevent pathological loops in
/// fonts whose chained-context rules might otherwise be self-feeding.
fn apply_one_lookup(font: &Font<'_>, lookup_idx: u16, ty: u16, gids: &[u16]) -> Vec<u16> {
    let mut current: Vec<u16> = gids.to_vec();
    if current.is_empty() {
        return current;
    }

    // Reverse-chaining contextual single substitution (LookupType 8)
    // must process the input right-to-left: per the GSUB chapter, "in
    // processing a reverse chaining substitution, i begins at the
    // logical end of the string and moves to the beginning." It is a
    // single-substitution lookup (one glyph → one glyph), so the run
    // length never changes and a back-to-front walk over fixed indices
    // is exact — a substitution made at a higher index is in place when
    // a lower index inspects it as part of its lookahead context, which
    // is the behaviour the reverse-processing rule exists to guarantee.
    // The forward `while pos` loop below cannot express this (it would
    // let an earlier position see a not-yet-substituted lookahead glyph
    // and fire when it should not), so type 8 is handled here.
    if ty == 8 {
        for pos in (0..current.len()).rev() {
            if let Some(rep) = font.gsub_apply_lookup_type_8(lookup_idx, &current, pos) {
                current[pos] = rep;
            }
        }
        return current;
    }

    let mut pos = 0usize;
    let mut iter_budget = current.len() * 4 + 8;
    while pos < current.len() && iter_budget > 0 {
        iter_budget -= 1;
        match ty {
            1 => {
                if let Some(rep) = font.gsub_apply_lookup_type_1(lookup_idx, current[pos]) {
                    current[pos] = rep;
                }
                pos += 1;
            }
            2 => {
                if let Some(seq) = font.gsub_apply_lookup_type_2(lookup_idx, current[pos]) {
                    let consumed = 1usize;
                    let new_len = seq.len();
                    current.splice(pos..pos + consumed, seq);
                    pos += new_len;
                } else {
                    pos += 1;
                }
            }
            3 => {
                // Alternate substitution — pick alternate index 0
                // (the default per spec; user-driven indices belong on
                // a higher API surface).
                if let Some(rep) = font.gsub_apply_lookup_type_3(lookup_idx, current[pos], 0) {
                    current[pos] = rep;
                }
                pos += 1;
            }
            4 => {
                if let Some((rep, consumed)) =
                    font.gsub_apply_lookup_type_4(lookup_idx, &current[pos..])
                {
                    if consumed >= 1 && pos + consumed <= current.len() {
                        current.splice(pos..pos + consumed, std::iter::once(rep));
                        pos += 1;
                    } else {
                        pos += 1;
                    }
                } else {
                    pos += 1;
                }
            }
            5 => {
                if let Some(rewritten) = font.gsub_apply_lookup_type_5(lookup_idx, &current, pos) {
                    current = rewritten;
                    pos += 1;
                } else {
                    pos += 1;
                }
            }
            6 => {
                if let Some(rewritten) = font.gsub_apply_lookup_type_6(lookup_idx, &current, pos) {
                    current = rewritten;
                    pos += 1;
                } else {
                    pos += 1;
                }
            }
            // LookupType 8 (reverse-chaining) is handled by the
            // right-to-left walk above and returns before this loop, so
            // it never reaches here.
            _ => {
                pos += 1;
            }
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty run is the empty run, no matter what the font publishes.
    #[test]
    fn empty_run_is_empty_for_both_ccmp_and_calt() {
        // Build a minimal `Font` view over the DejaVuSans bytes so the
        // probe runs against a real font's lookup list.
        let bytes = include_bytes!("../../tests/fixtures/DejaVuSans.ttf").to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("DejaVu Sans parses");
        face.with_font(|font| {
            assert_eq!(apply_ccmp(font, &[]).len(), 0);
            assert_eq!(apply_calt(font, &[]).len(), 0);
        })
        .unwrap();
    }

    /// Glyph 0 (`.notdef`) is in no coverage table, so both passes are
    /// identity for a `.notdef`-only run.
    #[test]
    fn notdef_only_run_passes_through() {
        let bytes = include_bytes!("../../tests/fixtures/DejaVuSans.ttf").to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("DejaVu Sans parses");
        face.with_font(|font| {
            let input = vec![0u16, 0u16, 0u16];
            assert_eq!(apply_ccmp(font, &input), input);
            assert_eq!(apply_calt(font, &input), input);
        })
        .unwrap();
    }
}
