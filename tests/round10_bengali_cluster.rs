//! Round-10 Bengali complex-script shaping integration test.
//!
//! Goal: shape Bengali clusters through `FaceChain::shape` and verify
//! that the shaper emits the **reordered** glyph sequence — Bengali's
//! THREE pre-base matras (U+09BF "i", U+09C7 "e", U+09C8 "ai") all
//! must come BEFORE the base consonant in the output even though they
//! follow the consonant in logical (input) order.
//!
//! ## Fixture gap
//!
//! Round 10 ships **without** a Bengali font fixture in-tree.
//! DejaVuSans does NOT cover the Bengali block (U+0980..U+09FF) — its
//! cmap returns 0 for every Bengali codepoint. Other vendored fonts
//! (DejaVuSansMono, SourceSans3, InterVariable) likewise lack Bengali
//! coverage.
//!
//! When neither glyph is available, the shaper emits two `.notdef`
//! glyphs (gid 0) which makes the reorder visually indistinguishable
//! from the un-reordered sequence. The integration test therefore
//! **skips** with an `eprintln!` note when the fixture font lacks
//! Bengali glyphs — exactly mirroring the round-5 (CJK / emoji) and
//! round-8 (Devanagari) optional-fixture patterns.
//!
//! Once a Bengali font lands (NotoSansBengali-Regular.ttf is the
//! obvious candidate — OFL-licensed; ~290 KB; suitable for the
//! `samples.oxideav.org/fonts/` CDN cache used by `font_fixtures`),
//! the skip path will deactivate and the test will assert the
//! reordered gid sequence.
//!
//! ## What the unit tests already cover
//!
//! `crate::shaping::indic` ships an exhaustive Bengali unit-test
//! suite (categorisation + cluster boundary detection + per-script
//! pre-base reorder + reph identification) and `crate::face_chain::tests`
//! proves the `apply_indic_reorder` pre-cmap pass produces the right
//! char permutation + emits the right `RephMark` sidecar entries.

use oxideav_scribe::{Face, FaceChain};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn gid_for_codepoint(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

#[test]
fn bengali_pre_base_matra_i_reorders_before_base() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    let ka_gid = gid_for_codepoint(&face, '\u{0995}');
    let matra_i_gid = gid_for_codepoint(&face, '\u{09BF}');
    if ka_gid == 0 || matra_i_gid == 0 || ka_gid == matra_i_gid {
        eprintln!(
            "[round10-bengali] Fixture font lacks distinct Bengali KA + sign-i \
             (ka_gid={ka_gid}, matra_i_gid={matra_i_gid}) — \
             skipping integration test. See module docs for the fixture-gap note."
        );
        return;
    }

    let chain = FaceChain::new(face);
    // Shape the LOGICAL-order input (KA + sign-i). The Bengali pre-base
    // reorder must move the matra to the front, so the shaped gid
    // sequence is [matra_i_gid, ka_gid] — NOT [ka_gid, matra_i_gid].
    let placed = chain
        .shape("\u{0995}\u{09BF}", 32.0)
        .expect("shape Bengali ki succeeds");
    assert_eq!(
        placed.len(),
        2,
        "Bengali reorder must keep glyph count == char count (no GSUB merge in round 10)"
    );
    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    let expected = vec![matra_i_gid, ka_gid];
    assert_eq!(
        actual_gids, expected,
        "round-10 Bengali reorder must emit [matra, KA] gids\n  actual = {actual_gids:?}\n  expected = {expected:?}"
    );
}

#[test]
fn bengali_path_does_not_disturb_ascii_runs() {
    // Regression guard: the Bengali pre-cmap pass must not touch
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
