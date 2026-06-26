//! Round 374 — `layout::shape_visual_line`: the join between the
//! complete UAX #9 §3 / §3.4 reordering pipeline and the OpenType
//! shaper. It shapes each bidi **level run** in logical order (so
//! ligatures / joining / contextual rules see the natural character
//! sequence) and arranges the shaped runs in left-to-right visual
//! order, returning positioned glyphs a renderer paints with the pen
//! moving left-to-right.
//!
//! Provenance: exercises the public `bidi::` per-rule entry points
//! through `layout::shape_visual_line` (each citing
//! `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`) plus the
//! OpenType shaper (`docs/text/opentype/`).

use oxideav_scribe::layout::shape_visual_line;
use oxideav_scribe::{Face, FaceChain, Shaper};

fn dejavu_chain() -> FaceChain {
    let bytes = include_bytes!("fixtures/DejaVuSans.ttf").to_vec();
    let face = Face::from_ttf_bytes(bytes).expect("DejaVu Sans parses");
    FaceChain::new(face)
}

const HE_ALEF: char = '\u{05D0}';
const HE_BET: char = '\u{05D1}';
const HE_GIMEL: char = '\u{05D2}';

#[test]
fn empty_line_is_empty() {
    let chain = dejavu_chain();
    let line = shape_visual_line(&chain, "", 16.0, None).expect("shape");
    assert!(line.is_empty());
    assert_eq!(line.len(), 0);
    assert_eq!(line.base_level, 0);
    assert_eq!(line.width(), 0.0);
}

#[test]
fn pure_ltr_matches_direct_shaping() {
    // An all-Latin line is one level-0 run, so the visual-line output
    // must be byte-identical to a plain `Shaper::shape`.
    let chain = dejavu_chain();
    let text = "Hello, world!";
    let line = shape_visual_line(&chain, text, 16.0, None).expect("shape");
    assert_eq!(line.base_level, 0);
    let direct = Shaper::shape(chain.primary(), text, 16.0).expect("direct shape");
    assert_eq!(line.glyphs, direct);
}

#[test]
fn pure_rtl_reverses_run_glyphs() {
    // Three Hebrew letters: a single level-1 run. Logical shaping then
    // glyph reversal means the visual glyph order is the logical order
    // reversed — gimel, bet, alef.
    let chain = dejavu_chain();
    let text: String = [HE_ALEF, HE_BET, HE_GIMEL].iter().collect();
    let line = shape_visual_line(&chain, &text, 16.0, None).expect("shape");
    assert_eq!(line.base_level, 1);
    assert_eq!(line.len(), 3);

    // The logical shaping of the same three letters, glyph IDs in
    // logical order.
    let logical = Shaper::shape(chain.primary(), &text, 16.0).expect("direct");
    let logical_gids: Vec<u16> = logical.iter().map(|g| g.glyph_id).collect();
    let visual_gids: Vec<u16> = line.glyphs.iter().map(|g| g.glyph_id).collect();
    let mut reversed = logical_gids.clone();
    reversed.reverse();
    assert_eq!(visual_gids, reversed);
    // None of the Hebrew letters should be .notdef (DejaVu covers them).
    assert!(
        visual_gids.iter().all(|&g| g != 0),
        "got notdef: {visual_gids:?}"
    );
}

#[test]
fn mixed_ltr_then_rtl_orders_runs_visually() {
    // Logical "ab" + alef + bet, base level resolves LTR (first strong
    // is 'a'). Visual order: the Latin run "ab" stays leftmost, the
    // Hebrew run follows to its right but internally reversed → the
    // visual glyph stream is [a, b, bet, alef].
    let chain = dejavu_chain();
    let text: String = format!("ab{HE_ALEF}{HE_BET}");
    let line = shape_visual_line(&chain, &text, 16.0, None).expect("shape");
    assert_eq!(line.base_level, 0);
    assert_eq!(line.len(), 4);

    let a = Shaper::shape(chain.primary(), "a", 16.0).unwrap()[0].glyph_id;
    let b = Shaper::shape(chain.primary(), "b", 16.0).unwrap()[0].glyph_id;
    let alef = Shaper::shape(chain.primary(), &HE_ALEF.to_string(), 16.0).unwrap()[0].glyph_id;
    let bet = Shaper::shape(chain.primary(), &HE_BET.to_string(), 16.0).unwrap()[0].glyph_id;

    let gids: Vec<u16> = line.glyphs.iter().map(|g| g.glyph_id).collect();
    assert_eq!(gids, vec![a, b, bet, alef]);
}

#[test]
fn rtl_base_with_latin_island() {
    // A Hebrew line with an embedded Latin word. Base level 1 (RTL).
    // Logical: alef bet SPACE 'a' 'b'. The Latin "ab" is a level-2 LTR
    // island. Visual (LTR paint order): the line as a whole reads
    // right-to-left, so the Hebrew comes last visually (rightmost) and
    // the Latin island sits to its left, internally in logical order.
    // Expected visual glyph stream: [a, b, SPACE, bet, alef].
    let chain = dejavu_chain();
    let text: String = format!("{HE_ALEF}{HE_BET} ab");
    let line = shape_visual_line(&chain, &text, 16.0, None).expect("shape");
    assert_eq!(line.base_level, 1);
    assert_eq!(line.len(), 5);

    let a = Shaper::shape(chain.primary(), "a", 16.0).unwrap()[0].glyph_id;
    let b = Shaper::shape(chain.primary(), "b", 16.0).unwrap()[0].glyph_id;
    let space = Shaper::shape(chain.primary(), " ", 16.0).unwrap()[0].glyph_id;
    let alef = Shaper::shape(chain.primary(), &HE_ALEF.to_string(), 16.0).unwrap()[0].glyph_id;
    let bet = Shaper::shape(chain.primary(), &HE_BET.to_string(), 16.0).unwrap()[0].glyph_id;

    let gids: Vec<u16> = line.glyphs.iter().map(|g| g.glyph_id).collect();
    assert_eq!(gids, vec![a, b, space, bet, alef]);
}

#[test]
fn width_is_sum_of_all_run_advances() {
    // The visual line's total width must equal the sum of the
    // individual runs' widths, regardless of reordering.
    let chain = dejavu_chain();
    let text: String = format!("ab{HE_ALEF}{HE_BET}");
    let line = shape_visual_line(&chain, &text, 24.0, None).expect("shape");
    let latin = Shaper::shape(chain.primary(), "ab", 24.0).unwrap();
    let hebrew: String = [HE_ALEF, HE_BET].iter().collect();
    let heb = Shaper::shape(chain.primary(), &hebrew, 24.0).unwrap();
    let expected: f32 = latin.iter().map(|g| g.x_advance + g.x_offset).sum::<f32>()
        + heb.iter().map(|g| g.x_advance + g.x_offset).sum::<f32>();
    assert!(
        (line.width() - expected).abs() < 1e-3,
        "width {} vs expected {}",
        line.width(),
        expected
    );
}

#[test]
fn base_level_override_forces_direction() {
    // Forcing base level 1 on an all-Latin line keeps the Latin run's
    // internal order (one level-2 LTR island) but reports base RTL.
    let chain = dejavu_chain();
    let forced = shape_visual_line(&chain, "abc", 16.0, Some(1)).expect("shape");
    assert_eq!(forced.base_level, 1);
    let direct = Shaper::shape(chain.primary(), "abc", 16.0).unwrap();
    let dg: Vec<u16> = direct.iter().map(|g| g.glyph_id).collect();
    let fg: Vec<u16> = forced.glyphs.iter().map(|g| g.glyph_id).collect();
    assert_eq!(fg, dg);
}
