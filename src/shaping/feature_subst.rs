//! Round 89 — caller-driven GSUB LookupType 1 (Single Substitution)
//! feature application.
//!
//! Where the round-15 [`crate::shaping::general`] pass hard-codes the
//! two OpenType "always-on" features (`ccmp` + `calt`) into the run
//! pipeline, this module exposes the inverse capability: an
//! **explicit, user-provided feature list** — typical entries are the
//! display-toggled features `smcp` (small caps), `c2sc` (small caps
//! from caps), `case` (case-sensitive forms), `frac` (fractions),
//! `salt` (stylistic alternates), `ss01..ss20` (stylistic sets),
//! `sups` / `subs` (superscript / subscript), `numr` / `dnom`
//! (numerator / denominator), `ordn` (ordinal), `zero` (slashed
//! zero), `pnum` / `tnum` (proportional / tabular numerals), and the
//! `cv01..cv99` per-character variant family — and applies the
//! single-substitution lookups those features dispatch.
//!
//! ## What's wired
//!
//! - **GSUB LookupType 1 (Single Substitution)** only. The OpenType
//!   spec §6.2.1 (a.k.a. chapter 6 "GSUB - Glyph Substitution Table",
//!   section 2.1 "Single Substitution Subtable") defines two formats —
//!   **Format 1 (delta)** replaces every covered glyph by
//!   `gid + deltaGlyphID` (mod 65536), and **Format 2 (substitute-array)**
//!   replaces every covered glyph by the entry at the same coverage
//!   index in a `substituteGlyphIDs[]` array. Both formats are
//!   implemented inside `oxideav-ttf`'s
//!   [`oxideav_ttf::Font::gsub_apply_lookup_type_1`]; this module is
//!   pure dispatcher logic on top.
//! - **LookupType 7 (Extension)** wrappers around a Type 1 lookup are
//!   transparent — the underlying `oxideav-ttf` accessor unwraps
//!   ExtensionSubst before reporting the lookup type.
//!
//! Lookups of any other declared type (Multiple = 2, Alternate = 3,
//! Ligature = 4, Context = 5, ChainContext = 6, ReverseChainContext
//! = 8) are **silently skipped**. The brief for round 89 is single
//! substitution only — multi / ligature / context will come back in
//! a later round through the broader `apply_one_lookup` walker in
//! [`crate::shaping::general`].
//!
//! ## Why "single substitution only"
//!
//! The display-toggled feature catalogue is overwhelmingly
//! type-1-driven in practice:
//!
//! - `smcp` / `c2sc` → one upper / lower glyph → one small-cap glyph
//!   (Format 2 array typically).
//! - `case` → one paren / bracket / hyphen → its case-sensitive
//!   variant (Format 2).
//! - `salt` / `ss01..ss20` → one glyph → one stylistic alternate
//!   (Format 1 delta or Format 2 array).
//! - `frac` → digit-to-numerator/denominator routing (the actual
//!   `1/2` collapsing is contextual, but the digit reshape is type
//!   1).
//! - `sups` / `subs` → digit → superscript / subscript digit (Format
//!   2 typically).
//!
//! Fonts that ship a type-2 / type-4 sub-lookup under `frac` (the
//! contextual collapse rule) get the type-1 component applied here
//! and the contextual rule **silently skipped** — which is enough
//! for the round-89 surface (digits visibly reshape) but doesn't
//! exhaust the `frac` feature. The TODO marker covers that gap.
//!
//! ## Script tag probing
//!
//! Same priority as [`crate::shaping::general`]: `latn` → `cyrl` →
//! `grek` → `DFLT`. The first script tag whose feature list contains
//! a requested feature wins — its lookup-index list is harvested,
//! the remaining tags are not consulted for that feature. Two
//! requested features can resolve under two different script tags
//! (e.g. `smcp` under `latn`, a Cyrillic-specific `salt` under
//! `cyrl`); each is resolved independently.
//!
//! ## Idempotence + ordering
//!
//! Features are applied in the order they appear in the caller's
//! `features` slice. The OpenType spec doesn't pin an inter-feature
//! application order beyond "required-feature first"; production
//! shapers either follow the font's declaration order or apply
//! features per a registered-feature priority list. We pick the
//! simpler "caller-controlled order" so calling
//! `shape_text(text, &[*b"smcp", *b"salt"])` lets the caller flip
//! the order if needed without scribe imposing one.
//!
//! Each lookup is applied independently across the run; the lookup's
//! coverage table determines per-glyph whether it fires. Lookups
//! whose coverage doesn't match are a silent no-op.

use oxideav_ttf::Font;

