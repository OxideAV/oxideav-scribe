//! Round-3 synthetic-bold integration tests.
//!
//! Verifies:
//! 1. `Style { weight: 700 }` against a Regular (400) face produces a
//!    visibly heavier rendering than `Style::REGULAR` (more non-zero
//!    alpha pixels and the bitmap may be larger to accommodate the
//!    dilated outline).
//! 2. `Style { weight: 400 }` against a Bold (700) face does NOT thin
//!    the strokes — we never *un*-bold a face.
//! 3. The synthetic-bold dilation honours the size: a 64-px Bold has
//!    a measurably larger pixel-count delta than a 16-px Bold.

use oxideav_scribe::{render_text_styled, synthetic_bold_radius, Face, Style, WHITE};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn load_face() -> Face {
    Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans must parse")
}

#[test]
fn bold_request_thickens_strokes_on_regular_face() {
    let face = load_face();
    // DejaVu Sans is Book / weight 400.
    assert!(
        face.weight_class() <= 500,
        "fixture should be a regular-ish face, got {}",
        face.weight_class()
    );

    let regular = render_text_styled(&face, "Hi", 32.0, WHITE, Style::REGULAR).expect("regular");
    let bold_style = Style::REGULAR.with_weight(700);
    let bold = render_text_styled(&face, "Hi", 32.0, WHITE, bold_style).expect("bold");

    // Bold should have visibly more non-zero alpha pixels than regular
    // (thicker strokes = more coverage).
    let r_count = regular.nonzero_alpha_count();
    let b_count = bold.nonzero_alpha_count();
    assert!(
        b_count > r_count,
        "synthetic bold should add non-zero alpha pixels: regular={r_count}, bold={b_count}"
    );
    // And the bitmap may be wider (dilation grows the bbox).
    assert!(
        bold.width >= regular.width,
        "bold bitmap should be at least as wide: regular={} bold={}",
        regular.width,
        bold.width
    );
}

#[test]
fn regular_request_on_bold_face_does_not_thin() {
    // We can't test against a real Bold face without a fixture, but
    // we can verify the radius function returns 0 for that direction.
    let r = synthetic_bold_radius(Style::REGULAR, 700, 32.0);
    assert_eq!(r, 0.0, "regular request on bold face must not synthesise");
}

#[test]
fn bold_radius_grows_with_size() {
    // The dilation radius scales with size_px; verify the rendered
    // pixel-count delta does too.
    let face = load_face();
    let bold_style = Style::REGULAR.with_weight(700);

    let r_small = render_text_styled(&face, "i", 16.0, WHITE, Style::REGULAR).expect("r_small");
    let b_small = render_text_styled(&face, "i", 16.0, WHITE, bold_style).expect("b_small");
    let r_large = render_text_styled(&face, "i", 64.0, WHITE, Style::REGULAR).expect("r_large");
    let b_large = render_text_styled(&face, "i", 64.0, WHITE, bold_style).expect("b_large");

    let small_delta = b_small.nonzero_alpha_count() as i32 - r_small.nonzero_alpha_count() as i32;
    let large_delta = b_large.nonzero_alpha_count() as i32 - r_large.nonzero_alpha_count() as i32;
    assert!(small_delta > 0, "small bold should add pixels");
    assert!(large_delta > 0, "large bold should add pixels");
    assert!(
        large_delta > small_delta,
        "larger size should add more bold pixels: small={small_delta}, large={large_delta}"
    );
}

#[test]
fn medium_weight_below_threshold_is_no_op() {
    // weight=500 (Medium) on a 400 (Regular) face is below the
    // SYNTHETIC_BOLD_THRESHOLD; should produce identical output.
    let face = load_face();
    let medium_style = Style::REGULAR.with_weight(500);

    let regular = render_text_styled(&face, "Test", 24.0, WHITE, Style::REGULAR).expect("reg");
    let medium = render_text_styled(&face, "Test", 24.0, WHITE, medium_style).expect("med");

    assert_eq!(
        regular.data, medium.data,
        "weight=500 vs 400 (delta=100) is below threshold and must be a no-op"
    );
}
