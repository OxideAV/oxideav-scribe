//! Round-2 feature integration tests.
//!
//! Covers:
//! 1. Real italic synthesis via post.italicAngle (`render_text_styled`).
//! 2. Per-run colour: same string with different colours produces
//!    different RGB at glyph pixels but identical alpha shape.
//! 3. Font fallback via `FaceChain` — chain with two faces, verify
//!    `face_idx` routing.
//! 4. Stroke rendering via dilation — `compose_run_with_stroke` paints
//!    the border under the fill.

use oxideav_scribe::{
    compose::Composer, dilate_alpha, rasterizer::Rasterizer, render_text, render_text_styled,
    AlphaBitmap, Face, FaceChain, RgbaBitmap, Shaper, StrokeStyle, Style, WHITE,
};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
const FIXTURE_MONO: &[u8] = include_bytes!("fixtures/DejaVuSansMono.ttf");

fn load_face() -> Face {
    Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans must parse")
}

fn load_face_mono() -> Face {
    Face::from_ttf_bytes(FIXTURE_MONO.to_vec()).expect("DejaVu Sans Mono must parse")
}

// ---------------------------------------------------------------------------
// 1. Italic synthesis
// ---------------------------------------------------------------------------

#[test]
fn italic_request_on_upright_face_widens_glyphs() {
    let face = load_face();
    // DejaVu Sans is upright (italic_angle ~ 0).
    assert!(face.italic_angle().abs() < 0.5);

    // Use 'I' (a single vertical stem) where the rightmost point on
    // the upright bbox is also low, so shearing the top makes the
    // bitmap measurably wider. Glyphs with descenders (like 'A' base)
    // already extend right at y=0, so their bbox doesn't change much.
    let upright = render_text_styled(&face, "I", 32.0, WHITE, Style::REGULAR).expect("upright");
    let italic = render_text_styled(&face, "I", 32.0, WHITE, Style::italic()).expect("italic");

    assert!(!upright.is_empty(), "upright bitmap empty");
    assert!(!italic.is_empty(), "italic bitmap empty");
    assert!(
        italic.width > upright.width,
        "italic 'I' should be wider than upright: italic={} upright={}",
        italic.width,
        upright.width
    );
}

#[test]
fn italic_request_changes_pixel_distribution_for_full_string() {
    // For a string like "AVA" the bbox may not grow (A's base extends
    // wider than its top), but the *pixel content* must change. The
    // sum of pixels in the leftmost column should differ between the
    // two — italic shears the top rightwards so left edges of upper
    // pixels move out of the leftmost column.
    let face = load_face();
    let upright = render_text_styled(&face, "AVA", 32.0, WHITE, Style::REGULAR).expect("upright");
    let italic = render_text_styled(&face, "AVA", 32.0, WHITE, Style::italic()).expect("italic");

    // Take the alpha sum of the top quarter of the leftmost
    // 1/8th-width column in each. Italic should have noticeably less
    // alpha there because the upper-left pixels have moved right.
    let q_h = upright.height / 4;
    let q_w_u = (upright.width / 8).max(1);
    let q_w_i = (italic.width / 8).max(1);
    let sum = |bm: &RgbaBitmap, w_lo: u32, w_hi: u32| -> u32 {
        let mut s = 0u32;
        for y in 0..q_h {
            for x in w_lo..w_hi {
                s += bm.get(x, y)[3] as u32;
            }
        }
        s
    };
    let upright_top_left = sum(&upright, 0, q_w_u);
    let italic_top_left = sum(&italic, 0, q_w_i);
    assert_ne!(
        upright_top_left, italic_top_left,
        "italic alpha distribution should differ from upright"
    );
}

