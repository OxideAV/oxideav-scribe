//! Round 217 — UAX #9 §3.3.2 explicit-level / override / isolate
//! stack pass (rules X1..X9) integration tests.
//!
//! Mirrors the per-rule unit tests in `src/bidi.rs` but exercises the
//! public `oxideav_scribe::resolve_explicit_levels` re-export so the
//! surface stays stable for external callers.
//!
//! Provenance: every input is constructed by hand from the rule
//! examples in UAX #9 Revision 50 / Unicode 16.0 §3.3.2 (the dated
//! snapshot at `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`).

use oxideav_scribe::{
    bidi_class, paragraph_level, resolve_explicit_levels, BidiClass, ExplicitLevels, MAX_DEPTH,
};

// --- X-rules basics: paragraph-level dispatch -----------------------

#[test]
fn r217_public_empty_paragraph() {
    let out = resolve_explicit_levels(&[], 0);
    assert!(out.levels.is_empty());
    assert!(out.effective_classes.is_empty());
    assert!(out.removed.is_empty());
}

#[test]
fn r217_public_plain_latin_paragraph_level_zero() {
    // "Hello" — all L at level 0, no formatting characters, X9
    // removes nothing.
    let cls: Vec<BidiClass> = "Hello".chars().map(bidi_class).collect();
    let pl = paragraph_level("Hello");
    assert_eq!(pl, 0);
    let out = resolve_explicit_levels(&cls, pl);
    assert_eq!(out.levels, vec![0; 5]);
    assert!(out.removed.iter().all(|r| !r));
    // No override → effective classes equal input.
    assert_eq!(out.effective_classes, cls);
}

#[test]
fn r217_public_plain_arabic_paragraph_level_one() {
    // "مرحبا" — all AL at level 1.
    let s = "مرحبا";
    let cls: Vec<BidiClass> = s.chars().map(bidi_class).collect();
    let pl = paragraph_level(s);
    assert_eq!(pl, 1);
    let out = resolve_explicit_levels(&cls, pl);
    assert_eq!(out.levels, vec![1; 5]);
    assert!(out.removed.iter().all(|r| !r));
}

// --- X2..X5: explicit embedding + override pushes -------------------

#[test]
fn r217_public_rle_push_pops_at_pdf() {
    // RLE L PDF — RLE pushes level 1, the L is at level 1, PDF
    // pops back.
    let cls = vec![BidiClass::RLE, BidiClass::L, BidiClass::PDF];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels[1], 1);
    assert_eq!(out.removed, vec![true, false, true]);
}

#[test]
fn r217_public_lre_pushes_level_two() {
    // LRE L PDF — LRE pushes the least even level > 0 = 2.
    let cls = vec![BidiClass::LRE, BidiClass::AL, BidiClass::PDF];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels[1], 2);
}

#[test]
fn r217_public_rlo_overrides_inner_class_to_r() {
    // RLO L PDF — the override rewrites the inner L to R.
    let cls = vec![BidiClass::RLO, BidiClass::L, BidiClass::PDF];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.effective_classes[1], BidiClass::R);
    assert_eq!(out.levels[1], 1);
}

#[test]
fn r217_public_lro_overrides_inner_class_to_l() {
    // LRO AL PDF — the override rewrites the inner AL to L.
    let cls = vec![BidiClass::LRO, BidiClass::AL, BidiClass::PDF];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.effective_classes[1], BidiClass::L);
    assert_eq!(out.levels[1], 2);
}

// --- X5a / X5b: isolate initiators ---------------------------------

#[test]
fn r217_public_rli_isolate_inner_level_one() {
    // RLI L PDI — inside the RLI scope the L gets level 1; PDI
    // matches and pops, so the *next* char (if any) is back at
    // the enclosing scope.
    let cls = vec![BidiClass::RLI, BidiClass::L, BidiClass::PDI];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels[1], 1);
    // RLI / PDI both NOT X9-removed.
    assert_eq!(out.removed, vec![false, false, false]);
}

#[test]
fn r217_public_lri_isolate_inner_level_two() {
    let cls = vec![BidiClass::LRI, BidiClass::AL, BidiClass::PDI];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels[1], 2);
}

#[test]
fn r217_public_lri_inside_rtl_paragraph_pushes_two() {
    // LRI at paragraph level 1 pushes least even > 1 = 2.
    let cls = vec![BidiClass::LRI, BidiClass::L, BidiClass::PDI];
    let out = resolve_explicit_levels(&cls, 1);
    assert_eq!(out.levels, vec![1, 2, 1]);
}

