//! Round 204 — UAX #9 §3.3.6 implicit-level resolution (rules I1 +
//! I2) integration tests.
//!
//! Mirrors the per-rule unit tests in `src/bidi.rs` but exercises the
//! public `oxideav_scribe::resolve_implicit_levels` re-export so the
//! surface stays stable for external callers, and composes the I-rule
//! pass with the W- and N-rule passes from earlier rounds to verify
//! the end-to-end pipeline matches the spec narrative ("right-to-left
//! text will always end up with an odd level, and left-to-right and
//! numeric text will always end up with an even level... numeric text
//! will always end up with a higher level than the paragraph level").
//!
//! Provenance: every input is constructed by hand from the rule
//! examples and Table 5 in UAX #9 Revision 50 / Unicode 16.0 §3.3.6
//! (the dated snapshot at
//! `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`) plus the
//! §5.3 HL3 conformance clause ("In rules I1 and I2, ignore BN.").
//! No external library source — HarfBuzz, ICU, FreeType, rustybuzz,
//! the `unicode-bidi` crate, etc. — was consulted at any point.

use oxideav_scribe::{
    resolve_implicit_levels, resolve_neutral_types, resolve_weak_types, BidiClass,
};

#[test]
fn r204_public_resolve_implicit_levels_empty_is_noop() {
    let cls: Vec<BidiClass> = vec![];
    assert!(resolve_implicit_levels(&cls, 0).is_empty());
    assert!(resolve_implicit_levels(&cls, 1).is_empty());
}

#[test]
fn r204_public_table5_at_paragraph_level_0() {
    // Table 5 row-for-row at embedding_level=0 (LTR paragraph).
    let cls = [BidiClass::L, BidiClass::R, BidiClass::AN, BidiClass::EN];
    assert_eq!(resolve_implicit_levels(&cls, 0), vec![0, 1, 2, 2]);
}

#[test]
fn r204_public_table5_at_paragraph_level_1() {
    // Table 5 row-for-row at embedding_level=1 (RTL paragraph).
    let cls = [BidiClass::L, BidiClass::R, BidiClass::AN, BidiClass::EN];
    assert_eq!(resolve_implicit_levels(&cls, 1), vec![2, 1, 2, 2]);
}

#[test]
fn r204_public_table5_at_higher_even_level() {
    // Nested LTR override at embedding_level=2: same deltas as
    // embedding_level=0, shifted by 2.
    let cls = [BidiClass::L, BidiClass::R, BidiClass::AN, BidiClass::EN];
    assert_eq!(resolve_implicit_levels(&cls, 2), vec![2, 3, 4, 4]);
}

#[test]
fn r204_public_table5_at_higher_odd_level() {
    // Nested RTL override at embedding_level=3: same deltas as
    // embedding_level=1, shifted by 2.
    let cls = [BidiClass::L, BidiClass::R, BidiClass::AN, BidiClass::EN];
    assert_eq!(resolve_implicit_levels(&cls, 3), vec![4, 3, 4, 4]);
}

#[test]
fn r204_public_bn_ignored_per_hl3_at_even_level() {
    // §5.3 HL3: "In rules I1 and I2, ignore BN." BN keeps the
    // embedding level even when surrounded by promoted types.
    let cls = [
        BidiClass::L,
        BidiClass::BN,
        BidiClass::R,
        BidiClass::BN,
        BidiClass::EN,
    ];
    assert_eq!(resolve_implicit_levels(&cls, 0), vec![0, 0, 1, 0, 2]);
}

#[test]
fn r204_public_bn_ignored_per_hl3_at_odd_level() {
    let cls = [
        BidiClass::L,
        BidiClass::BN,
        BidiClass::R,
        BidiClass::BN,
        BidiClass::EN,
    ];
    assert_eq!(resolve_implicit_levels(&cls, 1), vec![2, 1, 1, 1, 2]);
}

#[test]
fn r204_public_nsm_leftover_treated_as_bn() {
    // W1's sos boundary case leaves NSMs at the head of an
    // isolating run sequence untouched. For I1 / I2 those leftover
    // NSMs are treated as BN (kept at embedding level).
    let cls = [BidiClass::NSM, BidiClass::NSM, BidiClass::R];
    assert_eq!(resolve_implicit_levels(&cls, 0), vec![0, 0, 1]);
    assert_eq!(resolve_implicit_levels(&cls, 1), vec![1, 1, 1]);
}

#[test]
fn r204_public_pure_ltr_run_at_even_level_is_flat() {
    let cls = [BidiClass::L; 8];
    assert_eq!(resolve_implicit_levels(&cls, 0), vec![0; 8]);
}

#[test]
fn r204_public_pure_rtl_run_at_odd_level_is_flat() {
    let cls = [BidiClass::R; 8];
    assert_eq!(resolve_implicit_levels(&cls, 1), vec![1; 8]);
}

