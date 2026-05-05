//! Round-12 Khmer complex-script shaping integration test.
//!
//! Goal: shape Khmer clusters through `FaceChain::shape` and verify
//! that the shaper emits the **reordered** glyph sequence — Khmer's
//! pre-base matras (U+17BE..U+17C5) all must come BEFORE the base
//! consonant in the output even though they follow the consonant in
//! logical (input) order. Also exercises the COENG (U+17D2) chain that
//! glues subjoined consonants into a single cluster.
//!
//! ## Fixture gap
//!
//! Round 12 ships **without** a Khmer font fixture in-tree.
//! DejaVuSans does NOT cover the Khmer block (U+1780..U+17FF) — its
//! cmap returns 0 for every Khmer codepoint. Other vendored fonts
//! likewise lack Khmer coverage.
//!
//! When neither glyph is available, the shaper emits two `.notdef`
//! glyphs (gid 0) which makes the reorder visually indistinguishable
//! from the un-reordered sequence. The integration test therefore
//! **skips** with an `eprintln!` note when the fixture font lacks
//! Khmer glyphs.
//!
//! Once a Khmer font lands (NotoSansKhmer-Regular.ttf is the obvious
//! candidate — OFL-licensed; suitable for the
//! `samples.oxideav.org/fonts/` CDN cache used by `font_fixtures`), the
//! skip path will deactivate and the test will assert the reordered
//! gid sequence.
//!
//! ## What the unit tests already cover
//!
//! `crate::shaping::indic` ships a per-script unit-test suite for
//! Khmer (categorisation + cluster boundary detection of subjoined
//! chains + per-script pre-base reorder + reph-disabled assertion) and
//! `crate::face_chain::tests` proves the `apply_indic_reorder` pre-cmap
//! pass produces the right char permutation + emits the right
//! `ClusterSpan` sidecar entries (including a 5-char three-deep
//! subjoined chain test).

use oxideav_scribe::{Face, FaceChain};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn gid_for_codepoint(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

#[test]
fn khmer_pre_base_matra_e_reorders_before_base() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    // U+1780 KHMER LETTER KA + U+17C1 KHMER VOWEL SIGN E (pre-base).
    let ka_gid = gid_for_codepoint(&face, '\u{1780}');
    let matra_e_gid = gid_for_codepoint(&face, '\u{17C1}');
    if ka_gid == 0 || matra_e_gid == 0 || ka_gid == matra_e_gid {
        eprintln!(
            "[round12-khmer] Fixture font lacks distinct Khmer KA + sign-e \
             (ka_gid={ka_gid}, matra_e_gid={matra_e_gid}) — \
             skipping integration test. See module docs for the fixture-gap note."
        );
        return;
    }

    let chain = FaceChain::new(face);
    let placed = chain
        .shape("\u{1780}\u{17C1}", 32.0)
        .expect("shape Khmer ke succeeds");
    assert_eq!(
        placed.len(),
        2,
        "Khmer reorder must keep glyph count == char count (no GSUB merge in round 12)"
    );
    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    let expected = vec![matra_e_gid, ka_gid];
    assert_eq!(
        actual_gids, expected,
        "round-12 Khmer reorder must emit [matra, KA] gids\n  actual = {actual_gids:?}\n  expected = {expected:?}"
    );
}

#[test]
fn khmer_coeng_subjoined_chain_renders_in_one_pass() {
    // KA + COENG + KHA — subjoined cluster. Without a real Khmer font
    // the shaper emits .notdef gids; we still verify the pipeline does
    // not panic and emits one glyph per character (no pre-base reorder
    // since this cluster has no pre-base matra).
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    let chain = FaceChain::new(face);
    let placed = chain
        .shape("\u{1780}\u{17D2}\u{1781}", 32.0)
        .expect("shape Khmer KA + COENG + KHA succeeds");
    assert_eq!(
        placed.len(),
        3,
        "Khmer subjoined cluster must keep glyph count == char count"
    );
}

#[test]
fn khmer_path_does_not_disturb_ascii_runs() {
    // Regression guard: the Khmer pre-cmap pass must not touch
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