// --- X5c: FSI resolves to RLI or LRI -------------------------------

#[test]
fn r217_public_fsi_with_strong_l_resolves_lri() {
    // FSI L PDI → P2 sees L first → P3 returns 0 → treated as
    // LRI → pushes least even > 0 = 2.
    let cls = vec![BidiClass::FSI, BidiClass::L, BidiClass::PDI];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels[1], 2);
}

#[test]
fn r217_public_fsi_with_strong_r_resolves_rli() {
    // FSI AL PDI → P2 sees AL first → P3 returns 1 → treated as
    // RLI → pushes least odd > 0 = 1.
    let cls = vec![BidiClass::FSI, BidiClass::AL, BidiClass::PDI];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels[1], 1);
}

#[test]
fn r217_public_fsi_inside_isolate_skipped_by_p2() {
    // FSI LRI AL PDI L PDI — the inner LRI..PDI region is
    // skipped by P2 inside the FSI's mini-pass; the strong
    // visible to P2 inside FSI is the trailing L → resolves as
    // LRI at the FSI position.
    let cls = vec![
        BidiClass::FSI,
        BidiClass::LRI,
        BidiClass::AL,
        BidiClass::PDI,
        BidiClass::L,
        BidiClass::PDI,
    ];
    let out = resolve_explicit_levels(&cls, 0);
    // Outer FSI resolves to LRI (because P2 inside skips the
    // inner LRI..PDI and lands on the trailing L). LRI pushes
    // level 2; inside that scope the trailing L is at level 2;
    // the AL inside the inner LRI is at level 3 (one more).
    // The matching PDI for the outer FSI pops back to 0.
    //
    // Per X5b spec text "Set the LRI/RLI's embedding level to
    // the embedding level of the last entry on the directional
    // status stack" — the isolate initiator gets the *enclosing*
    // scope's level (here 0), not the new-scope level.
    assert_eq!(out.levels[0], 0); // FSI's own level = enclosing(0).
    assert_eq!(out.levels[4], 2); // Trailing L inside the FSI's LRI scope.
    assert_eq!(out.levels[5], 0); // Outer PDI back to paragraph level.
}

// --- X6: regular characters under override --------------------------

#[test]
fn r217_public_override_rewrites_only_non_formatting() {
    // RLO L L PDF — both Ls get rewritten to R.
    let cls = vec![BidiClass::RLO, BidiClass::L, BidiClass::L, BidiClass::PDF];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.effective_classes[1], BidiClass::R);
    assert_eq!(out.effective_classes[2], BidiClass::R);
}

// --- X6a: PDI behaviour --------------------------------------------

#[test]
fn r217_public_pdi_pops_embedding_inside_isolate() {
    // RLI RLE L PDI L — the PDI matches the RLI, unwinds the
    // embedding stack down to and including the matched isolate
    // frame; the trailing L is back at the paragraph level.
    let cls = vec![
        BidiClass::RLI,
        BidiClass::RLE,
        BidiClass::L,
        BidiClass::PDI,
        BidiClass::L,
    ];
    let out = resolve_explicit_levels(&cls, 0);
    // RLI scope = 1, RLE inside RLI = 3, inner L = 3.
    assert_eq!(out.levels[2], 3);
    // Trailing L at paragraph level 0.
    assert_eq!(out.levels[4], 0);
}

#[test]
fn r217_public_unmatched_pdi_ignored() {
    // PDI L — PDI at top level with no isolate above it is
    // ignored; the L is at the paragraph level.
    let cls = vec![BidiClass::PDI, BidiClass::L];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels[1], 0);
    // PDI is NOT X9-removed.
    assert!(!out.removed[0]);
}

// --- X7: PDF behaviour ---------------------------------------------

#[test]
fn r217_public_pdf_matches_inside_isolate_only() {
    // RLE L PDF L — first PDF matches the RLE; second L is at the
    // paragraph level.
    let cls = vec![BidiClass::RLE, BidiClass::L, BidiClass::PDF, BidiClass::L];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels[1], 1);
    assert_eq!(out.levels[3], 0);
}

#[test]
fn r217_public_unmatched_pdf_at_top_level_ignored() {
    // PDF L — PDF at top with no embedding above it is ignored.
    let cls = vec![BidiClass::PDF, BidiClass::L];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels[1], 0);
    assert!(out.removed[0]);
}

// --- X8: B characters always at paragraph level ---------------------

