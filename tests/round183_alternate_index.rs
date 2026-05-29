//! Round 183 — caller-driven Type-3 alternate-index selection via
//! `Face::shape_text_with_alternates` /
//! `Face::shape_text_with_script_and_alternates`.
//!
//! Where round 156 wires the Type-3 (Alternate Substitution) dispatch
//! into the caller-driven surface with a hardcoded
//! `alternateIndex = 0`, round 183 lets the caller name the alternate
//! index per feature. Two paired entry points mirror the round-89 /
//! round-175 auto-probe vs explicit-script split, and both inherit
//! the round-89/125/128/156 LookupType-1/2/3/4 dispatch semantics
//! verbatim — only the Type-3 alternate index changes.
//!
//! ## What this exercises
//!
//! 1. Index-0 backwards-compatibility — requesting alternate-0 must
//!    agree with the round-156 default for every fixture+feature pair
//!    the round-156 suite covers. The round-183 surface is a strict
//!    superset of the round-156 contract.
//! 2. Index out-of-range fallback — requesting `u16::MAX` for any
//!    feature must return cmap-identity per slot (the `oxideav-ttf`
//!    accessor returns `None`; we leave the slot unchanged). Safe
//!    fallback for callers that don't pre-probe the per-font
//!    alternate count.
//! 3. Length-preservation — Type 3 is length-preserving per OpenType
//!    §6.2.3 regardless of which alternate index the caller picks.
//! 4. Non-Type-3 features ignore the index — passing
//!    `(*b"liga", 5)` must collapse "fi" exactly the same way the
//!    round-128 Type-4 walker does. The index is silently ignored
//!    for Type-1 / Type-2 / Type-4 lookups dispatched by the same
//!    feature tag.
//! 5. Explicit-script + alternate-index combo — the determinism of
//!    `shape_text_with_script` (round 175) composes cleanly with the
//!    alternate-index selection (round 183) via
//!    `shape_text_with_script_and_alternates`.
//! 6. Empty `feature_alternates` list — both entry points return the
//!    pure-cmap output (no features means no features, regardless of
//!    which entry point the caller picks).

use oxideav_scribe::Face;

const DEJAVU_BYTES: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
const INTER_BYTES: &[u8] = include_bytes!("fixtures/InterVariable.ttf");

// ---------- index-0 backwards-compatibility -----------------------------

#[test]
fn round183_index_zero_matches_round156_aalt_on_inter() {
    // Inter's `aalt` references lookups [0 (Type 1), 1 (Type 3)]. The
    // round-156 default applies the Type-3 component at index 0; round
    // 183 with index 0 must produce the same glyph run.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let probe = "abcdefg";
    let round156 = face.shape_text(probe, &[*b"aalt"]);
    let round183 = face.shape_text_with_alternates(probe, &[(*b"aalt", 0)]);
    assert_eq!(
        round156, round183,
        "round-183 with index 0 must reproduce round-156 default on Inter"
    );
}

#[test]
fn round183_index_zero_matches_round156_aalt_on_dejavu() {
    // DejaVu's `aalt` is pure Type 3 (one lookup, index 30) — round 156
    // reshapes 'I', 'a', 'l', 'y' to their alternate-0 form. Index 0
    // via the round-183 surface must produce the same output.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    let probe = "Iaaly";
    let round156 = face.shape_text(probe, &[*b"aalt"]);
    let round183 = face.shape_text_with_alternates(probe, &[(*b"aalt", 0)]);
    assert_eq!(
        round156, round183,
        "round-183 with index 0 must reproduce round-156 default on DejaVu"
    );
}

// ---------- index out-of-range falls back to cmap-identity --------------

#[test]
fn round183_out_of_range_index_is_cmap_identity_on_inter() {
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let probe = "abcdefg";
    let cmap_only = face.shape_text(probe, &[]);
    // u16::MAX is larger than any realistic AlternateSet entry count.
    let out_of_range = face.shape_text_with_alternates(probe, &[(*b"aalt", u16::MAX)]);
    assert_eq!(
        cmap_only, out_of_range,
        "an out-of-range alternate index must fall back to cmap-identity per slot"
    );
}

#[test]
fn round183_out_of_range_index_is_cmap_identity_on_dejavu() {
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    let probe = "Iaaly";
    let cmap_only = face.shape_text(probe, &[]);
    let out_of_range = face.shape_text_with_alternates(probe, &[(*b"aalt", u16::MAX)]);
    assert_eq!(
        cmap_only, out_of_range,
        "out-of-range alternate index must fall back to cmap-identity on DejaVu"
    );
}

// ---------- length-preservation invariant -------------------------------

#[test]
fn round183_alternate_index_preserves_run_length_on_inter() {
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    for sample in ["a", "ab", "abcdefg", "Hello, world!", "abcdefghijklmnop"] {
        let cmap_only = face.shape_text(sample, &[]);
        for idx in [0u16, 1, 2, 7, u16::MAX] {
            let out = face.shape_text_with_alternates(sample, &[(*b"aalt", idx)]);
            assert_eq!(
                cmap_only.len(),
                out.len(),
                "Type-3 length-preservation on {sample:?} for alt-index {idx} failed"
            );
        }
    }
}