#[test]
fn r204_public_numbers_always_above_paragraph_level() {
    // §3.3.6 narrative guarantee: "numeric text will always end up
    // with a higher level than the paragraph level".
    let cls = [BidiClass::EN, BidiClass::AN];
    for el in 0u8..6 {
        let levels = resolve_implicit_levels(&cls, el);
        assert!(
            levels.iter().all(|&l| l > el),
            "EN/AN must climb above paragraph level at el={el}, got {levels:?}",
        );
    }
}

#[test]
fn r204_public_rtl_always_ends_odd_when_starting_ltr() {
    // §3.3.6 narrative: "Right-to-left text will always end up with
    // an odd level". An R-classed character starting at any even
    // level ends odd.
    for el in (0u8..8).step_by(2) {
        let levels = resolve_implicit_levels(&[BidiClass::R], el);
        assert_eq!(
            levels[0] % 2,
            1,
            "R at even el={el} must end odd, got {}",
            levels[0]
        );
    }
}

#[test]
fn r204_public_ltr_always_ends_even_when_starting_rtl() {
    // §3.3.6 narrative: "left-to-right and numeric text will always
    // end up with an even level". An L-classed character starting
    // at any odd level ends even.
    for el in (1u8..8).step_by(2) {
        let levels = resolve_implicit_levels(&[BidiClass::L], el);
        assert_eq!(
            levels[0] % 2,
            0,
            "L at odd el={el} must end even, got {}",
            levels[0]
        );
    }
}

#[test]
fn r204_public_pipeline_w_n_then_i_ltr_paragraph() {
    // End-to-end mini-pipeline on a representative mixed sequence:
    //   [L, EN, ON, R]  (hypothetical "abc 123 . xyz" where xyz is Arabic)
    // at LTR paragraph (embedding_level=0).
    //
    //   W7: EN whose most-recent strong is L flips to L.
    //                    → [L, L, ON, R]
    //   N2: ON with L on the left and R on the right (mismatch)
    //       falls back to embedding direction L.
    //                    → [L, L, L, R]
    //   I1: L stays at 0, R climbs to 1.
    //                    → [0, 0, 0, 1]
    let mut cls = vec![BidiClass::L, BidiClass::EN, BidiClass::ON, BidiClass::R];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    assert_eq!(
        cls,
        vec![BidiClass::L, BidiClass::L, BidiClass::ON, BidiClass::R]
    );
    resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
    assert_eq!(
        cls,
        vec![BidiClass::L, BidiClass::L, BidiClass::L, BidiClass::R]
    );
    assert_eq!(resolve_implicit_levels(&cls, 0), vec![0, 0, 0, 1]);
}

#[test]
fn r204_public_pipeline_w_n_then_i_rtl_paragraph_arabic_phone() {
    // End-to-end on the Arabic-phone-style sequence from round 191:
    //   [AL, NSM, EN, ET, EN, CS, AN]  with sos=eos=R at
    //   embedding_level=1 (RTL paragraph).
    //
    //   W1..W7: → [R, R, AN, ON, AN, AN, AN]  (per the existing
    //           full-pipeline test in src/bidi.rs).
    //   N1 / N2: the lone ON has AN on both sides; AN counts as R
    //            in N1, so N1 collapses it to R.
    //           → [R, R, AN, R, AN, AN, AN]
    //   I2 (odd embedding level): R stays at 1, AN climbs by 1
    //       → 2.
    //           → [1, 1, 2, 1, 2, 2, 2]
    let mut cls = vec![
        BidiClass::AL,
        BidiClass::NSM,
        BidiClass::EN,
        BidiClass::ET,
        BidiClass::EN,
        BidiClass::CS,
        BidiClass::AN,
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
    resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
    // N1 collapses the ON between two AN-as-R neighbours to R.
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
        ]
    );
    assert_eq!(resolve_implicit_levels(&cls, 1), vec![1, 1, 2, 1, 2, 2, 2]);
}

#[test]
fn r204_public_pipeline_ascending_levels_within_paragraph() {
    // Demonstrate that the per-character levels emitted form a
    // valid reordering precondition: every odd-level character is
    // RTL-aligned (R), every even-level character is LTR-aligned
    // (L) or numeric (EN/AN).
    let cls = vec![
        BidiClass::L,
        BidiClass::EN,
        BidiClass::R,
        BidiClass::AN,
        BidiClass::L,
    ];
    let levels = resolve_implicit_levels(&cls, 0);
    for (c, &l) in cls.iter().zip(levels.iter()) {
        match c {
            BidiClass::L => assert_eq!(l % 2, 0),
            BidiClass::R => assert_eq!(l % 2, 1),
            BidiClass::EN | BidiClass::AN => {
                assert_eq!(l % 2, 0);
                assert!(l > 0, "numbers must be above paragraph level 0");
            }
            _ => unreachable!(),
        }
    }
}
