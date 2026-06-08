//! Round 257 — UAX #9 §3.3.5 rule **N0** (bracket-pair resolution)
//! integration tests.
//!
//! Mirrors the per-rule unit tests in `src/bidi.rs` but exercises the
//! public re-exports from the crate root so the surface stays stable
//! for external callers. Composes the BD16 walker
//! ([`oxideav_scribe::bracket_pairs`]) with the N0 rewriter
//! ([`oxideav_scribe::resolve_bracket_pairs`]) and the full
//! paragraph driver
//! ([`oxideav_scribe::process_paragraph_with_brackets`]).
//!
//! Provenance: every input is constructed by hand from BD14 / BD15 /
//! BD16 (§3.1.3) and rule **N0** in UAX #9 Revision 50 / Unicode 16.0
//! (the dated snapshot at
//! `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`).

use oxideav_scribe::{
    bidi_class, bracket_pairs, paired_bracket, process_paragraph_classes_with_brackets,
    process_paragraph_with_brackets, resolve_bracket_pairs, BidiClass, BracketKind,
};

// --- BD14 / BD15: paired-bracket lookup ----------------------------

#[test]
fn r257_public_paired_bracket_round_trips() {
    for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
        assert_eq!(paired_bracket(open), Some((close, BracketKind::Open)));
        assert_eq!(paired_bracket(close), Some((open, BracketKind::Close)));
    }
    for c in ['a', '1', ' ', ',', '<', '>'] {
        assert_eq!(paired_bracket(c), None);
    }
}

// --- BD16 worked examples from UAX #9 §3.1.3 ----------------------

/// BD16 §3.1.3 worked-example table line: `a ( b ) c → 2-4` (1-indexed).
#[test]
fn r257_public_bd16_balanced_single_pair() {
    let chars: Vec<char> = "a(b)c".chars().collect();
    let classes: Vec<_> = chars.iter().copied().map(bidi_class).collect();
    assert_eq!(bracket_pairs(&chars, &classes), vec![(1, 3)]);
}

/// BD16 §3.1.3 worked-example table line: `a ) b ( c → None`.
#[test]
fn r257_public_bd16_closer_before_opener_unpaired() {
    let chars: Vec<char> = "a)b(c".chars().collect();
    let classes: Vec<_> = chars.iter().copied().map(bidi_class).collect();
    assert!(bracket_pairs(&chars, &classes).is_empty());
}

/// BD16 §3.1.3 worked-example table line: `a ( b ] c → None`.
#[test]
fn r257_public_bd16_mismatched_closer_unpaired() {
    let chars: Vec<char> = "a(b]c".chars().collect();
    let classes: Vec<_> = chars.iter().copied().map(bidi_class).collect();
    assert!(bracket_pairs(&chars, &classes).is_empty());
}

/// BD16 §3.1.3 worked-example table line: `a ( b ( c ) d → 4-6`
/// (inner pair only — the outer `(` never finds a matching `)`).
#[test]
fn r257_public_bd16_inner_matches_when_outer_never_closes() {
    let chars: Vec<char> = "a(b(c)d".chars().collect();
    let classes: Vec<_> = chars.iter().copied().map(bidi_class).collect();
    assert_eq!(bracket_pairs(&chars, &classes), vec![(3, 5)]);
}

/// BD16 §3.1.3 worked-example table line: `a ( b ( c ) d ) → 2-8, 4-6`.
#[test]
fn r257_public_bd16_nested_pairs_sorted_by_opener() {
    let chars: Vec<char> = "a(b(c)d)".chars().collect();
    let classes: Vec<_> = chars.iter().copied().map(bidi_class).collect();
    assert_eq!(bracket_pairs(&chars, &classes), vec![(1, 7), (3, 5)]);
}

/// BD16 §3.1.3 worked-example table line: `a ( b ) c ) d → 2-4` (the
/// trailing `)` has no live opener and is dropped).
#[test]
fn r257_public_bd16_unmatched_trailing_closer_dropped() {
    let chars: Vec<char> = "a(b)c)d".chars().collect();
    let classes: Vec<_> = chars.iter().copied().map(bidi_class).collect();
    assert_eq!(bracket_pairs(&chars, &classes), vec![(1, 3)]);
}

