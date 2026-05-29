//! Round 89 / 125 / 128 / 156 — caller-driven GSUB single + multiple +
//! ligature + alternate substitution feature application.
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
//! zero), `pnum` / `tnum` (proportional / tabular numerals),
//! `aalt` (access all alternates), and the `cv01..cv99` per-character
//! variant family — and applies the single-/multiple-/ligature-/
//! alternate-substitution lookups those features dispatch.
//!
//! ## What's wired
//!
//! - **GSUB LookupType 1 (Single Substitution)** (round 89). The
//!   OpenType spec §6.2.1 (a.k.a. chapter 6 "GSUB - Glyph Substitution
//!   Table", section 2.1 "Single Substitution Subtable") defines two
//!   formats — **Format 1 (delta)** replaces every covered glyph by
//!   `gid + deltaGlyphID` (mod 65536), and **Format 2 (substitute-array)**
//!   replaces every covered glyph by the entry at the same coverage
//!   index in a `substituteGlyphIDs[]` array. Both formats are
//!   implemented inside `oxideav-ttf`'s
//!   [`oxideav_ttf::Font::gsub_apply_lookup_type_1`]; this module is
//!   pure dispatcher logic on top.
//! - **GSUB LookupType 2 (Multiple Substitution)** (round 125). The
//!   OpenType spec §6.2.2 ("Multiple Substitution Subtable") defines
//!   **Format 1** — a Coverage on the first input glyph plus a
//!   per-coverage `Sequence` record with a `glyphCount` and a
//!   `substituteGlyphIDs[]` array. The substitution replaces the
//!   covered input glyph with the `glyphCount`-long sequence
//!   (`glyphCount = 0` is legal and is interpreted as a deletion —
//!   the input glyph is removed without replacement). The decode
//!   itself lives in `oxideav-ttf`'s
//!   [`oxideav_ttf::Font::gsub_apply_lookup_type_2`]; this module
//!   walks every covered slot and splices the returned sequence into
//!   the running glyph buffer, advancing past the inserted run so a
//!   re-application of the same lookup doesn't re-match the
//!   substitution's output.
//! - **GSUB LookupType 4 (Ligature Substitution)** (round 128). The
//!   OpenType spec §6.2.4 ("Ligature Substitution Subtable") defines
//!   **Format 1** — a Coverage on the *first* component glyph plus a
//!   per-coverage `LigatureSet`, each of which lists `Ligature`
//!   records carrying the trailing component glyph IDs (positions 2,
//!   3, …) and the replacement `ligGlyph`. When the input prefix
//!   starting at the cursor matches all components, the matched
//!   `componentCount` glyphs are replaced by the single ligature
//!   glyph. The decode itself lives in `oxideav-ttf`'s
//!   [`oxideav_ttf::Font::gsub_apply_lookup_type_4`] which returns
//!   `Some((replacement_gid, consumed))` on a hit; this module walks
//!   the glyph run left-to-right, splices `gids[pos..pos+consumed]`
//!   to `[replacement]`, and advances the cursor by 1 (past the new
//!   ligature glyph). The advance-by-1 is what `Shaper::shape`'s
//!   round-1 ligature pass does as well — and it's correct because a
//!   ligature lookup's coverage is on the *first* component, so the
//!   ligature glyph (whose GID typically lives outside the basic
//!   alphabet) won't re-match the same lookup.
//! - **GSUB LookupType 3 (Alternate Substitution)** (round 156). The
//!   OpenType spec §6.2.3 ("Alternate Substitution Subtable") defines
//!   **Format 1** — a Coverage on each input glyph plus, per-coverage,
//!   an `AlternateSet` listing one or more `alternateGlyphIDs[]`. The
//!   substitution replaces a covered glyph with one entry from its
//!   `AlternateSet`; the spec doesn't pin an inter-alternate selection
//!   policy ("the application of the OpenType Layout engine selects an
//!   alternate") so we default to `alternateIndex = 0`, which is what
//!   `aalt` / `salt` are designed to produce when consulted without a
//!   user-specified pick. The decode itself lives in `oxideav-ttf`'s
//!   [`oxideav_ttf::Font::gsub_apply_lookup_type_3`]; this module
//!   walks every covered slot and substitutes the alternate-0 glyph
//!   (length-preserving, mirrors the Type-1 walker — Alternate is
//!   single-substitution-with-a-twist).
//! - **LookupType 7 (Extension)** wrappers around a Type 1 / Type 2
//!   / Type 3 / Type 4 lookup are transparent — the underlying
//!   `oxideav-ttf` accessor unwraps ExtensionSubst before reporting
//!   the lookup type.
//!
//! Lookups of the remaining declared types (Context = 5, ChainContext
//! = 6, ReverseChainContext = 8) are **silently skipped**. The brief
//! for round 156 is single + multiple + ligature + alternate
//! substitution — contextual / chained-contextual / reverse-chained
//! substitution dispatch on the caller-driven surface is left for a
//! later round (the always-on `ccmp` / `calt` passes in
//! [`crate::shaping::general`] already cover those types end-to-end
//! through the broader `apply_one_lookup` walker).
//!
//! ## What lookup types the display-toggled catalogue uses
//!
//! Most display-toggled features the caller-driven `shape_text` API
//! is meant to expose are dispatched as one or more of the four
//! supported lookup types. The mapping:
//!
//! - `smcp` / `c2sc` → one upper / lower glyph → one small-cap glyph
//!   (LookupType 1, typically Format 2 array).
//! - `case` → one paren / bracket / hyphen → its case-sensitive
//!   variant (LookupType 1, typically Format 2).
//! - `salt` / `ss01..ss20` → one glyph → one stylistic alternate
//!   (LookupType 1, Format 1 delta or Format 2 array; some fonts
//!   use LookupType 3 here when there's more than one alternate per
//!   covered glyph — the round-156 pass picks alternate 0).
//! - `aalt` → "access all alternates" — typically a mix of LookupType
//!   1 (single substitution into the principal alternate) and
//!   LookupType 3 (the full per-glyph `AlternateSet` for ad-hoc
//!   alternate access). Round 156 dispatches both.
//! - `frac` → digit-to-numerator / denominator routing (LookupType 1
//!   for the digit reshape; the contextual `1/2` collapse is a
//!   chained-context Type-6 rule and is silently skipped here).
//! - `sups` / `subs` → digit → superscript / subscript digit
//!   (LookupType 1, Format 2 typically).
//! - `liga` / `dlig` / `rlig` → multi-glyph → single ligature glyph
//!   (LookupType 4, Format 1 — exclusively in practice).
//! - `ccmp` → split precomposed glyph → base + combining mark
//!   (LookupType 2 Format 1 — already wired in round 125).
//!
//! Fonts that ship a non-Type-1/2/4 sub-lookup under `frac` (the
//! contextual collapse rule) get the type-1 component applied here
//! and the contextual rule **silently skipped** — which is enough
//! for the round-89 surface (digits visibly reshape) but doesn't
//! exhaust the `frac` feature.
//!
//! ## Script tag probing
//!
//! The auto-probing entry [`shape_text_with_font`] walks the registered
//! OpenType script tag priority list:
//! `latn` → `cyrl` → `grek` → `arab` → `hebr` → `thai` → `deva` →
//! `beng` → `taml` / `tml2` → `gujr` / `gjr2` → `guru` / `gur2` →
//! `knda` / `knd2` → `mlym` / `mlm2` → `orya` / `ory2` → `telu` /
//! `tel2` → `sinh` → `khmr` → `lao ` → `mymr` / `mym2` → `hang` →
//! `hani` → `kana` → `DFLT`. The first script tag whose feature list
//! contains a requested feature wins — its lookup-index list is
//! harvested, the remaining tags are not consulted for that feature.
//! Two requested features can resolve under two different script tags
//! (e.g. `smcp` under `latn`, a Cyrillic-specific `salt` under
//! `cyrl`); each is resolved independently. The Indic v2 tags
//! (`tml2`, `dev2`, etc.) and Hangul / Han / Kana tags broaden the
//! caller-driven surface to scripts that previously only had GSUB
//! reachable through the per-script shaping pipelines.
//!
//! Callers that already know the script of their run — typically
//! because the run came out of a script-segmenter or because the
//! caller is shaping a known-language string — should prefer
//! [`shape_text_with_script_with_font`] which takes an explicit
//! script tag, skipping the priority walk and avoiding the
//! occasional ambiguity when two scripts publish the same feature
//! tag (e.g. `liga` under both `latn` and `arab`).
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
///    every LookupType-1 (single), LookupType-2 (multiple),
///    LookupType-3 (alternate, default `alternateIndex = 0`), and
///    LookupType-4 (ligature) lookup to the running glyph list.
/// 3. Return the final glyph IDs (length may differ from the cmap'd
///    input when a LookupType-2 lookup splits or deletes glyphs, or
///    when a LookupType-4 lookup collapses N components into one
///    ligature glyph).
///
/// Empty `text` yields an empty `Vec`. Empty `features` yields the
/// pure-cmap output (no GSUB applied) — useful as a "what does this
/// font do for cmap-only" baseline.
///
/// See the module-level docs for the lookup-type and script-tag
/// scoping rules.
pub fn shape_text_with_font(font: &Font<'_>, text: &str, features: &[[u8; 4]]) -> Vec<u16> {
    shape_text_inner(font, text, features, None)
}

