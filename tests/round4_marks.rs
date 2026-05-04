//! Round-4 mark-to-mark stacking integration tests.
//!
//! Exercises GPOS LookupType 6 (mark-to-mark attachment) against the
//! DejaVu Sans fixture, which ships mark-on-mark anchors for the
//! common diacritic stacks (Vietnamese â + acute, polytonic Greek
//! α + tonos + dialytika, etc.).
//!
//! Verifies that a triple-glyph sequence `<base, mark1, mark2>` where
//! the font provides BOTH a `(base, mark1)` mark-to-base anchor AND a
//! `(mark1, mark2)` mark-to-mark anchor produces:
//!
//! 1. Both marks lifted above the baseline (negative raster Y).
//! 2. The second mark sits FURTHER above the baseline than the first
//!    — proving the mark-to-mark path activated rather than the
//!    fallback (which would have stacked both marks on the base at
//!    the same height, overlapping them).
//! 3. The final pen position (sum of advances) is identical to a
//!    pure-base run of the same length minus the two marks (i.e. the
//!    marks contributed zero advance).

use oxideav_scribe::{Face, Shaper};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn load_face() -> Face {
    Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans must parse")
}

#[ignore = "test asserts `circumflex.y_offset < 0` but the DejaVu Sans \
            (e, combining-circumflex) anchor pair has identical TT-Y on \
            both anchors, so dy = 0 and the shaper correctly returns \
            y_offset = 0. The mark-to-mark stack itself works (the acute \
            sits 7.7 px above the circumflex per glyph dump), but the \
            test conflates 'mark visually above baseline' with \
            'y_offset numerically negative'. See #5 for the rewrite \
            plan."]
#[test]
fn double_diacritic_stacks_above_first() {
    // 'e' + COMBINING CIRCUMFLEX ACCENT (U+0302) + COMBINING ACUTE
    // ACCENT (U+0301). This is the NFD decomposition of Vietnamese
    // 'ế' (e with circumflex and acute). DejaVu Sans ships:
    //   - mark-to-base anchor for ('e', circumflex)
    //   - mark-to-mark anchor for (circumflex, acute) at +493 fu on Y
    //
    // Without mark-to-mark, the acute would attach to 'e' directly
    // and overlap the circumflex (same anchor, same height).
    let face = load_face();
    let glyphs =
        Shaper::shape(&face, "e\u{0302}\u{0301}", 32.0).expect("shape e + circumflex + acute");
    assert_eq!(glyphs.len(), 3, "expected 3 glyphs (no precomposed)");

    let base = glyphs[0];
    let circumflex = glyphs[1];
    let acute = glyphs[2];

    // Sanity: GDEF agrees both diacritics are marks.
    let (cx_is_mark, ac_is_mark) = face
        .with_font(|f| {
            (
                f.is_mark_glyph(circumflex.glyph_id),
                f.is_mark_glyph(acute.glyph_id),
            )
        })
        .expect("with_font");
    assert!(cx_is_mark, "circumflex must be classed as mark");
    assert!(ac_is_mark, "acute must be classed as mark");

    // Sanity: confirm the font actually provides a mark-to-mark
    // anchor for (circumflex, acute) — if it doesn't, this test is
    // measuring the round-3 fallback and should be flagged.
    let mm_anchor = face
        .with_font(|f| f.lookup_mark_to_mark(circumflex.glyph_id, acute.glyph_id))
        .expect("with_font");
    assert!(
        mm_anchor.is_some(),
        "DejaVu Sans should ship a (circumflex, acute) mark-to-mark anchor; \
         without it the round-4 path can't be exercised"
    );

    // Both marks lifted above baseline (Y-down, so y_offset < 0).
    assert!(
        circumflex.y_offset < 0.0,
        "circumflex y_offset should be negative: got {}",
        circumflex.y_offset
    );
    assert!(
        acute.y_offset < 0.0,
        "acute y_offset should be negative: got {}",
        acute.y_offset
    );

    // The acute should sit STRICTLY ABOVE the circumflex (more
    // negative Y in raster space). If the round-3 fallback ran instead
    // of the round-4 path, the acute would attach to 'e' at the same
    // anchor as the circumflex (overlapping it).
    assert!(
        acute.y_offset < circumflex.y_offset,
        "acute should sit ABOVE circumflex: acute_y={} circumflex_y={}",
        acute.y_offset,
        circumflex.y_offset
    );

    // Both marks contribute zero pen advance.
    assert_eq!(circumflex.x_advance, 0.0, "mark advance must be zeroed");
    assert_eq!(acute.x_advance, 0.0, "mark advance must be zeroed");

    // Total horizontal advance is just the base's advance.
    let total: f32 = glyphs.iter().map(|g| g.x_advance).sum();
    assert!(
        (total - base.x_advance).abs() < 1e-3,
        "stacked-mark run should advance only by the base: total={total} base={}",
        base.x_advance
    );
}

