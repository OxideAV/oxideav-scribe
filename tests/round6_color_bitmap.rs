//! Round-6 integration tests (#356) for the CBDT bilinear-resample +
//! composer integration.
//!
//! Three things are exercised here:
//!
//! 1. `RgbaBitmap::resample_bilinear` against a synthetic fixture
//!    (no network), verifying the bilinear maths produce identity
//!    on a same-size resample, halve cleanly on a 2×2 → 1×1 average,
//!    and clamp at edges without reading out-of-bounds.
//!
//! 2. `Face::raster_color_glyph_at` against Noto Color Emoji — when
//!    asked for a 32 px-tall glyph it returns a bitmap whose dimensions
//!    are within ±1 px of the strike-aspect-preserved 32 px target
//!    (round-to-nearest), and whose `bearing_x` / `bearing_y` /
//!    `advance` are scaled by `size_px / strike_ppem`.
//!
//! 3. `Composer::compose_run` against Noto Color Emoji — composing a
//!    single emoji glyph into an `RgbaBitmap` produces non-zero
//!    colour pixels (not just alpha), proving the colour-bitmap
//!    dispatch fires inside the composer.
//!
//! The Noto-font tests are gated on `OXIDEAV_NETWORK_TESTS=1` via the
//! shared `font_fixtures` helper; the synthetic test runs unconditionally.

#[path = "font_fixtures/mod.rs"]
mod font_fixtures;

use font_fixtures::{load_fixture, NOTO_COLOR_EMOJI_TTF};
use oxideav_scribe::{Composer, Face, RgbaBitmap, Shaper, WHITE};

/// Bilinear resample identity — destination size matches the source,
/// so every pixel must come back unchanged. Catches sign / off-by-one
/// errors in the half-pixel mapping.
#[test]
fn bilinear_identity_resample_returns_input() {
    // 2×2 RGBA: red, green, blue, yellow.
    let mut src = RgbaBitmap::new(2, 2);
    src.data = vec![
        255, 0, 0, 255, // (0,0)
        0, 255, 0, 255, // (1,0)
        0, 0, 255, 255, // (0,1)
        255, 255, 0, 255, // (1,1)
    ];
    let out = src.resample_bilinear(2, 2);
    assert_eq!(out.width, 2);
    assert_eq!(out.height, 2);
    assert_eq!(out.data, src.data, "identity resample must round-trip");
}

/// 2×2 → 1×1 bilinear average. With centre-sample mapping the lone
/// destination pixel maps to `(src.w/2 - 0.5, src.h/2 - 0.5) = (0.5, 0.5)`,
/// i.e. dead-centre between all four input pixels — so each contributes
/// 1/4 of the output. Red+green+blue+yellow → (128, 128, 64, 255)
/// (rounded; within ±1 of the bilinear maths).
#[test]
fn bilinear_average_of_2x2_to_1x1_is_centroid() {
    let mut src = RgbaBitmap::new(2, 2);
    src.data = vec![
        255, 0, 0, 255, // red
        0, 255, 0, 255, // green
        0, 0, 255, 255, // blue
        255, 255, 0, 255, // yellow
    ];
    let out = src.resample_bilinear(1, 1);
    assert_eq!(out.width, 1);
    assert_eq!(out.height, 1);
    let p = out.get(0, 0);
    // Red sum: 255 + 0 + 0 + 255 = 510 → 127 or 128 with quarter weights.
    // Green sum: 0 + 255 + 0 + 255 = 510 → same.
    // Blue sum: 0 + 0 + 255 + 0 = 255 → 64.
    // Alpha sum: 1020 → 255.
    assert!(
        (p[0] as i32 - 128).abs() <= 1,
        "R channel: expected ~128, got {}",
        p[0]
    );
    assert!(
        (p[1] as i32 - 128).abs() <= 1,
        "G channel: expected ~128, got {}",
        p[1]
    );
    assert!(
        (p[2] as i32 - 64).abs() <= 1,
        "B channel: expected ~64, got {}",
        p[2]
    );
    assert_eq!(p[3], 255, "alpha must round to 255");
}