/// Shape `text` against `font` with the caller-specified GSUB feature
/// tags applied under the explicit `script_tag`. Returns the post-
/// substitution glyph IDs.
///
/// Unlike [`shape_text_with_font`], this entry point bypasses the
/// script-tag priority walk and resolves every feature tag against
/// `script_tag` alone (with the language system's `DefaultLangSys`).
/// If `script_tag` is unknown to the font, every requested feature
/// resolves to an empty lookup list and the function returns the
/// pure-cmap output.
///
/// Callers should use this entry point when they already know the
/// script of the run — typically because the run came out of a
/// script-segmenter or because the caller is shaping a known-language
/// string. Two examples:
///
/// - An Arabic shaper that has already done its joining-class pass
///   and wants `liga` resolved against `arab` rather than `latn`
///   (the script-priority walk would otherwise pick `latn` first if
///   the font publishes `liga` under both — a rare but real case
///   for multi-script fonts).
/// - A CJK pipeline that wants `vert` / `vrt2` resolved against
///   `hani` / `kana` / `hang` to switch a horizontal-form run to
///   the vertical-form glyphs.
///
/// Mirrors [`shape_text_with_font`]'s LookupType-1/2/3/4 dispatch
/// semantics: Type 1 (single substitution) and Type 3 (alternate,
/// `alternateIndex = 0`) are length-preserving; Type 2 (multiple
/// substitution) may split or delete; Type 4 (ligature) shortens the
/// run. Lookups of other declared types referenced by the requested
/// features are silently skipped.
pub fn shape_text_with_script_with_font(
    font: &Font<'_>,
    text: &str,
    script_tag: [u8; 4],
    features: &[[u8; 4]],
) -> Vec<u16> {
    shape_text_inner(font, text, features, Some(script_tag))
}

