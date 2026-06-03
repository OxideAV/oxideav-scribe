//! Round 227 — UAX #9 §3 whole-paragraph driver
//! (`process_paragraph` / `process_paragraph_classes` / `ParagraphBidi`).
//!
//! Exercises the public composition entry point that wires the
//! existing per-rule passes (P → X → W → N → I) into a single call
//! returning a [`oxideav_scribe::ParagraphBidi`] carrier ready for
//! line-level L1 + L2.
//!
//! Provenance: every input is constructed by hand from the rule
//! examples in UAX #9 Revision 50 / Unicode 16.0 §3 + §3.3.1 + §3.3.2
//! + §3.3.3 + §3.3.4 + §3.3.5 + §3.3.6 + §3.4 (the dated snapshot at
//!   `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`).

use oxideav_scribe::{
    bidi_class, process_paragraph, process_paragraph_classes, reorder_line, reset_trailing_levels,
    BidiClass, ParagraphBidi,
};

// --- §3.3.1 P2 + P3: paragraph-level resolution --------------------

#[test]
fn r227_public_p2_first_strong_l_returns_ltr_paragraph() {
    let cls = vec![BidiClass::L, BidiClass::L, BidiClass::L];
    let p = process_paragraph_classes(&cls, None);
    assert_eq!(p.paragraph_level, 0);
    assert_eq!(p.levels, vec![0, 0, 0]);
}

#[test]
fn r227_public_p2_first_strong_r_returns_rtl_paragraph() {
    let cls = vec![BidiClass::R, BidiClass::R, BidiClass::R];
    let p = process_paragraph_classes(&cls, None);
    assert_eq!(p.paragraph_level, 1);
    assert_eq!(p.levels, vec![1, 1, 1]);
}

#[test]
fn r227_public_p2_first_strong_al_returns_rtl_paragraph() {
    let cls = vec![BidiClass::AL, BidiClass::AL];
    let p = process_paragraph_classes(&cls, None);
    assert_eq!(p.paragraph_level, 1);
    // W3 rewrites AL → R inside the per-sequence pass; the
    // effective_classes published in the carrier preserves the AL
    // since X1..X9 do not run W-rules.
    assert_eq!(p.effective_classes, vec![BidiClass::AL, BidiClass::AL]);
    assert_eq!(p.levels, vec![1, 1]);
}

#[test]
fn r227_public_p3_no_strong_defaults_to_ltr() {
    // No strong character → P3 returns 0.
    let cls = vec![BidiClass::WS, BidiClass::ON, BidiClass::WS];
    let p = process_paragraph_classes(&cls, None);
    assert_eq!(p.paragraph_level, 0);
    assert_eq!(p.levels, vec![0, 0, 0]);
}

#[test]
fn r227_public_p2_skips_isolate_span_per_bd8() {
    // BD8: the P2 walk skips the contents of LRI .. matching PDI.
    let cls = vec![BidiClass::LRI, BidiClass::L, BidiClass::PDI, BidiClass::R];
    let p = process_paragraph_classes(&cls, None);
    assert_eq!(p.paragraph_level, 1);
}

// --- HL1: caller-provided base level overrides P2 / P3 -------------

#[test]
fn r227_public_hl1_base_level_one_forces_rtl_on_l_paragraph() {
    let cls = vec![BidiClass::L, BidiClass::L, BidiClass::L];
    let p = process_paragraph_classes(&cls, Some(1));
    assert_eq!(p.paragraph_level, 1);
    // I1 under odd EL: L → EL + 1.
    assert_eq!(p.levels, vec![2, 2, 2]);
}

#[test]
fn r227_public_hl1_base_level_zero_forces_ltr_on_r_paragraph() {
    let cls = vec![BidiClass::R, BidiClass::R, BidiClass::R];
    let p = process_paragraph_classes(&cls, Some(0));
    assert_eq!(p.paragraph_level, 0);
    // I1 under even EL: R → EL + 1.
    assert_eq!(p.levels, vec![1, 1, 1]);
}

// --- §3 end-to-end: mixed L / R compositions ----------------------

#[test]
fn r227_public_mixed_l_r_block_lifts_r_to_level_one_in_ltr_paragraph() {
    // "ABC abc" with abc as Hebrew. Paragraph level 0.
    let cls = vec![
        BidiClass::L,
        BidiClass::L,
        BidiClass::L, // ABC
        BidiClass::WS,
        BidiClass::R,
        BidiClass::R,
        BidiClass::R, // hbr
    ];
    let p = process_paragraph_classes(&cls, None);
    assert_eq!(p.paragraph_level, 0);
    assert_eq!(p.levels, vec![0, 0, 0, 0, 1, 1, 1]);
}

