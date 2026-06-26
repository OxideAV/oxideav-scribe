//! Round 374 — `layout::shape_paragraphs`: the document-level layout
//! driver. Splits on the UAX #9 bidi-class-`B` paragraph-separator set
//! (LF, CR, CRLF, NEL `U+0085`, PARAGRAPH SEPARATOR `U+2029`), resolves
//! each paragraph's own base direction (P1 / P2 / P3), and wraps +
//! bidi-shapes each paragraph independently. (LINE SEPARATOR `U+2028`
//! is bidi-class `WS`, a line break within a paragraph, so it does not
//! start a new paragraph.)
//!
//! Provenance: exercises `layout::shape_paragraphs` over the public
//! `bidi::split_paragraphs` / `bidi::paragraph_level` (each citing
//! `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`) + the shaper.

use oxideav_scribe::layout::{shape_paragraphs, wrap_and_shape_lines};
use oxideav_scribe::{Face, FaceChain};

fn dejavu_chain() -> FaceChain {
    let bytes = include_bytes!("fixtures/DejaVuSans.ttf").to_vec();
    let face = Face::from_ttf_bytes(bytes).expect("DejaVu Sans parses");
    FaceChain::new(face)
}

const HE_ALEF: char = '\u{05D0}';
const HE_BET: char = '\u{05D1}';

#[test]
fn empty_text_is_no_paragraphs() {
    let chain = dejavu_chain();
    let paras = shape_paragraphs(&chain, "", 16.0, 200.0, None).expect("layout");
    assert!(paras.is_empty());
}

#[test]
fn newline_splits_into_paragraphs() {
    let chain = dejavu_chain();
    let paras = shape_paragraphs(&chain, "hello\nworld", 16.0, 1000.0, None).expect("layout");
    assert_eq!(paras.len(), 2);
    assert_eq!(paras[0].base_level, 0);
    assert_eq!(paras[1].base_level, 0);
    // Each paragraph is one line within the generous width.
    assert_eq!(paras[0].lines.len(), 1);
    assert_eq!(paras[1].lines.len(), 1);
}

#[test]
fn paragraph_separator_u2029_forces_a_break() {
    // U+2029 PARAGRAPH SEPARATOR is bidi-class B — a separator that
    // plain `\n`-splitting (ASCII only) would miss.
    let chain = dejavu_chain();
    let text = "ab\u{2029}cd";
    let paras = shape_paragraphs(&chain, text, 16.0, 1000.0, None).expect("layout");
    assert_eq!(paras.len(), 2, "U+2029 should split into two paragraphs");
}

#[test]
fn nel_u0085_forces_a_break() {
    // NEL U+0085 is bidi-class B — another non-ASCII paragraph separator.
    let chain = dejavu_chain();
    let text = "ab\u{0085}cd";
    let paras = shape_paragraphs(&chain, text, 16.0, 1000.0, None).expect("layout");
    assert_eq!(paras.len(), 2, "NEL should split into two paragraphs");
}

#[test]
fn line_separator_u2028_does_not_start_a_paragraph() {
    // U+2028 is bidi-class WS, not B → it stays inside one paragraph.
    let chain = dejavu_chain();
    let text = "ab\u{2028}cd";
    let paras = shape_paragraphs(&chain, text, 16.0, 1000.0, None).expect("layout");
    assert_eq!(
        paras.len(),
        1,
        "U+2028 is a line break, not a paragraph break"
    );
}

#[test]
fn each_paragraph_resolves_its_own_direction() {
    // Paragraph 1 is Hebrew (RTL), paragraph 2 is Latin (LTR). With
    // base_level = None each resolves independently per P2 / P3.
    let chain = dejavu_chain();
    let heb: String = [HE_ALEF, HE_BET].iter().collect();
    let text = format!("{heb}\nabc");
    let paras = shape_paragraphs(&chain, &text, 16.0, 1000.0, None).expect("layout");
    assert_eq!(paras.len(), 2);
    assert_eq!(paras[0].base_level, 1, "Hebrew paragraph is RTL");
    assert_eq!(paras[1].base_level, 0, "Latin paragraph is LTR");
}

#[test]
fn base_level_override_forces_uniform_direction() {
    // Some(1) forces every paragraph to RTL regardless of content.
    let chain = dejavu_chain();
    let paras = shape_paragraphs(&chain, "abc\ndef", 16.0, 1000.0, Some(1)).expect("layout");
    assert_eq!(paras.len(), 2);
    for p in &paras {
        assert_eq!(p.base_level, 1);
    }
}

#[test]
fn paragraph_wraps_to_width() {
    let chain = dejavu_chain();
    let text = "The quick brown fox\njumps over the lazy dog today";
    let paras = shape_paragraphs(&chain, text, 16.0, 120.0, None).expect("layout");
    assert_eq!(paras.len(), 2);
    // The second paragraph is longer and must wrap into multiple lines.
    assert!(
        paras[1].lines.len() > 1,
        "second paragraph should wrap: {} lines",
        paras[1].lines.len()
    );
}

#[test]
fn single_paragraph_matches_wrap_and_shape() {
    // With no separators, shape_paragraphs is one paragraph whose lines
    // match wrap_and_shape_lines at the paragraph's resolved level.
    let chain = dejavu_chain();
    let text = "alpha beta gamma delta epsilon zeta";
    let max = 100.0_f32;
    let paras = shape_paragraphs(&chain, text, 16.0, max, None).expect("layout");
    assert_eq!(paras.len(), 1);
    let expected = wrap_and_shape_lines(&chain, text, 16.0, max, Some(0)).expect("wrap+shape");
    assert_eq!(paras[0].lines.len(), expected.len());
    for (got, want) in paras[0].lines.iter().zip(expected.iter()) {
        assert_eq!(got.glyphs, want.glyphs);
    }
}

#[test]
fn empty_paragraph_between_separators_yields_blank_line() {
    // "a\n\nb": three paragraphs, the middle one empty → one blank line.
    let chain = dejavu_chain();
    let paras = shape_paragraphs(&chain, "a\n\nb", 16.0, 1000.0, None).expect("layout");
    assert_eq!(paras.len(), 3);
    assert_eq!(paras[1].lines.len(), 1);
    assert!(paras[1].lines[0].is_empty());
}
