//! Round 268 — UAX #9 §3.4 rule **L4** (bidi mirroring) integration
//! tests.
//!
//! Mirrors the per-rule unit tests in `src/bidi.rs` but exercises the
//! public re-exports from the crate root so the surface stays stable
//! for external callers. Composes the mirror lookup
//! ([`oxideav_scribe::mirrored_glyph`]) with the L4 rewriter
//! ([`oxideav_scribe::apply_mirroring`]), the round-257 bracket-aware
//! paragraph driver
//! ([`oxideav_scribe::process_paragraph_with_brackets`]), and the
//! §3.4 L1 / L2 / L3 line passes.
//!
//! Provenance: every input is constructed by hand from rule **L4**
//! (§3.4) and §7 *Mirroring* in UAX #9 Revision 50 / Unicode 16.0
//! (the dated snapshot at
//! `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`).

use oxideav_scribe::{
    apply_mirroring, mirrored_glyph, paired_bracket, process_paragraph_with_brackets,
    reorder_combining_marks, reorder_line, reset_trailing_levels, BidiClass,
};

// --- mirrored_glyph: the Bidi_Mirroring_Glyph seed lookup -----------

#[test]
fn r268_public_mirrored_glyph_round_trips_each_ascii_pair() {
    for (a, b) in [('(', ')'), ('[', ']'), ('{', '}')] {
        assert_eq!(mirrored_glyph(a), Some(b));
        assert_eq!(mirrored_glyph(b), Some(a));
    }
}

#[test]
fn r268_public_mirrored_glyph_none_for_letters_digits_punctuation() {
    for c in ['a', 'Q', '5', ' ', ',', '.', ';', '\u{05D0}', '\u{0627}'] {
        assert_eq!(mirrored_glyph(c), None, "{c:?} is outside the seed set");
    }
}

/// §3.4 L4 note: "for backward compatibility the characters U+FD3E
/// ORNATE LEFT PARENTHESIS and U+FD3F ORNATE RIGHT PARENTHESIS are
/// not mirrored."
#[test]
fn r268_public_ornate_parentheses_excluded() {
    assert_eq!(mirrored_glyph('\u{FD3E}'), None);
    assert_eq!(mirrored_glyph('\u{FD3F}'), None);
}

#[test]
fn r268_public_mirrored_glyph_is_an_involution() {
    for c in ['(', ')', '[', ']', '{', '}'] {
        let m = mirrored_glyph(c).expect("seed-set member has a mirror");
        assert_eq!(mirrored_glyph(m), Some(c));
    }
}

/// For the ASCII paired brackets the acceptable mirror per §7 is the
/// BD14 / BD15 paired character — the round-257 and round-268 lookups
/// must agree on the seed set.
#[test]
fn r268_public_mirror_agrees_with_paired_bracket() {
    for c in ['(', ')', '[', ']', '{', '}'] {
        let (paired, _kind) = paired_bracket(c).expect("paired bracket");
        assert_eq!(mirrored_glyph(c), Some(paired));
    }
}

// --- apply_mirroring: rule L4 ---------------------------------------

/// §3.4 L4 worked example: U+0028 LEFT PARENTHESIS "appears as '('
/// when its resolved level is even, and as the mirrored glyph ')'
/// when its resolved level is odd".
#[test]
fn r268_public_l4_worked_example_even_vs_odd() {
    let mut even = ['('];
    apply_mirroring(&mut even, &[0]);
    assert_eq!(even, ['(']);

    let mut odd = ['('];
    apply_mirroring(&mut odd, &[1]);
    assert_eq!(odd, [')']);
}

#[test]
fn r268_public_l4_even_levels_no_op() {
    let mut chars: Vec<char> = "a(b)[c]{d}".chars().collect();
    let levels = vec![0u8; chars.len()];
    apply_mirroring(&mut chars, &levels);
    assert_eq!(chars, "a(b)[c]{d}".chars().collect::<Vec<_>>());
}

#[test]
fn r268_public_l4_odd_levels_mirror_every_seed_bracket() {
    let mut chars: Vec<char> = "([{)]}".chars().collect();
    let levels = vec![1u8; chars.len()];
    apply_mirroring(&mut chars, &levels);
    assert_eq!(chars, ")]}([{".chars().collect::<Vec<_>>());
}

#[test]
fn r268_public_l4_mixed_levels_mirror_only_odd_positions() {
    // Levels alternate even / odd over identical brackets.
    let mut chars = ['(', '(', ')', ')'];
    apply_mirroring(&mut chars, &[0, 1, 1, 0]);
    assert_eq!(chars, ['(', ')', '(', ')']);
}

#[test]
fn r268_public_l4_non_mirrored_characters_pass_through_at_odd_level() {
    let mut chars = ['\u{05D0}', 'x', '3', '\u{FD3E}'];
    apply_mirroring(&mut chars, &[1, 1, 1, 1]);
    assert_eq!(chars, ['\u{05D0}', 'x', '3', '\u{FD3E}']);
}

#[test]
fn r268_public_l4_higher_odd_levels_also_mirror() {
    // Any odd resolved level means directionality R — not just 1.
    let mut chars = ['[', ']'];
    apply_mirroring(&mut chars, &[3, 3]);
    assert_eq!(chars, [']', '[']);
}