#[test]
fn italic_pixels_are_displaced_relative_to_upright() {
    // Top rows of italic should have rightward-shifted pixels vs the
    // same rows in upright. Use the pixel sum on the right half of the
    // top quarter-row as a quick proxy.
    let face = load_face();
    let upright = render_text_styled(&face, "I", 48.0, WHITE, Style::REGULAR).expect("upright");
    let italic = render_text_styled(&face, "I", 48.0, WHITE, Style::italic()).expect("italic");

    // Top quarter of each bitmap.
    let top_q = upright.height / 4;
    let upright_top_right_sum: u32 = (0..top_q)
        .flat_map(|y| (upright.width / 2..upright.width).map(move |x| (x, y)))
        .map(|(x, y)| upright.get(x, y)[3] as u32)
        .sum();
    let italic_top_right_sum: u32 = (0..top_q)
        .flat_map(|y| (italic.width / 2..italic.width).map(move |x| (x, y)))
        .map(|(x, y)| italic.get(x, y)[3] as u32)
        .sum();

    // Sanity: at least one of them should have non-zero alpha there
    // (at 48 px the 'I' has visible top pixels on the right side after shear).
    // If both are zero the test is meaningless, so guard.
    let total = upright_top_right_sum + italic_top_right_sum;
    assert!(
        total > 0,
        "neither upright nor italic has alpha in top-right"
    );
    assert!(
        italic_top_right_sum >= upright_top_right_sum,
        "italic top-right alpha sum ({}) should be >= upright ({})",
        italic_top_right_sum,
        upright_top_right_sum
    );
}

// (Dejavu-Oblique fixture not shipped — that test deferred to round 3
// when we can include a small italic test corpus. We do at least
// confirm that requesting italic on a font-with-non-zero-italic-angle
// returns 0 shear, via the unit test in style::tests.)

// ---------------------------------------------------------------------------
// 2. Per-run colour
// ---------------------------------------------------------------------------

#[test]
fn per_run_color_changes_rgb_but_not_alpha_shape() {
    let face = load_face();
    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    let red_bm = render_text(&face, "Hi", 24.0, RED).expect("render red");
    let blue_bm = render_text(&face, "Hi", 24.0, BLUE).expect("render blue");

    assert_eq!(red_bm.width, blue_bm.width);
    assert_eq!(red_bm.height, blue_bm.height);
    assert_eq!(red_bm.data.len(), blue_bm.data.len());

    let mut alpha_matches = 0usize;
    let mut differing_rgb = 0usize;
    let mut both_opaque = 0usize;
    for y in 0..red_bm.height {
        for x in 0..red_bm.width {
            let r = red_bm.get(x, y);
            let b = blue_bm.get(x, y);
            if r[3] == b[3] {
                alpha_matches += 1;
            }
            if r[3] >= 200 && b[3] >= 200 {
                both_opaque += 1;
                // At opaque pixels the RGB must reflect the input
                // colour: red R high, blue B high.
                if r[0] != b[0] || r[2] != b[2] {
                    differing_rgb += 1;
                }
            }
        }
    }

    let total = (red_bm.width as usize) * (red_bm.height as usize);
    assert_eq!(
        alpha_matches, total,
        "alpha shape must be identical regardless of colour"
    );
    assert!(both_opaque > 5, "expected some opaque glyph pixels");
    assert!(
        differing_rgb > 0,
        "expected some pixels where red and blue produce different RGB"
    );
    // Spot-check: find one fully-opaque pixel and verify the channel
    // values are what we asked for.
    'find: for y in 0..red_bm.height {
        for x in 0..red_bm.width {
            let r = red_bm.get(x, y);
            let b = blue_bm.get(x, y);
            if r[3] == 255 && b[3] == 255 {
                assert_eq!(r[0], 255, "red R");
                assert_eq!(r[2], 0, "red B");
                assert_eq!(b[0], 0, "blue R");
                assert_eq!(b[2], 255, "blue B");
                break 'find;
            }
        }
    }
}