/// BD16 §3.1.3 worked-example table line: `a ( b [ c ) d ] → 2-6`.
/// The `)` matches the deeper-on-stack `(`, popping the `[`. The
/// trailing `]` then has no opener and is dropped.
#[test]
fn r257_public_bd16_pop_through_intervening_mismatched_opener() {
    let chars: Vec<char> = "a(b[c)d]".chars().collect();
    let classes: Vec<_> = chars.iter().copied().map(bidi_class).collect();
    assert_eq!(bracket_pairs(&chars, &classes), vec![(1, 5)]);
}

/// BD16 §3.1.3 worked-example table line: `a ( b ] c ) d → 2-6` (the
/// mismatched `]` is consumed without popping; the later `)` then
/// matches the live `(`).
#[test]
fn r257_public_bd16_mismatched_closer_inside_does_not_pop() {
    let chars: Vec<char> = "a(b]c)d".chars().collect();
    let classes: Vec<_> = chars.iter().copied().map(bidi_class).collect();
    assert_eq!(bracket_pairs(&chars, &classes), vec![(1, 5)]);
}

/// BD16 §3.1.3 worked-example table line: `a ( b { c } d ) → 2-8, 4-6`.
#[test]
fn r257_public_bd16_curly_inside_paren_pairs_separately() {
    let chars: Vec<char> = "a(b{c}d)".chars().collect();
    let classes: Vec<_> = chars.iter().copied().map(bidi_class).collect();
    assert_eq!(bracket_pairs(&chars, &classes), vec![(1, 7), (3, 5)]);
}

// --- N0 b: inside-strong matches embedding direction ---------------

#[test]
fn r257_public_n0b_ltr_inside_l_brackets_become_l() {
    let chars: Vec<char> = "a(b)c".chars().collect();
    let mut cls: Vec<_> = chars.iter().copied().map(bidi_class).collect();
    let pairs = bracket_pairs(&chars, &cls);
    resolve_bracket_pairs(&mut cls, &pairs, 0, BidiClass::L);
    assert_eq!(cls[1], BidiClass::L);
    assert_eq!(cls[3], BidiClass::L);
}

#[test]
fn r257_public_n0b_rtl_inside_r_brackets_become_r() {
    // R ( R ) R, embedding 1, sos R.
    let mut cls = vec![
        BidiClass::R,
        BidiClass::ON,
        BidiClass::R,
        BidiClass::ON,
        BidiClass::R,
    ];
    let pairs = vec![(1usize, 3usize)];
    resolve_bracket_pairs(&mut cls, &pairs, 1, BidiClass::R);
    assert_eq!(cls[1], BidiClass::R);
    assert_eq!(cls[3], BidiClass::R);
}

// --- N0 c.1: inside-opposite + preceding-strong-also-opposite ------

#[test]
fn r257_public_n0c1_preceding_strong_matches_inside_opposite() {
    // RTL embedding, inside L (opposite), preceding strong L → N0 c.1.
    let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::L, BidiClass::ON];
    let pairs = vec![(1usize, 3usize)];
    resolve_bracket_pairs(&mut cls, &pairs, 1, BidiClass::R);
    assert_eq!(cls[1], BidiClass::L);
    assert_eq!(cls[3], BidiClass::L);
}

// --- N0 c.2: inside-opposite + preceding-strong-matches-embedding --

#[test]
fn r257_public_n0c2_preceding_strong_matches_embedding() {
    // RTL embedding, inside L (opposite), preceding strong R → N0 c.2.
    let mut cls = vec![BidiClass::R, BidiClass::ON, BidiClass::L, BidiClass::ON];
    let pairs = vec![(1usize, 3usize)];
    resolve_bracket_pairs(&mut cls, &pairs, 1, BidiClass::R);
    assert_eq!(cls[1], BidiClass::R);
    assert_eq!(cls[3], BidiClass::R);
}

// --- N0 d: no inside-strong leaves pair untouched ------------------

#[test]
fn r257_public_n0d_no_inside_strong_leaves_on() {
    let mut cls = vec![
        BidiClass::L,
        BidiClass::ON,
        BidiClass::WS,
        BidiClass::ON,
        BidiClass::L,
    ];
    let pairs = vec![(1usize, 3usize)];
    resolve_bracket_pairs(&mut cls, &pairs, 0, BidiClass::L);
    assert_eq!(cls[1], BidiClass::ON);
    assert_eq!(cls[3], BidiClass::ON);
}

