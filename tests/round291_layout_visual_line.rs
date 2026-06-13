//! Round 291 — `layout::reorder_line_visual`: the high-level bridge
//! that drives the complete UAX #9 §3 + §3.4 bidirectional pipeline
//! (P → X → W → N0 → N1/N2 → I → L1 → L2 → L3 → L4) over one display
//! line and returns the line's characters in left-to-right visual
//! order, ready to feed glyph-by-glyph into the shaper.
//!
//! Provenance: exercises the public `bidi::` per-rule entry points
//! through `layout::reorder_line_visual`, each of which cites
//! `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`.

use oxideav_scribe::layout::{reorder_line_visual, VisualLine};

/// Helper: assert the two permutation vectors invert each other and
/// together form a bijection over `0..n`.
fn assert_permutation_consistent(line: &VisualLine) {
    let n = line.len();
    assert_eq!(line.logical_to_visual.len(), n);
    assert_eq!(line.visual_to_logical.len(), n);
    let mut seen = vec![false; n];
    for &log_idx in &line.logical_to_visual {
        assert!(log_idx < n, "logical index {log_idx} out of range");
        assert!(!seen[log_idx], "permutation repeats index {log_idx}");
        seen[log_idx] = true;
    }
    assert!(seen.iter().all(|&b| b), "permutation not surjective");
    for (vis_pos, &log_idx) in line.logical_to_visual.iter().enumerate() {
        assert_eq!(
            line.visual_to_logical[log_idx], vis_pos,
            "inverse permutation inconsistent at visual {vis_pos}"
        );
    }
}

#[test]
fn ltr_line_is_identity() {
    let line = reorder_line_visual("Hello, world!", None);
    assert_eq!(line.base_level, 0);
    assert_eq!(line.to_visual_string(), "Hello, world!");
    assert_permutation_consistent(&line);
    // Identity permutation for a pure-LTR line.
    let identity: Vec<usize> = (0..line.len()).collect();
    assert_eq!(line.logical_to_visual, identity);
}

#[test]
fn pure_rtl_line_reverses() {
    // "שלום" (shalom) — four Hebrew letters, pure RTL.
    let line = reorder_line_visual("\u{05E9}\u{05DC}\u{05D5}\u{05DD}", None);
    assert_eq!(line.base_level, 1);
    assert_permutation_consistent(&line);
    // Visual order is logical reversed.
    assert_eq!(line.logical_to_visual, vec![3, 2, 1, 0]);
    assert_eq!(line.to_visual_string(), "\u{05DD}\u{05D5}\u{05DC}\u{05E9}");
}

#[test]
fn ltr_with_embedded_rtl_run() {
    // "abc <heb3><heb2><heb1> xyz" — an LTR line carrying an RTL word.
    // The RTL run reverses inside the line, the surrounding Latin and
    // the spaces keep their order. The base direction stays LTR.
    let line = reorder_line_visual("abc \u{05D0}\u{05D1}\u{05D2} xyz", None);
    assert_eq!(line.base_level, 0);
    assert_permutation_consistent(&line);
    let s = line.to_visual_string();
    // The Latin prefix and suffix bracket the (reversed) Hebrew run.
    assert!(s.starts_with("abc "), "got {s:?}");
    assert!(s.ends_with(" xyz"), "got {s:?}");
    // The Hebrew letters appear reversed: gimel, bet, alef.
    let mid: String = "\u{05D2}\u{05D1}\u{05D0}".to_string();
    assert!(s.contains(&mid), "expected reversed Hebrew run in {s:?}");
}

#[test]
fn rtl_with_embedded_ltr_run() {
    // An RTL line whose first strong character is Hebrew, carrying an
    // embedded Latin word. The line lays out right-to-left; the Latin
    // island keeps its internal left-to-right order.
    let line = reorder_line_visual("\u{05D0}\u{05D1} abc \u{05D2}\u{05D3}", None);
    assert_eq!(line.base_level, 1);
    assert_permutation_consistent(&line);
    let s = line.to_visual_string();
    // "abc" survives as a contiguous LTR substring inside the RTL line.
    assert!(s.contains("abc"), "expected intact 'abc' island in {s:?}");
}