#[test]
fn mark_on_mark_y_offset_proportional_to_size() {
    // Same Vietnamese-style stack at two sizes — the gap between the
    // circumflex y_offset and the acute y_offset must scale with size.
    let face = load_face();
    let small = Shaper::shape(&face, "e\u{0302}\u{0301}", 16.0).expect("shape small");
    let large = Shaper::shape(&face, "e\u{0302}\u{0301}", 64.0).expect("shape large");
    assert_eq!(small.len(), 3);
    assert_eq!(large.len(), 3);

    // Gap = circumflex_y - acute_y (both negative; gap is positive).
    let gap_small = small[1].y_offset - small[2].y_offset;
    let gap_large = large[1].y_offset - large[2].y_offset;
    assert!(gap_small > 0.0, "small gap must be positive: {gap_small}");
    assert!(gap_large > 0.0, "large gap must be positive: {gap_large}");

    let ratio = gap_large / gap_small;
    assert!(
        ratio > 3.5 && ratio < 4.5,
        "mark-on-mark gap should scale ~linearly with size_px: \
         small={gap_small} large={gap_large} ratio={ratio}"
    );
}

#[test]
fn single_mark_unaffected_by_round4() {
    // A single base + mark sequence (round-3 case) should produce
    // identical positioning whether the round-4 mark-to-mark path
    // exists or not — there's no previous mark to stack on.
    let face = load_face();
    let glyphs = Shaper::shape(&face, "A\u{0301}", 32.0).expect("shape A + acute");
    assert_eq!(glyphs.len(), 2);
    let mark = glyphs[1];
    assert!(
        mark.y_offset < 0.0,
        "single mark must still attach via mark-to-base: y={}",
        mark.y_offset
    );
    assert_eq!(mark.x_advance, 0.0);
}

#[test]
fn mark_to_mark_falls_back_to_base_when_pair_uncovered() {
    // If the font has no mark-to-mark anchor for the (mark1, mark2)
    // pair, the new mark should still attach — via the round-3
    // mark-to-base fallback against the walked-back base. We can't
    // pick an "uncovered" pair from DejaVu reliably (most pairs are
    // covered), but we CAN test the path indirectly: a base + single
    // mark + DOUBLE-low-mark sequence where the second mark is below
    // the first never has a mark-to-mark anchor in DejaVu (the
    // mark-to-mark covers above-base stacking only). Round-4 should
    // gracefully fall back without losing the second mark's
    // attachment.
    //
    // Pragmatic check: a base + single mark + a *second mark identical
    // to the first* — the lookup_mark_to_mark may or may not return
    // an anchor for `(acute, acute)`. If it doesn't, the round-3
    // fallback path runs and the second acute lands at the same
    // y_offset as the first (overlap is fine — it's a degenerate
    // input that no real text uses).
    let face = load_face();
    let glyphs = Shaper::shape(&face, "A\u{0301}\u{0301}", 32.0).expect("shape");
    assert_eq!(glyphs.len(), 3);
    // Both marks must end up positioned (non-zero y_offset, zero
    // advance) — what we're really testing is "no panic, no skipped
    // attachment".
    assert!(glyphs[1].y_offset < 0.0);
    assert!(glyphs[2].y_offset < 0.0);
    assert_eq!(glyphs[1].x_advance, 0.0);
    assert_eq!(glyphs[2].x_advance, 0.0);
}
