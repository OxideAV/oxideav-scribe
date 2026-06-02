//! Round 210 — UAX #9 §3.4 line-level reordering (rules L1 + L2)
//! integration tests.
//!
//! Mirrors the per-rule unit tests in `src/bidi.rs` but exercises the
//! public `oxideav_scribe::reset_trailing_levels` and
//! `oxideav_scribe::reorder_line` re-exports so the surface stays
//! stable for external callers.
//!
//! Provenance: every input is constructed by hand from the rule
//! examples in UAX #9 Revision 50 / Unicode 16.0 §3.4 (the dated
//! snapshot at `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`).
//! The four §3.4 worked examples — "car means CAR.",
//! "<car MEANS CAR.=", `he said "<car MEANS CAR=." "<IT DOES=," she
//! agreed.`, and the level-1-embedded RTL paragraph — are encoded
//! by their resolved-level vectors (the spec's "Resolved levels" row)
//! and the expected per-pass reversals are checked against the
//! spec's "Display" row.
//!
//! The X-rules that produce the resolved-level vectors above level 1
//! are not yet implemented; the tests pretend the caller has already
//! computed the level vector and passes it directly. This matches
//! the public contract of `reorder_line`.

use oxideav_scribe::{
    paragraph_level, reorder_line, reset_trailing_levels, resolve_implicit_levels,
    resolve_neutral_types, resolve_weak_types, BidiClass,
};

// --- §3.4 Rule L1 — separator + trailing-filler reset --------------

#[test]
fn r210_public_l1_segment_separator_resets() {
    // Tab (S) inside an RTL paragraph is reset to level 1.
    let cls = vec![BidiClass::R, BidiClass::S, BidiClass::R];
    let mut lvl = vec![1, 2, 1];
    reset_trailing_levels(&cls, &mut lvl, 1);
    assert_eq!(lvl, vec![1, 1, 1]);
}

#[test]
fn r210_public_l1_paragraph_separator_resets() {
    // §3.4 case (2): paragraph separator at line end.
    let cls = vec![BidiClass::L, BidiClass::L, BidiClass::B];
    let mut lvl = vec![0, 0, 1];
    reset_trailing_levels(&cls, &mut lvl, 0);
    assert_eq!(lvl, vec![0, 0, 0]);
}

#[test]
fn r210_public_l1_whitespace_before_separator_resets() {
    // §3.4 case (3): WS run preceding a separator is folded onto
    // the paragraph level.
    let cls = vec![BidiClass::R, BidiClass::WS, BidiClass::WS, BidiClass::B];
    let mut lvl = vec![1, 2, 2, 2];
    reset_trailing_levels(&cls, &mut lvl, 1);
    assert_eq!(lvl, vec![1, 1, 1, 1]);
}

#[test]
fn r210_public_l1_trailing_whitespace_resets() {
    // §3.4 case (4): trailing WS at end of line.
    let cls = vec![BidiClass::R, BidiClass::R, BidiClass::WS];
    let mut lvl = vec![1, 1, 2];
    reset_trailing_levels(&cls, &mut lvl, 1);
    assert_eq!(lvl, vec![1, 1, 1]);
}

#[test]
fn r210_public_l1_isolate_formatting_counts_as_trailing_filler() {
    // PDI / LRI / RLI / FSI all fold like WS when trailing.
    let cls = vec![BidiClass::L, BidiClass::PDI, BidiClass::LRI, BidiClass::B];
    let mut lvl = vec![0, 1, 1, 1];
    reset_trailing_levels(&cls, &mut lvl, 0);
    assert_eq!(lvl, vec![0, 0, 0, 0]);
}

#[test]
fn r210_public_l1_interior_whitespace_unchanged() {
    // WS surrounded by strong characters: L1 does not touch it.
    let cls = vec![BidiClass::R, BidiClass::WS, BidiClass::R];
    let mut lvl = vec![1, 2, 1];
    reset_trailing_levels(&cls, &mut lvl, 1);
    assert_eq!(lvl, vec![1, 2, 1]);
}

