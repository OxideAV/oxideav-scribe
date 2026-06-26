//! Round 374 — `layout::wrap_and_shape_lines`: the one-call path from a
//! paragraph of logical text to width-wrapped, bidi-ordered, shaped
//! visual lines. Composes `wrap_lines` (width-constrained breaking) with
//! `shape_visual_line` (per-line UAX #9 reorder + shape).
//!
//! Provenance: exercises `layout::wrap_and_shape_lines` over the public
//! line breaker + the bidi/shaper join (each citing
//! `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` +
//! `docs/text/opentype/`).

use oxideav_scribe::layout::{shape_visual_line, wrap_and_shape_lines, wrap_lines};
use oxideav_scribe::{Face, FaceChain, Shaper};

fn dejavu_chain() -> FaceChain {
    let bytes = include_bytes!("fixtures/DejaVuSans.ttf").to_vec();
    let face = Face::from_ttf_bytes(bytes).expect("DejaVu Sans parses");
    FaceChain::new(face)
}

const HE_ALEF: char = '\u{05D0}';
const HE_BET: char = '\u{05D1}';

#[test]
fn empty_text_is_no_lines() {
    let chain = dejavu_chain();
    let lines = wrap_and_shape_lines(&chain, "", 16.0, 200.0, None).expect("wrap+shape");
    assert!(lines.is_empty());
}

#[test]
fn unconstrained_width_keeps_newline_lines() {
    // max_width <= 0 → one line per hard newline, each shaped.
    let chain = dejavu_chain();
    let lines = wrap_and_shape_lines(&chain, "ab\ncd", 16.0, 0.0, None).expect("wrap+shape");
    assert_eq!(lines.len(), 2);
    // Line 0 == shape "ab", line 1 == shape "cd".
    let l0 = shape_visual_line(&chain, "ab", 16.0, None).unwrap();
    let l1 = shape_visual_line(&chain, "cd", 16.0, None).unwrap();
    assert_eq!(lines[0].glyphs, l0.glyphs);
    assert_eq!(lines[1].glyphs, l1.glyphs);
}

#[test]
fn every_shaped_line_fits_within_max_width() {
    let chain = dejavu_chain();
    let text = "The quick brown fox jumps over the lazy dog";
    let max = 120.0_f32;
    let lines = wrap_and_shape_lines(&chain, text, 16.0, max, None).expect("wrap+shape");
    assert!(lines.len() > 1, "expected the text to wrap into >1 line");
    for line in &lines {
        // wrap_lines guarantees each non-hard-broken line fits; a
        // trailing space can push the measured width slightly, so allow
        // one space-glyph of slack.
        let space_w = Shaper::shape(chain.primary(), " ", 16.0).unwrap()[0].x_advance;
        assert!(
            line.width() <= max + space_w,
            "line width {} exceeds max {} (+ space {})",
            line.width(),
            max,
            space_w
        );
    }
}

#[test]
fn lines_match_independent_wrap_then_shape() {
    // wrap_and_shape_lines must equal wrap_lines + per-line
    // shape_visual_line — the documented composition.
    let chain = dejavu_chain();
    let text = "alpha beta gamma delta epsilon";
    let max = 100.0_f32;
    let combined = wrap_and_shape_lines(&chain, text, 16.0, max, None).expect("wrap+shape");
    let strings = wrap_lines(chain.primary(), text, 16.0, max).expect("wrap");
    assert_eq!(combined.len(), strings.len());
    for (line, s) in combined.iter().zip(strings.iter()) {
        let expected = shape_visual_line(&chain, s, 16.0, None).unwrap();
        assert_eq!(line.glyphs, expected.glyphs);
        assert_eq!(line.base_level, expected.base_level);
    }
}

#[test]
fn rtl_base_override_applies_to_every_line() {
    // A multi-line Hebrew paragraph forced to base level 1: every line
    // reports base_level 1 even if the wrap happens to put a Latin or
    // neutral token first.
    let chain = dejavu_chain();
    let heb: String = format!("{HE_ALEF}{HE_BET} {HE_ALEF}{HE_BET} {HE_ALEF}{HE_BET}");
    let lines = wrap_and_shape_lines(&chain, &heb, 16.0, 40.0, Some(1)).expect("wrap+shape");
    assert!(!lines.is_empty());
    for line in &lines {
        assert_eq!(line.base_level, 1);
    }
}