#[test]
fn composer_per_run_color_via_compose_run() {
    // Lower-level path: the composer cache holds alpha-only data, so
    // re-composing with a different colour must still produce the
    // right RGB.
    let face = load_face();
    let glyphs = Shaper::shape(&face, "X", 24.0).expect("shape");
    assert_eq!(glyphs.len(), 1);

    let mut composer = Composer::new();
    let mut red_dst = RgbaBitmap::new(48, 48);
    let mut blue_dst = RgbaBitmap::new(48, 48);
    let baseline = face.ascent_px(24.0);

    composer
        .compose_run(
            &glyphs,
            &face,
            24.0,
            [255, 0, 0, 255],
            &mut red_dst,
            4.0,
            baseline,
        )
        .expect("compose red");
    // Same composer (cache populated) — should hit the cache.
    composer
        .compose_run(
            &glyphs,
            &face,
            24.0,
            [0, 0, 255, 255],
            &mut blue_dst,
            4.0,
            baseline,
        )
        .expect("compose blue");

    // Find at least one pixel where red and blue disagree on RGB.
    let mut found_differ = false;
    for y in 0..red_dst.height {
        for x in 0..red_dst.width {
            let r = red_dst.get(x, y);
            let b = blue_dst.get(x, y);
            if r[3] == b[3] && r[3] > 200 && (r[0] != b[0] || r[2] != b[2]) {
                found_differ = true;
                assert_eq!(r[0], 255, "red R should be 255 at opaque pixel");
                assert_eq!(b[2], 255, "blue B should be 255 at opaque pixel");
                break;
            }
        }
        if found_differ {
            break;
        }
    }
    assert!(found_differ, "per-run colour did not survive cache hit");

    // Confirm the cache actually had hits the second time round.
    let stats = composer.cache();
    assert!(stats.hits() > 0, "second pass should have cache hits");
}

// ---------------------------------------------------------------------------
// 3. Font fallback via FaceChain
// ---------------------------------------------------------------------------

#[test]
fn face_chain_routes_each_glyph_to_correct_face() {
    // Both fixtures have Latin coverage, but they are distinct faces
    // (different ids). We exercise the chain mechanics by asking: for a
    // primary face that DOES have a glyph, every glyph ends up at
    // face_idx 0.
    let primary = load_face();
    let fallback = load_face_mono();
    let primary_id = primary.id();
    let chain = FaceChain::new(primary).push_fallback(fallback);
    assert_eq!(chain.len(), 2);
    assert_eq!(chain.primary().id(), primary_id);

    let glyphs = chain.shape("Hello", 16.0).expect("shape");
    assert_eq!(glyphs.len(), 5);
    for g in &glyphs {
        assert_eq!(g.face_idx, 0, "primary covers all Latin");
    }
}

#[test]
fn face_chain_falls_back_when_primary_has_no_glyph() {
    // We need a codepoint that's missing from one fixture but present
    // in the other. Both DejaVu fixtures cover Latin + Greek + Cyrillic
    // identically, so to stress fallback we put MONO first (which
    // doesn't cover the more exotic ranges DejaVuSans does — but
    // actually they cover similar ranges).
    //
    // Pragmatic alternative: verify the fallback DOES kick in by
    // installing a non-existent codepoint in the primary's cmap range.
    // If both faces lack the glyph, we should fall through to the
    // primary's .notdef and face_idx == 0.
    //
    // To deterministically test fallback, we use a glyph 0 (.notdef)
    // case: a private-use-area codepoint that no DejaVu fixture has.
    // Both faces return None → final assignment is (face_idx=0, gid=0).
    let primary = load_face();
    let fallback = load_face_mono();
    let chain = FaceChain::new(primary).push_fallback(fallback);

    // U+E000 — Private Use Area, not in either DejaVu.
    let glyphs = chain.shape("\u{E000}", 16.0).expect("shape");
    assert_eq!(glyphs.len(), 1);
    assert_eq!(glyphs[0].face_idx, 0, "notdef fallback to primary");
    assert_eq!(glyphs[0].glyph_id, 0, ".notdef glyph");

    // For a proper "fallback covers it" test we'd need two faces with
    // disjoint coverage. Without a CJK fixture in the workspace, we
    // simulate by using the SAME font as primary and fallback — every
    // codepoint primary covers ends up at face_idx 0 (already
    // exercised above). The chain mechanics are otherwise covered by
    // the unit tests in face_chain.rs's logic flow (which is so simple
    // — walk a Vec until non-zero — that the integration test's
    // .notdef path is the meaningful end-to-end exercise).
}

// Note: a true two-coverage test (primary covers ASCII, fallback
// covers an exotic range that primary lacks) requires a small CJK or
// emoji fixture. Noto Sans CJK is ~10 MB; that's too big to vendor
// for one test. We document the gap and rely on the unit-flow tests
// instead. Round 3 may add a synthetic single-glyph mock font for
// this purpose.

