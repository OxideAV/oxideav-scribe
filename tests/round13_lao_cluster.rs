//! Round-13 Lao shaping integration test.
//!
//! Goal: shape Lao text through `FaceChain::shape` and verify
//! that the shaper PRESERVES storage order (Lao has no halant and no
//! pre-base matra reorder — Lao pre-base vowels U+0EC0..U+0EC4 already
//! appear BEFORE their consonant in storage / keyboard order, which is
//! the visually-correct position).
//!
//! Lao is the structural twin of Thai (the round-12 entry from the
//! Brahmic non-Indic family): both have no halant, no conjunct
//! formation, and pre-base vowels in storage order matching their
//! visual position.
//!
//! ## Fixture gap
//!
//! Round 13 ships **without** a Lao font fixture in-tree. DejaVuSans
//! does NOT cover the Lao block (U+0E80..U+0EFF) — its cmap returns 0
//! for every Lao codepoint. Other vendored fonts (DejaVuSansMono,
//! SourceSans3, InterVariable) likewise lack Lao coverage.
//!
//! When neither glyph is available, the shaper emits two `.notdef`
//! glyphs (gid 0) which makes the storage-order preservation visually
//! indistinguishable from the un-preserved sequence. The integration
//! test therefore **skips** with an `eprintln!` note when the fixture
//! font lacks Lao glyphs — exactly mirroring the round-10 / round-11 /
//! round-12 optional-fixture patterns.
//!
//! Once a Lao font lands (NotoSansLao-Regular.ttf is the obvious
//! candidate — OFL-licensed; suitable for the
//! `samples.oxideav.org/fonts/` CDN cache used by `font_fixtures`),
//! the skip path will deactivate and the test will assert the
//! preserved gid sequence.
//!
//! ## What the unit tests already cover
//!
//! `crate::shaping::indic` ships a per-script unit-test suite for
//! Lao (categorisation + cluster boundary detection of pre-base vowel
//! breaks + tone mark attachment + halant absence + reph-disabled
//! assertion) and `crate::face_chain::tests` proves the
//! `apply_indic_reorder` pre-cmap pass leaves Lao storage order
//! untouched.

use oxideav_scribe::{Face, FaceChain};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn gid_for_codepoint(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

#[test]
fn lao_pre_base_vowel_then_consonant_preserves_storage_order() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    // U+0EC0 SARA E (pre-base in storage order) + U+0E81 LAO LETTER KO.
    let sara_e_gid = gid_for_codepoint(&face, '\u{0EC0}');
    let ko_gid = gid_for_codepoint(&face, '\u{0E81}');
    if sara_e_gid == 0 || ko_gid == 0 || sara_e_gid == ko_gid {
        eprintln!(
            "[round13-lao] Fixture font lacks distinct Lao SARA E + KO \
             (sara_e_gid={sara_e_gid}, ko_gid={ko_gid}) — \
             skipping integration test. See module docs for the fixture-gap note."
        );
        return;
    }

    let chain = FaceChain::new(face);
    let placed = chain
        .shape("\u{0EC0}\u{0E81}", 32.0)
        .expect("shape Lao pre-base vowel + consonant succeeds");
    assert_eq!(
        placed.len(),
        2,
        "Lao must keep glyph count == char count (no halant, no GSUB merge)"
    );
    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    // STORAGE ORDER preserved — Lao pre-base vowels are already in
    // their visual position in raw input, so no reorder happens.
    let expected = vec![sara_e_gid, ko_gid];
    assert_eq!(
        actual_gids, expected,
        "round-13 Lao must preserve storage order [SARA E, KO]\n  actual = {actual_gids:?}\n  expected = {expected:?}"
    );
}

#[test]
fn lao_consonant_with_tone_mark_passes_through() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    // KO + MAI EK (tone mark) — should pass through unchanged
    // (tone mark attaches via mark-to-base GPOS in the rendering
    // pipeline; the cluster pass is a no-op).
    let ko_gid = gid_for_codepoint(&face, '\u{0E81}');
    let mai_ek_gid = gid_for_codepoint(&face, '\u{0EC8}');
    if ko_gid == 0 || mai_ek_gid == 0 {
        eprintln!(
            "[round13-lao] Fixture font lacks Lao consonant or tone mark — \
             skipping tone-mark integration test."
        );
        return;
    }
    let chain = FaceChain::new(face);
    let placed = chain
        .shape("\u{0E81}\u{0EC8}", 32.0)
        .expect("shape Lao KO + MAI EK succeeds");
    assert_eq!(placed.len(), 2);
    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    let expected = vec![ko_gid, mai_ek_gid];
    assert_eq!(actual_gids, expected);
}

#[test]
fn lao_path_does_not_disturb_ascii_runs() {
    // Regression guard: the Lao pre-cmap pass must not touch
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