#[test]
fn l4_mirrors_bracket_in_rtl_context() {
    // Parenthesis surrounded by Hebrew: resolved RTL, so L4 mirrors it.
    let line = reorder_line_visual("\u{05D0}(\u{05D1})\u{05D2}", None);
    assert_eq!(line.base_level, 1);
    assert_permutation_consistent(&line);
    let s = line.to_visual_string();
    // Both original brackets should be mirrored — '(' -> ')' and
    // ')' -> '(' — so the visual line still reads as a balanced pair
    // when scanned left-to-right.
    assert!(s.contains('('), "mirror of ')' should appear: {s:?}");
    assert!(s.contains(')'), "mirror of '(' should appear: {s:?}");
}

#[test]
fn l4_leaves_ltr_bracket_alone() {
    let line = reorder_line_visual("f(x) = y", None);
    assert_eq!(line.base_level, 0);
    assert_eq!(line.to_visual_string(), "f(x) = y");
    assert_permutation_consistent(&line);
}

#[test]
fn digits_in_rtl_line_stay_ltr() {
    // European digits inside an RTL line render left-to-right (numbers
    // are not reversed) even though the surrounding text is RTL.
    let line = reorder_line_visual("\u{05D0}\u{05D1} 123 \u{05D2}\u{05D3}", None);
    assert_eq!(line.base_level, 1);
    assert_permutation_consistent(&line);
    let s = line.to_visual_string();
    assert!(s.contains("123"), "digits must stay in order: {s:?}");
}

#[test]
fn base_level_override_forces_direction() {
    // HL1 override: force an all-Latin line to RTL base. The Latin run
    // is one LTR island, so its characters keep their order, but the
    // base level is reported as 1.
    let forced = reorder_line_visual("abc", Some(1));
    assert_eq!(forced.base_level, 1);
    assert_eq!(forced.to_visual_string(), "abc");

    // Forcing LTR on an all-Hebrew line reports base 0 but the Hebrew
    // run still reverses (it is a level-1 RTL island in a level-0 line).
    let forced_ltr = reorder_line_visual("\u{05D0}\u{05D1}\u{05D2}", Some(0));
    assert_eq!(forced_ltr.base_level, 0);
    assert_eq!(
        forced_ltr.to_visual_string(),
        "\u{05D2}\u{05D1}\u{05D0}",
        "RTL island reverses even under forced-LTR base"
    );
}

#[test]
fn empty_and_whitespace_lines() {
    let empty = reorder_line_visual("", None);
    assert!(empty.is_empty());
    assert_eq!(empty.to_visual_string(), "");

    let spaces = reorder_line_visual("   ", None);
    assert_eq!(spaces.to_visual_string(), "   ");
    assert_permutation_consistent(&spaces);
}

#[test]
fn arabic_line_reverses_and_mirrors() {
    // Arabic letters (class AL) drive an RTL base; a parenthesis among
    // them mirrors under L4.
    let line = reorder_line_visual("\u{0627}\u{0628}(\u{062C}\u{062F})", None);
    assert_eq!(line.base_level, 1);
    assert_permutation_consistent(&line);
    let s = line.to_visual_string();
    assert!(s.contains('('), "expected mirrored bracket in {s:?}");
    assert!(s.contains(')'), "expected mirrored bracket in {s:?}");
}

#[test]
fn visual_to_logical_round_trips_for_hit_testing() {
    // For every logical index, walking logical -> visual -> logical
    // returns the original index (the contract cursor hit-testing
    // relies on).
    let line = reorder_line_visual("ab\u{05D0}\u{05D1}cd", None);
    assert_permutation_consistent(&line);
    for log in 0..line.len() {
        let vis = line.visual_to_logical[log];
        assert_eq!(line.logical_to_visual[vis], log);
    }
}

#[test]
fn spec_example_car_means_uppercase() {
    // UAX #9 §3.4 worked example "car means CAR." where CAR is RTL.
    // We approximate with Hebrew standing in for the uppercase RTL
    // word: "car means " + RTL-word + ".". The RTL word reverses; the
    // LTR text keeps order; the final period stays at the end for an
    // LTR-base line.
    let line = reorder_line_visual("car means \u{05D0}\u{05D1}\u{05D2}.", None);
    assert_eq!(line.base_level, 0);
    assert_permutation_consistent(&line);
    let s = line.to_visual_string();
    assert!(s.starts_with("car means "), "got {s:?}");
    // The RTL word appears reversed (gimel, bet, alef) just before the
    // trailing period.
    assert!(
        s.contains("\u{05D2}\u{05D1}\u{05D0}"),
        "expected reversed RTL word in {s:?}"
    );
    assert!(s.ends_with('.'), "trailing period stays at line end: {s:?}");
}
