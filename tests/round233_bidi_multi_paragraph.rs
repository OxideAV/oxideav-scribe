//! Round 233 — UAX #9 §3 P1 multi-paragraph driver
//! (`process_text` / `TextBidi` / `ParagraphSlice`).
//!
//! Exercises the top-level entry point that composes the §3.3.1 P1
//! paragraph split with the per-paragraph §3 driver introduced in
//! round 227 (`process_paragraph` / `process_paragraph_classes`),
//! returning one [`oxideav_scribe::ParagraphBidi`] per paragraph
//! alongside whole-input byte / character bookkeeping.
//!
//! Provenance: inputs are constructed by hand from the rule examples
//! in UAX #9 Revision 50 / Unicode 16.0 §3.3.1 P1 (paragraph split)
//! and §3 (per-paragraph application) at the dated snapshot
//!   `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`.

use oxideav_scribe::{
    bidi_class, paragraph_level, process_paragraph, process_text, reorder_line,
    reset_trailing_levels, split_paragraphs, BidiClass, ParagraphSlice,
};

// --- §3.3.1 P1: paragraph split ------------------------------------

#[test]
fn r233_public_empty_input_has_no_paragraphs() {
    let t = process_text("", None);
    assert!(t.is_empty());
    assert_eq!(t.len(), 0);
    assert_eq!(t.total_chars, 0);
}

#[test]
fn r233_public_single_paragraph_no_separator() {
    // "Hello" has no class-B character → P1 produces one paragraph.
    let t = process_text("Hello", None);
    assert_eq!(t.len(), 1);
    let p = &t.paragraphs[0];
    assert_eq!(p.byte_range, 0..5);
    assert_eq!(p.char_offset, 0);
    assert_eq!(p.bidi.paragraph_level, 0);
    assert_eq!(p.bidi.levels, vec![0; 5]);
    assert_eq!(t.total_chars, 5);
}

#[test]
fn r233_public_lf_terminator_kept_with_preceding_paragraph() {
    // P1: "A paragraph separator (type B) is kept with the previous
    // paragraph." First paragraph here is "Hi\n" (3 chars).
    let t = process_text("Hi\nyo", None);
    assert_eq!(t.len(), 2);
    let p0 = &t.paragraphs[0];
    let p1 = &t.paragraphs[1];
    assert_eq!(p0.byte_range, 0..3);
    assert_eq!(p0.bidi.classes.last(), Some(&BidiClass::B));
    assert_eq!(p0.bidi.levels.len(), 3);
    assert_eq!(p1.byte_range, 3..5);
    assert_eq!(p1.char_offset, 3);
    assert_eq!(p1.bidi.levels.len(), 2);
}

#[test]
fn r233_public_terminal_lf_does_not_create_phantom_paragraph() {
    // "Hi\n" is one paragraph; the LF closes it but starts no new
    // paragraph after itself.
    let t = process_text("Hi\n", None);
    assert_eq!(t.len(), 1);
    assert_eq!(t.paragraphs[0].byte_range, 0..3);
    assert_eq!(t.paragraphs[0].bidi.classes.len(), 3);
}

#[test]
fn r233_public_split_on_every_class_b_codepoint() {
    // Each of these is class B per the scribe `bidi_class` table; P1
    // splits on each.
    for sep in &[
        '\u{000A}', // LF
        '\u{000D}', // CR
        '\u{0085}', // NEL
        '\u{001C}', // FS
        '\u{001D}', // GS
        '\u{001E}', // RS
        '\u{2029}', // PS
    ] {
        assert_eq!(bidi_class(*sep), BidiClass::B);
        let s = format!("A{sep}B");
        let t = process_text(&s, None);
        assert_eq!(t.len(), 2, "{sep:?}");
    }
}

#[test]
fn r233_public_consecutive_separators_make_one_paragraph_each() {
    // "A\n\nB" → "A\n", "\n", "B" (3 paragraphs; the second is the
    // lone LF kept with itself per P1).
    let t = process_text("A\n\nB", None);
    assert_eq!(t.len(), 3);
    assert_eq!(t.paragraphs[0].byte_range, 0..2);
    assert_eq!(t.paragraphs[1].byte_range, 2..3);
    assert_eq!(t.paragraphs[2].byte_range, 3..4);
    assert_eq!(t.paragraphs[1].bidi.classes, vec![BidiClass::B]);
}

// --- §3.3.1 P2 / P3 / HL1 per paragraph ----------------------------

#[test]
fn r233_public_per_paragraph_p2_resolves_independently() {
    // First paragraph is Latin (L), second is Hebrew (R). P2 walks
    // each paragraph alone, so paragraph levels diverge.
    let t = process_text("Hi\n\u{05D0}\u{05D1}", None);
    assert_eq!(t.len(), 2);
    assert_eq!(t.paragraphs[0].bidi.paragraph_level, 0);
    assert_eq!(t.paragraphs[1].bidi.paragraph_level, 1);
}

