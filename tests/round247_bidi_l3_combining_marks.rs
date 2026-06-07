//! Round 247 — UAX #9 §3.4 rule **L3** (combining-mark reordering for
//! right-to-left bases) integration tests.
//!
//! Mirrors the per-rule unit tests in `src/bidi.rs` but exercises the
//! public `oxideav_scribe::reorder_combining_marks` re-export so the
//! surface stays stable for external callers. Composes with the
//! existing L1 + L2 pair from round 210 to show the full §3.4
//! pipeline an RTL-aware renderer drives.
//!
//! Provenance: every input is constructed by hand from rule **L3**
//! in UAX #9 Revision 50 / Unicode 16.0 §3.4 (the dated snapshot at
//! `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`):
//!
//! > Combining marks applied to a right-to-left base character will
//! > at this point precede their base character. If the rendering
//! > engine expects them to follow the base characters in the final
//! > display process, then the ordering of the marks and the base
//! > character must be reversed.

use oxideav_scribe::{reorder_combining_marks, reorder_line, reset_trailing_levels, BidiClass};

// --- Single-base RTL clusters -------------------------------------

#[test]
fn r247_public_rtl_base_one_mark_reverses_to_base_first() {
    // Logical: R NSM, level 1 1. L2 → [1, 0]. L3 → [0, 1].
    let cls = [BidiClass::R, BidiClass::NSM];
    let lvl = [1, 1];
    let mut visual = reorder_line(&lvl);
    assert_eq!(visual, vec![1, 0]);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    assert_eq!(visual, vec![0, 1]);
}

#[test]
fn r247_public_rtl_base_two_marks_reverses_block() {
    // R NSM NSM at level 1. L2 → [2, 1, 0]. L3 → [0, 1, 2].
    let cls = [BidiClass::R, BidiClass::NSM, BidiClass::NSM];
    let lvl = [1, 1, 1];
    let mut visual = reorder_line(&lvl);
    assert_eq!(visual, vec![2, 1, 0]);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    assert_eq!(visual, vec![0, 1, 2]);
}

#[test]
fn r247_public_rtl_base_three_marks_reverses_block() {
    let cls = [BidiClass::R, BidiClass::NSM, BidiClass::NSM, BidiClass::NSM];
    let lvl = [1, 1, 1, 1];
    let mut visual = reorder_line(&lvl);
    assert_eq!(visual, vec![3, 2, 1, 0]);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    assert_eq!(visual, vec![0, 1, 2, 3]);
}

#[test]
fn r247_public_arabic_letter_with_mark_uses_al_base() {
    // AL is the strong-Arabic class; W3 retypes it to R for level
    // resolution, so this fragment also lands at level 1.
    let cls = [BidiClass::AL, BidiClass::NSM];
    let lvl = [1, 1];
    let mut visual = reorder_line(&lvl);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    assert_eq!(visual, vec![0, 1]);
}

// --- Multi-cluster RTL --------------------------------------------

#[test]
fn r247_public_two_rtl_clusters_rotate_independently() {
    // R NSM R NSM at level 1.
    let cls = [BidiClass::R, BidiClass::NSM, BidiClass::R, BidiClass::NSM];
    let lvl = [1, 1, 1, 1];
    let mut visual = reorder_line(&lvl);
    assert_eq!(visual, vec![3, 2, 1, 0]);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    // Two [NSM, base] blocks each reverse: result is
    // [base2, nsm3, base0, nsm1].
    assert_eq!(visual, vec![2, 3, 0, 1]);
}

#[test]
fn r247_public_rtl_clusters_with_different_mark_counts() {
    // R NSM NSM R NSM at level 1.
    let cls = [
        BidiClass::R,
        BidiClass::NSM,
        BidiClass::NSM,
        BidiClass::R,
        BidiClass::NSM,
    ];
    let lvl = [1, 1, 1, 1, 1];
    let mut visual = reorder_line(&lvl);
    assert_eq!(visual, vec![4, 3, 2, 1, 0]);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    // First block [4, 3] reverses to [3, 4]; second block
    // [2, 1, 0] reverses to [0, 1, 2]. Final: [3, 4, 0, 1, 2].
    assert_eq!(visual, vec![3, 4, 0, 1, 2]);
}

// --- Mixed LTR / RTL paragraphs -----------------------------------

#[test]
fn r247_public_ltr_marks_untouched() {
    // L NSM at level 0 — L2 is a no-op for even levels, marks
    // already follow base. L3 leaves the visual order alone.
    let cls = [BidiClass::L, BidiClass::NSM];
    let lvl = [0, 0];
    let mut visual = reorder_line(&lvl);
    assert_eq!(visual, vec![0, 1]);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    assert_eq!(visual, vec![0, 1]);
}

#[test]
fn r247_public_rtl_island_in_ltr_paragraph() {
    // Logical L L R NSM L L (a Hebrew letter + accent embedded
    // inside Latin text) with levels 0 0 1 1 0 0.
    let cls = [
        BidiClass::L,
        BidiClass::L,
        BidiClass::R,
        BidiClass::NSM,
        BidiClass::L,
        BidiClass::L,
    ];
    let lvl = [0, 0, 1, 1, 0, 0];
    let mut visual = reorder_line(&lvl);
    // L2 reverses the [R, NSM] level-1 island.
    assert_eq!(visual, vec![0, 1, 3, 2, 4, 5]);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    // L3 swaps the island back so the base precedes its mark.
    assert_eq!(visual, vec![0, 1, 2, 3, 4, 5]);
}