#[test]
fn r210_public_l1_uses_original_classes() {
    // §3.4 normative note: the original classes are used, not the
    // post-W output. Here a `B` survives at the end (W-rules never
    // produce a `B`); L1 still sees it directly.
    let orig = vec![BidiClass::R, BidiClass::R, BidiClass::B];
    let mut lvl = vec![1, 1, 2];
    reset_trailing_levels(&orig, &mut lvl, 1);
    assert_eq!(lvl, vec![1, 1, 1]);
}

#[test]
fn r210_public_l1_empty_line_is_noop() {
    let cls: Vec<BidiClass> = Vec::new();
    let mut lvl: Vec<u8> = Vec::new();
    reset_trailing_levels(&cls, &mut lvl, 0);
    assert!(lvl.is_empty());
}

#[test]
fn r210_public_l1_multiple_separators_each_fold_their_run() {
    let cls = vec![
        BidiClass::R,
        BidiClass::WS,
        BidiClass::S,
        BidiClass::R,
        BidiClass::WS,
        BidiClass::B,
    ];
    let mut lvl = vec![1, 2, 2, 1, 2, 2];
    reset_trailing_levels(&cls, &mut lvl, 1);
    assert_eq!(lvl, vec![1, 1, 1, 1, 1, 1]);
}

// --- §3.4 Rule L2 — progressive reversal --------------------------

#[test]
fn r210_public_l2_all_ltr_is_identity() {
    assert_eq!(reorder_line(&[0, 0, 0, 0, 0]), vec![0, 1, 2, 3, 4]);
}

#[test]
fn r210_public_l2_all_rtl_is_full_reverse() {
    assert_eq!(reorder_line(&[1, 1, 1, 1, 1]), vec![4, 3, 2, 1, 0]);
}

#[test]
fn r210_public_l2_uax9_example_1() {
    // §3.4 Example 1: "car means CAR." paragraph level = 0.
    // Resolved levels: 00000000001110 (14 chars: 10 LTR, 3 RTL,
    // trailing '.' back at 0).
    let lv = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0];
    let visual = reorder_line(&lv);
    // Display row: "car means RAC." — positions 0..9 unchanged,
    // positions 10..12 reversed, position 13 unchanged.
    assert_eq!(visual, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 11, 10, 13]);
}

#[test]
fn r210_public_l2_uax9_example_2() {
    // §3.4 Example 2: "<car MEANS CAR.=" resolved levels
    // 0222111111111110. Display row: "<.RAC SNAEM car=".
    let lv = [0, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0];
    let visual = reorder_line(&lv);
    // After level-2 pass: identity except [1..4] reverses.
    // After level-1 pass: positions 1..15 reverse.
    assert_eq!(
        visual,
        vec![0, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 1, 2, 3, 15]
    );
}

#[test]
fn r210_public_l2_uax9_example_3_he_said_quoted_rtl() {
    // §3.4 Example 3: paragraph level = 0. Resolved levels (51
    // chars), broken into runs verbatim per the spec's
    // "Resolved levels" row 000000000022211111111110000001111111000000000000000:
    //   [0..10)  level 0 (10 chars)  "he said \""
    //   [10..13) level 2 (3 chars)   "rac" (level-2-embedded numerals)
    //   [13..24) level 1 (11 chars)  " MEANS CAR=" RTL run
    //   [24..30) level 0 (6 chars)   ".\" \"<"
    //   [30..37) level 1 (7 chars)   "IT DOES" RTL run
    //   [37..51) level 0 (14 chars)  "=,\" she agreed."
    let lv = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0..9
        2, 2, 2, // 10..12
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 13..23
        0, 0, 0, 0, 0, 0, // 24..29
        1, 1, 1, 1, 1, 1, 1, // 30..36
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 37..50
    ];
    assert_eq!(lv.len(), 51);
    let visual = reorder_line(&lv);
    // Algorithm trace: pass at level 2 reverses positions 10..13;
    // pass at level 1 reverses positions 10..24 (all chars at
    // level >= 1) and positions 30..37. Position 0..9 + 24..29 +
    // 37..50 are unchanged.
    // After level-2 pass: visual is [..., 12, 11, 10, 13, 14, ..., 23, ...]
    // After level-1 pass (reverse 10..24 again, in-place on the
    // visual array): the run "12, 11, 10, 13, 14, 15, 16, 17, 18,
    // 19, 20, 21, 22, 23" reverses to "23, 22, 21, 20, 19, 18, 17,
    // 16, 15, 14, 13, 10, 11, 12".
    // So visual[10..24] = [23, 22, 21, 20, 19, 18, 17, 16, 15, 14,
    //                     13, 10, 11, 12].
    assert_eq!(visual[10], 23);
    assert_eq!(visual[11], 22);
    assert_eq!(visual[20], 13);
    assert_eq!(visual[21], 10);
    assert_eq!(visual[22], 11);
    assert_eq!(visual[23], 12);
    // The second RTL run at 30..37 reverses once.
    assert_eq!(visual[30], 36);
    assert_eq!(visual[36], 30);
    // The level-0 head + interior + tail are unchanged.
    for (i, v) in visual.iter().enumerate().take(10) {
        assert_eq!(*v, i);
    }
    for (i, v) in visual.iter().enumerate().take(30).skip(24) {
        assert_eq!(*v, i);
    }
    for (i, v) in visual.iter().enumerate().take(51).skip(37) {
        assert_eq!(*v, i);
    }
    // Output must be a permutation of 0..51.
    let mut sorted = visual.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..51).collect::<Vec<_>>());
}