/// Shape `text` against `font` with the caller-specified GSUB
/// feature tags applied. Returns the post-substitution glyph IDs.
///
/// Steps:
/// 1. cmap every character in `text` (`.notdef` for missing chars).
/// 2. For each requested feature tag (in caller order), resolve the
///    lookup-index list under the script-tag priority and apply
///    every LookupType-1 lookup to the running glyph list.
/// 3. Return the final glyph IDs.
///
/// Empty `text` yields an empty `Vec`. Empty `features` yields the
/// pure-cmap output (no GSUB applied) — useful as a "what does this
/// font do for cmap-only" baseline.
///
/// See the module-level docs for the lookup-type and script-tag
/// scoping rules.
pub fn shape_text_with_font(font: &Font<'_>, text: &str, features: &[[u8; 4]]) -> Vec<u16> {
    if text.is_empty() {
        return Vec::new();
    }
    // Step 1: cmap.
    let mut gids: Vec<u16> = text
        .chars()
        .map(|ch| font.glyph_index(ch).unwrap_or(0))
        .collect();

    if features.is_empty() {
        return gids;
    }

    // Step 2: per-feature LookupType-1 dispatch.
    let lookup_list = font.gsub_lookup_list();
    let lookup_type_of = |idx: u16| -> Option<u16> {
        lookup_list
            .iter()
            .find(|(i, _, _)| *i == idx)
            .map(|(_, ty, _)| *ty)
    };

    for feature_tag in features {
        let lookups = resolve_feature_lookups(font, feature_tag);
        for lookup_idx in lookups {
            // Single-substitution only — silently skip any other
            // declared type. The brief for round 89 is type 1; the
            // broader walker in `shaping::general::apply_one_lookup`
            // is the place to call for full type dispatch.
            if lookup_type_of(lookup_idx) != Some(1) {
                continue;
            }
            for slot in gids.iter_mut() {
                if let Some(rep) = font.gsub_apply_lookup_type_1(lookup_idx, *slot) {
                    *slot = rep;
                }
            }
        }
    }

    gids
}

