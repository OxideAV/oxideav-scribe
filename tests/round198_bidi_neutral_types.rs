//! Round 198 — UAX #9 §3.3.5 neutral-type resolution (rules N1 + N2)
//! integration tests.
//!
//! Mirrors the per-rule unit tests in `src/bidi.rs` but exercises the
//! public `oxideav_scribe::resolve_neutral_types` re-export so the
//! surface stays stable for external callers.
//!
//! Provenance: every input is constructed by hand from the rule
//! examples in UAX #9 Revision 50 / Unicode 16.0 §3.3.5 (the dated
//! snapshot at `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`).
//! No external library source was consulted at any point — every
//! expectation derives from the UAX #9 rule examples cited above.
//!
//! N0 (bracket-pair resolution) is **not** covered by this round —
//! the per-codepoint Bidi_Paired_Bracket / Bidi_Paired_Bracket_Type
//! data file is not yet vendored. The N-rule routine documents the
//! gap; the §3.3.5 paragraph text states that "if the enclosed text
//! contains no strong types the bracket pairs will both resolve to
//! the same level when resolved individually using rules N1 and
//! N2", so the surface is forward-compatible: an N0 implementation
//! lands as a pre-N1 pass that turns bracket positions into strong
//! types, after which the N1 + N2 surface here continues to apply
//! unchanged.

use oxideav_scribe::{resolve_neutral_types, resolve_weak_types, BidiClass};

#[test]
fn r198_public_resolve_neutral_types_empty_is_noop() {
    let mut cls: Vec<BidiClass> = vec![];
    resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
    assert!(cls.is_empty());
}

#[test]
fn r198_public_n1_l_ni_l_collapses() {
    // Spec §3.3.5 N1 example #1: L NI L → L L L.
    let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::L];
    resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::L, BidiClass::L, BidiClass::L]);
}

#[test]
fn r198_public_n1_r_ni_r_collapses() {
    // Spec §3.3.5 N1 example #2: R NI R → R R R.
    let mut cls = vec![BidiClass::R, BidiClass::ON, BidiClass::R];
    resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::R, BidiClass::R, BidiClass::R]);
}

#[test]
fn r198_public_n1_full_numbers_as_r_table() {
    // Spec §3.3.5 N1 examples #3..#10: every R/AN/EN cross with NI
    // resolves to R because AN / EN both count as R for the N1
    // strong-neighbour search.
    let cases: &[(&[BidiClass], &[BidiClass])] = &[
        // R NI AN → R R AN
        (
            &[BidiClass::R, BidiClass::ON, BidiClass::AN],
            &[BidiClass::R, BidiClass::R, BidiClass::AN],
        ),
        // R NI EN → R R EN
        (
            &[BidiClass::R, BidiClass::ON, BidiClass::EN],
            &[BidiClass::R, BidiClass::R, BidiClass::EN],
        ),
        // AN NI R → AN R R
        (
            &[BidiClass::AN, BidiClass::ON, BidiClass::R],
            &[BidiClass::AN, BidiClass::R, BidiClass::R],
        ),
        // AN NI AN → AN R AN
        (
            &[BidiClass::AN, BidiClass::ON, BidiClass::AN],
            &[BidiClass::AN, BidiClass::R, BidiClass::AN],
        ),
        // AN NI EN → AN R EN
        (
            &[BidiClass::AN, BidiClass::ON, BidiClass::EN],
            &[BidiClass::AN, BidiClass::R, BidiClass::EN],
        ),
        // EN NI R → EN R R
        (
            &[BidiClass::EN, BidiClass::ON, BidiClass::R],
            &[BidiClass::EN, BidiClass::R, BidiClass::R],
        ),
        // EN NI AN → EN R AN
        (
            &[BidiClass::EN, BidiClass::ON, BidiClass::AN],
            &[BidiClass::EN, BidiClass::R, BidiClass::AN],
        ),
        // EN NI EN → EN R EN
        (
            &[BidiClass::EN, BidiClass::ON, BidiClass::EN],
            &[BidiClass::EN, BidiClass::R, BidiClass::EN],
        ),
    ];
    for (input, expected) in cases {
        let mut cls = input.to_vec();
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
        assert_eq!(&cls[..], *expected, "input was {input:?}");
    }
}

#[test]
fn r198_public_n2_embedding_direction_fallback() {
    // Spec §3.3.5 N2 narration: "Any remaining NIs take the embedding
    // direction." Test all 4 cases of mismatched strong neighbours
    // crossed with both embedding levels.
    //
    // L NI R, level 0 (L) → L L R.
    let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::R];
    resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::L, BidiClass::L, BidiClass::R]);
    // L NI R, level 1 (R) → L R R.
    let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::R];
    resolve_neutral_types(&mut cls, 1, BidiClass::L, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::L, BidiClass::R, BidiClass::R]);
    // R NI L, level 0 (L) → R L L.
    let mut cls = vec![BidiClass::R, BidiClass::ON, BidiClass::L];
    resolve_neutral_types(&mut cls, 0, BidiClass::R, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::R, BidiClass::L, BidiClass::L]);
    // R NI L, level 1 (R) → R R L.
    let mut cls = vec![BidiClass::R, BidiClass::ON, BidiClass::L];
    resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::R, BidiClass::R, BidiClass::L]);
}

