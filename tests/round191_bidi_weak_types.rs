//! Round 191 — UAX #9 §3.3.4 weak-type resolution (rules W1..W7)
//! integration tests.
//!
//! Mirrors the per-rule unit tests in `src/bidi.rs` but exercises the
//! public `oxideav_scribe::resolve_weak_types` re-export so the
//! surface stays stable for external callers.
//!
//! Provenance: every input is constructed by hand from the rule
//! examples in UAX #9 Revision 50 / Unicode 16.0 §3.3.4 (the dated
//! snapshot at `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`).
//! No external library source — HarfBuzz, ICU, FreeType, rustybuzz,
//! the `unicode-bidi` crate, etc. — was consulted at any point.

use oxideav_scribe::{resolve_weak_types, BidiClass};

#[test]
fn r191_public_resolve_weak_types_empty_is_noop() {
    let mut cls: Vec<BidiClass> = vec![];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert!(cls.is_empty());
}

#[test]
fn r191_public_w1_spec_examples() {
    // Spec example 1: AL NSM NSM → AL AL AL (then W3 → R R R).
    let mut cls = vec![BidiClass::AL, BidiClass::NSM, BidiClass::NSM];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::R, BidiClass::R, BidiClass::R]);

    // Spec example 2: <sos=R> NSM → <sos> R. With a trailing L the
    // sequence remains <R L> after W1; no further rule modifies it.
    let mut cls = vec![BidiClass::NSM, BidiClass::L];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::R, BidiClass::L]);

    // Spec example 3: LRI NSM → LRI ON.
    let mut cls = vec![BidiClass::LRI, BidiClass::NSM];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::LRI, BidiClass::ON]);

    // Spec example 4: PDI NSM → PDI ON.
    let mut cls = vec![BidiClass::PDI, BidiClass::NSM];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::PDI, BidiClass::ON]);
}

#[test]
fn r191_public_w2_spec_examples() {
    // Spec example: AL EN → AL AN (then W3 → R AN).
    let mut cls = vec![BidiClass::AL, BidiClass::EN];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::R, BidiClass::AN]);

    // Spec example: AL NI EN → AL NI AN (then W3 → R NI AN).
    let mut cls = vec![BidiClass::AL, BidiClass::ON, BidiClass::EN];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::R, BidiClass::ON, BidiClass::AN]);

    // Spec example: <sos> NI EN → <sos> NI EN (W2 sees no AL; sos
    // alone is not AL even when paragraph level is RTL).
    let mut cls = vec![BidiClass::ON, BidiClass::EN];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::ON, BidiClass::EN]);

    // Spec example: R NI EN → R NI EN (R as last strong is not AL).
    let mut cls = vec![BidiClass::R, BidiClass::ON, BidiClass::EN];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::R, BidiClass::ON, BidiClass::EN]);

    // Spec example: L NI EN → L NI EN by W2 (last strong is L, not
    // AL). W7 then fires → L NI L.
    let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::EN];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::L, BidiClass::ON, BidiClass::L]);
}

#[test]
fn r191_public_w3_collapses_every_al() {
    // Any AL surviving W1+W2 becomes R.
    let mut cls = vec![BidiClass::AL, BidiClass::L, BidiClass::AL];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::R, BidiClass::L, BidiClass::R]);
}

#[test]
fn r191_public_w4_spec_examples() {
    // EN ES EN → EN EN EN.
    let mut cls = vec![BidiClass::EN, BidiClass::ES, BidiClass::EN];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::EN, BidiClass::EN, BidiClass::EN]);

    // EN CS EN → EN EN EN.
    let mut cls = vec![BidiClass::EN, BidiClass::CS, BidiClass::EN];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::EN, BidiClass::EN, BidiClass::EN]);

    // AN CS AN → AN AN AN.
    let mut cls = vec![BidiClass::AN, BidiClass::CS, BidiClass::AN];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::AN, BidiClass::AN, BidiClass::AN]);
}

#[test]
fn r191_public_w5_spec_examples() {
    // ET ET EN → EN EN EN.
    let mut cls = vec![BidiClass::ET, BidiClass::ET, BidiClass::EN];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::EN, BidiClass::EN, BidiClass::EN]);

    // EN ET ET → EN EN EN.
    let mut cls = vec![BidiClass::EN, BidiClass::ET, BidiClass::ET];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::EN, BidiClass::EN, BidiClass::EN]);

    // AN ET EN → AN EN EN.
    let mut cls = vec![BidiClass::AN, BidiClass::ET, BidiClass::EN];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::AN, BidiClass::EN, BidiClass::EN]);
}

