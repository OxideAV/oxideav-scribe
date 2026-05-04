//! Round-7 Arabic contextual joining integration test.
//!
//! Shape "السلام" (al-salām) through `FaceChain::shape` and assert that
//! the resulting glyph IDs match the **Arabic Presentation Forms-B**
//! gids of the contextual shapes (init / medi / fina / isol), NOT the
//! gids of the isolated base codepoints. This proves the joining state
//! machine + presentation-form translation in
//! `oxideav_scribe::shaping::arabic` is wired into the shape pipeline.
//!
//! DejaVuSans ships glyphs for U+FE70..U+FEFF — the Arabic
//! Presentation Forms-B block — so the round-7 substitution path lands
//! visibly-correct contextual glyphs without requiring feature-tagged
//! GSUB lookups. (Fonts that lack PF-B glyphs fall back to the base
//! isolated form, the round-6 behaviour.)
//!
//! ## Word breakdown
//!
//! "السلام" = ALEF U+0627 + LAM U+0644 + SEEN U+0633 + LAM U+0644 +
//!            ALEF U+0627 + MEEM U+0645
//!
//! Joining classes (R = right-joining, D = dual-joining):
//!   R D D D R D
//!
//! Forms picked by the state machine in logical order:
//!   ALEF(R)  Isol — no left, R can't extend right
//!   LAM(D)   Init
//!   SEEN(D)  Medi
//!   LAM(D)   Medi
//!   ALEF(R)  Fina — left LAM extends; R can't extend right
//!   MEEM(D)  Isol — preceding R doesn't extend; no right
//!
//! Translated to Presentation Forms-B codepoints:
//!   ALEF Isol  → U+FE8D
//!   LAM  Init  → U+FEDF
//!   SEEN Medi  → U+FEB4
//!   LAM  Medi  → U+FEE0
//!   ALEF Fina  → U+FE8E
//!   MEEM Isol  → U+FEE1
//!
//! GSUB type-4 ligature substitution (already wired in round 1) then
//! collapses LAM-medi + ALEF-fina into the **LAM-ALEF FINAL ligature**
//! glyph (U+FEFC, gid 5366 in DejaVuSans). The expected post-shaping
//! glyph sequence is therefore 5 glyphs, not 6.

use oxideav_scribe::{Face, FaceChain};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn gid_for_codepoint(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

#[test]
fn al_salam_shapes_to_presentation_form_b_gids() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");

    // Sanity: DejaVuSans has every PF-B glyph we need. If a future
    // fixture swap lands a font without these, the test will skip
    // (rather than misleadingly fail).
    let pf_codepoints = [
        '\u{FE8D}', // [0] ALEF ISOLATED
        '\u{FEDF}', // [1] LAM INITIAL
        '\u{FEB4}', // [2] SEEN MEDIAL
        '\u{FEE0}', // [3] LAM MEDIAL — collapses with ALEF FINAL via GSUB
        '\u{FE8E}', // [4] ALEF FINAL
        '\u{FEE1}', // [5] MEEM ISOLATED
        '\u{FEFC}', // [6] LAM-ALEF FINAL ligature (GSUB output)
    ];
    for &cp in &pf_codepoints {
        if gid_for_codepoint(&face, cp) == 0 {
            eprintln!(
                "[round7-arabic] DejaVuSans missing U+{:04X}; skipping integration test",
                cp as u32
            );
            return;
        }
    }

    // Naive isolated-form gids — what a non-joining shaper would emit
    // for the input string.
    let base_chars = [
        '\u{0627}', '\u{0644}', '\u{0633}', '\u{0644}', '\u{0627}', '\u{0645}',
    ];
    let naive_gids: Vec<u16> = base_chars
        .iter()
        .map(|&c| gid_for_codepoint(&face, c))
        .collect();

    // Expected glyph sequence after PF-B substitution AND GSUB
    // ligature collapse: ALEF-isol, LAM-init, SEEN-medi, then
    // LAM-medi + ALEF-fina merged into the LAM-ALEF FINAL ligature
    // (U+FEFC), then MEEM-isol. 5 glyphs total.
    let expected_gids: Vec<u16> = vec![
        gid_for_codepoint(&face, '\u{FE8D}'), // ALEF ISOL
        gid_for_codepoint(&face, '\u{FEDF}'), // LAM INIT
        gid_for_codepoint(&face, '\u{FEB4}'), // SEEN MEDI
        gid_for_codepoint(&face, '\u{FEFC}'), // LAM-ALEF FINAL ligature
        gid_for_codepoint(&face, '\u{FEE1}'), // MEEM ISOL
    ];

    // Sanity: every expected gid must differ from the corresponding
    // naive gid (otherwise the test would tautologically pass on a
    // non-shaping pipeline).
    let differing: usize = naive_gids
        .iter()
        .filter(|g| !expected_gids.contains(g))
        .count();
    assert!(
        differing >= 5,
        "fixture sanity: naive isolated gids must not appear in the joined output\n  naive    = {naive_gids:?}\n  expected = {expected_gids:?}",
    );

    // Now shape via the chain — this is the round-7 hot path.
    let chain = FaceChain::new(face);
    let placed = chain.shape("السلام", 32.0).expect("shape al-salam succeeds");

    let actual_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    eprintln!("[round7-arabic] gids     = {actual_gids:?}");
    eprintln!("[round7-arabic] expected = {expected_gids:?}");
    eprintln!("[round7-arabic] naive    = {naive_gids:?}");

    assert_eq!(
        actual_gids, expected_gids,
        "round-7 shaper must emit PF-B + LAM-ALEF-ligature gids\n  actual   = {actual_gids:?}\n  expected = {expected_gids:?}\n  naive    = {naive_gids:?}",
    );
    // Belt + braces: assert no naive isolated gid leaked into the
    // output (i.e. the contextual substitution actually fired).
    for g in &actual_gids {
        assert!(
            !naive_gids.contains(g),
            "naive isolated gid {g} present in shaped output {actual_gids:?}",
        );
    }
}

#[test]
fn isolated_arabic_letter_emits_isolated_form() {
    // A single ALEF in isolation should pick Isol form → U+FE8D.
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    let isol_alef_gid = gid_for_codepoint(&face, '\u{FE8D}');
    if isol_alef_gid == 0 {
        eprintln!("[round7-arabic] DejaVuSans missing U+FE8D; skipping");
        return;
    }
    let chain = FaceChain::new(face);
    let placed = chain.shape("\u{0627}", 32.0).expect("shape ALEF succeeds");
    assert_eq!(placed.len(), 1);
    assert_eq!(
        placed[0].glyph_id, isol_alef_gid,
        "isolated ALEF should map to U+FE8D gid"
    );
}

#[test]
fn ascii_text_unchanged_by_arabic_shaping_path() {
    // Non-Arabic text must pass through the new shaping pre-pass
    // unchanged (regression guard against the joining hook accidentally
    // touching Latin runs).
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans parses");
    let h_gid = gid_for_codepoint(&face, 'H');
    let i_gid = gid_for_codepoint(&face, 'i');
    let chain = FaceChain::new(face);
    let placed = chain.shape("Hi", 32.0).expect("shape Hi succeeds");
    assert_eq!(placed.len(), 2);
    assert_eq!(placed[0].glyph_id, h_gid);
    assert_eq!(placed[1].glyph_id, i_gid);
}