#[test]
fn r217_public_b_inside_embedding_still_at_paragraph_level() {
    // RLE L B PDF — the B is at level 0 (paragraph) per X8.
    let cls = vec![BidiClass::RLE, BidiClass::L, BidiClass::B, BidiClass::PDF];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels[1], 1);
    assert_eq!(out.levels[2], 0); // B → paragraph level.
}

// --- X9: removal flags ---------------------------------------------

#[test]
fn r217_public_x9_removes_rle_lre_rlo_lro_pdf_bn() {
    // Every formatting type that X9 removes shows up in the
    // `removed` flag set.
    let cls = vec![
        BidiClass::RLE,
        BidiClass::LRE,
        BidiClass::RLO,
        BidiClass::LRO,
        BidiClass::BN,
        BidiClass::L,
        BidiClass::PDF,
        BidiClass::PDF,
        BidiClass::PDF,
        BidiClass::PDF,
    ];
    let out = resolve_explicit_levels(&cls, 0);
    // First five are X9-removed; the L survives; the four PDFs
    // are X9-removed.
    let expected_removed = vec![true, true, true, true, true, false, true, true, true, true];
    assert_eq!(out.removed, expected_removed);
}

#[test]
fn r217_public_x9_does_not_remove_isolate_formats() {
    // LRI RLI FSI PDI — none are X9-removed.
    let cls = vec![
        BidiClass::LRI,
        BidiClass::RLI,
        BidiClass::FSI,
        BidiClass::PDI,
        BidiClass::PDI,
        BidiClass::PDI,
    ];
    let out = resolve_explicit_levels(&cls, 0);
    assert!(out.removed.iter().all(|r| !r));
}

// --- Overflow handling ---------------------------------------------

#[test]
fn r217_public_overflow_embedding_pinned_to_max_depth() {
    // 64 RLEs (each adds +2 to the level after the first) — the
    // 64th attempt is the first that would exceed MAX_DEPTH = 125,
    // so overflow_embedding kicks in. The trailing L is pinned to
    // the highest valid level.
    let mut cls = vec![BidiClass::RLE; 64];
    cls.push(BidiClass::L);
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels[64], 125);
}

#[test]
fn r217_public_overflow_isolate_decrements_on_matching_pdi() {
    // Two isolates: outer RLI at level 1, inner RLI at level 3,
    // a normal letter, then two PDIs to close. Both PDIs pop
    // their respective isolates. No overflow.
    let cls = vec![
        BidiClass::RLI,
        BidiClass::RLI,
        BidiClass::L,
        BidiClass::PDI,
        BidiClass::PDI,
        BidiClass::L,
    ];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels[2], 3);
    assert_eq!(out.levels[5], 0);
}

// --- Public-surface re-export sanity --------------------------------

#[test]
fn r217_public_max_depth_constant_is_125() {
    assert_eq!(MAX_DEPTH, 125);
}

#[test]
fn r217_public_explicit_levels_struct_carries_three_vecs() {
    // Type-level: ExplicitLevels has the three documented fields.
    let out: ExplicitLevels = resolve_explicit_levels(&[BidiClass::L], 0);
    let _: Vec<u8> = out.levels;
    let _: Vec<BidiClass> = out.effective_classes;
    let _: Vec<bool> = out.removed;
}

// --- Real-world mixed text: from string input -----------------------

#[test]
fn r217_public_mixed_latin_hebrew_via_paragraph_level() {
    // "abc אבג" — Latin then Hebrew at paragraph level 0. No
    // explicit formatting → every char gets level 0 from X6.
    // (The W / N / I phases later bump the Hebrew up to level 1.)
    let s = "abc אבג";
    let cls: Vec<BidiClass> = s.chars().map(bidi_class).collect();
    let pl = paragraph_level(s);
    assert_eq!(pl, 0);
    let out = resolve_explicit_levels(&cls, pl);
    assert_eq!(out.levels, vec![0; s.chars().count()]);
}

#[test]
fn r217_public_explicit_rle_around_latin_no_op_on_l() {
    // RLE Hello PDF at paragraph 0 — the Latin gets level 1; the
    // override status is neutral, so the effective classes are
    // unchanged.
    let mut cls = vec![BidiClass::RLE];
    cls.extend("Hello".chars().map(bidi_class));
    cls.push(BidiClass::PDF);
    let out = resolve_explicit_levels(&cls, 0);
    for i in 1..=5 {
        assert_eq!(out.levels[i], 1);
        assert_eq!(out.effective_classes[i], BidiClass::L);
    }
}