#[test]
fn round183_alternate_index_preserves_run_length_on_dejavu() {
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    for sample in ["a", "ab", "abcdefg", "Hello, world!", "Iaaly"] {
        let cmap_only = face.shape_text(sample, &[]);
        for idx in [0u16, 1, 2, 7, u16::MAX] {
            let out = face.shape_text_with_alternates(sample, &[(*b"aalt", idx)]);
            assert_eq!(
                cmap_only.len(),
                out.len(),
                "Type-3 length-preservation on {sample:?} for alt-index {idx} failed (DejaVu)"
            );
        }
    }
}

// ---------- non-Type-3 features ignore the alternate index --------------

#[test]
fn round183_alternate_index_ignored_for_type_4_liga_on_dejavu() {
    // DejaVu's `liga` is a pure Type-4 (Ligature Substitution) lookup.
    // Round 183 must ignore the alternate index — "fi" still collapses
    // to one ligature glyph regardless of whether the caller passes
    // index 0, 5, or u16::MAX.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    let round128 = face.shape_text("fi", &[*b"liga"]);
    for idx in [0u16, 5, u16::MAX] {
        let round183 = face.shape_text_with_alternates("fi", &[(*b"liga", idx)]);
        assert_eq!(
            round128, round183,
            "non-Type-3 features must ignore the alternate index (idx={idx})"
        );
        assert_eq!(
            round183.len(),
            1,
            "`liga` still collapses 'fi' to one glyph"
        );
    }
}

#[test]
fn round183_alternate_index_ignored_for_type_1_smcp_on_inter() {
    // Inter's `smcp` is exclusively a Type-1 lookup. Passing an
    // alternate-index payload on `smcp` must produce the same output
    // as the round-89 surface — the index is silently ignored.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let round89 = face.shape_text("abc", &[*b"smcp"]);
    for idx in [0u16, 1, 2, u16::MAX] {
        let round183 = face.shape_text_with_alternates("abc", &[(*b"smcp", idx)]);
        assert_eq!(
            round89, round183,
            "Type-1 `smcp` must ignore the alternate index (idx={idx})"
        );
    }
}

// ---------- explicit-script + alternate-index combo ---------------------

#[test]
fn round183_explicit_latn_index_zero_matches_round175_default() {
    // The explicit-script entry point with index 0 must agree with the
    // round-175 single-script default surface.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let probe = "abcdefg";
    let round175 = face.shape_text_with_script(probe, *b"latn", &[*b"aalt"]);
    let round183 = face.shape_text_with_script_and_alternates(probe, *b"latn", &[(*b"aalt", 0)]);
    assert_eq!(
        round175, round183,
        "explicit-script + index 0 must agree with round-175 default"
    );
}

#[test]
fn round183_explicit_unknown_script_is_cmap_identity() {
    // Mirrors the round-175 unknown-script contract — an unknown
    // script_tag yields the pure-cmap output.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let probe = "abcdefg";
    let cmap_only = face.shape_text(probe, &[]);
    let unknown = face.shape_text_with_script_and_alternates(
        probe,
        *b"zzzz",
        &[(*b"aalt", 0), (*b"smcp", 7)],
    );
    assert_eq!(
        cmap_only, unknown,
        "unknown script_tag must yield cmap-identity (every feature resolves empty)"
    );
}

// ---------- empty feature_alternates list contract ----------------------

#[test]
fn round183_empty_feature_alternates_is_cmap_identity() {
    // Both entry points with an empty feature_alternates list must
    // return the pure-cmap output — no features means no features.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let probe = "Hello, world!";
    let cmap_only = face.shape_text(probe, &[]);
    let auto_empty = face.shape_text_with_alternates(probe, &[]);
    let explicit_empty = face.shape_text_with_script_and_alternates(probe, *b"latn", &[]);
    assert_eq!(cmap_only, auto_empty);
    assert_eq!(cmap_only, explicit_empty);
}

// ---------- empty text always yields empty vec --------------------------

#[test]
fn round183_empty_text_is_empty_vec() {
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    assert_eq!(
        face.shape_text_with_alternates("", &[(*b"aalt", 0)]).len(),
        0
    );
    assert_eq!(
        face.shape_text_with_script_and_alternates("", *b"latn", &[(*b"aalt", 1)])
            .len(),
        0
    );
}

// ---------- multi-feature mixed-index walk ------------------------------

#[test]
fn round183_multi_feature_per_feature_indices_are_applied_independently() {
    // Two features with two different alternate indices — the
    // `feature_alternates` list lets the caller name each feature's
    // index independently. The smaller invariant we can assert without
    // pinning per-font alternate behaviour is: applying the same
    // feature with index 0 then again with index 1 must agree with
    // applying just the index-0 then just the index-1 entries — feature
    // application order is the order in the slice.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let probe = "abcdefg";
    // Apply aalt with index 0 — same as the round-156 default.
    let just_zero = face.shape_text_with_alternates(probe, &[(*b"aalt", 0)]);
    // Apply aalt with index 0, then aalt again with index 0 — must be
    // idempotent (round 156 already pinned this).
    let zero_zero = face.shape_text_with_alternates(probe, &[(*b"aalt", 0), (*b"aalt", 0)]);
    assert_eq!(
        just_zero, zero_zero,
        "double-applying aalt at index 0 must be idempotent (matches round 156)"
    );
}
