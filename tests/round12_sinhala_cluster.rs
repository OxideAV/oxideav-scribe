//! Round-12 Sinhala complex-script shaping integration test.
//!
//! Goal: shape Sinhala clusters through `FaceChain::shape` and verify
//! that the shaper emits the **reordered** glyph sequence — Sinhala's
//! pre-base matras (U+0DD9 "e", U+0DDA "ee", U+0DDB "ai" plus the
//! precomposed two-part vowels U+0DDC..U+0DDE) all must come BEFORE
//! the base consonant in the output even though they follow the
//! consonant in logical (input) order.
//!
//! ## Fixture gap
//!
//! Round 12 ships **without** a Sinhala font fixture in-tree.
//! DejaVuSans does NOT cover the Sinhala block (U+0D80..U+0DFF) — its
//! cmap returns 0 for every Sinhala codepoint. Other vendored fonts
//! (DejaVuSansMono, SourceSans3, InterVariable) likewise lack Sinhala
//! coverage.
//!
//! When neither glyph is available, the shaper emits two `.notdef`
//! glyphs (gid 0) which makes the reorder visually indistinguishable
//! from the un-reordered sequence. The integration test therefore
//! **skips** with an `eprintln!` note when the fixture font lacks
//! Sinhala glyphs — exactly mirroring the round-5 (CJK / emoji),
//! round-10 (Bengali / Tamil), and round-11 (Telugu / Kannada /
//! Gujarati) optional-fixture patterns.
//!
//! Once a Sinhala font lands (NotoSansSinhala-Regular.ttf is the
//! obvious candidate — OFL-licensed; suitable for the
//! `samples.oxideav.org/fonts/` CDN cache used by `font_fixtures`),
//! the skip path will deactivate and the test will assert the
//! reordered gid sequence.
//!
//! ## What the unit tests already cover
//!
//! `crate::shaping::indic` ships a per-script unit-test suite for
//! Sinhala (categorisation + cluster boundary detection + per-script
//! pre-base reorder + reph-disabled assertion) and
//! `crate::face_chain::tests` proves the `apply_indic_reorder` pre-cmap
//! pass produces the right char permutation + emits the right
//! `ClusterSpan` sidecar entries.

use oxideav_scribe::{Face, FaceChain};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn gid_for_codepoint(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

#[test]
fn sinhala_pre_base_matra_e_reorders_before_base() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    // U+0D9A SINHALA LETTER ALPAPRAANA KAYANNA + U+0DD9 SINHALA VOWEL
    // SIGN KOMBUVA (sign-e, pre-base).
    let ka_gid = gid_for_codepoint(&face, '\u{0D9A}');
    let matra_e_gid = gid_for_codepoint(&face, '\u{0DD9}');
    if ka_gid == 0 || matra_e_gid == 0 || ka_gid == matra_e_gid {
        eprintln!(
            "[round12-sinhala] Fixture font lacks distinct Sinhala KA + sign-e \
             (ka_gid={ka_gid}, matra_e_gid={matra_e_gid}) — \
             skipping integration test. See module docs for the fixture-gap note."
        );
        return;
    }

    let chain = FaceChain::new(face);
    // Shape the LOGICAL-order input (KA + sign-e). The Sinhala pre-base
    // reorder must move the matra to the front, so the shaped gid
    // sequence is [matra_e_gid, ka_gid] — NOT [ka_gid, matra_e_gid].
    let placed = chain
        .shape("\u{0D9A}\u{0DD9}", 32.0)
        .expect("shape Sinhala ke succeeds");
    assert_eq!(
        placed.len(),
        2,
        "Sinhala reorder must keep glyph count == char count (no GSUB merge in round 12)"
    );
    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    let expected = vec![matra_e_gid, ka_gid];
    assert_eq!(
        actual_gids, expected,
        "round-12 Sinhala reorder must emit [matra, KA] gids\n  actual = {actual_gids:?}\n  expected = {expected:?}"
    );
}

#[test]
fn sinhala_path_does_not_disturb_ascii_runs() {
    // Regression guard: the Sinhala pre-cmap pass must not touch
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
