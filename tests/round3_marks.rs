//! Round-3 mark-to-base attachment integration tests.
//!
//! Exercises GPOS LookupType 4 against a real font (DejaVu Sans, which
//! ships mark-to-base anchors for combining diacritics). Verifies:
//!
//! 1. A combining mark following its base is identified as a mark via
//!    GDEF and gets non-zero y_offset (the diacritic is lifted onto
//!    the base's anchor).
//! 2. The mark's x_advance is zeroed so a following glyph lands at
//!    the post-base pen position rather than after the mark.
//! 3. A standalone mark (no preceding base) is left untouched.

use oxideav_scribe::{Face, Shaper};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn load_face() -> Face {
    Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans must parse")
}

#[test]
fn combining_acute_attaches_to_a() {
    // 'A' (U+0041) + COMBINING ACUTE ACCENT (U+0301).
    // DejaVu Sans is one of the most commonly distributed fonts that
    // includes mark anchors for the combining diacritics — this pair
    // is a canonical mark-to-base test case.
    let face = load_face();
    let glyphs = Shaper::shape(&face, "A\u{0301}", 32.0).expect("shape");
    assert_eq!(glyphs.len(), 2, "expected 2 glyphs (no precomposed Á)");

    let base = glyphs[0];
    let mark = glyphs[1];

    // Sanity: confirm the mark glyph is recognised as a mark by GDEF.
    let mark_gid = mark.glyph_id;
    let is_mark = face
        .with_font(|f| f.is_mark_glyph(mark_gid))
        .expect("with_font");
    assert!(
        is_mark,
        "DejaVu Sans's combining acute (gid {mark_gid}) should be flagged as a mark by GDEF",
    );

    // Sanity: confirm there's a mark-to-base anchor for this pair.
    let base_gid = base.glyph_id;
    let anchor = face
        .with_font(|f| f.lookup_mark_to_base(base_gid, mark_gid))
        .expect("with_font");
    assert!(
        anchor.is_some(),
        "DejaVu Sans should ship a mark-to-base anchor for ('A' = {base_gid}, combining-acute = {mark_gid})"
    );

    // After shaping, the mark must have:
    // 1. Non-zero y_offset (lifted up onto the base anchor) — combining
    //    acute sits *above* 'A' so its y_offset (raster Y-down) must be
    //    NEGATIVE.
    assert!(
        mark.y_offset < 0.0,
        "combining acute should be lifted above the baseline (y_offset < 0); got {}",
        mark.y_offset
    );

    // 2. Zero advance (the mark stacks on the base, doesn't push
    //    subsequent glyphs).
    assert_eq!(
        mark.x_advance, 0.0,
        "mark advance should be zeroed after attachment"
    );
}

#[test]
fn combining_mark_y_offset_proportional_to_size() {
    // Same pair at two different sizes — the y_offset must scale with
    // size_px (anchors are font-unit based; the shaper applies the
    // pixel scale).
    let face = load_face();
    let small = Shaper::shape(&face, "A\u{0301}", 16.0).expect("shape small");
    let large = Shaper::shape(&face, "A\u{0301}", 64.0).expect("shape large");

    assert_eq!(small.len(), 2);
    assert_eq!(large.len(), 2);
    let y_small = small[1].y_offset;
    let y_large = large[1].y_offset;
    assert!(y_small < 0.0);
    assert!(y_large < 0.0);

    // 4× the size → roughly 4× the offset (allowing slop because of
    // any sub-pixel rounding inside the shaper).
    let ratio = y_large / y_small;
    assert!(
        ratio > 3.5 && ratio < 4.5,
        "y_offset should scale ~linearly with size_px: small={y_small}, large={y_large}, ratio={ratio}"
    );
}

#[test]
fn standalone_mark_is_left_untouched() {
    // A combining mark without a preceding base — there's nothing to
    // attach to. The shaper should leave the offsets at 0 (and not
    // panic).
    let face = load_face();
    let glyphs = Shaper::shape(&face, "\u{0301}", 24.0).expect("shape");
    assert_eq!(glyphs.len(), 1);
    let g = glyphs[0];
    assert_eq!(g.x_offset, 0.0, "no base → no anchor delta");
    assert_eq!(g.y_offset, 0.0, "no base → no anchor delta");
    // Advance should be the glyph's own (non-zeroed because no
    // attachment happened).
    let original_adv = face
        .with_font(|f| {
            let upem = f.units_per_em().max(1) as f32;
            let scale = 24.0_f32 / upem;
            f.glyph_advance(g.glyph_id) as f32 * scale
        })
        .expect("with_font");
    assert!(
        (g.x_advance - original_adv).abs() < 1e-3,
        "standalone mark should keep its own advance: got {} expected {}",
        g.x_advance,
        original_adv
    );
}

#[test]
fn pure_base_run_unchanged_by_round3() {
    // Sanity: a string with no marks should produce identical glyphs
    // to a pure round-2 shape (no advance zeroing, no offset shift
    // from mark-to-base).
    let face = load_face();
    let glyphs = Shaper::shape(&face, "AVA", 24.0).expect("shape");
    assert_eq!(glyphs.len(), 3);
    for g in &glyphs {
        assert_eq!(g.y_offset, 0.0, "no marks → no y_offset");
        assert!(g.x_advance > 0.0, "base advances are non-zero");
    }
}