/// Resolve `feature_tag` against the script-tag priority list
/// (`latn` → `cyrl` → `grek` → `DFLT`). Returns the lookup indices
/// of the *first* script tag whose feature list contains a matching
/// feature. Empty when no script publishes this feature.
fn resolve_feature_lookups(font: &Font<'_>, feature_tag: &[u8; 4]) -> Vec<u16> {
    let script_tags: [[u8; 4]; 4] = [*b"latn", *b"cyrl", *b"grek", *b"DFLT"];
    let mut hits: Vec<u16> = Vec::new();
    for tag in script_tags {
        let features = font.gsub_features_for_script(tag, None);
        for feat in features {
            if &feat.tag == feature_tag {
                hits.extend_from_slice(&feat.lookup_indices);
            }
        }
        if !hits.is_empty() {
            return hits;
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEJAVU_BYTES: &[u8] = include_bytes!("../../tests/fixtures/DejaVuSans.ttf");
    const INTER_BYTES: &[u8] = include_bytes!("../../tests/fixtures/InterVariable.ttf");

    /// Empty input is always the empty run — features list shouldn't
    /// matter.
    #[test]
    fn empty_text_is_empty_vec() {
        let bytes = DEJAVU_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("DejaVu parses");
        face.with_font(|font| {
            assert_eq!(shape_text_with_font(font, "", &[]).len(), 0);
            assert_eq!(shape_text_with_font(font, "", &[*b"smcp"]).len(), 0);
        })
        .unwrap();
    }

    /// Empty feature list returns the pure-cmap output unchanged.
    #[test]
    fn empty_features_is_cmap_identity() {
        let bytes = DEJAVU_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("DejaVu parses");
        face.with_font(|font| {
            let got = shape_text_with_font(font, "abc", &[]);
            let expected: Vec<u16> = "abc"
                .chars()
                .map(|c| font.glyph_index(c).unwrap_or(0))
                .collect();
            assert_eq!(got, expected);
        })
        .unwrap();
    }

    /// Inter Variable publishes `smcp` under `latn`. Applying it to
    /// lowercase "abc" must yield three glyphs that differ — at
    /// minimum some of them — from the cmap output (they're the
    /// small-cap variants). We don't insist *every* slot moves
    /// because Inter's smcp coverage is sparse (the lowercase "c"
    /// shares a glyph with its small-cap form in some Inter
    /// versions, so its slot may legitimately pass through
    /// unchanged); the contract is the substitution surface ran, not
    /// that every glyph rebased.
    #[test]
    fn inter_smcp_substitutes_lowercase_ascii() {
        let bytes = INTER_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("Inter parses");
        face.with_font(|font| {
            let cmap_only = shape_text_with_font(font, "abc", &[]);
            let smcp_on = shape_text_with_font(font, "abc", &[*b"smcp"]);
            assert_eq!(cmap_only.len(), 3);
            assert_eq!(smcp_on.len(), 3);
            assert_ne!(
                cmap_only, smcp_on,
                "smcp must reshape lowercase ASCII to small-cap glyphs"
            );
            // At least one slot changed.
            let changed = cmap_only.iter().zip(smcp_on.iter()).any(|(a, b)| a != b);
            assert!(
                changed,
                "smcp on Inter must remap at least one lowercase ASCII slot"
            );
        })
        .unwrap();
    }

    /// Inter publishes `sups` (superscripts). Applying it to digits
    /// must reshape at least one slot — the digits 0..9 all have
    /// dedicated superscript glyphs in Inter so we expect a
    /// substantial coverage hit, but we don't insist on every slot
    /// changing (a future Inter release might consolidate or move
    /// glyphs).
    #[test]
    fn inter_sups_substitutes_digits() {
        let bytes = INTER_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("Inter parses");
        face.with_font(|font| {
            let cmap_only = shape_text_with_font(font, "0123", &[]);
            let sups_on = shape_text_with_font(font, "0123", &[*b"sups"]);
            assert_eq!(cmap_only.len(), 4);
            assert_eq!(sups_on.len(), 4);
            assert_ne!(cmap_only, sups_on);
            let changed = cmap_only.iter().zip(sups_on.iter()).any(|(a, b)| a != b);
            assert!(changed, "sups must remap at least one digit slot");
        })
        .unwrap();
    }

    /// `subs` (subscripts) mirror of the `sups` test — independent
    /// feature, must produce a different output from both cmap-only
    /// and from `sups`.
    #[test]
    fn inter_subs_is_distinct_from_sups() {
        let bytes = INTER_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("Inter parses");
        face.with_font(|font| {
            let cmap_only = shape_text_with_font(font, "0123", &[]);
            let sups_on = shape_text_with_font(font, "0123", &[*b"sups"]);
            let subs_on = shape_text_with_font(font, "0123", &[*b"subs"]);
            assert_ne!(cmap_only, subs_on);
            assert_ne!(sups_on, subs_on, "sups and subs must reshape distinctly");
        })
        .unwrap();
    }

    /// Two features applied in sequence — first `smcp` turns the
    /// lowercase into small caps, then `case` (case-sensitive forms)
    /// runs against punctuation. The `case` feature on lowercase
    /// alone is a no-op (its coverage targets the punctuation), so
    /// the result is `smcp`-only on a pure-lowercase input.
    #[test]
    fn inter_smcp_then_case_on_lowercase_is_smcp_alone() {
        let bytes = INTER_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("Inter parses");
        face.with_font(|font| {
            let smcp_alone = shape_text_with_font(font, "abc", &[*b"smcp"]);
            let smcp_then_case = shape_text_with_font(font, "abc", &[*b"smcp", *b"case"]);
            assert_eq!(
                smcp_alone, smcp_then_case,
                "case is a no-op on lowercase ASCII; result must match smcp alone"
            );
        })
        .unwrap();
    }

    /// Requesting an unknown feature tag is a clean no-op.
    #[test]
    fn unknown_feature_tag_is_identity() {
        let bytes = INTER_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("Inter parses");
        face.with_font(|font| {
            let cmap_only = shape_text_with_font(font, "abc", &[]);
            // `zzzz` is not a registered OpenType feature.
            let unknown = shape_text_with_font(font, "abc", &[*b"zzzz"]);
            assert_eq!(cmap_only, unknown);
        })
        .unwrap();
    }

    /// A font without GSUB (or without the requested feature) is a
    /// no-op. DejaVu Sans does not publish `smcp` (its small-caps
    /// support is via a separate font file, not GSUB), so requesting
    /// it falls through to cmap identity.
    #[test]
    fn dejavu_smcp_unsupported_is_cmap_identity() {
        let bytes = DEJAVU_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("DejaVu parses");
        face.with_font(|font| {
            assert!(
                !face.has_gsub_feature(*b"latn", *b"smcp"),
                "DejaVu Sans is the no-smcp control fixture for this test"
            );
            let cmap_only = shape_text_with_font(font, "abc", &[]);
            let smcp_on = shape_text_with_font(font, "abc", &[*b"smcp"]);
            assert_eq!(cmap_only, smcp_on);
        })
        .unwrap();
    }

    /// Non-LookupType-1 lookups must be silently skipped. The `liga`
    /// feature on DejaVu Sans ships a LookupType-4 lookup; pointing
    /// `shape_text_with_font` at `liga` is a no-op (the round-89
    /// surface is single-substitution only — `liga` ligature work
    /// happens through `Shaper::shape` / `FaceChain::shape`'s
    /// existing pipeline).
    #[test]
    fn liga_is_skipped_because_it_is_lookup_type_4() {
        let bytes = DEJAVU_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("DejaVu parses");
        face.with_font(|font| {
            assert!(
                face.has_gsub_feature(*b"latn", *b"liga"),
                "DejaVu publishes `liga` — round-89 must observe the lookup type 4 skip on it"
            );
            // "fi" cmap'd individually then "shape_text"-applied
            // through `liga` must NOT collapse into the fi-ligature
            // glyph (the round-89 surface filters out non-type-1
            // lookups). The pre-existing `Shaper::shape` path still
            // produces the ligature — this test isolates the
            // single-substitution-only contract of `shape_text`.
            let cmap_only = shape_text_with_font(font, "fi", &[]);
            let liga_attempted = shape_text_with_font(font, "fi", &[*b"liga"]);
            assert_eq!(
                cmap_only, liga_attempted,
                "liga (lookup type 4) must be silently skipped by the round-89 type-1 surface"
            );
        })
        .unwrap();
    }
}