/// Internal shared body for [`shape_text_with_font`] and
/// [`shape_text_with_script_with_font`]. When `script_tag` is
/// `Some(tag)`, every feature is resolved against `tag` alone; when
/// `None`, the script-tag priority list is walked per
/// [`resolve_feature_lookups`].
fn shape_text_inner(
    font: &Font<'_>,
    text: &str,
    features: &[[u8; 4]],
    script_tag: Option<[u8; 4]>,
) -> Vec<u16> {
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

    // Step 2: per-feature dispatch.
    let lookup_list = font.gsub_lookup_list();
    let lookup_type_of = |idx: u16| -> Option<u16> {
        lookup_list
            .iter()
            .find(|(i, _, _)| *i == idx)
            .map(|(_, ty, _)| *ty)
    };

    for feature_tag in features {
        let lookups = match script_tag {
            Some(tag) => resolve_feature_lookups_single_script(font, tag, feature_tag),
            None => resolve_feature_lookups(font, feature_tag),
        };
        for lookup_idx in lookups {
            match lookup_type_of(lookup_idx) {
                Some(1) => {
                    // LookupType 1 (Single Substitution): one input
                    // glyph → one output glyph. Length-preserving.
                    for slot in gids.iter_mut() {
                        if let Some(rep) = font.gsub_apply_lookup_type_1(lookup_idx, *slot) {
                            *slot = rep;
                        }
                    }
                }
                Some(2) => {
                    // LookupType 2 (Multiple Substitution, Format 1):
                    // one input glyph → N output glyphs (N may be 0
                    // for the deletion edge case the spec permits).
                    // Walk the buffer left-to-right; when a slot is
                    // covered, splice the returned sequence in place
                    // of the single input glyph and advance past the
                    // inserted run so the same lookup doesn't re-
                    // match its own output (mirrors the
                    // `apply_one_lookup` strategy in
                    // `shaping::general`).
                    let mut pos = 0usize;
                    while pos < gids.len() {
                        if let Some(seq) = font.gsub_apply_lookup_type_2(lookup_idx, gids[pos]) {
                            let new_len = seq.len();
                            gids.splice(pos..pos + 1, seq);
                            pos += new_len;
                        } else {
                            pos += 1;
                        }
                    }
                }
                Some(3) => {
                    // LookupType 3 (Alternate Substitution, Format 1):
                    // covered glyph → one entry from its `AlternateSet`.
                    // The spec doesn't pin which alternate the engine
                    // picks ("the application of the OpenType Layout
                    // engine selects an alternate"); we default to
                    // index 0, which is what `aalt` / `salt` are
                    // designed to produce without a user-specified
                    // pick. Length-preserving, mirrors the Type-1
                    // walker exactly. A higher-level surface that
                    // wanted to expose user-driven alternate selection
                    // would belong above this layer (see
                    // `oxideav-ttf`'s `gsub_apply_lookup_type_3` for
                    // the per-call `alternate_index` argument).
                    for slot in gids.iter_mut() {
                        if let Some(rep) = font.gsub_apply_lookup_type_3(lookup_idx, *slot, 0) {
                            *slot = rep;
                        }
                    }
                }
                Some(4) => {
                    // LookupType 4 (Ligature Substitution, Format 1):
                    // N input component glyphs → one ligature glyph.
                    // Walk the buffer left-to-right; at each cursor
                    // position, ask `oxideav-ttf` whether the lookup
                    // matches the prefix starting at the cursor.
                    // `gsub_apply_lookup_type_4(idx, &gids[pos..])`
                    // returns `Some((replacement, consumed))` when a
                    // ligature applies; we splice
                    // `gids[pos..pos+consumed]` to the single
                    // replacement glyph and advance the cursor by 1
                    // (past the new ligature). The ligature glyph
                    // typically lives outside the basic-alphabet GID
                    // range, so re-matching the same lookup on the
                    // output is benign — but advancing by 1 is what
                    // `Shaper::shape`'s round-1 ligature pass already
                    // does and is the natural mirror of the type-2
                    // walker above.
                    let mut pos = 0usize;
                    while pos < gids.len() {
                        if let Some((replacement, consumed)) =
                            font.gsub_apply_lookup_type_4(lookup_idx, &gids[pos..])
                        {
                            if consumed == 0 {
                                // Defensive: a degenerate
                                // `componentCount = 0` ligature record
                                // would loop without this guard. The
                                // spec doesn't allow it, but a
                                // malformed font shouldn't be able to
                                // hang the shaper.
                                pos += 1;
                                continue;
                            }
                            gids.splice(pos..pos + consumed, std::iter::once(replacement));
                            pos += 1;
                        } else {
                            pos += 1;
                        }
                    }
                }
                _ => {
                    // Any other declared type is silently skipped —
                    // the round-89/125/128/156 surface is single +
                    // multiple + ligature + alternate substitution
                    // only. Contextual / Chained / Reverse-Chained
                    // lookups belong to the broader `apply_one_lookup`
                    // walker in `shaping::general`.
                }
            }
        }
    }

    gids
}

