//! Round-12 Thai shaping integration test.
//!
//! Goal: shape Thai text through `FaceChain::shape` and verify
//! that the shaper PRESERVES storage order (Thai has no halant and
//! no pre-base matra reorder — Thai pre-base vowels U+0E40..U+0E44
//! already appear BEFORE their consonant in storage / keyboard order,
//! which is the visually-correct position).
//!
//! Thai is the structural outlier of the round-12 Brahmic non-Indic
//! family: while Sinhala / Khmer use halant-style cluster machines,
//! Thai's cluster machine is just the segmenter (each consonant starts
//! a new cluster, tone marks + above/below vowel signs attach).
//!
//! ## Fixture
//!
//! DejaVuSans **does** ship Thai glyphs — the regression test below
//! verifies that the shaper preserves the [SARA E, KO KAI] gid sequence
//! (storage and visual order both [pre-base vowel, consonant]). If
//! DejaVuSans loses Thai coverage at some future date the test
//! degrades to a skip note.
//!
//! ## What the unit tests already cover
//!
//! `crate::shaping::indic` ships a per-script unit-test suite for
//! Thai (categorisation + cluster boundary detection of pre-base vowel
//! breaks + tone mark attachment + halant absence + reph-disabled
//! assertion) and `crate::face_chain::tests` proves the
//! `apply_indic_reorder` pre-cmap pass leaves Thai storage order
//! untouched.

use oxideav_scribe::{Face, FaceChain};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn gid_for_codepoint(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

#[test]
fn thai_pre_base_vowel_then_consonant_preserves_storage_order() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    // U+0E40 SARA E (pre-base in storage order) + U+0E01 KO KAI.
    let sara_e_gid = gid_for_codepoint(&face, '\u{0E40}');
    let ko_kai_gid = gid_for_codepoint(&face, '\u{0E01}');
    if sara_e_gid == 0 || ko_kai_gid == 0 || sara_e_gid == ko_kai_gid {
        eprintln!(
            "[round12-thai] Fixture font lacks distinct Thai SARA E + KO KAI \
             (sara_e_gid={sara_e_gid}, ko_kai_gid={ko_kai_gid}) — \
             skipping integration test."
        );
        return;
    }

    let chain = FaceChain::new(face);
    let placed = chain
        .shape("\u{0E40}\u{0E01}", 32.0)
        .expect("shape Thai pre-base vowel + consonant succeeds");
    assert_eq!(
        placed.len(),
        2,
        "Thai must keep glyph count == char count (no halant, no GSUB merge)"
    );
    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    // STORAGE ORDER preserved — Thai pre-base vowels are already in
    // their visual position in raw input, so no reorder happens.
    let expected = vec![sara_e_gid, ko_kai_gid];
    assert_eq!(
        actual_gids, expected,
        "round-12 Thai must preserve storage order [SARA E, KO KAI]\n  actual = {actual_gids:?}\n  expected = {expected:?}"
    );
}

#[test]
fn thai_consonant_with_tone_mark_passes_through() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    // KO KAI + MAI EK (tone mark) — should pass through unchanged
    // (tone mark attaches via mark-to-base GPOS in the rendering
    // pipeline; the cluster pass is a no-op).
    let ko_kai_gid = gid_for_codepoint(&face, '\u{0E01}');
    let mai_ek_gid = gid_for_codepoint(&face, '\u{0E48}');
    if ko_kai_gid == 0 || mai_ek_gid == 0 {
        eprintln!(
            "[round12-thai] Fixture font lacks Thai consonant or tone mark — \
             skipping tone-mark integration test."
        );
        return;
    }
    let chain = FaceChain::new(face);
    let placed = chain
        .shape("\u{0E01}\u{0E48}", 32.0)
        .expect("shape Thai KO KAI + MAI EK succeeds");
    assert_eq!(placed.len(), 2);
    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    let expected = vec![ko_kai_gid, mai_ek_gid];
    assert_eq!(actual_gids, expected);
}

#[test]
fn thai_path_does_not_disturb_ascii_runs() {
    // Regression guard: the Thai pre-cmap pass must not touch
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