#[test]
fn r247_public_ltr_island_in_rtl_paragraph() {
    // Logical R R L NSM R R with levels 1 1 2 2 1 1 (a Latin word
    // + accent embedded inside an RTL paragraph). L2 reverses the
    // outer level-1 frame and the inner level-2 island stays put
    // visually because it reverses twice (once at level 2, once
    // at level 1).
    let cls = [
        BidiClass::R,
        BidiClass::R,
        BidiClass::L,
        BidiClass::NSM,
        BidiClass::R,
        BidiClass::R,
    ];
    let lvl = [1, 1, 2, 2, 1, 1];
    let mut visual = reorder_line(&lvl);
    // Hand-trace: max=2, lowest_odd=1.
    // Pass level=2: [0,1,3,2,4,5].
    // Pass level=1: full reverse → [5,4,2,3,1,0].
    assert_eq!(visual, vec![5, 4, 2, 3, 1, 0]);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    // NSMs at level 2 (even): out of scope for L3. The L2-reordered
    // Latin word stays in visual `[L, NSM]` order. No change.
    assert_eq!(visual, vec![5, 4, 2, 3, 1, 0]);
}

// --- Edge cases ---------------------------------------------------

#[test]
fn r247_public_empty_no_op() {
    let mut visual: Vec<usize> = Vec::new();
    reorder_combining_marks(&[], &[], &mut visual);
    assert!(visual.is_empty());
}

#[test]
fn r247_public_orphan_leading_nsm_left_alone() {
    // No base in the same level-1 block.
    let cls = [BidiClass::NSM, BidiClass::NSM, BidiClass::NSM];
    let lvl = [1, 1, 1];
    let mut visual = reorder_line(&lvl);
    assert_eq!(visual, vec![2, 1, 0]);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    assert_eq!(visual, vec![2, 1, 0]);
}

#[test]
fn r247_public_idempotent() {
    // Calling L3 twice yields the same permutation as once.
    let cls = [
        BidiClass::R,
        BidiClass::NSM,
        BidiClass::NSM,
        BidiClass::R,
        BidiClass::NSM,
    ];
    let lvl = [1, 1, 1, 1, 1];
    let mut visual = reorder_line(&lvl);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    let once = visual.clone();
    reorder_combining_marks(&cls, &lvl, &mut visual);
    assert_eq!(visual, once);
}

#[test]
fn r247_public_l3_is_a_permutation() {
    // The function must always return a permutation of 0..n.
    let cls = [
        BidiClass::L,
        BidiClass::L,
        BidiClass::R,
        BidiClass::NSM,
        BidiClass::NSM,
        BidiClass::L,
        BidiClass::R,
        BidiClass::NSM,
    ];
    let lvl = [0, 0, 1, 1, 1, 0, 1, 1];
    let mut visual = reorder_line(&lvl);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    let mut sorted = visual.clone();
    sorted.sort();
    assert_eq!(sorted, (0..cls.len()).collect::<Vec<_>>());
}

// --- Compose with L1 ----------------------------------------------

#[test]
fn r247_public_compose_with_l1_then_l2_then_l3() {
    // RTL line "R NSM WS" — L1 anchors trailing WS to the
    // paragraph level (1), L2 reverses the whole level-1 line,
    // L3 puts the base back in front of its mark.
    let cls = vec![BidiClass::R, BidiClass::NSM, BidiClass::WS];
    let mut lvl = vec![1, 1, 2];
    reset_trailing_levels(&cls, &mut lvl, 1);
    assert_eq!(lvl, vec![1, 1, 1]);
    let mut visual = reorder_line(&lvl);
    assert_eq!(visual, vec![2, 1, 0]);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    // The leading visual position is now the WS (an orphan NSM
    // followed by a non-NSM trailing-filler base). Re-trace:
    // vi=0, visual[0]=2 (WS) — not NSM, skip.
    // vi=1, visual[1]=1 (NSM, level 1) — walk forward,
    //   visual[2]=0 (R, level 1) → base. Reverse [1, 0] → [0, 1].
    // Final: [2, 0, 1].
    assert_eq!(visual, vec![2, 0, 1]);
}

#[test]
fn r247_public_rtl_mark_then_pure_rtl_word() {
    // "R R R NSM R" at level 1: a Hebrew word with an accent on
    // the third letter from the left in logical order.
    let cls = [
        BidiClass::R,
        BidiClass::R,
        BidiClass::R,
        BidiClass::NSM,
        BidiClass::R,
    ];
    let lvl = [1, 1, 1, 1, 1];
    let mut visual = reorder_line(&lvl);
    assert_eq!(visual, vec![4, 3, 2, 1, 0]);
    reorder_combining_marks(&cls, &lvl, &mut visual);
    // Re-trace: vi=0 (4, R) → not NSM, skip. vi=1 (3, NSM) → walk
    // to vi=2 (2, R) → base. Reverse [3, 2] → [2, 3]. Result:
    // [4, 2, 3, 1, 0].
    assert_eq!(visual, vec![4, 2, 3, 1, 0]);
}