/// Resolve `feature_tag` against the broadened script-tag priority
/// list. Returns the lookup indices of the *first* script tag whose
/// feature list contains a matching feature. Empty when no script
/// publishes this feature.
///
/// The list covers the registered OpenType script tags for every
/// shaping context scribe touches today: Latin / Cyrillic / Greek
/// (the round-15 always-on group), the Arabic / Hebrew / Thai script
/// tags whose runs flow through `shaping::arabic` / `shaping::indic`,
/// the Indic v1 + v2 tags (`taml` vs `tml2`, etc. — fonts published
/// after OpenType 1.6 typically expose features under the v2 tag),
/// the South-East Asian Brahmic tags (`khmr`, `lao `, `mymr` /
/// `mym2`), and the CJK tags (`hang` for Hangul, `hani` for Han,
/// `kana` for Hiragana / Katakana). The `DFLT` script tag remains
/// the final fallback per OpenType convention. The list is ordered
/// so that the round-15-original four tags (`latn` / `cyrl` / `grek`
/// / `DFLT`) keep their resolution priority for the legacy pure-
/// Latin usage; non-Latin tags are appended after `DFLT` only when
/// no earlier tag matched — which is a no-op in practice because
/// `DFLT` already terminates the search in the Latin case.
///
/// Two same-named feature tags published under two different scripts
/// (e.g. `liga` under both `latn` and `arab`) resolve to whichever
/// the earlier script tag in the list publishes. Callers that need
/// deterministic single-script resolution should drop to
/// [`shape_text_with_script_with_font`] which takes an explicit
/// script tag.
fn resolve_feature_lookups(font: &Font<'_>, feature_tag: &[u8; 4]) -> Vec<u16> {
    for &tag in script_tag_probe_list() {
        let hits = resolve_feature_lookups_single_script(font, tag, feature_tag);
        if !hits.is_empty() {
            return hits;
        }
    }
    Vec::new()
}