#[test]
fn r191_public_w6_spec_examples() {
    // AN ET → AN ON.
    let mut cls = vec![BidiClass::AN, BidiClass::ET];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::AN, BidiClass::ON]);

    // L ES EN → L ON EN by W6, then W7 → L ON L.
    let mut cls = vec![BidiClass::L, BidiClass::ES, BidiClass::EN];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::L, BidiClass::ON, BidiClass::L]);

    // EN CS AN → EN ON AN.
    let mut cls = vec![BidiClass::EN, BidiClass::CS, BidiClass::AN];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::EN, BidiClass::ON, BidiClass::AN]);

    // ET AN → ON AN.
    let mut cls = vec![BidiClass::ET, BidiClass::AN];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    assert_eq!(cls, vec![BidiClass::ON, BidiClass::AN]);
}

#[test]
fn r191_public_w7_spec_examples() {
    // L NI EN → L NI L.
    let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::EN];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::L, BidiClass::ON, BidiClass::L]);

    // R NI EN → R NI EN.
    let mut cls = vec![BidiClass::R, BidiClass::ON, BidiClass::EN];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(cls, vec![BidiClass::R, BidiClass::ON, BidiClass::EN]);
}

#[test]
fn r191_public_pipeline_arabic_phone_number_pattern() {
    // Real-world-ish pattern: Arabic letter (AL), a NSM combining
    // mark, an Arabic-Indic-supplement digit run, a thousands
    // separator, more digits.
    //
    //   AL NSM EN ET EN CS EN
    //
    // Step-by-step:
    //   W1: NSM after AL → AL.    [AL AL EN ET EN CS EN]
    //   W2: every EN has AL as last strong → AN.
    //                              [AL AL AN ET AN CS AN]
    //   W3: ALs → R.               [R  R  AN ET AN CS AN]
    //   W4: CS between two ANs → AN.
    //                              [R  R  AN ET AN AN AN]
    //   W5: ET is adjacent to AN, not EN (no longer any EN). No flip.
    //   W6: ET → ON.               [R  R  AN ON AN AN AN]
    //   W7: no EN left to inspect.
    let mut cls = vec![
        BidiClass::AL,
        BidiClass::NSM,
        BidiClass::EN,
        BidiClass::ET,
        BidiClass::EN,
        BidiClass::CS,
        BidiClass::EN,
    ];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    assert_eq!(
        cls,
        vec![
            BidiClass::R,
            BidiClass::R,
            BidiClass::AN,
            BidiClass::ON,
            BidiClass::AN,
            BidiClass::AN,
            BidiClass::AN,
        ]
    );
}

#[test]
fn r191_public_pipeline_latin_priced_number_pattern() {
    // Pattern: "L NI $ EN ET" — a Latin paragraph that quotes a USD
    // price. $ is ET; the leading L makes W7 dominate the trailing
    // EN run.
    //
    //   L ON ET EN ET
    //
    //   W1: no NSMs.
    //   W2: no AL anywhere → ENs untouched by W2.
    //   W3: no AL.
    //   W4: no separator-between-two-numbers pattern. (The ET is not
    //        a separator considered by W4.)
    //   W5: leading ET adjacent to EN → EN; trailing ET adjacent to
    //        EN → EN.
    //                              [L  ON EN EN EN]
    //   W6: nothing left.
    //   W7: ENs after L (L is the most recent strong) → L L L.
    //                              [L  ON L  L  L]
    let mut cls = vec![
        BidiClass::L,
        BidiClass::ON,
        BidiClass::ET,
        BidiClass::EN,
        BidiClass::ET,
    ];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(
        cls,
        vec![
            BidiClass::L,
            BidiClass::ON,
            BidiClass::L,
            BidiClass::L,
            BidiClass::L,
        ]
    );
}

#[test]
fn r191_public_w_rules_are_in_place_no_allocation_observed() {
    // resolve_weak_types operates in place: the caller's existing
    // allocation is reused and the length doesn't change.
    let mut cls = vec![
        BidiClass::L,
        BidiClass::ON,
        BidiClass::EN,
        BidiClass::ES,
        BidiClass::EN,
    ];
    let len_before = cls.len();
    let cap_before = cls.capacity();
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(cls.len(), len_before);
    assert_eq!(cls.capacity(), cap_before);
}
