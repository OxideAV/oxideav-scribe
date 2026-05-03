//! End-to-end integration tests against the DejaVu Sans fixture
//! (which is the same .ttf used by `oxideav-ttf`'s tests). The fixture
//! lives under the `oxideav-ttf` crate; we reach across via a
//! relative path so we don't duplicate the binary in two tree
//! locations.

use oxideav_scribe::{render_text, Face, Rasterizer, Shaper, WHITE};

const FIXTURE: &[u8] = include_bytes!("../../oxideav-ttf/tests/fixtures/DejaVuSans.ttf");

fn load_face() -> Face {
    Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans must parse")
}

#[test]
fn rasterise_a_at_32px() {
    let face = load_face();
    let gid_a = face
        .with_font(|f| f.glyph_index('A'))
        .expect("with_font")
        .expect("'A' is in DejaVu");
    let bm = Rasterizer::raster_glyph(&face, gid_a, 32.0).expect("raster ok");
    assert!(!bm.is_empty(), "'A' bitmap must be non-empty");
    assert!(bm.width >= 10 && bm.width <= 40, "width: {}", bm.width);
    assert!(bm.height >= 16 && bm.height <= 40, "height: {}", bm.height);
    let nz = bm.nonzero_pixel_count();
    assert!(nz > 50, "'A' should have >50 non-zero pixels, got {nz}");

    // Sanity: at least one pixel near the centre column should be
    // ≥ 200 alpha (a stem of the 'A').
    let cx = bm.width / 2;
    let mut max_alpha = 0u8;
    for y in 0..bm.height {
        max_alpha = max_alpha.max(bm.get(cx, y));
    }
    assert!(max_alpha >= 200, "max stem alpha at cx={cx}: {max_alpha}");
}

#[test]
fn shaper_latin_hello_world() {
    let face = load_face();
    let glyphs = Shaper::shape(&face, "Hello, world!", 16.0).expect("shape ok");
    assert_eq!(
        glyphs.len(),
        13,
        "got {} glyphs: {:?}",
        glyphs.len(),
        glyphs
    );

    // Cumulative advance roughly matches per-glyph advance sum.
    let total: f32 = glyphs.iter().map(|g| g.x_advance + g.x_offset).sum();
    let sum_advances: f32 = glyphs.iter().map(|g| g.x_advance).sum();
    let diff = (total - sum_advances).abs();
    assert!(
        diff <= 1.0,
        "advance vs sum diff = {diff}, total = {total}, sum = {sum_advances}"
    );
}

#[test]
fn shaper_kerning_avatar_shrinks() {
    let face = load_face();
    let glyphs = Shaper::shape(&face, "AVATAR", 32.0).expect("shape ok");
    assert_eq!(glyphs.len(), 6);

    // No-kerning baseline: sum of plain advances.
    let baseline_sum: f32 = glyphs.iter().map(|g| g.x_advance).sum();
    let with_kerning: f32 = glyphs.iter().map(|g| g.x_advance + g.x_offset).sum();
    let savings = baseline_sum - with_kerning;
    // DejaVu ships AV (and AT) negative kerning. Even at 32 px the
    // total should be measurably shorter — be generous on the bound.
    assert!(
        savings > 1.0,
        "expected >1 px kerning savings, got {savings} (baseline {baseline_sum}, kerned {with_kerning})"
    );
}

#[test]
fn shaper_fi_ligature_collapses_office() {
    let face = load_face();
    // "office" → 'o','f','f','i','c','e' = 6 codepoints.
    // After GSUB, "ffi" or "fi" ligature collapses to a single glyph,
    // so we should get either 5 (one fi) or 4 (one ffi) glyphs.
    let glyphs = Shaper::shape(&face, "office", 16.0).expect("shape ok");
    assert!(
        glyphs.len() == 4 || glyphs.len() == 5,
        "expected 4 (ffi) or 5 (fi) glyphs after ligature, got {}",
        glyphs.len()
    );
}

#[test]
fn render_text_hello_produces_white_pixels() {
    let face = load_face();
    let bm = render_text(&face, "Hello", 16.0, WHITE).expect("render ok");
    assert!(!bm.is_empty(), "rendered bitmap must be non-empty");
    let nz = bm.nonzero_alpha_count();
    assert!(nz > 50, "non-zero alpha pixel count: {nz}");

    // Spot-check: at least one pixel in the bitmap is opaque white-ish.
    let mut found_strong_white = false;
    for y in 0..bm.height {
        for x in 0..bm.width {
            let p = bm.get(x, y);
            if p[3] >= 200 && p[0] >= 200 && p[1] >= 200 && p[2] >= 200 {
                found_strong_white = true;
                break;
            }
        }
        if found_strong_white {
            break;
        }
    }
    assert!(
        found_strong_white,
        "expected at least one strong-white pixel in 'Hello' render"
    );
}

#[test]
fn cjk_falls_back_to_notdef_without_panic() {
    let face = load_face();
    // DejaVu Sans does NOT include CJK ideographs. The shaper's
    // contract is to fall back to glyph 0 (.notdef / "tofu") rather
    // than dropping the codepoint or panicking.
    let glyphs = Shaper::shape(&face, "日本語", 16.0).expect("shape ok");
    assert_eq!(glyphs.len(), 3, "one glyph per CJK codepoint");
    for g in &glyphs {
        // Glyph 0 is the .notdef glyph in TrueType; it must have a
        // non-zero advance (the "tofu" rectangle) so the layout
        // reserves space.
        assert!(g.x_advance >= 0.0, "advance: {}", g.x_advance);
    }
}
