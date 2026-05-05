//! Round-13 Burmese / Myanmar complex-script shaping integration test.
//!
//! Goal: shape Burmese clusters through `FaceChain::shape` and verify
//! that the shaper emits the **reordered** glyph sequence — Burmese's
//! sole pre-base matra (U+1031 sign-e) must come BEFORE the base
//! consonant in the output even though it follows the consonant in
//! logical (input) order. The kinzi pattern (NGA U+1004, Asat U+103A,
//! Virama U+1039, Consonant) is identified by the cluster machine and
//! feeds the `rphf` GSUB lookup when the active face publishes one.
//!
//! ## Fixture gap
//!
//! Round 13 ships **without** a Burmese font fixture in-tree.
//! DejaVuSans does NOT cover the Myanmar block (U+1000..U+109F) — its
//! cmap returns 0 for every Burmese codepoint. Other vendored fonts
//! (DejaVuSansMono, SourceSans3, InterVariable) likewise lack Burmese
//! coverage.
//!
//! When neither glyph is available, the shaper emits two `.notdef`
//! glyphs (gid 0) which makes the reorder visually indistinguishable
//! from the un-reordered sequence. The integration test therefore
//! **skips** with an `eprintln!` note when the fixture font lacks
//! Burmese glyphs — exactly mirroring the round-10 / round-11 /
//! round-12 optional-fixture patterns.
//!
//! Once a Burmese font lands (NotoSansMyanmar-Regular.ttf is the
//! obvious candidate — OFL-licensed; suitable for the
//! `samples.oxideav.org/fonts/` CDN cache used by `font_fixtures`),
//! the skip path will deactivate and the test will assert the
//! reordered gid sequence.
//!
//! ## What the unit tests already cover
//!
//! `crate::shaping::indic` ships a per-script unit-test suite for
//! Burmese (categorisation per Asat/Virama/medial/pre-base + cluster
//! boundary detection of the asat-vs-virama distinction + kinzi
//! pattern recognition + reph_kind dispatch) and
//! `crate::face_chain::tests` proves the `apply_indic_reorder` pre-cmap
//! pass produces the right char permutation + emits the right
//! `ClusterSpan` and reph-mark sidecar entries.

use oxideav_scribe::{Face, FaceChain};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn gid_for_codepoint(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

#[test]
fn burmese_pre_base_matra_e_reorders_before_base() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    // U+1000 MYANMAR LETTER KA + U+1031 MYANMAR VOWEL SIGN E (pre-base).
    let ka_gid = gid_for_codepoint(&face, '\u{1000}');
    let matra_e_gid = gid_for_codepoint(&face, '\u{1031}');
    if ka_gid == 0 || matra_e_gid == 0 || ka_gid == matra_e_gid {
        eprintln!(
            "[round13-burmese] Fixture font lacks distinct Burmese KA + sign-e \
             (ka_gid={ka_gid}, matra_e_gid={matra_e_gid}) — \
             skipping integration test. See module docs for the fixture-gap note."
        );
        return;
    }

    let chain = FaceChain::new(face);
    // Shape the LOGICAL-order input (KA + sign-e). The Burmese pre-base
    // reorder must move the matra to the front, so the shaped gid
    // sequence is [matra_e_gid, ka_gid] — NOT [ka_gid, matra_e_gid].
    let placed = chain
        .shape("\u{1000}\u{1031}", 32.0)
        .expect("shape Burmese ke succeeds");
    assert_eq!(
        placed.len(),
        2,
        "Burmese reorder must keep glyph count == char count"
    );
    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    let expected = vec![matra_e_gid, ka_gid];
    assert_eq!(
        actual_gids, expected,
        "round-13 Burmese reorder must emit [matra, KA] gids\n  actual = {actual_gids:?}\n  expected = {expected:?}"
    );
}

#[test]
fn burmese_kinzi_pattern_keeps_cluster_intact() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    let nga_gid = gid_for_codepoint(&face, '\u{1004}');
    let asat_gid = gid_for_codepoint(&face, '\u{103A}');
    let virama_gid = gid_for_codepoint(&face, '\u{1039}');
    let ka_gid = gid_for_codepoint(&face, '\u{1000}');
    if nga_gid == 0 || asat_gid == 0 || virama_gid == 0 || ka_gid == 0 {
        eprintln!(
            "[round13-burmese] Fixture font lacks Burmese kinzi components \
             (NGA={nga_gid}, Asat={asat_gid}, Virama={virama_gid}, KA={ka_gid}) — \
             skipping kinzi integration test."
        );
        return;
    }
    let chain = FaceChain::new(face);
    // NGA + Asat + Virama + KA — the canonical kinzi pattern.
    // Without a `rphf` GSUB lookup the cluster passes through with
    // four glyphs in the original order; with a `rphf` lookup the NGA
    // is rewritten and the virama dropped.
    let placed = chain
        .shape("\u{1004}\u{103A}\u{1039}\u{1000}", 32.0)
        .expect("shape Burmese kinzi succeeds");
    // Glyph count is either 4 (no rphf) or 3 (rphf collapses the
    // virama). Either is acceptable — both are visually correct.
    assert!(
        placed.len() == 3 || placed.len() == 4,
        "Burmese kinzi cluster must emit 3 or 4 glyphs, got {}",
        placed.len()
    );
}

#[test]
fn burmese_path_does_not_disturb_ascii_runs() {
    // Regression guard: the Burmese pre-cmap pass must not touch
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
