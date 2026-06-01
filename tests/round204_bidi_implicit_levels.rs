//! Round 204 — UAX #9 §3.3.6 implicit-level resolution (rules I1 + I2)
//! integration tests.
//!
//! Mirrors the per-rule unit tests in `src/bidi.rs` but exercises the
//! public `oxideav_scribe::resolve_implicit_levels` re-export so the
//! surface stays stable for external callers.
//!
//! Provenance: every input is constructed by hand from the rule
//! examples in UAX #9 Revision 50 / Unicode 16.0 §3.3.6 (the dated
//! snapshot at `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`).
//!
//! The §5.2 "ignore BN" carve-out is exercised directly; the §3.3.6
//! Table 5 row coverage is exhaustive (L / R / EN / AN at both even
//! and odd embedding levels). The X-rules that set per-character
//! embedding levels above 1 are not yet implemented; the test suite
//! pretends the caller has already computed the embedding level and
//! passes it directly, which matches the public contract.

use oxideav_scribe::{
    resolve_implicit_levels, resolve_neutral_types, resolve_weak_types, BidiClass,
};

// --- §3.3.6 Table 5 row 1: type L ---------------------------------

#[test]
fn r204_public_table5_l_at_even_stays() {
    // L at EL 0 → 0 (Table 5 row L, even column).
    let cls = vec![BidiClass::L];
    assert_eq!(resolve_implicit_levels(&cls, 0), vec![0]);
    // L at EL 2 → 2.
    assert_eq!(resolve_implicit_levels(&cls, 2), vec![2]);
}

#[test]
fn r204_public_table5_l_at_odd_goes_up_one() {
    // L at EL 1 → 2 (Table 5 row L, odd column = EL+1).
    let cls = vec![BidiClass::L];
    assert_eq!(resolve_implicit_levels(&cls, 1), vec![2]);
    // L at EL 3 → 4.
    assert_eq!(resolve_implicit_levels(&cls, 3), vec![4]);
}

// --- §3.3.6 Table 5 row 2: type R ---------------------------------

#[test]
fn r204_public_table5_r_at_even_goes_up_one() {
    // R at EL 0 → 1 (Table 5 row R, even column = EL+1).
    let cls = vec![BidiClass::R];
    assert_eq!(resolve_implicit_levels(&cls, 0), vec![1]);
    // R at EL 2 → 3.
    assert_eq!(resolve_implicit_levels(&cls, 2), vec![3]);
}

#[test]
fn r204_public_table5_r_at_odd_stays() {
    // R at EL 1 → 1 (Table 5 row R, odd column = EL).
    let cls = vec![BidiClass::R];
    assert_eq!(resolve_implicit_levels(&cls, 1), vec![1]);
    // R at EL 3 → 3.
    assert_eq!(resolve_implicit_levels(&cls, 3), vec![3]);
}

// --- §3.3.6 Table 5 row 3: type AN --------------------------------

#[test]
fn r204_public_table5_an_at_even_goes_up_two() {
    // AN at EL 0 → 2 (Table 5 row AN, even column = EL+2).
    let cls = vec![BidiClass::AN];
    assert_eq!(resolve_implicit_levels(&cls, 0), vec![2]);
    assert_eq!(resolve_implicit_levels(&cls, 2), vec![4]);
}

#[test]
fn r204_public_table5_an_at_odd_goes_up_one() {
    // AN at EL 1 → 2 (Table 5 row AN, odd column = EL+1).
    let cls = vec![BidiClass::AN];
    assert_eq!(resolve_implicit_levels(&cls, 1), vec![2]);
    assert_eq!(resolve_implicit_levels(&cls, 3), vec![4]);
}

// --- §3.3.6 Table 5 row 4: type EN --------------------------------

#[test]
fn r204_public_table5_en_at_even_goes_up_two() {
    // EN at EL 0 → 2 (Table 5 row EN, even column = EL+2).
    let cls = vec![BidiClass::EN];
    assert_eq!(resolve_implicit_levels(&cls, 0), vec![2]);
    assert_eq!(resolve_implicit_levels(&cls, 2), vec![4]);
}

#[test]
fn r204_public_table5_en_at_odd_goes_up_one() {
    // EN at EL 1 → 2 (Table 5 row EN, odd column = EL+1).
    let cls = vec![BidiClass::EN];
    assert_eq!(resolve_implicit_levels(&cls, 1), vec![2]);
    assert_eq!(resolve_implicit_levels(&cls, 3), vec![4]);
}

// --- §5.2 carve-out: ignore BN ------------------------------------

