//! Round-10 Tamil complex-script shaping integration test.
//!
//! Goal: shape Tamil clusters through `FaceChain::shape` and verify
//! that the shaper emits the **reordered** glyph sequence — Tamil's
//! THREE pre-base matras (U+0BC6 "e", U+0BC7 "ee", U+0BC8 "ai") all
//! must come BEFORE the base consonant in the output even though they
//! follow the consonant in logical (input) order.
//!
//! ## Fixture gap
//!
//! See the round-10 Bengali integration test for the same skip-on-
//! missing-fixture pattern. DejaVuSans does NOT cover the Tamil block
//! (U+0B80..U+0BFF). The natural fixture is NotoSansTamil-Regular.ttf
//! (~330 KB; OFL).
//!
//! ## Reph behaviour for Tamil
//!
//! Tamil RA (U+0BB0) does NOT form a reph in modern orthography — the
//! `RephMark` sidecar emitted by `apply_indic_reorder` is empty for
//! Tamil clusters even when the input matches RA + halant + consonant.
//! This is verified at the unit-test level in
//! `face_chain::tests::tamil_RA_plus_halant_does_NOT_emit_reph_mark`.
//!
//! ## What the unit tests already cover
//!
//! `crate::shaping::indic` ships an exhaustive Tamil unit-test suite
//! (categorisation + cluster boundary detection + per-script pre-base
//! reorder + reph-disabled assertion) and `crate::face_chain::tests`
//! proves the `apply_indic_reorder` pre-cmap pass produces the right
//! char permutation.

use oxideav_scribe::{Face, FaceChain};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn gid_for_codepoint(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

#[test]
fn tamil_pre_base_matra_e_reorders_before_base() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    let ka_gid = gid_for_codepoint(&face, '\u{0B95}');
    let matra_e_gid = gid_for_codepoint(&face, '\u{0BC6}');
    if ka_gid == 0 || matra_e_gid == 0 || ka_gid == matra_e_gid {
        eprintln!(
            "[round10-tamil] Fixture font lacks distinct Tamil KA + sign-e \
             (ka_gid={ka_gid}, matra_e_gid={matra_e_gid}) — \
             skipping integration test. See module docs for the fixture-gap note."
        );
        return;
    }

    let chain = FaceChain::new(face);
    // Shape the LOGICAL-order input (KA + sign-e). Tamil pre-base
    // reorder must move the matra to the front, so the shaped gid
    // sequence is [matra_e_gid, ka_gid].
    let placed = chain
        .shape("\u{0B95}\u{0BC6}", 32.0)
        .expect("shape Tamil ke succeeds");
    assert_eq!(
        placed.len(),
        2,
        "Tamil reorder must keep glyph count == char count"
    );
    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    let expected = vec![matra_e_gid, ka_gid];
    assert_eq!(
        actual_gids, expected,
        "round-10 Tamil reorder must emit [matra, KA] gids\n  actual = {actual_gids:?}\n  expected = {expected:?}"
    );
}

#[test]
fn tamil_path_does_not_disturb_ascii_runs() {
    // Regression guard: the Tamil pre-cmap pass must not touch
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
