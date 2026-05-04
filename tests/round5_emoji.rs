//! Round-5 integration test: CBDT/CBLC color bitmap glyphs against
//! real Noto Color Emoji.
//!
//! Noto Color Emoji ships its glyph data exclusively as embedded PNGs
//! in the CBDT table — there is no `glyf` / `loca` outline path (the
//! `_WindowsCompatible` variant adds an empty stub `glyf` for OS
//! compatibility but the canonical `NotoColorEmoji.ttf` we test
//! against doesn't). Round 5 of `oxideav-ttf` made `loca`+`glyf` jointly
//! optional + added the CBDT/CBLC parsers; round 5 of `oxideav-scribe`
//! consumes them via `oxideav-png` to produce an `RgbaBitmap`.
//!
//! What this test verifies (per the task spec):
//!
//! - The font loads via `Face::from_ttf_bytes` (no longer rejected for
//!   the missing `glyf`/`loca`).
//! - `face.has_color_bitmaps()` returns `true`.
//! - `font.glyph_index('🎉' as u32)` resolves.
//! - `font.glyph_color_bitmap(gid, 96)` returns `Some(_)` (the CBDT
//!   walker found the glyph at the closest strike to 96 px).
//! - `face.raster_color_glyph(gid, 96.0)` decodes to a non-empty RGBA
//!   bitmap with at least one pixel of non-zero alpha (i.e. the PNG
//!   decoded successfully).
//! - `Shaper::shape("🎉 ok")` produces ≥1 positioned glyph for the
//!   emoji (i.e. the shaper survived the no-outline / colour-bitmap
//!   path).

#[path = "font_fixtures/mod.rs"]
mod font_fixtures;

use font_fixtures::{load_fixture, NOTO_COLOR_EMOJI_TTF};
use oxideav_scribe::{Face, Shaper};

#[test]
fn noto_color_emoji_loads_and_renders() {
    let bytes = match load_fixture(&NOTO_COLOR_EMOJI_TTF) {
        Some(b) => b,
        None => return, // skip silently — fixture-helper printed why
    };

    // Loading is round-5's first non-regression: pre-round-5 the parser
    // refused fonts without `glyf`/`loca`. Now they're jointly
    // optional and CBDT-only fonts go through.
    let face = Face::from_ttf_bytes(bytes).expect("Noto Color Emoji loads");
    let family = face.family_name().unwrap_or("(unknown)");
    eprintln!(
        "[round5-emoji] family={family:?} units_per_em={}",
        face.units_per_em()
    );

    assert!(
        face.has_color_bitmaps(),
        "Noto Color Emoji should report color-bitmap support",
    );
    let strikes = face.color_strike_sizes();
    eprintln!("[round5-emoji] strikes: {strikes:?}");
    assert!(
        !strikes.is_empty(),
        "expected at least one CBDT strike, got none",
    );
    // Noto Color Emoji ships a single 109 ppem strike historically;
    // newer revs ship 136. Either way ppem_y > 32 — that's the only
    // size-class assertion we make.
    assert!(
        strikes.iter().any(|(_x, y)| *y >= 32),
        "no strike with ppem_y >= 32: {strikes:?}",
    );

    // Glyph lookup for U+1F389 PARTY POPPER ('🎉').
    let gid = face
        .with_font(|f| f.glyph_index('\u{1F389}'))
        .expect("with_font ok");
    let gid = gid.expect("Noto Color Emoji must map U+1F389");
    assert!(gid != 0, "U+1F389 resolved to .notdef");
    eprintln!("[round5-emoji] U+1F389 → glyph id {gid}");

    // CBLC walker should hand us a CBDT entry for the glyph at ~96 px.
    let raw = face
        .with_font(|f| {
            f.glyph_color_bitmap(gid, 96).map(|cb| {
                (
                    cb.png_bytes.len(),
                    cb.width,
                    cb.height,
                    cb.bearing_x,
                    cb.bearing_y,
                    cb.advance,
                    cb.ppem,
                )
            })
        })
        .expect("with_font ok");
    let (png_len, w, h, bx, by, adv, ppem) = raw.expect("CBDT must have glyph 🎉");
    eprintln!(
        "[round5-emoji] CBDT entry: {w}x{h} px (bearing {bx},{by}; advance {adv}; \
         strike ppem {ppem}; PNG payload {png_len} B)"
    );
    assert!(png_len > 0, "PNG payload empty");
    assert!(w > 0 && h > 0, "metrics report 0-sized bitmap");

    // End-to-end: rasterise via the scribe-side wrapper (which decodes
    // the PNG via oxideav-png).
    let cgb = face
        .raster_color_glyph(gid, 96.0)
        .expect("raster_color_glyph ok")
        .expect("raster_color_glyph returned None");
    eprintln!(
        "[round5-emoji] decoded RGBA bitmap: {}x{} (advance {} px, ppem {})",
        cgb.bitmap.width, cgb.bitmap.height, cgb.advance, cgb.ppem
    );
    assert!(
        !cgb.bitmap.is_empty(),
        "decoded RGBA bitmap is empty (PNG decode probably failed)",
    );
    let nz = cgb.bitmap.nonzero_alpha_count();
    assert!(
        nz > 0,
        "decoded RGBA bitmap has zero pixels with non-zero alpha — \
         either the PNG decoded blank or the format conversion lost \
         the alpha plane",
    );
    // Sanity: the emoji should fill a substantial fraction of its
    // bbox. A party-popper glyph in Noto Color Emoji covers maybe
    // 50% of the strike; require a much weaker 5% so we're robust to
    // upstream redesigns.
    let total = cgb.bitmap.width as usize * cgb.bitmap.height as usize;
    let ratio = nz as f32 / total.max(1) as f32;
    eprintln!(
        "[round5-emoji] non-zero alpha pixels: {nz} / {total} ({:.1}%)",
        ratio * 100.0,
    );
    assert!(
        ratio > 0.05,
        "only {:.2}% of the bitmap has non-zero alpha — looks blank",
        ratio * 100.0,
    );

    // Shape "🎉 ok" — single-face shape() over the emoji font. The
    // emoji has no outline (so glyph_outline returns empty) but the
    // shaper itself should produce 4 positioned glyphs:
    //   - 🎉 (cbdt-only — gid > 0, advance > 0 from hmtx)
    //   - SPACE
    //   - o
    //   - k
    // The Latin code points may resolve to .notdef in Noto Color
    // Emoji (it doesn't include Basic Latin) and that's fine — the
    // assertion is only "the emoji glyph survived shaping".
    let shaped = Shaper::shape(&face, "\u{1F389} ok", 96.0).expect("shape");
    eprintln!("[round5-emoji] shaped {} glyphs: {shaped:#?}", shaped.len());
    assert!(
        !shaped.is_empty(),
        "Shaper produced 0 glyphs for the 🎉 string",
    );
    let emoji_position = shaped[0];
    assert_eq!(emoji_position.glyph_id, gid, "first glyph is 🎉");
    // hmtx may carry an explicit advance for the emoji glyph; if not
    // it falls back to the last hmtx entry. Either way it should be
    // positive at 96 px.
    assert!(
        emoji_position.x_advance > 0.0,
        "🎉 has zero advance ({:?})",
        emoji_position
    );
}