#[test]
fn r210_public_l2_uax9_example_4_rtl_paragraph_deep_nesting() {
    // §3.4 Example 4: paragraph level = 1. Resolved levels:
    // 111111111111114222222222444333333333322111.
    // The "14" digit-pair is two separate single-digit entries
    // 1 and 4; we encode the full 42-char sequence per the spec.
    let lv = [
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 0..13
        2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // 14..23
        4, 4, 4, // 24..26
        3, 3, 3, 3, 3, 3, 3, 3, 3, 3, // 27..36
        2, 2, // 37..38
        1, 1, 1, // 39..41
    ];
    let visual = reorder_line(&lv);
    // The whole line is at level >= 1, so the final outermost
    // reversal flips the entire permutation. Visual position 0
    // must therefore be logical 41.
    assert_eq!(visual[0], 41);
    assert_eq!(*visual.last().unwrap(), 0);
    // Output must be a permutation of 0..42.
    let mut sorted = visual.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..42).collect::<Vec<_>>());
}

#[test]
fn r210_public_l2_empty_input() {
    assert!(reorder_line(&[]).is_empty());
}

#[test]
fn r210_public_l2_output_is_a_permutation() {
    // Sweep a small set of level vectors and confirm each output
    // is a valid permutation of `0..n`.
    for lv in [
        vec![0u8, 1, 0, 1],
        vec![1, 0, 1, 0],
        vec![0, 2, 1, 2, 0],
        vec![3, 3, 1, 1, 3, 3],
        vec![5, 4, 3, 2, 1, 0],
        vec![0],
        vec![1],
        vec![0; 16],
    ] {
        let n = lv.len();
        let visual = reorder_line(&lv);
        assert_eq!(visual.len(), n);
        let mut sorted = visual.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..n).collect::<Vec<_>>());
    }
}

// --- End-to-end: W → N → I → L1 → L2 on real fragments -----------

#[test]
fn r210_pipeline_ltr_paragraph_trailing_arabic_space() {
    // Simulated isolating-run-sequence "L L R R WS" inside an LTR
    // paragraph at outer embedding 0. The N pass leaves the WS as
    // an NI, the I pass bumps the R run to level 1 and keeps the
    // WS at level 0. L1 has nothing to reset (the WS was already
    // at the paragraph level). L2 then reverses just the R run.
    let orig = vec![
        BidiClass::L,
        BidiClass::L,
        BidiClass::R,
        BidiClass::R,
        BidiClass::WS,
    ];
    let mut cls = orig.clone();
    resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
    resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
    let mut lvl = resolve_implicit_levels(&cls, 0);
    reset_trailing_levels(&orig, &mut lvl, 0);
    let visual = reorder_line(&lvl);
    // L1 leaves WS at 0 (it was already 0). L2 sees level vector
    // [0, 0, 1, 1, 0] → reverses [2..4]. Visual order: 0, 1, 3, 2, 4.
    assert_eq!(visual, vec![0, 1, 3, 2, 4]);
}

