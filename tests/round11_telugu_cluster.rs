//! Round-11 Telugu complex-script shaping integration test.
//!
//! Goal: shape Telugu clusters through `FaceChain::shape` and verify
//! that the shaper emits the **reordered** glyph sequence — Telugu's
//! pre-base matras (U+0C46 "e", U+0C47 "ee", U+0C48 "ai") all must
//! come BEFORE the base consonant in the output even though they
//! follow the consonant in logical (input) order.
//!
//! ## Fixture gap
//!
//! Round 11 ships **without** a Telugu font fixture in-tree.
//! DejaVuSans does NOT cover the Telugu block (U+0C00..U+0C7F) — its
//! cmap returns 0 for every Telugu codepoint. Other vendored fonts
//! (DejaVuSansMono, SourceSans3, InterVariable) likewise lack Telugu
//! coverage.
//!
//! When neither glyph is available, the shaper emits two `.notdef`
//! glyphs (gid 0) which makes the reorder visually indistinguishable
//! from the un-reordered sequence. The integration test therefore
//! **skips** with an `eprintln!` note when the fixture font lacks
//! Telugu glyphs — exactly mirroring the round-5 (CJK / emoji) and
//! round-10 (Bengali / Tamil) optional-fixture patterns.
//!
//! Once a Telugu font lands (NotoSansTelugu-Regular.ttf is the
//! obvious candidate — OFL-licensed; suitable for the
//! `samples.oxideav.org/fonts/` CDN cache used by `font_fixtures`),
//! the skip path will deactivate and the test will assert the
//! reordered gid sequence.
//!
//! ## What the unit tests already cover
//!
//! `crate::shaping::indic` ships a per-script unit-test suite for
//! Telugu (categorisation + cluster boundary detection + per-script
//! pre-base reorder + reph identification) and `crate::face_chain::tests`
//! proves the `apply_indic_reorder` pre-cmap pass produces the right
//! char permutation + emits the right `ClusterSpan` sidecar entries.

use oxideav_scribe::{Face, FaceChain};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn gid_for_codepoint(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

#[test]
fn telugu_pre_base_matra_e_reorders_before_base() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    let ka_gid = gid_for_codepoint(&face, '\u{0C15}');
    let matra_e_gid = gid_for_codepoint(&face, '\u{0C46}');
    if ka_gid == 0 || matra_e_gid == 0 || ka_gid == matra_e_gid {
        eprintln!(
            "[round11-telugu] Fixture font lacks distinct Telugu KA + sign-e \
             (ka_gid={ka_gid}, matra_e_gid={matra_e_gid}) — \
             skipping integration test. See module docs for the fixture-gap note."
        );
        return;
    }

    let chain = FaceChain::new(face);
    // Shape the LOGICAL-order input (KA + sign-e). The Telugu pre-base
    // reorder must move the matra to the front, so the shaped gid
    // sequence is [matra_e_gid, ka_gid] — NOT [ka_gid, matra_e_gid].
    let placed = chain
        .shape("\u{0C15}\u{0C46}", 32.0)
        .expect("shape Telugu ke succeeds");
    assert_eq!(
        placed.len(),
        2,
        "Telugu reorder must keep glyph count == char count (no GSUB merge in round 11)"
    );
    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    let expected = vec![matra_e_gid, ka_gid];
    assert_eq!(
        actual_gids, expected,
        "round-11 Telugu reorder must emit [matra, KA] gids\n  actual = {actual_gids:?}\n  expected = {expected:?}"
    );
}

#[test]
fn telugu_path_does_not_disturb_ascii_runs() {
    // Regression guard: the Telugu pre-cmap pass must not touch
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