/// Up-sample a 2×2 to 4×4 — the four source corner pixels must appear
/// at the four destination corners (±1 due to centre-sample rounding),
/// and edge clamping must keep us inside the source extent.
#[test]
fn bilinear_upsample_preserves_corners() {
    let mut src = RgbaBitmap::new(2, 2);
    src.data = vec![
        255, 0, 0, 255, //
        0, 255, 0, 255, //
        0, 0, 255, 255, //
        255, 255, 0, 255, //
    ];
    let out = src.resample_bilinear(4, 4);
    assert_eq!(out.width, 4);
    assert_eq!(out.height, 4);
    // The (0,0) destination centre maps to source (-0.25, -0.25) which
    // clamps to (0, 0) — pure red.
    let tl = out.get(0, 0);
    assert!(
        tl[0] >= 200 && tl[1] < 50 && tl[2] < 50,
        "TL ~red, got {tl:?}"
    );
    // (3, 0) maps to source (1.25, -0.25) which clamps Y to 0 and
    // interpolates X between green and red — at fx=1.0 (clamped) we
    // get pure green.
    let tr = out.get(3, 0);
    assert!(
        tr[1] >= 200 && tr[0] < 50 && tr[2] < 50,
        "TR ~green, got {tr:?}"
    );
    // (0, 3) maps to source (-0.25, 1.25) → clamps to (0, 1) → pure blue.
    let bl = out.get(0, 3);
    assert!(
        bl[2] >= 200 && bl[0] < 50 && bl[1] < 50,
        "BL ~blue, got {bl:?}"
    );
    // (3, 3) → source (1.25, 1.25) → clamps to (1, 1) → pure yellow.
    let br = out.get(3, 3);
    assert!(
        br[0] >= 200 && br[1] >= 200 && br[2] < 50,
        "BR ~yellow, got {br:?}"
    );
}

/// Resampling with a zero destination dimension returns an empty bitmap
/// (no panic, no out-of-bounds reads).
#[test]
fn bilinear_zero_dest_returns_empty() {
    let mut src = RgbaBitmap::new(2, 2);
    src.data = vec![255; 16];
    let out = src.resample_bilinear(0, 5);
    assert!(out.is_empty());
    let out = src.resample_bilinear(5, 0);
    assert!(out.is_empty());
}

/// Resampling an empty bitmap returns an empty bitmap.
#[test]
fn bilinear_empty_source_returns_empty() {
    let src = RgbaBitmap::default();
    let out = src.resample_bilinear(8, 8);
    assert!(out.is_empty());
}