/// Resolve `feature_tag` against `script_tag` alone (no priority
/// walk). Returns the lookup indices of every matching feature
/// record on the script's DefaultLangSys, in declaration order.
/// Empty when `script_tag` is unknown to the font or its feature
/// list doesn't contain `feature_tag`.
fn resolve_feature_lookups_single_script(
    font: &Font<'_>,
    script_tag: [u8; 4],
    feature_tag: &[u8; 4],
) -> Vec<u16> {
    let mut hits: Vec<u16> = Vec::new();
    let features = font.gsub_features_for_script(script_tag, None);
    for feat in features {
        if &feat.tag == feature_tag {
            hits.extend_from_slice(&feat.lookup_indices);
        }
    }
    hits
}

/// The script-tag priority list used by [`resolve_feature_lookups`].
/// Order matches the documentation in the module-level doc-comment.
///
/// Each tag is a four-byte OpenType script tag (space-padded where
/// the registered name is shorter than four bytes, e.g. `lao `).
/// The list keeps the round-15 four-tag prefix (Latin / Cyrillic /
/// Greek / DFLT) at the head so existing callers' resolution order
/// is unchanged; the round-175 extension appends the registered
/// non-Latin script tags after `DFLT` so that non-Latin runs can now
/// reach GSUB through the caller-driven surface.
const SCRIPT_TAG_PROBE_LIST: &[[u8; 4]] = &[
    // Round-15 original tags (priority-preserving prefix).
    *b"latn", *b"cyrl", *b"grek", *b"DFLT", // Arabic + Hebrew (RTL scripts).
    *b"arab", *b"hebr", // Thai + Lao (no-halant Brahmic).
    *b"thai", *b"lao ",
    // Indic v1 + v2. OpenType 1.6 introduced the `*v2` tags for
    // Indic-shaping fonts that follow the modernised cluster
    // model; both are probed (v1 first; v2 second).
    *b"deva", *b"dev2", *b"beng", *b"bng2", *b"taml", *b"tml2", *b"gujr", *b"gjr2", *b"guru",
    *b"gur2", *b"knda", *b"knd2", *b"mlym", *b"mlm2", *b"orya", *b"ory2", *b"telu", *b"tel2",
    *b"sinh", // South-East Asian.
    *b"khmr", *b"mymr", *b"mym2", // CJK.
    *b"hang", *b"hani", *b"kana",
];