#[test]
fn r198_public_n2_spec_footnote_examples() {
    // Spec §3.3.5 N2 narrative footnote (sos=R, eos=L examples):
    //
    //   L   NI eos → L   L eos
    //   R   NI eos → R   e eos   (eos=L, embedding direction wins)
    //   sos NI L   → sos e L     (sos=R, mismatch, embedding wins)
    //   sos NI R   → sos R R
    //
    // Encoded here as boundary-spanning slices.

    // L NI <eos=L>, level 0: trailing strong from eos is L; L on
    // left, L on right → N1 → L L. (Final entry is eos; we model
    // only the [L, NI] slice with eos=L.)
    let mut cls = vec![BidiClass::L, BidiClass::ON];
    resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::L, BidiClass::L]);

    // R NI <eos=L>, level 0 (L): mismatch (R vs L) → embedding L.
    // → R L.
    let mut cls = vec![BidiClass::R, BidiClass::ON];
    resolve_neutral_types(&mut cls, 0, BidiClass::R, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::R, BidiClass::L]);

    // <sos=R> NI L, level 0 (L): mismatch (R vs L) → embedding L.
    // → L L.
    let mut cls = vec![BidiClass::ON, BidiClass::L];
    resolve_neutral_types(&mut cls, 0, BidiClass::R, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::L, BidiClass::L]);

    // <sos=R> NI R, level 1 (R): N1 (R both sides) → R R.
    let mut cls = vec![BidiClass::ON, BidiClass::R];
    resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::R, BidiClass::R]);
}

#[test]
fn r198_public_n1_full_ni_alias_collapses() {
    // Every member of the NI alias (B / S / WS / ON / LRI / RLI /
    // FSI / PDI) participates in the same run and all flip to the
    // resolved direction.
    let mut cls = vec![
        BidiClass::L,
        BidiClass::B,
        BidiClass::S,
        BidiClass::WS,
        BidiClass::ON,
        BidiClass::LRI,
        BidiClass::RLI,
        BidiClass::FSI,
        BidiClass::PDI,
        BidiClass::L,
    ];
    resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
    assert!(cls.iter().all(|c| *c == BidiClass::L));
}

#[test]
fn r198_public_nsm_and_bn_pass_through() {
    // NSM and BN are NOT in the NI alias — N1 / N2 must not touch
    // them.
    let mut cls = vec![
        BidiClass::R,
        BidiClass::NSM,
        BidiClass::ON,
        BidiClass::BN,
        BidiClass::R,
    ];
    resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
    // The lone ON between R-NSM and BN-R: left strong walks back
    // past NSM and lands on R; right strong walks forward past BN
    // and lands on R. Match → ON → R. NSM and BN stay.
    assert_eq!(
        cls,
        vec![
            BidiClass::R,
            BidiClass::NSM,
            BidiClass::R,
            BidiClass::BN,
            BidiClass::R,
        ]
    );
}

#[test]
fn r198_public_full_pipeline_w_then_n() {
    // The realistic compose test: an Arabic-style mixed run through
    // resolve_weak_types and then resolve_neutral_types lands at a
    // fully-resolved no-NI / no-AL / no-ES/ET/CS slice ready for
    // the §3.3.6 implicit-level pass.
    let mut cls = vec![
        BidiClass::AL,
        BidiClass::NSM,
        BidiClass::EN,
        BidiClass::ET,
        BidiClass::EN,
        BidiClass::CS,
        BidiClass::AN,
        BidiClass::WS,
        BidiClass::R,
    ];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
    // After W: [R R AN ON AN AN AN WS R].
    // After N: position 3 ON is between AN (R-like) and AN (R-like)
    //          → N1 → R. Position 7 WS is between AN (R-like) and R
    //          → N1 → R.
    // Final: [R R AN R AN AN AN R R].
    assert_eq!(
        cls,
        vec![
            BidiClass::R,
            BidiClass::R,
            BidiClass::AN,
            BidiClass::R,
            BidiClass::AN,
            BidiClass::AN,
            BidiClass::AN,
            BidiClass::R,
            BidiClass::R,
        ]
    );
    // Invariant: no NI survives the N pass.
    for c in &cls {
        assert!(
            !c.is_neutral_or_isolate(),
            "neutral / isolate {c:?} survived N1+N2",
        );
    }
}

#[test]
fn r198_public_paragraph_only_neutrals_uses_eos_sos() {
    // A run with no strong elements at all: both endpoints fall back
    // to sos / eos. The pair drives N1 / N2.
    //
    // sos=L eos=L → N1 fires (match) → all L.
    let mut cls = vec![BidiClass::ON, BidiClass::WS, BidiClass::ON];
    resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::L, BidiClass::L, BidiClass::L]);
    // sos=R eos=L mismatch → N2 → embedding (1 = R) → all R.
    let mut cls = vec![BidiClass::ON, BidiClass::WS, BidiClass::ON];
    resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::R, BidiClass::R, BidiClass::R]);
}

#[test]
fn r198_public_idempotent_when_no_ni_present() {
    // Running N on a slice with no NI is a no-op.
    let original = vec![
        BidiClass::L,
        BidiClass::R,
        BidiClass::EN,
        BidiClass::AN,
        BidiClass::NSM,
        BidiClass::BN,
    ];
    let mut cls = original.clone();
    resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
    assert_eq!(cls, original);
    let mut cls = original.clone();
    resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
    assert_eq!(cls, original);
}