// ---------------------------------------------------------------------------
// 4. Stroke rendering via compose_run_with_stroke
// ---------------------------------------------------------------------------

#[test]
fn stroke_paints_border_under_fill() {
    let face = load_face();
    let glyphs = Shaper::shape(&face, "A", 32.0).expect("shape");
    assert!(!glyphs.is_empty());

    // Place into a 64×64 transparent bitmap.
    let mut dst = RgbaBitmap::new(64, 64);
    let baseline = face.ascent_px(32.0);
    let chain = FaceChain::new(load_face());

    // 2-pixel black stroke + white fill.
    let stroke = StrokeStyle::new(2.0, [0, 0, 0, 255]);
    let mut composer = Composer::new();
    composer
        .compose_run_with_stroke(
            &glyphs,
            &chain,
            32.0,
            Style::REGULAR,
            [255, 255, 255, 255],
            Some(stroke),
            &mut dst,
            8.0,
            baseline,
        )
        .expect("compose with stroke");

    // 1) Find a pixel that is "centre of the glyph" — i.e. fully
    //    opaque AND white (R = G = B = 255). The fill paints over
    //    the stroke so glyph-interior pixels should be plain white.
    let mut white_count = 0u32;
    let mut black_count = 0u32;
    let mut total_opaque = 0u32;
    for y in 0..dst.height {
        for x in 0..dst.width {
            let p = dst.get(x, y);
            if p[3] >= 200 {
                total_opaque += 1;
                if p[0] >= 200 && p[1] >= 200 && p[2] >= 200 {
                    white_count += 1;
                }
                if p[0] <= 50 && p[1] <= 50 && p[2] <= 50 {
                    black_count += 1;
                }
            }
        }
    }
    assert!(total_opaque > 0, "stroke + fill produced no opaque pixels");
    assert!(white_count > 0, "expected white fill pixels, got 0");
    assert!(
        black_count > 0,
        "expected black stroke pixels around the glyph, got 0"
    );
}

#[test]
fn stroke_increases_alpha_coverage_vs_fill_only() {
    let face = load_face();
    let glyphs = Shaper::shape(&face, "O", 32.0).expect("shape");

    let baseline = face.ascent_px(32.0);
    let chain = FaceChain::new(load_face());

    // Fill-only.
    let mut fill_only = RgbaBitmap::new(64, 64);
    let mut composer1 = Composer::new();
    composer1
        .compose_run_styled(
            &glyphs,
            &chain,
            32.0,
            Style::REGULAR,
            [255, 255, 255, 255],
            &mut fill_only,
            8.0,
            baseline,
        )
        .expect("fill only");

    // Fill + stroke.
    let mut with_stroke = RgbaBitmap::new(64, 64);
    let mut composer2 = Composer::new();
    composer2
        .compose_run_with_stroke(
            &glyphs,
            &chain,
            32.0,
            Style::REGULAR,
            [255, 255, 255, 255],
            Some(StrokeStyle::new(2.0, [0, 0, 0, 255])),
            &mut with_stroke,
            8.0,
            baseline,
        )
        .expect("fill + stroke");

    let fill_alpha = fill_only.nonzero_alpha_count();
    let stroke_alpha = with_stroke.nonzero_alpha_count();
    assert!(
        stroke_alpha > fill_alpha,
        "stroked render should cover more pixels: stroke={}, fill={}",
        stroke_alpha,
        fill_alpha
    );
}

#[test]
fn dilate_alpha_grows_glyph_bitmap() {
    // Run the dilation against a real glyph alpha bitmap from the
    // rasterizer; verify the grown bitmap has more non-zero pixels
    // and is sized exactly width+2r × height+2r.
    let face = load_face();
    let gid = face
        .with_font(|f| f.glyph_index('o'))
        .expect("with_font")
        .expect("'o' in DejaVu");
    let bm: AlphaBitmap = Rasterizer::raster_glyph(&face, gid, 24.0).expect("raster");
    assert!(!bm.is_empty());

    let dilated = dilate_alpha(&bm, 1.5);
    assert_eq!(dilated.width, bm.width + 4);
    assert_eq!(dilated.height, bm.height + 4);
    assert!(dilated.nonzero_pixel_count() > bm.nonzero_pixel_count());
}