#[test]
fn r233_public_per_paragraph_p3_default_is_ltr() {
    // No strong character in either paragraph → P3 default 0.
    let t = process_text(" \n \n", None);
    assert_eq!(t.len(), 2);
    for p in &t.paragraphs {
        assert_eq!(p.bidi.paragraph_level, 0);
    }
}

#[test]
fn r233_public_base_level_override_applies_uniformly() {
    // HL1 via `base_level = Some(_)`: every paragraph forced to that
    // level. (Per-paragraph overrides require loop process_paragraph
    // manually.)
    let t = process_text("Hi\nyo", Some(1));
    assert_eq!(t.len(), 2);
    for p in &t.paragraphs {
        assert_eq!(p.bidi.paragraph_level, 1);
    }
}

#[test]
fn r233_public_base_level_low_bit_clamp_per_paragraph() {
    // base_level = 4 → low bit 0 → LTR; 5 → low bit 1 → RTL.
    let t_even = process_text("\u{05D0}\nA", Some(4));
    for p in &t_even.paragraphs {
        assert_eq!(p.bidi.paragraph_level, 0);
    }
    let t_odd = process_text("A\n\u{05D0}", Some(5));
    for p in &t_odd.paragraphs {
        assert_eq!(p.bidi.paragraph_level, 1);
    }
}

// --- byte-range / char-offset bookkeeping --------------------------

#[test]
fn r233_public_byte_range_tiles_input_contiguously() {
    let s = "AAA\nBBB\nCCC";
    let t = process_text(s, None);
    assert_eq!(t.len(), 3);
    assert_eq!(t.paragraphs[0].byte_range.start, 0);
    assert_eq!(t.paragraphs.last().unwrap().byte_range.end, s.len());
    for w in t.paragraphs.windows(2) {
        assert_eq!(w[0].byte_range.end, w[1].byte_range.start);
    }
}

#[test]
fn r233_public_char_offsets_accumulate_per_paragraph() {
    let t = process_text("AB\nCDE\nF", None);
    assert_eq!(t.paragraphs[0].char_offset, 0);
    assert_eq!(t.paragraphs[1].char_offset, 3);
    assert_eq!(t.paragraphs[2].char_offset, 7);
    assert_eq!(t.total_chars, 8);
}

#[test]
fn r233_public_char_byte_offsets_are_whole_input_indices() {
    // Hebrew Alef = 2 UTF-8 bytes; the second paragraph's char_byte_offsets
    // must reference whole-input bytes, not paragraph-local ones.
    let s = "A\n\u{05D0}\u{05D1}";
    let t = process_text(s, None);
    assert_eq!(t.paragraphs[0].char_byte_offsets, vec![0, 1]);
    assert_eq!(t.paragraphs[1].char_byte_offsets, vec![2, 4]);
    for p in &t.paragraphs {
        for (i, &off) in p.char_byte_offsets.iter().enumerate() {
            let c = s[off..].chars().next().unwrap();
            assert_eq!(bidi_class(c), p.bidi.classes[i]);
        }
    }
}

#[test]
fn r233_public_total_chars_matches_input_char_count() {
    for s in &[
        "Hi",
        "Hi\nyo",
        "Hi\n\u{05D0}\u{05D1}",
        "A\nB\nC\n",
        "\u{05D0}\u{05D1}\n\u{0627}\u{0628}",
    ] {
        let t = process_text(s, None);
        assert_eq!(t.total_chars, s.chars().count(), "for {s:?}");
    }
}

// --- locate_char convenience ---------------------------------------

#[test]
fn r233_public_locate_char_round_trips_with_offset_arithmetic() {
    let t = process_text("AB\nCD\nE", None);
    for k in 0..t.total_chars {
        let (pi, ki) = t.locate_char(k).expect("k in bounds");
        assert_eq!(t.paragraphs[pi].char_offset + ki, k);
    }
}

#[test]
fn r233_public_locate_char_out_of_bounds_returns_none() {
    let t = process_text("AB\nCD", None);
    assert_eq!(t.locate_char(5), None);
    assert_eq!(t.locate_char(99), None);
}

// --- composition equivalence ---------------------------------------

#[test]
fn r233_public_process_text_equivalent_to_per_paragraph_loop() {
    // process_text MUST be observationally identical to a manual loop
    // over split_paragraphs + process_paragraph (modulo byte-offset
    // rebasing).
    let s = "Hi \u{05D0}\u{05D1}\n\u{0627}\u{0628}\nBye";
    let t = process_text(s, None);
    let slices = split_paragraphs(s);
    assert_eq!(t.paragraphs.len(), slices.len());
    let mut byte_start = 0usize;
    for (carrier, slice) in t.paragraphs.iter().zip(slices.iter()) {
        let (expected, expected_offsets) = process_paragraph(slice, None);
        assert_eq!(carrier.bidi, expected);
        let shifted: Vec<usize> = expected_offsets.iter().map(|o| o + byte_start).collect();
        assert_eq!(carrier.char_byte_offsets, shifted);
        byte_start += slice.len();
    }
}