#[test]
fn r227_public_mixed_rtl_paragraph_lifts_l_to_level_two() {
    // RTL paragraph (first strong R), embedded Latin block goes
    // to level 2 (X-stack puts it at the surrounding RTL scope's
    // level 1, then I1 under odd-EL says L → +1 = 2).
    let cls = vec![
        BidiClass::R,
        BidiClass::R,
        BidiClass::WS,
        BidiClass::L,
        BidiClass::L,
        BidiClass::L,
        BidiClass::WS,
        BidiClass::R,
        BidiClass::R,
    ];
    let p = process_paragraph_classes(&cls, None);
    assert_eq!(p.paragraph_level, 1);
    assert_eq!(p.levels, vec![1, 1, 1, 2, 2, 2, 1, 1, 1]);
}

#[test]
fn r227_public_arabic_numbers_pipeline_w3_collapses_al_to_r() {
    // "AL EN" — W2 rewrites EN → AN because the most-recent strong
    // is AL; W3 then rewrites AL → R; I1 lifts the AN to EL + 2.
    let cls = vec![BidiClass::AL, BidiClass::EN];
    let p = process_paragraph_classes(&cls, None);
    assert_eq!(p.paragraph_level, 1);
    // AL → R at level 1; AN at level 2 (I1 odd-EL: AN → +1 → 2).
    assert_eq!(p.levels, vec![1, 2]);
}

// --- §3.3.2 X9 removed positions carry the X-rule level ----------

#[test]
fn r227_public_x9_removed_chars_keep_x_level_in_paragraph_levels() {
    // L RLE L PDF L — RLE / PDF are removed by X9; W / N / I skip
    // them. Their `levels` entry is whatever X1..X9 assigned.
    let cls = vec![
        BidiClass::L,
        BidiClass::RLE,
        BidiClass::L,
        BidiClass::PDF,
        BidiClass::L,
    ];
    let p = process_paragraph_classes(&cls, None);
    assert_eq!(p.removed, vec![false, true, false, true, false]);
    // Outer L at level 0; middle L sits inside an RLE-pushed odd
    // scope (X-level 1) → I1 odd-EL lifts L by +1 → 2.
    assert_eq!(p.levels[0], 0);
    assert_eq!(p.levels[2], 2);
    assert_eq!(p.levels[4], 0);
}

// --- §3 driver on text strings (char_byte_offsets parity) --------

#[test]
fn r227_public_process_paragraph_text_ascii_offsets_match_indices() {
    let (p, offsets) = process_paragraph("Hello", None);
    assert_eq!(p.paragraph_level, 0);
    assert_eq!(p.classes.len(), 5);
    assert_eq!(offsets, vec![0, 1, 2, 3, 4]);
    assert_eq!(p.levels, vec![0; 5]);
}

#[test]
fn r227_public_process_paragraph_text_hebrew_returns_rtl_paragraph() {
    // Three Hebrew letters: each is 2 UTF-8 bytes (U+05D0..U+05D2).
    let (p, offsets) = process_paragraph("\u{05D0}\u{05D1}\u{05D2}", None);
    assert_eq!(p.paragraph_level, 1);
    assert_eq!(offsets, vec![0, 2, 4]);
    assert_eq!(p.levels, vec![1, 1, 1]);
}

#[test]
fn r227_public_process_paragraph_text_mixed_l_arabic_paragraph_level_l() {
    // "Hi <arabic-letter>" — the first strong char is L, so the
    // paragraph is LTR. The AL block goes to level 1 via X-stack +
    // W3 + I1.
    let s = "Hi \u{0627}\u{0628}";
    let (p, offsets) = process_paragraph(s, None);
    assert_eq!(p.paragraph_level, 0);
    assert_eq!(p.classes.len(), 5);
    // Each Arabic letter is 2 bytes; "Hi " is 3 ASCII bytes.
    assert_eq!(offsets, vec![0, 1, 2, 3, 5]);
    // L L WS at level 0; AL AL at level 1.
    assert_eq!(p.levels, vec![0, 0, 0, 1, 1]);
}

// --- ParagraphBidi line-level reordering --------------------------

#[test]
fn r227_public_reorder_paragraph_ltr_only_is_identity() {
    let p = process_paragraph_classes(&[BidiClass::L; 4], None);
    let perm = p.reorder_paragraph();
    assert_eq!(perm, vec![0, 1, 2, 3]);
}