#[test]
fn r204_public_bn_is_ignored() {
    // §5.2: "In rules I1 and I2, ignore BN." Mixed input with a BN
    // wedged between every strong/numeric type. The BN sits at the
    // embedding level; every other position takes the Table 5 row.
    let cls = vec![
        BidiClass::L,
        BidiClass::BN,
        BidiClass::R,
        BidiClass::BN,
        BidiClass::EN,
        BidiClass::BN,
        BidiClass::AN,
    ];
    // EL 0 (even): L=0, BN=0, R=1, BN=0, EN=2, BN=0, AN=2.
    assert_eq!(resolve_implicit_levels(&cls, 0), vec![0, 0, 1, 0, 2, 0, 2]);
    // EL 1 (odd): L=2, BN=1, R=1, BN=1, EN=2, BN=1, AN=2.
    assert_eq!(resolve_implicit_levels(&cls, 1), vec![2, 1, 1, 1, 2, 1, 2]);
}

// --- Boundary / regression cases ----------------------------------

#[test]
fn r204_public_empty_input_yields_empty_output() {
    assert_eq!(resolve_implicit_levels(&[], 0), Vec::<u8>::new());
    assert_eq!(resolve_implicit_levels(&[], 1), Vec::<u8>::new());
    assert_eq!(resolve_implicit_levels(&[], 125), Vec::<u8>::new());
}

#[test]
fn r204_public_max_depth_arithmetic_is_linear() {
    // UAX #9 §3.3.6 narration: "it is possible for text to end up at
    // level max_depth+1 as a result of this process." Confirm the
    // public surface does not clamp: at EL 124 (even, the deepest
    // even level reachable inside max_depth = 125), EN / AN reach
    // 126 — one above max_depth.
    let cls = vec![BidiClass::L, BidiClass::R, BidiClass::EN, BidiClass::AN];
    assert_eq!(resolve_implicit_levels(&cls, 124), vec![124, 125, 126, 126]);
}

#[test]
fn r204_public_end_to_end_w_n_i_pipeline_ltr_paragraph() {
    // Full §3.3.4 + §3.3.5 + §3.3.6 pipeline on a synthetic LTR
    // paragraph fragment: "L NI EN" (the canonical W7 example). After
    // W7, EN whose most-recent strong is L flips to L. After N,
    // the ON between L and L collapses to L. After I, every position
    // sits at level 0.
    let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::EN];
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
    let levels = resolve_implicit_levels(&cls, 0);
    assert_eq!(levels, vec![0, 0, 0]);
}

#[test]
fn r204_public_end_to_end_w_n_i_pipeline_rtl_paragraph() {
    // Full §3.3.4 + §3.3.5 + §3.3.6 pipeline on a synthetic RTL
    // paragraph fragment with embedded Arabic numbers: "AL NSM AN".
    // After W: NSM → AL → R, AL → R, AN stays. So slice becomes
    // [R, R, AN]. N has nothing to do (no NI). I at EL 1: R → 1,
    // R → 1, AN → 2 (Table 5 row AN, odd column = EL+1).
    let mut cls = vec![BidiClass::AL, BidiClass::NSM, BidiClass::AN];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
    let levels = resolve_implicit_levels(&cls, 1);
    assert_eq!(levels, vec![1, 1, 2]);
}

#[test]
fn r204_public_end_to_end_w_n_i_arabic_with_european_numbers() {
    // The §3.3.5 closing prose example: storage "IT IS A bmw 500, OK."
    // simplified — focus on the embedded "500" sequence within an
    // otherwise-RTL paragraph. We model the run "R EN ET EN R" at
    // EL 1: after W (no AL → no W2 / W3 rewrites; W5 flips ET to EN
    // because ET is between EN and EN), classes become
    // [R EN EN EN R]. N has nothing to do (no NI). I at EL 1: R → 1,
    // EN → 2, EN → 2, EN → 2, R → 1.
    let mut cls = vec![
        BidiClass::R,
        BidiClass::EN,
        BidiClass::ET,
        BidiClass::EN,
        BidiClass::R,
    ];
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
    let levels = resolve_implicit_levels(&cls, 1);
    assert_eq!(levels, vec![1, 2, 2, 2, 1]);
}

#[test]
fn r204_public_level_vector_length_matches_input_length() {
    // Defensive: any input length produces a same-length output.
    for n in 0..50 {
        let cls = vec![BidiClass::L; n];
        let levels = resolve_implicit_levels(&cls, 0);
        assert_eq!(levels.len(), n, "len mismatch for n={n}");
    }
}