/// Accessor for the script-tag probe list used by
/// [`resolve_feature_lookups`]. Exposed at module scope (rather than
/// inlined) so the tests can grep the list for the registered tags
/// the documentation claims are covered.
fn script_tag_probe_list() -> &'static [[u8; 4]] {
    SCRIPT_TAG_PROBE_LIST
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

    /// LookupType 4 (Ligature Substitution) is dispatched by
    /// `shape_text` as of round 128. DejaVu Sans publishes `liga`
    /// as a LookupType-4 lookup that collapses 'f'+'i' into the
    /// fi-ligature glyph. The round-128 contract: shaping "fi"
    /// with the `liga` feature returns *one* glyph (the ligature),
    /// not two (cmap output) — and that glyph is *different* from
    /// either input glyph.
    #[test]
    fn liga_collapses_fi_via_lookup_type_4() {
        let bytes = DEJAVU_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("DejaVu parses");
        face.with_font(|font| {
            assert!(
                face.has_gsub_feature(*b"latn", *b"liga"),
                "DejaVu publishes `liga` — round-128 dispatches its lookup type 4"
            );
            let cmap_only = shape_text_with_font(font, "fi", &[]);
            let liga_on = shape_text_with_font(font, "fi", &[*b"liga"]);
            assert_eq!(cmap_only.len(), 2, "cmap maps 'f' and 'i' to two glyphs");
            assert_eq!(
                liga_on.len(),
                1,
                "round-128 collapses 'fi' into a single ligature glyph"
            );
            assert_ne!(
                liga_on[0], cmap_only[0],
                "the fi-ligature glyph differs from cmap('f')"
            );
            assert_ne!(
                liga_on[0], cmap_only[1],
                "the fi-ligature glyph differs from cmap('i')"
            );
        })
        .unwrap();
    }

    /// `liga` on a run of text where some slots can ligate and
    /// others can't must collapse only the ligatable prefix. DejaVu
    /// publishes "fi", "fl", and "ffi" / "ffl" but not e.g. "ab"; a
    /// string mixing both must keep the non-ligatable letters at
    /// their cmap output.
    #[test]
    fn liga_leaves_uncovered_glyphs_alone() {
        let bytes = DEJAVU_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("DejaVu parses");
        face.with_font(|font| {
            // "abfi" → 'a', 'b' (no ligature) + 'fi' (ligature).
            let cmap_only = shape_text_with_font(font, "abfi", &[]);
            let liga_on = shape_text_with_font(font, "abfi", &[*b"liga"]);
            assert_eq!(cmap_only.len(), 4);
            assert_eq!(
                liga_on.len(),
                3,
                "'a', 'b' pass through; 'fi' collapses to one glyph"
            );
            // First two slots must be exactly the cmap output.
            assert_eq!(liga_on[0], cmap_only[0]);
            assert_eq!(liga_on[1], cmap_only[1]);
            // Trailing slot is the fi-ligature, which is a different
            // glyph from either 'f' or 'i'.
            assert_ne!(liga_on[2], cmap_only[2]);
            assert_ne!(liga_on[2], cmap_only[3]);
        })
        .unwrap();
    }

    /// `liga` applied to a string with no covered components is the
    /// cmap-identity. We use "abc" — DejaVu's `liga` lookup only
    /// fires on f/i/l/t component sequences, so a pure-alphabetical
    /// input must pass through unchanged.
    #[test]
    fn liga_is_identity_on_uncovered_run() {
        let bytes = DEJAVU_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("DejaVu parses");
        face.with_font(|font| {
            let cmap_only = shape_text_with_font(font, "abc", &[]);
            let liga_on = shape_text_with_font(font, "abc", &[*b"liga"]);
            assert_eq!(
                cmap_only, liga_on,
                "no component prefix in 'abc' matches DejaVu's liga lookup"
            );
        })
        .unwrap();
    }

    /// LookupType 3 (Alternate Substitution) is dispatched by
    /// `shape_text` as of round 156. Inter's `aalt` references a
    /// Type-3 lookup that covers lowercase 'a' with a non-cmap
    /// alternate at `alternateIndex = 0`. The round-156 contract:
    /// `shape_text("a", &[aalt])` reshapes the slot via Type 3,
    /// distinct from `cmap('a')`, with length preserved at 1.
    #[test]
    fn aalt_dispatches_lookup_type_3_on_inter() {
        let bytes = INTER_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("Inter parses");
        face.with_font(|font| {
            let cmap_only = shape_text_with_font(font, "a", &[]);
            let aalt = shape_text_with_font(font, "a", &[*b"aalt"]);
            assert_eq!(cmap_only.len(), 1);
            assert_eq!(aalt.len(), 1, "Type 3 is length-preserving");
            assert_ne!(
                cmap_only[0], aalt[0],
                "round-156 reshapes 'a' via aalt's Type-3 alternate-0"
            );
        })
        .unwrap();
    }

    /// `aalt` on DejaVu Sans is a single Type-3 lookup. Re-applying
    /// the feature is a no-op because the AlternateSet coverage
    /// matches the input glyphs only, not the substitutes — so two
    /// applications must produce the same output as one.
    #[test]
    fn aalt_is_idempotent_on_dejavu() {
        let bytes = DEJAVU_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("DejaVu parses");
        face.with_font(|font| {
            let once = shape_text_with_font(font, "Iaaly", &[*b"aalt"]);
            let twice = shape_text_with_font(font, "Iaaly", &[*b"aalt", *b"aalt"]);
            assert_eq!(once, twice, "aalt's Type-3 component must be idempotent");
        })
        .unwrap();
    }

    // ----- Round-175: explicit-script + broadened auto-probe -----

    /// The script-tag probe list keeps the round-15 four-tag prefix
    /// at the head. This is the precondition that guarantees existing
    /// Latin / Cyrillic / Greek / DFLT callers see no behaviour
    /// change from the round-175 broadening.
    #[test]
    fn round175_probe_list_prefix_is_round15_priority() {
        let list = script_tag_probe_list();
        assert!(list.len() >= 4, "probe list keeps round-15 four-tag prefix");
        assert_eq!(&list[..4], &[*b"latn", *b"cyrl", *b"grek", *b"DFLT"]);
    }

    /// The round-175 broadening must register every non-Latin script
    /// tag the documentation enumerates — Arabic / Hebrew / Thai /
    /// Lao / Indic v1+v2 / Khmer / Burmese v1+v2 / Hangul / Han /
    /// Kana. Otherwise the module docstring misleads callers.
    #[test]
    fn round175_probe_list_covers_broadened_scripts() {
        let list = script_tag_probe_list();
        for tag in [
            *b"arab", *b"hebr", *b"thai", *b"lao ", *b"deva", *b"dev2", *b"beng", *b"bng2",
            *b"taml", *b"tml2", *b"gujr", *b"gjr2", *b"guru", *b"gur2", *b"knda", *b"knd2",
            *b"mlym", *b"mlm2", *b"orya", *b"ory2", *b"telu", *b"tel2", *b"sinh", *b"khmr",
            *b"mymr", *b"mym2", *b"hang", *b"hani", *b"kana",
        ] {
            assert!(
                list.contains(&tag),
                "round-175 probe list must contain script tag {:?}",
                core::str::from_utf8(&tag).unwrap_or("???")
            );
        }
    }

    /// Explicit-script entry point with an unknown script tag must
    /// resolve every feature to an empty lookup list and return the
    /// pure-cmap output. `zzzz` is not a registered OpenType script
    /// tag (mirrors the unknown-feature-tag test from round 89).
    #[test]
    fn round175_explicit_unknown_script_is_cmap_identity() {
        let bytes = INTER_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("Inter parses");
        face.with_font(|font| {
            let cmap_only = shape_text_with_font(font, "abc", &[]);
            let explicit_unknown =
                shape_text_with_script_with_font(font, "abc", *b"zzzz", &[*b"smcp"]);
            assert_eq!(
                cmap_only, explicit_unknown,
                "unknown script-tag must skip every feature"
            );
        })
        .unwrap();
    }

    /// Explicit-script entry point with an empty `features` slice
    /// always returns the pure-cmap output, regardless of script-tag
    /// validity. This mirrors the auto-probe contract.
    #[test]
    fn round175_explicit_empty_features_is_cmap_identity() {
        let bytes = INTER_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("Inter parses");
        face.with_font(|font| {
            let cmap_only = shape_text_with_font(font, "Hello", &[]);
            let explicit_empty = shape_text_with_script_with_font(font, "Hello", *b"latn", &[]);
            assert_eq!(cmap_only, explicit_empty);
        })
        .unwrap();
    }

    /// Explicit-script entry point matches the auto-probe output when
    /// the auto-probe would have picked the same script. Inter's
    /// `smcp` resolves under `latn`, so requesting it explicitly
    /// against `latn` must agree with the auto-probe result.
    #[test]
    fn round175_explicit_latn_matches_auto_probe_on_smcp() {
        let bytes = INTER_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("Inter parses");
        face.with_font(|font| {
            let auto = shape_text_with_font(font, "abc", &[*b"smcp"]);
            let explicit_latn =
                shape_text_with_script_with_font(font, "abc", *b"latn", &[*b"smcp"]);
            assert_eq!(
                auto, explicit_latn,
                "explicit `latn` resolution must agree with auto-probe on smcp"
            );
        })
        .unwrap();
    }

    /// Explicit-script entry point with `DFLT` matches the auto-probe
    /// for features that resolve under `DFLT` only (rare for Inter;
    /// the assertion here is just that the explicit path doesn't
    /// reach a different `DFLT` feature list than the auto-probe
    /// would after `latn`/`cyrl`/`grek` miss). We probe the bare-cmap
    /// case against a feature that no script publishes — the result
    /// must be cmap identity whether we hit the explicit `DFLT` or
    /// the auto-probe walk-and-fall-through.
    #[test]
    fn round175_explicit_dflt_unknown_feature_is_cmap_identity() {
        let bytes = INTER_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("Inter parses");
        face.with_font(|font| {
            let cmap_only = shape_text_with_font(font, "abc", &[]);
            let auto = shape_text_with_font(font, "abc", &[*b"zzzz"]);
            let explicit_dflt =
                shape_text_with_script_with_font(font, "abc", *b"DFLT", &[*b"zzzz"]);
            assert_eq!(cmap_only, auto);
            assert_eq!(cmap_only, explicit_dflt);
        })
        .unwrap();
    }

    /// The broadened auto-probe must preserve the round-15 four-tag
    /// resolution for an existing Latin feature. Concretely: Inter's
    /// `smcp` still resolves under `latn` and the rebroadened probe
    /// returns the same glyph run as the round-15 baseline did. This
    /// is the no-regression guarantee for the broadening.
    #[test]
    fn round175_broadened_probe_preserves_latn_smcp_result() {
        let bytes = INTER_BYTES.to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("Inter parses");
        face.with_font(|font| {
            // Reconstruct the round-15 4-tag-only resolution by hand.
            let mut round15_hits: Vec<u16> = Vec::new();
            for tag in [*b"latn", *b"cyrl", *b"grek", *b"DFLT"] {
                let features = font.gsub_features_for_script(tag, None);
                for feat in features {
                    if feat.tag == *b"smcp" {
                        round15_hits.extend_from_slice(&feat.lookup_indices);
                    }
                }
                if !round15_hits.is_empty() {
                    break;
                }
            }
            let round175_hits = resolve_feature_lookups(font, b"smcp");
            assert_eq!(
                round15_hits, round175_hits,
                "broadened probe must preserve the round-15 `smcp`/`latn` resolution"
            );
        })
        .unwrap();
    }
}