#[test]
fn r268_public_l4_empty_input_no_op() {
    let mut chars: Vec<char> = vec![];
    apply_mirroring(&mut chars, &[]);
    assert!(chars.is_empty());
}

#[test]
fn r268_public_l4_double_application_restores_original() {
    let original: Vec<char> = "{\u{05D0}}".chars().collect();
    let mut chars = original.clone();
    let levels = [1u8, 1, 1];
    apply_mirroring(&mut chars, &levels);
    assert_ne!(chars, original);
    apply_mirroring(&mut chars, &levels);
    assert_eq!(chars, original);
}

#[test]
#[should_panic(expected = "length mismatch")]
fn r268_public_l4_length_mismatch_panics() {
    let mut chars = ['(', ')'];
    apply_mirroring(&mut chars, &[1]);
}

// --- composition with the §3 driver + §3.4 line passes --------------

/// RTL paragraph "א(ב)ג": N0 b resolves both brackets to R, I2 keeps
/// every position at the odd embedding level 1, so L4 mirrors both
/// brackets in the logical stream.
#[test]
fn r268_public_l4_composes_with_bracket_driver_rtl() {
    let text = "\u{05D0}(\u{05D1})\u{05D2}";
    let (p, _offsets) = process_paragraph_with_brackets(text, None);
    assert_eq!(p.paragraph_level, 1);
    assert_eq!(p.levels, vec![1, 1, 1, 1, 1]);

    let mut chars: Vec<char> = text.chars().collect();
    apply_mirroring(&mut chars, &p.levels);
    assert_eq!(chars, vec!['\u{05D0}', ')', '\u{05D1}', '(', '\u{05D2}']);
}

/// The same composition rendered to display order: after L2 reverses
/// the whole level-1 line, the mirrored brackets read correctly in
/// visual order — the opening shape faces the enclosed text.
#[test]
fn r268_public_l4_plus_l2_yields_correct_display_stream() {
    let text = "\u{05D0}(\u{05D1})\u{05D2}";
    let (p, _offsets) = process_paragraph_with_brackets(text, None);

    // L4 over the logical sequence.
    let mut chars: Vec<char> = text.chars().collect();
    apply_mirroring(&mut chars, &p.levels);

    // L2 permutation, then walk it to build the display stream.
    let visual = p.reorder_paragraph();
    let display: String = visual.iter().map(|&i| chars[i]).collect();
    assert_eq!(display, "\u{05D2}(\u{05D1})\u{05D0}");
}

/// LTR paragraph with brackets: every level stays even, so L4 leaves
/// the line untouched end-to-end.
#[test]
fn r268_public_l4_ltr_paragraph_untouched() {
    let text = "ab(cd)ef";
    let (p, _offsets) = process_paragraph_with_brackets(text, None);
    assert_eq!(p.paragraph_level, 0);
    assert!(p.levels.iter().all(|&l| l % 2 == 0));

    let mut chars: Vec<char> = text.chars().collect();
    apply_mirroring(&mut chars, &p.levels);
    assert_eq!(chars, text.chars().collect::<Vec<_>>());
}

/// Full §3.4 line pipeline in rule order — L1, L2, L3, L4 — over an
/// RTL line carrying a combining mark and a bracket pair plus
/// trailing whitespace.
#[test]
fn r268_public_full_l1_l2_l3_l4_pipeline() {
    // Logical: R NSM ( R ) WS — Hebrew base + combining mark, then a
    // bracketed Hebrew letter, then a trailing space.
    let text = "\u{05D0}\u{0301}(\u{05D1}) ";
    let (p, _offsets) = process_paragraph_with_brackets(text, None);
    assert_eq!(p.paragraph_level, 1);

    // L1: trailing whitespace resets to the paragraph level (1 here —
    // observationally a no-op for this line, asserted for rule order).
    let mut levels = p.levels.clone();
    reset_trailing_levels(&p.classes, &mut levels, p.paragraph_level);
    assert_eq!(levels[5], 1);

    // L2: the whole odd-level line reverses.
    let mut visual = reorder_line(&levels);
    assert_eq!(visual, vec![5, 4, 3, 2, 1, 0]);

    // L3: the [NSM, base] block rotates back to [base, NSM].
    reorder_combining_marks(&p.classes, &levels, &mut visual);
    assert_eq!(visual, vec![5, 4, 3, 2, 0, 1]);

    // L4: both brackets sit at an odd resolved level → mirrored.
    let mut chars: Vec<char> = text.chars().collect();
    apply_mirroring(&mut chars, &levels);
    assert_eq!(chars[2], ')');
    assert_eq!(chars[4], '(');

    // Display stream: walk the L3-adjusted permutation over the
    // L4-mirrored characters.
    let display: String = visual.iter().map(|&i| chars[i]).collect();
    assert_eq!(display, " (\u{05D1})\u{05D0}\u{0301}");
}

/// The original classes survive the driver unchanged (L4 reads only
/// levels; sanity-check the carrier the composition relies on).
#[test]
fn r268_public_driver_classes_preserved_for_line_passes() {
    let text = "\u{05D0}(\u{05D1})\u{05D2}";
    let (p, _offsets) = process_paragraph_with_brackets(text, None);
    assert_eq!(
        p.classes,
        vec![
            BidiClass::R,
            BidiClass::ON,
            BidiClass::R,
            BidiClass::ON,
            BidiClass::R,
        ]
    );
}
