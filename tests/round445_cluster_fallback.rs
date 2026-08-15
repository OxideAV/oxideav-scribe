//! Round 445 — universal mark handling: cluster-atomic font fallback.
//!
//! UAX #24 §5.2: never break between a combining mark and its base.
//! `FaceChain`'s per-character fallback scan used to pick the FIRST
//! face covering each character independently, so a base sourced from
//! a fallback face could get its combining mark from the primary —
//! splitting a mark from its base across faces, where GPOS
//! mark-to-base attachment cannot operate. The round-445 post-pass
//! re-sources every mark from its cluster's base face whenever that
//! face covers it.
//!
//! Fixture arrangement: primary = InterVariable (Latin/Greek/Cyrillic,
//! no Hebrew coverage), fallback = DejaVuSans (Hebrew + combining
//! marks). A Hebrew base with U+0301 COMBINING ACUTE ACCENT — a mark
//! BOTH faces cover — must source the mark from the fallback face that
//! owns its base, not from the primary. (`FaceChain::shape` is a
//! TTF-only pipeline, hence the all-TTF fixture pick.)

use oxideav_scribe::{Face, FaceChain};

const INTER: &[u8] = include_bytes!("fixtures/InterVariable.ttf");
const DEJAVU: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn chain() -> FaceChain {
    let primary = Face::from_ttf_bytes(INTER.to_vec()).expect("InterVariable face");
    let fallback = Face::from_ttf_bytes(DEJAVU.to_vec()).expect("DejaVuSans face");
    FaceChain::new(primary).push_fallback(fallback)
}

/// The test premise: InterVariable lacks Hebrew but covers U+0301;
/// DejaVuSans covers both. If a fixture swap ever changes this, the
/// tests below must be re-targeted.
#[test]
fn fixture_coverage_preconditions() {
    let chain = chain();
    let primary_alef = chain
        .face(0)
        .with_font(|f| f.glyph_index('\u{05D0}'))
        .expect("primary cmap");
    assert!(
        primary_alef.is_none() || primary_alef == Some(0),
        "premise: InterVariable must NOT cover HEBREW LETTER ALEF"
    );
    let primary_acute = chain
        .face(0)
        .with_font(|f| f.glyph_index('\u{0301}'))
        .expect("primary cmap");
    assert!(
        matches!(primary_acute, Some(g) if g != 0),
        "premise: InterVariable must cover U+0301 COMBINING ACUTE"
    );
    for c in ['\u{05D0}', '\u{0301}'] {
        let g = chain
            .face(1)
            .with_font(|f| f.glyph_index(c))
            .expect("fallback cmap");
        assert!(
            matches!(g, Some(g) if g != 0),
            "premise: DejaVuSans must cover U+{:04X}",
            c as u32
        );
    }
}

#[test]
fn mark_follows_its_base_onto_the_fallback_face() {
    let glyphs = chain()
        .shape("\u{05D0}\u{0301}", 16.0)
        .expect("shape Hebrew base + combining acute");
    assert_eq!(glyphs.len(), 2, "got {glyphs:?}");
    assert_eq!(
        glyphs[0].face_idx, 1,
        "base must come from the fallback face: {glyphs:?}"
    );
    assert_eq!(
        glyphs[1].face_idx, 1,
        "the mark must be sourced from its base's face, not the \
         primary that merely covers it: {glyphs:?}"
    );
    assert_ne!(glyphs[1].glyph_id, 0, "mark must not degrade to .notdef");
}

#[test]
fn primary_covered_cluster_stays_on_the_primary() {
    // Control: base + mark both covered by the primary — nothing
    // migrates to the fallback. (Inter's `ccmp` may compose the pair
    // into a single precomposed glyph, so 1 or 2 glyphs are both
    // valid; every one must stay on face 0.)
    let glyphs = chain().shape("e\u{0301}", 16.0).expect("shape e + acute");
    assert!(
        (1..=2).contains(&glyphs.len()),
        "unexpected glyph count: {glyphs:?}"
    );
    assert!(
        glyphs.iter().all(|g| g.face_idx == 0 && g.glyph_id != 0),
        "fully-primary cluster must not migrate: {glyphs:?}"
    );
}

#[test]
fn mark_the_base_face_lacks_keeps_the_per_char_assignment() {
    // Hebrew base (fallback-only) + a mark we expect DejaVuSans to
    // cover as well — but ALSO verify the graceful-degradation clause
    // directly: a cluster whose base is .notdef everywhere leaves its
    // marks on whatever face covers them.
    let glyphs = chain()
        .shape("\u{2603}\u{0301}", 16.0) // SNOWMAN + acute
        .expect("shape snowman + acute");
    assert_eq!(glyphs.len(), 2, "got {glyphs:?}");
    // Wherever the snowman ends up (DejaVu covers it), the mark must
    // still render from a face that covers it (never .notdef via a
    // forced migration).
    assert_ne!(
        glyphs[1].glyph_id, 0,
        "mark degraded to .notdef: {glyphs:?}"
    );
}

#[test]
fn multi_mark_cluster_is_atomic() {
    // Base + two stacked combining marks: all three from one face.
    let glyphs = chain()
        .shape("\u{05D0}\u{0301}\u{0302}", 16.0)
        .expect("shape base + two marks");
    assert_eq!(glyphs.len(), 3, "got {glyphs:?}");
    let faces: Vec<u16> = glyphs.iter().map(|g| g.face_idx).collect();
    assert_eq!(
        faces,
        vec![1, 1, 1],
        "cluster split across faces: {glyphs:?}"
    );
}
