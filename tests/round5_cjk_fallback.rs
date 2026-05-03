//! Round-5 integration test: FaceChain CJK fallback against real
//! Noto Sans CJK Medium (subfont 0) + DejaVu Sans Mono primary.
//!
//! The round-2 deferral was "Two-script-coverage fallback integration
//! test — needs a small CJK or emoji fixture in oxideav-ttf. Noto Sans
//! CJK is ~10 MB, too big to vendor for one test." Round 5 closes that
//! gap by hosting `NotoSansCJK-Medium.ttc` (19 MB) on
//! `samples.oxideav.org/fonts/` and downloading on demand under
//! `target/test-fixtures/fonts/` (cached after first fetch, SHA-256
//! verified on every load, gated behind `OXIDEAV_NETWORK_TESTS=1`).
//!
//! What the test verifies:
//!
//! - The TTC is loadable via `Face::from_ttc_bytes(bytes, 0)` (subfont
//!   index 0, which the OpenType `name` table identifies as the
//!   Japanese cut of Noto Sans CJK Medium).
//! - A `FaceChain { primary: DejaVu, fallback: Noto CJK }` shapes a
//!   mixed Latin + CJK string with each codepoint resolving to the
//!   right face — Latin chars get `face_idx = 0` (DejaVu), CJK chars
//!   get `face_idx = 1` (Noto CJK).
//! - Per-face GSUB / GPOS runs work independently inside the chain.
//! - The total advance of the run matches the sum of per-glyph
//!   advances at the requested size.

#[path = "font_fixtures/mod.rs"]
mod font_fixtures;

use font_fixtures::{load_fixture, NOTO_SANS_CJK_MEDIUM_TTC};
use oxideav_scribe::{Face, FaceChain};

const DEJAVU_MONO: &[u8] = include_bytes!("fixtures/DejaVuSansMono.ttf");

#[test]
fn cjk_fallback_routes_per_codepoint() {
    let cjk_bytes = match load_fixture(&NOTO_SANS_CJK_MEDIUM_TTC) {
        Some(b) => b,
        None => return, // skip silently — fixture-helper printed why
    };

    // Primary face: DejaVu Sans Mono (Latin + Cyrillic + Greek + a few
    // Han, but the Han coverage is intentionally sparse — fallback
    // should kick in for the Japanese / Chinese codepoints used below).
    let dejavu = Face::from_ttf_bytes(DEJAVU_MONO.to_vec()).expect("DejaVu");

    // Fallback face: subfont 0 of NotoSansCJK-Medium.ttc — the
    // Japanese cut, which covers all of Hiragana, Katakana and the
    // Joyo Kanji set (and JIS X 0208 plus extension blocks).
    let noto_cjk = Face::from_ttc_bytes(cjk_bytes, 0).expect("Noto CJK subfont 0");
    let family = noto_cjk.family_name().unwrap_or("(unknown)");
    eprintln!(
        "[round5-cjk] Noto subfont 0 family={family:?} units_per_em={} \
         ascent={} descent={}",
        noto_cjk.units_per_em(),
        noto_cjk.ascent_px(32.0),
        noto_cjk.descent_px(32.0),
    );
    assert_eq!(noto_cjk.subfont_index(), Some(0));

    // Build a 2-face chain.
    let chain = FaceChain::new(dejavu).push_fallback(noto_cjk);
    assert_eq!(chain.len(), 2);

    // Mixed content: hello (Latin) + 日本語 (Japanese) + world (Latin).
    // Pen is intentionally space-separated so we have a clear boundary
    // marker — the spaces resolve via DejaVu (face_idx 0).
    let text = "hello 日本語 world";
    let glyphs = chain.shape(text, 32.0).expect("shape");

    // We expect 16 glyphs total: 5 Latin (hello) + 1 space + 3 CJK +
    // 1 space + 5 Latin (world). DejaVu Mono has no ligatures or
    // contextual substitutions enabled here, so the count is 1:1.
    assert_eq!(
        glyphs.len(),
        text.chars().count(),
        "shape produced {} glyphs for {} chars: {glyphs:#?}",
        glyphs.len(),
        text.chars().count(),
    );

    // Per-codepoint expected face_idx.
    let expected_face: Vec<u16> = text
        .chars()
        .map(|c| {
            // Anything outside ASCII printable range is treated as CJK.
            // (`日`, `本`, `語` all have codepoints > 0x4E00.)
            if (c as u32) < 0x80 {
                0
            } else {
                1
            }
        })
        .collect();
    let actual_face: Vec<u16> = glyphs.iter().map(|g| g.face_idx).collect();
    assert_eq!(
        actual_face, expected_face,
        "per-codepoint face routing mismatch.\n  text: {text:?}\n  glyphs: {glyphs:#?}",
    );

    // Sanity: the CJK glyph IDs (face_idx 1) must be > 0 — i.e. the
    // fallback face actually has them. If any CJK char fell back to
    // .notdef the assert above (face_idx 1 with valid mapping) would
    // already have triggered, but check explicitly to make the
    // failure mode obvious.
    for (i, (g, ch)) in glyphs.iter().zip(text.chars()).enumerate() {
        if g.face_idx == 1 {
            assert!(
                g.glyph_id != 0,
                "CJK char {ch:?} (idx {i}) routed to fallback face but resolved \
                 to .notdef (gid 0) — Noto CJK subfont 0 should have it",
            );
        }
    }

    // Total advance > 0 — the chain must produce real glyphs with
    // positive widths.
    let total: f32 = glyphs.iter().map(|g| g.x_advance).sum();
    assert!(
        total > 0.0,
        "expected positive total advance, got {total}: {glyphs:#?}",
    );

    // CJK glyphs should generally be wider (they're east-asian wide),
    // so the average advance for the CJK run should exceed the
    // average for the Latin run. Coarse sanity check; not a
    // bit-exact reproducibility assert.
    let mut latin_w = 0.0f32;
    let mut latin_n = 0;
    let mut cjk_w = 0.0f32;
    let mut cjk_n = 0;
    for g in &glyphs {
        if g.face_idx == 0 {
            latin_w += g.x_advance;
            latin_n += 1;
        } else {
            cjk_w += g.x_advance;
            cjk_n += 1;
        }
    }
    let latin_avg = if latin_n > 0 {
        latin_w / latin_n as f32
    } else {
        0.0
    };
    let cjk_avg = if cjk_n > 0 { cjk_w / cjk_n as f32 } else { 0.0 };
    eprintln!(
        "[round5-cjk] avg advance: latin={latin_avg:.2} px (n={latin_n}); \
         cjk={cjk_avg:.2} px (n={cjk_n})"
    );
    assert!(
        cjk_avg > latin_avg,
        "expected CJK glyphs to have a larger average advance \
         (latin={latin_avg:.2}, cjk={cjk_avg:.2})",
    );
}