#[test]
fn r233_public_paragraph_level_matches_text_walker_per_paragraph() {
    // Each ParagraphSlice's bidi.paragraph_level should match the
    // class-driving paragraph_level walker applied to the paragraph's
    // own slice of the input.
    let s = "Hi\n\u{05D0}\u{05D1}\n123 \u{0627}";
    let t = process_text(s, None);
    let slices = split_paragraphs(s);
    for (carrier, slice) in t.paragraphs.iter().zip(slices.iter()) {
        assert_eq!(carrier.bidi.paragraph_level, paragraph_level(slice));
    }
}

// --- downstream reorder consumption --------------------------------

#[test]
fn r233_public_per_paragraph_reorder_works_via_carrier_helper() {
    // The carrier's reorder_paragraph helper still works after
    // multi-paragraph dispatch — confirms the per-paragraph fields
    // line up.
    let t = process_text("Hi\n\u{05D0}\u{05D1}", None);
    // Paragraph 0 ("Hi\n", LTR): identity permutation.
    let perm0 = t.paragraphs[0].bidi.reorder_paragraph();
    assert_eq!(perm0, vec![0, 1, 2]);
    // Paragraph 1 ("אב", RTL): reversed.
    let perm1 = t.paragraphs[1].bidi.reorder_paragraph();
    assert_eq!(perm1, vec![1, 0]);
}

#[test]
fn r233_public_caller_drives_l1_l2_per_paragraph_directly() {
    // Mirror the carrier's reorder_paragraph by hand to confirm the
    // exposed fields are sufficient for callers that want a custom
    // L1 / L2 pipeline.
    let s = "AB\n\u{05D0}\u{05D1}";
    let t = process_text(s, None);
    for p in &t.paragraphs {
        let mut levels = p.bidi.levels.clone();
        reset_trailing_levels(&p.bidi.classes, &mut levels, p.bidi.paragraph_level);
        let perm = reorder_line(&levels);
        // Round-trip: the permutation has the same length as levels
        // and is a complete bijection on 0..levels.len().
        assert_eq!(perm.len(), levels.len());
        let mut sorted = perm.clone();
        sorted.sort_unstable();
        let identity: Vec<usize> = (0..levels.len()).collect();
        assert_eq!(sorted, identity);
    }
}

// --- carrier types ------------------------------------------------

#[test]
fn r233_public_text_bidi_clone_eq_debug_derives() {
    let t1 = process_text("Hi\nyo", None);
    let t2 = t1.clone();
    assert_eq!(t1, t2);
    // Debug must produce non-empty output (Debug derive smoke test).
    let dbg = format!("{:?}", t1);
    assert!(!dbg.is_empty());
}

#[test]
fn r233_public_paragraph_slice_clone_eq_debug_derives() {
    let t = process_text("Hi", None);
    let p1: &ParagraphSlice = &t.paragraphs[0];
    let p2 = p1.clone();
    assert_eq!(p1, &p2);
    let dbg = format!("{:?}", p1);
    assert!(!dbg.is_empty());
}

// --- mixed-script end-to-end ---------------------------------------

#[test]
fn r233_public_multi_paragraph_l_rtl_pipeline_end_to_end() {
    // Document: "Hello\nשלום" — first paragraph LTR Latin, second
    // RTL Hebrew. Verify per-paragraph independence + global
    // bookkeeping.
    let s = "Hello\n\u{05E9}\u{05DC}\u{05D5}\u{05DD}";
    let t = process_text(s, None);
    assert_eq!(t.len(), 2);
    // Paragraph 0: 6 chars (Hello + LF), LTR.
    assert_eq!(t.paragraphs[0].bidi.levels.len(), 6);
    assert_eq!(t.paragraphs[0].bidi.paragraph_level, 0);
    // Paragraph 1: 4 Hebrew chars, RTL.
    assert_eq!(t.paragraphs[1].bidi.levels.len(), 4);
    assert_eq!(t.paragraphs[1].bidi.paragraph_level, 1);
    // Total chars = 10.
    assert_eq!(t.total_chars, 10);
    // locate_char on every position is consistent.
    for k in 0..t.total_chars {
        let (pi, ki) = t.locate_char(k).unwrap();
        assert!(ki < t.paragraphs[pi].bidi.levels.len());
    }
}

#[test]
fn r233_public_paragraph_separator_split_invariant() {
    // The sum of all paragraphs' classes lengths must equal the
    // whole-input char count.
    let s = "AAA\nBBB\n\u{05D0}\u{05D1}\nCCC";
    let t = process_text(s, None);
    let total: usize = t.paragraphs.iter().map(|p| p.bidi.classes.len()).sum();
    assert_eq!(total, s.chars().count());
    assert_eq!(total, t.total_chars);
}
