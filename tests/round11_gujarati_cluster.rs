//! Round-11 Gujarati complex-script shaping integration test.
//!
//! Gujarati is the closest in shape to Devanagari among the round-11
//! scripts (halant U+0ACD; pre-base matra U+0ABF; reph rule on RA
//! U+0AB0). The integration test follows the round-10 Bengali pattern:
//! shape KA + sign-i and assert the gid sequence reorders the matra to
//! the front, OR skip with `eprintln!` when the vendored DejaVuSans
//! cmap doesn't cover the Gujarati block (U+0A80..U+0AFF).

use oxideav_scribe::{Face, FaceChain};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn gid_for_codepoint(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

#[test]
fn gujarati_pre_base_matra_i_reorders_before_base() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    let ka_gid = gid_for_codepoint(&face, '\u{0A95}');
    let matra_i_gid = gid_for_codepoint(&face, '\u{0ABF}');
    if ka_gid == 0 || matra_i_gid == 0 || ka_gid == matra_i_gid {
        eprintln!(
            "[round11-gujarati] Fixture font lacks distinct Gujarati KA + sign-i \
             (ka_gid={ka_gid}, matra_i_gid={matra_i_gid}) — \
             skipping integration test."
        );
        return;
    }

    let chain = FaceChain::new(face);
    let placed = chain
        .shape("\u{0A95}\u{0ABF}", 32.0)
        .expect("shape Gujarati ki succeeds");
    assert_eq!(placed.len(), 2);
    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    assert_eq!(actual_gids, vec![matra_i_gid, ka_gid]);
}

#[test]
fn gujarati_path_does_not_disturb_ascii_runs() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    let chain = FaceChain::new(face);
    let placed = chain.shape("Hi", 32.0).expect("shape Hi succeeds");
    assert_eq!(placed.len(), 2);
}