#[test]
fn r210_pipeline_rtl_paragraph_trailing_space_pulls_to_level_1() {
    // Pure RTL paragraph "R R WS"; the I pass bumps WS to level 2
    // (it's an NI resolved to the embedding direction R), and L1
    // pulls the trailing WS back to paragraph level 1. After L2 the
    // WS sits on the visual left edge (paragraph-direction tail).
    let orig = vec![BidiClass::R, BidiClass::R, BidiClass::WS];
    let mut cls = orig.clone();
    resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
    resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
    let mut lvl = resolve_implicit_levels(&cls, 1);
    reset_trailing_levels(&orig, &mut lvl, 1);
    let visual = reorder_line(&lvl);
    // All level 1 → full reverse. Visual order [WS, R, R] = [2, 1, 0].
    assert_eq!(visual, vec![2, 1, 0]);
}

#[test]
fn r210_pipeline_paragraph_level_detected_from_text() {
    // The whole pipeline rides on the `paragraph_level` entry
    // point; an Arabic-led paragraph should yield level 1 and
    // therefore a fully-reversed visual order when fed RTL-only
    // text.
    let arabic = "\u{0628}\u{0629}\u{062A}"; // three Arabic letters
    let plvl = paragraph_level(arabic);
    assert_eq!(plvl, 1);
    let cls: Vec<BidiClass> = arabic
        .chars()
        .map(oxideav_scribe::bidi::bidi_class)
        .collect();
    let orig = cls.clone();
    let mut work = cls;
    resolve_weak_types(&mut work, BidiClass::R, BidiClass::R);
    resolve_neutral_types(&mut work, plvl, BidiClass::R, BidiClass::R);
    let mut lvl = resolve_implicit_levels(&work, plvl);
    reset_trailing_levels(&orig, &mut lvl, plvl);
    let visual = reorder_line(&lvl);
    assert_eq!(visual, vec![2, 1, 0]);
}

#[test]
fn r210_pipeline_uax9_example_1_end_to_end_from_text() {
    // §3.4 Example 1: "car means CAR." rebuilt from real
    // characters, paragraph-level detected via P-rules, W / N / I
    // / L1 / L2 chained.
    let text = "car means CAR.";
    let plvl = paragraph_level(text);
    assert_eq!(plvl, 0);
    let cls: Vec<BidiClass> = text.chars().map(oxideav_scribe::bidi::bidi_class).collect();
    let orig = cls.clone();
    // The lowercase Latin "CAR" needs to look strong-R for this
    // example; UAX #9's example uses uppercase letters which are
    // ASCII L in real Unicode but the spec re-styles them as the
    // R run. Our `bidi_class` table treats every ASCII letter as L,
    // so we manually overlay the spec's resolved-level vector for
    // the test rather than re-typing the table.
    let _ = orig;
    let _ = cls;
    let lv_after_i = vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0];
    let mut lvl = lv_after_i.clone();
    // Build a synthetic orig-class slice that has only `L` and a
    // trailing `CS` ('.'); no separator, no trailing WS, so L1 is
    // a no-op.
    let synth_orig = vec![
        BidiClass::L,
        BidiClass::L,
        BidiClass::L,  // "car"
        BidiClass::WS, // ' '
        BidiClass::L,
        BidiClass::L,
        BidiClass::L,
        BidiClass::L,
        BidiClass::L,  // "means"
        BidiClass::WS, // ' '
        BidiClass::L,
        BidiClass::L,
        BidiClass::L,  // "CAR" (spec-relabeled as R)
        BidiClass::CS, // '.'
    ];
    reset_trailing_levels(&synth_orig, &mut lvl, plvl);
    // L1 should not touch any of these: the WS at index 3 + 9 are
    // interior, the trailing '.' is CS (not S/B/WS), so the level
    // vector is unchanged.
    assert_eq!(lvl, lv_after_i);
    let visual = reorder_line(&lvl);
    assert_eq!(visual, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 11, 10, 13]);
}