/// `Face::raster_color_glyph_at` returns a bitmap pre-resampled to
/// `size_px`-ish dimensions (the strike-aspect-preserved scale of the
/// CBDT entry's strike-native pixel grid). For Noto Color Emoji's
/// 109 ppem strike at `size_px = 32`, the resulting bitmap is roughly
/// `round(96 * 32/109) × round(109 * 32/109) = 28 × 32` (the actual
/// CBDT entry is 96×96 + 7px top bearing + 7px left bearing → glyph
/// box ~96×109, scaled to ~28×32). Allow ±2 px slack for upstream
/// strike-metric drift.
#[test]
fn raster_color_glyph_at_resamples_to_target_size() {
    let bytes = match load_fixture(&NOTO_COLOR_EMOJI_TTF) {
        Some(b) => b,
        None => return, // skip silently
    };
    let face = Face::from_ttf_bytes(bytes).expect("Noto Color Emoji parses");
    let gid = face
        .with_font(|f| f.glyph_index('\u{1F389}'))
        .expect("with_font ok")
        .expect("U+1F389 must map");

    // Native strike — sanity baseline.
    let native = face
        .raster_color_glyph(gid, 32.0)
        .expect("raster_color_glyph ok")
        .expect("CBDT must have glyph 🎉");
    eprintln!(
        "[round6-color-bitmap] native strike: {}x{} ppem={}",
        native.bitmap.width, native.bitmap.height, native.ppem
    );
    assert!(
        native.bitmap.width >= 32 && native.bitmap.height >= 32,
        "expected native strike larger than 32 px, got {}x{}",
        native.bitmap.width,
        native.bitmap.height
    );

    // Resampled to ~32 px.
    let resampled = face
        .raster_color_glyph_at(gid, 32.0)
        .expect("raster_color_glyph_at ok")
        .expect("CBDT must have glyph 🎉");
    eprintln!(
        "[round6-color-bitmap] resampled: {}x{} bearing=({}, {}) advance={} ppem={}",
        resampled.bitmap.width,
        resampled.bitmap.height,
        resampled.bearing_x,
        resampled.bearing_y,
        resampled.advance,
        resampled.ppem
    );
    // The bitmap dimensions must scale by `32 / native.ppem`. With
    // native.ppem ≈ 109 and native bitmap 96×96, target is ~28×28.
    // Allow 2 px slack on each axis.
    let scale = 32.0 / native.ppem as f32;
    let expected_w = (native.bitmap.width as f32 * scale).round() as i32;
    let expected_h = (native.bitmap.height as f32 * scale).round() as i32;
    assert!(
        (resampled.bitmap.width as i32 - expected_w).abs() <= 2,
        "resampled width {} not within 2 of expected {}",
        resampled.bitmap.width,
        expected_w
    );
    assert!(
        (resampled.bitmap.height as i32 - expected_h).abs() <= 2,
        "resampled height {} not within 2 of expected {}",
        resampled.bitmap.height,
        expected_h
    );
    // Reported ppem on the resampled side is the requested raster
    // size, not the native strike's.
    assert_eq!(resampled.ppem, 32);
    // Resample must not have nuked the alpha plane.
    assert!(
        resampled.bitmap.nonzero_alpha_count() > 0,
        "resampled bitmap has zero non-transparent pixels"
    );
    // Bearings are scaled (sanity: not the unscaled native value).
    let expected_bx = (native.bearing_x as f32 * scale).round() as i32;
    let expected_by = (native.bearing_y as f32 * scale).round() as i32;
    assert_eq!(resampled.bearing_x, expected_bx);
    assert_eq!(resampled.bearing_y, expected_by);
}

/// `Composer::compose_run` for a single emoji glyph paints actual
/// **colour** pixels into the destination — proving the CBDT dispatch
/// in `compose_run_inner` is wired through. The composer's `color`
/// parameter is meaningless for colour bitmaps (CBDT carries its own
/// colour) so we pass white and confirm we get NON-white pixels back.
#[test]
fn composer_paints_color_pixels_for_emoji() {
    let bytes = match load_fixture(&NOTO_COLOR_EMOJI_TTF) {
        Some(b) => b,
        None => return, // skip silently
    };
    let face = Face::from_ttf_bytes(bytes).expect("Noto Color Emoji parses");
    let glyphs = Shaper::shape(&face, "\u{1F389}", 32.0).expect("shape");
    assert!(!glyphs.is_empty(), "shaper produced 0 glyphs for 🎉");
    eprintln!("[round6-color-bitmap] shaped: {glyphs:#?}");

    // Allocate a destination wide enough for the glyph + some slack
    // around the bearings. 64×64 covers a single 32 px emoji.
    let mut dst = RgbaBitmap::new(64, 64);
    let mut composer = Composer::new();
    // Pen at (8, 40) — leaves room for the negative bearing_y to land
    // glyph above the baseline within the canvas.
    composer
        .compose_run(&glyphs, &face, 32.0, WHITE, &mut dst, 8.0, 40.0)
        .expect("compose_run ok");

    // Count colour pixels: any pixel where R != G or G != B (alpha
    // matters too, but the emoji has lots of pure colour).
    let mut color_pixels = 0usize;
    let mut nonzero_alpha = 0usize;
    for px in dst.data.chunks_exact(4) {
        if px[3] != 0 {
            nonzero_alpha += 1;
        }
        if px[3] != 0 && (px[0] != px[1] || px[1] != px[2]) {
            color_pixels += 1;
        }
    }
    eprintln!(
        "[round6-color-bitmap] composer painted {nonzero_alpha} non-transparent pixels, \
         {color_pixels} of which are coloured"
    );
    assert!(
        nonzero_alpha > 0,
        "composer painted zero non-transparent pixels — colour-bitmap dispatch never fired"
    );
    assert!(
        color_pixels > 0,
        "composer painted only grayscale pixels — CBDT dispatch produced an alpha mask, \
         not a colour bitmap"
    );
}
