//! Round-8 Devanagari complex-script shaping integration test.
//!
//! Goal: shape "कि" (KA U+0915 + sign-i U+093F) through `FaceChain::shape`
//! and verify that the shaper emits the **reordered** glyph sequence —
//! the pre-base matra "i" must come BEFORE the base consonant KA in
//! the output, even though it follows KA in logical (input) order.
//!
//! ## Fixture gap
//!
//! Round 8 ships **without** a Devanagari font fixture in-tree.
//! DejaVuSans (the workhorse fixture for rounds 1-7) does NOT cover
//! the Devanagari block (U+0900..U+097F) — its cmap returns 0 for
//! both KA and the pre-base matra. Other vendored fonts
//! (DejaVuSansMono, SourceSans3) likewise lack Devanagari coverage.
//!
//! When neither glyph is available, the shaper emits two `.notdef`
//! glyphs (gid 0) which makes the reorder visually indistinguishable
//! from the un-reordered sequence. The integration test therefore
//! **skips** with an `eprintln!` note when the fixture font lacks
//! Devanagari glyphs — exactly mirroring the round-5 pattern for
//! optional CJK / colour-emoji fixtures.
//!
//! Once a Devanagari font lands (NotoSansDevanagari-Regular.ttf is
//! the obvious candidate — OFL-licensed; ~280 KB; suitable for the
//! `samples.oxideav.org/fonts/` CDN cache used by `font_fixtures`),
//! the skip path will deactivate and the test will assert the
//! reordered gid sequence.
//!
//! ## What the unit tests already cover
//!
//! `crate::shaping::indic` ships an exhaustive unit-test suite
//! (categorisation + cluster boundary detection + pre-base reorder +
//! reph identification) and `crate::face_chain::tests` proves the
//! `apply_devanagari_reorder` pre-cmap pass produces the right char
//! permutation. The only thing the integration test would add — once
//! a real font is available — is end-to-end verification that the
//! permuted chars survive cmap + GSUB + GPOS untouched. The unit
//! tests are the load-bearing correctness proof; the integration
//! test is belt-and-braces.

use oxideav_scribe::{Face, FaceChain};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn gid_for_codepoint(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

#[test]
fn ki_cluster_reorders_pre_base_matra_before_base() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");

    let ka_gid = gid_for_codepoint(&face, '\u{0915}');
    let matra_i_gid = gid_for_codepoint(&face, '\u{093F}');
    if ka_gid == 0 || matra_i_gid == 0 || ka_gid == matra_i_gid {
        eprintln!(
            "[round8-devanagari] Fixture font lacks distinct Devanagari KA + sign-i \
             (ka_gid={ka_gid}, matra_i_gid={matra_i_gid}) — \
             skipping integration test. See module docs for the fixture-gap note."
        );
        return;
    }

    let chain = FaceChain::new(face);
    // Shape the LOGICAL-order input "कि" (KA + sign-i). The Devanagari
    // pre-base reorder must move the matra to the front, so the
    // shaped gid sequence is [matra_i_gid, ka_gid] — NOT
    // [ka_gid, matra_i_gid].
    let placed = chain
        .shape("\u{0915}\u{093F}", 32.0)
        .expect("shape ki succeeds");
    assert_eq!(
        placed.len(),
        2,
        "Devanagari reorder must keep glyph count == char count (no GSUB merge in round 8)"
    );
    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    let expected = vec![matra_i_gid, ka_gid];
    let naive = vec![ka_gid, matra_i_gid];
    assert_eq!(
        actual_gids, expected,
        "round-8 reorder must emit [matra, KA] gids\n  actual = {actual_gids:?}\n  expected = {expected:?}\n  naive = {naive:?}"
    );
}

#[test]
fn devanagari_path_does_not_disturb_ascii_runs() {
    // Regression guard: the Devanagari pre-cmap pass must not touch
    // non-Indic characters.
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    let h_gid = gid_for_codepoint(&face, 'H');
    let i_gid = gid_for_codepoint(&face, 'i');
    let chain = FaceChain::new(face);
    let placed = chain.shape("Hi", 32.0).expect("shape Hi succeeds");
    assert_eq!(placed.len(), 2);
    assert_eq!(placed[0].glyph_id, h_gid);
    assert_eq!(placed[1].glyph_id, i_gid);
}

#[test]
fn devanagari_path_does_not_disturb_arabic_runs() {
    // Regression guard: the round-7 Arabic substitution must continue
    // to work with the round-8 Devanagari pre-cmap pass appended.
    // ALEF in isolation must still pick the PF-B isolated form
    // (U+FE8D) after the Devanagari pass runs (which is a no-op for
    // Arabic input).
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    let isol_alef_gid = gid_for_codepoint(&face, '\u{FE8D}');
    if isol_alef_gid == 0 {
        eprintln!("[round8-devanagari] DejaVuSans missing U+FE8D; skipping Arabic regression");
        return;
    }
    let chain = FaceChain::new(face);
    let placed = chain.shape("\u{0627}", 32.0).expect("shape ALEF succeeds");
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].glyph_id, isol_alef_gid);
}