// --- N0 EN/AN-as-R clarification ------------------------------------

#[test]
fn r257_public_n0b_en_inside_counted_as_r_for_rtl() {
    // R ( EN ) R, embedding 1 → EN treated as R → matches → brackets R.
    let mut cls = vec![
        BidiClass::R,
        BidiClass::ON,
        BidiClass::EN,
        BidiClass::ON,
        BidiClass::R,
    ];
    let pairs = vec![(1usize, 3usize)];
    resolve_bracket_pairs(&mut cls, &pairs, 1, BidiClass::R);
    assert_eq!(cls[1], BidiClass::R);
    assert_eq!(cls[3], BidiClass::R);
}

#[test]
fn r257_public_n0c2_an_preceding_counted_as_r_for_rtl() {
    // AN ( L ) ... — AN preceding-strong projects to R, L inside is
    // opposite of R embedding, so N0 c.2 fires → brackets R.
    let mut cls = vec![BidiClass::AN, BidiClass::ON, BidiClass::L, BidiClass::ON];
    let pairs = vec![(1usize, 3usize)];
    resolve_bracket_pairs(&mut cls, &pairs, 1, BidiClass::R);
    assert_eq!(cls[1], BidiClass::R);
    assert_eq!(cls[3], BidiClass::R);
}

// --- N0 sequential ordering: inner pair sees outer's rewrite -------

#[test]
fn r257_public_n0_sequential_inner_sees_outer_rewrite() {
    // RTL: R ( R ( L ) R ) R. Outer N0 b → outer brackets R. Inner
    // sees R (just rewritten) as preceding strong → N0 c.2 → inner R.
    let mut cls = vec![
        BidiClass::R,
        BidiClass::ON,
        BidiClass::R,
        BidiClass::ON,
        BidiClass::L,
        BidiClass::ON,
        BidiClass::R,
        BidiClass::ON,
        BidiClass::R,
    ];
    let pairs = vec![(1usize, 7usize), (3usize, 5usize)];
    resolve_bracket_pairs(&mut cls, &pairs, 1, BidiClass::R);
    for i in [1, 3, 5, 7] {
        assert_eq!(cls[i], BidiClass::R, "slot {i} should be R");
    }
}

// --- Full paragraph driver wiring ----------------------------------

#[test]
fn r257_public_driver_ltr_paragraph_brackets_at_level_0() {
    // "(a)" — LTR, paragraph level 0, all chars at 0.
    let (p, _) = process_paragraph_with_brackets("(a)", None);
    assert_eq!(p.paragraph_level, 0);
    assert_eq!(p.levels, vec![0, 0, 0]);
}

#[test]
fn r257_public_driver_rtl_paragraph_with_l_inside_brackets() {
    // RTL: HEBREW ( latin ) HEBREW.
    // N0: L inside an R embedding is opposite; preceding R matches
    // embedding → N0 c.2 → brackets resolve to R. The L run inside
    // bumps to even level 2 per I1 under odd embedding.
    let text = "\u{05D0}(abc)\u{05D1}";
    let (p, _) = process_paragraph_with_brackets(text, None);
    assert_eq!(p.paragraph_level, 1);
    assert_eq!(p.levels, vec![1, 1, 2, 2, 2, 1, 1]);
}

#[test]
fn r257_public_driver_rtl_paragraph_with_r_inside_brackets() {
    // RTL: HEBREW HEBREW ( HEBREW HEBREW ) — N0 b → brackets R.
    let text = "\u{05D0}\u{05D1}(\u{05D2}\u{05D3})";
    let (p, _) = process_paragraph_with_brackets(text, None);
    assert_eq!(p.paragraph_level, 1);
    assert_eq!(p.levels, vec![1; 6]);
}

#[test]
fn r257_public_driver_class_variant_length_match_required() {
    let result = std::panic::catch_unwind(|| {
        let chars = vec!['a', 'b'];
        let classes = vec![BidiClass::L];
        let _ = process_paragraph_classes_with_brackets(&classes, &chars, None);
    });
    assert!(result.is_err(), "length mismatch must panic");
}
