//! Round-4 oblique fixture integration tests.
//!
//! Round 2 introduced `Style::italic()` + `synthetic_italic_shear` and
//! deferred the "request italic on a font that's already italic"
//! integration test until a small oblique fixture landed in-tree.
//! Round 4 ships `DejaVuSansMono-Oblique.ttf` (252 KB) which has a
//! `post.italicAngle` of `-11.0°` — well outside the
//! `ITALIC_ANGLE_EPSILON_DEG = 0.5` band, so the synthetic-italic
//! suppression path activates.
//!
//! Mirrors the structure of `round2_features.rs::italic_*` tests but
//! with the roles swapped: instead of "upright → italic shears the
//! glyph", we verify "italic → italic does NOT double-shear".

use oxideav_scribe::{render_text_styled, synthetic_italic_shear, Face, Style, WHITE};

const FIXTURE_OBLIQUE: &[u8] = include_bytes!("fixtures/DejaVuSansMono-Oblique.ttf");
const FIXTURE_UPRIGHT: &[u8] = include_bytes!("fixtures/DejaVuSansMono.ttf");

fn load_oblique() -> Face {
    Face::from_ttf_bytes(FIXTURE_OBLIQUE.to_vec()).expect("DejaVu Sans Mono Oblique must parse")
}

fn load_upright_mono() -> Face {
    Face::from_ttf_bytes(FIXTURE_UPRIGHT.to_vec()).expect("DejaVu Sans Mono must parse")
}

#[test]
fn oblique_face_reports_negative_italic_angle() {
    // DejaVuSansMono-Oblique has `post.italicAngle = -11.0` (a
    // forward slant in the TT/post sign convention).
    let face = load_oblique();
    let angle = face.italic_angle();
    assert!(
        (-12.0..=-10.0).contains(&angle),
        "expected ~-11° italic angle, got {angle}"
    );
}

#[test]
fn synthetic_italic_shear_zero_on_oblique_face() {
    // The pure-function unit test in style::tests already covers this,
    // but we verify it on a real face to confirm the metadata
    // round-trips through `Face::italic_angle` correctly.
    let face = load_oblique();
    let shear = synthetic_italic_shear(Style::italic(), face.italic_angle());
    assert_eq!(
        shear, 0.0,
        "synthetic italic on already-italic face should be 0 shear; got {shear}"
    );
}

#[test]
fn italic_request_on_oblique_face_does_not_double_shear() {
    // Render the same glyph at the same size on an oblique face, once
    // with REGULAR (which the renderer treats as "honour the font's
    // own slant — no synthesis") and once with italic() (which
    // ALSO honours the font's own slant per the
    // `synthetic_italic_shear == 0` rule).
    //
    // Both renders should produce IDENTICAL bitmaps. If we mistakenly
    // double-sheared, the italic() render would be wider than REGULAR.
    let face = load_oblique();
    let regular =
        render_text_styled(&face, "I", 32.0, WHITE, Style::REGULAR).expect("render REGULAR");
    let italic =
        render_text_styled(&face, "I", 32.0, WHITE, Style::italic()).expect("render italic");

    assert_eq!(
        regular.width, italic.width,
        "italic on oblique face must match REGULAR width: REGULAR={} italic={}",
        regular.width, italic.width
    );
    assert_eq!(
        regular.height, italic.height,
        "italic on oblique face must match REGULAR height"
    );
    // Pixel-for-pixel equality. The bitmaps share the same alpha
    // mask because no shear was added.
    assert_eq!(
        regular.data, italic.data,
        "italic on oblique face must be bit-identical to REGULAR — \
         a non-zero diff means we double-sheared the outline"
    );
}

#[test]
fn upright_face_still_synthesises_under_italic_request() {
    // Sanity: the same Style::italic() request against the
    // matching upright fixture DOES synthesise (different bitmap
    // than REGULAR). Confirms the asymmetry between upright and
    // oblique faces survives end-to-end.
    let face = load_upright_mono();
    assert!(
        face.italic_angle().abs() < 0.5,
        "upright fixture should be near-zero italic angle"
    );
    let regular =
        render_text_styled(&face, "I", 32.0, WHITE, Style::REGULAR).expect("render REGULAR");
    let italic =
        render_text_styled(&face, "I", 32.0, WHITE, Style::italic()).expect("render italic");

    // An upright face under italic() request: synthesised shear
    // produces a wider 'I' (the top of the stem moves right of the
    // base of the stem).
    assert!(
        italic.width > regular.width,
        "italic on upright face SHOULD be wider: REGULAR={} italic={}",
        regular.width,
        italic.width
    );
}

#[test]
fn oblique_face_under_regular_request_renders_slanted() {
    // A REGULAR request on an oblique face should still render the
    // glyph with the font's own forward slant (the renderer never
    // un-italicises). Verify by comparing 'I' on the oblique face
    // (Style::REGULAR) vs 'I' on the upright mono face
    // (Style::REGULAR) — the oblique 'I' should have a measurably
    // different alpha distribution (its top stem pixels are shifted
    // right relative to the upright 'I' base).
    let oblique = load_oblique();
    let upright = load_upright_mono();
    let bm_obl =
        render_text_styled(&oblique, "I", 48.0, WHITE, Style::REGULAR).expect("oblique REGULAR");
    let bm_up =
        render_text_styled(&upright, "I", 48.0, WHITE, Style::REGULAR).expect("upright REGULAR");

    assert!(!bm_obl.is_empty());
    assert!(!bm_up.is_empty());

    // Top quarter, right half — oblique should have more alpha there
    // than upright (the slanted top stem occupies right-of-centre).
    let top_q = bm_up.height.min(bm_obl.height) / 4;
    let sum_top_right = |bm: &oxideav_scribe::RgbaBitmap| -> u32 {
        let mut s = 0u32;
        for y in 0..top_q {
            for x in (bm.width / 2)..bm.width {
                s += bm.get(x, y)[3] as u32;
            }
        }
        s
    };
    let obl_top_right = sum_top_right(&bm_obl);
    let up_top_right = sum_top_right(&bm_up);
    // Sanity: at least one has alpha there.
    assert!(
        obl_top_right + up_top_right > 0,
        "neither bitmap has alpha in top-right quadrant"
    );
    // Oblique should have at least as much alpha in the top-right
    // (the slant pushes the upper stem rightwards).
    assert!(
        obl_top_right >= up_top_right,
        "oblique top-right alpha sum ({obl_top_right}) should be >= upright ({up_top_right})"
    );
}