#[test]
fn r227_public_reorder_paragraph_rtl_block_reverses() {
    let cls = vec![
        BidiClass::L,
        BidiClass::L,
        BidiClass::L,
        BidiClass::R,
        BidiClass::R,
        BidiClass::R,
    ];
    let p = process_paragraph_classes(&cls, None);
    let perm = p.reorder_paragraph();
    // L block keeps order, R block reverses.
    assert_eq!(perm, vec![0, 1, 2, 5, 4, 3]);
}

#[test]
fn r227_public_reorder_line_range_per_line_works_on_split_paragraph() {
    // Wrap "ABCDEF" across two lines [0..3] and [3..6].
    let p = process_paragraph_classes(&[BidiClass::L; 6], None);
    let line1 = p.reorder_line_range(0..3);
    let line2 = p.reorder_line_range(3..6);
    assert_eq!(line1, vec![0, 1, 2]);
    assert_eq!(line2, vec![0, 1, 2]);
}

#[test]
fn r227_public_reorder_line_range_l1_resets_trailing_whitespace() {
    // RTL paragraph with trailing WS: after L1 the WS slides back
    // to the paragraph level. End-to-end test: build via the
    // driver, then check the per-line permutation matches what
    // running L1 + L2 manually produces.
    let cls = vec![BidiClass::R, BidiClass::R, BidiClass::WS, BidiClass::WS];
    let p = process_paragraph_classes(&cls, None);
    let perm = p.reorder_line_range(0..4);
    // Expected: trailing WS is L1-reset to level 1; under L2's
    // top-down reversal at level 1, the whole line reverses.
    let mut expected_levels = p.levels.clone();
    reset_trailing_levels(&p.classes, &mut expected_levels, p.paragraph_level);
    let expected_perm = reorder_line(&expected_levels);
    assert_eq!(perm, expected_perm);
}

// --- Worked example: §3.4 four published spec runs -----------------

#[test]
fn r227_public_spec_example_car_means_car_period() {
    // "car means CAR." — Latin lowercase + RTL block + period.
    // LTR paragraph. Lowercase Latin stays at 0; the imagined CAR
    // RTL block (encoded here as R R R) goes to level 1.
    let cls = vec![
        BidiClass::L,
        BidiClass::L,
        BidiClass::L, // car
        BidiClass::WS,
        BidiClass::L,
        BidiClass::L,
        BidiClass::L,
        BidiClass::L,
        BidiClass::L, // means
        BidiClass::WS,
        BidiClass::R,
        BidiClass::R,
        BidiClass::R,  // CAR (Hebrew)
        BidiClass::ON, // .
    ];
    let p = process_paragraph_classes(&cls, None);
    assert_eq!(p.paragraph_level, 0);
    let perm = p.reorder_paragraph();
    // Visual order: car means RAC. — the RTL block reverses, the
    // surrounding LTR stays in place.
    let expected = vec![
        0, 1, 2, // car
        3, // WS
        4, 5, 6, 7, 8, // means
        9, // WS
        12, 11, 10, // RAC (reversed)
        13, // .
    ];
    assert_eq!(perm, expected);
}

// --- carrier shape sanity ------------------------------------------

#[test]
fn r227_public_carrier_field_lengths_match_input() {
    let cls = vec![BidiClass::L, BidiClass::WS, BidiClass::R, BidiClass::L];
    let p = process_paragraph_classes(&cls, None);
    let n = cls.len();
    assert_eq!(p.classes.len(), n);
    assert_eq!(p.effective_classes.len(), n);
    assert_eq!(p.removed.len(), n);
    assert_eq!(p.levels.len(), n);
}

#[test]
fn r227_public_carrier_classes_preserves_input_unchanged() {
    // The `classes` field must mirror the input verbatim — L1 needs
    // the *original* types per §3.4 normative note.
    let cls: Vec<_> = "Hi \u{05D0}\u{05D1}".chars().map(bidi_class).collect();
    let p = process_paragraph_classes(&cls, None);
    assert_eq!(p.classes, cls);
}

#[test]
fn r227_public_paragraph_bidi_is_clone_eq_debug() {
    // Carrier must satisfy the standard derive set so a renderer
    // can snapshot it for memoisation.
    let p = process_paragraph_classes(&[BidiClass::L], None);
    let q: ParagraphBidi = p.clone();
    assert_eq!(p, q);
    let s = format!("{p:?}");
    assert!(s.contains("ParagraphBidi"));
}
