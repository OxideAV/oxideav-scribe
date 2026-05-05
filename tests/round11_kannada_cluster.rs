//! Round-11 Kannada complex-script shaping integration test.
//!
//! Goal: shape Kannada clusters through `FaceChain::shape` and verify
//! the per-script reorder + cluster-position GSUB pass run end-to-end
//! without crashing. Like round-10 Bengali / Tamil this test skips
//! when the vendored DejaVuSans cmap returns 0 for Kannada codepoints
//! (no fixture font yet covers the U+0C80..U+0CFF block).
//!
//! The unit-test suite in `crate::shaping::indic` exercises the
//! cluster machine for Kannada exhaustively (categorisation +
//! pre-base reorder + reph identification); this integration test
//! activates once a Kannada font lands.

use oxideav_scribe::{Face, FaceChain};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn gid_for_codepoint(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

#[test]
fn kannada_pre_base_matra_e_reorders_before_base() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    let ka_gid = gid_for_codepoint(&face, '\u{0C95}');
    let matra_e_gid = gid_for_codepoint(&face, '\u{0CC6}');
    if ka_gid == 0 || matra_e_gid == 0 || ka_gid == matra_e_gid {
        eprintln!(
            "[round11-kannada] Fixture font lacks distinct Kannada KA + sign-e \
             (ka_gid={ka_gid}, matra_e_gid={matra_e_gid}) — \
             skipping integration test. See module docs for the fixture-gap note."
        );
        return;
    }

    let chain = FaceChain::new(face);
    let placed = chain
        .shape("\u{0C95}\u{0CC6}", 32.0)
        .expect("shape Kannada ke succeeds");
    assert_eq!(placed.len(), 2);
    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    assert_eq!(actual_gids, vec![matra_e_gid, ka_gid]);
}

#[test]
fn kannada_path_does_not_disturb_ascii_runs() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    let chain = FaceChain::new(face);
    let placed = chain.shape("Hi", 32.0).expect("shape Hi succeeds");
    assert_eq!(placed.len(), 2);
}
