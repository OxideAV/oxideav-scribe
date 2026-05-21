//! Round-15 integration test: `ccmp` (Glyph Composition / Decomposition)
//! is invoked for `latn`-script runs and produces the expected
//! dotless-i substitution when a `LATIN SMALL LETTER I` (U+0069) is
//! followed by a `COMBINING DOT ABOVE` (U+0307).
//!
//! ## Why this is the canonical `ccmp` test
//!
//! OpenType's required-feature `ccmp` is what removes the dot of "i"
//! when a diacritic is going to stack on top (otherwise the diacritic
//! would clash with the inherent dot). DejaVu Sans implements this via
//! a chained-context substitution (LookupType 6) that fires only when
//! the input "i" is immediately followed by a glyph in the
//! "marks-above" coverage. The pre-round-15 pipeline ignored the GSUB
//! feature list entirely and therefore left the "i" with its dot —
//! visually wrong for any "i\u{0307}" / "i\u{0300}" combination.
//!
//! After the round-15 `ccmp` pre-pass, the shaped glyph at position 0
//! is no longer `face.glyph_index('i')` — it's the dotless-i variant
//! the font ships for this exact scenario.
//!
//! The combining-dot itself is preserved (its position is handled by
//! the existing GPOS mark-to-base attachment pass) — the round-15 win
//! is the base-glyph substitution.

use oxideav_scribe::{Face, FaceChain};

const DEJAVU_BYTES: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn cmap(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

#[test]
fn ccmp_substitutes_dotless_i_before_combining_above_mark_dejavu() {
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu Sans parses");
    let raw_i = cmap(&face, 'i');
    assert_ne!(raw_i, 0, "DejaVu Sans must have 'i' in cmap");

    let chain = FaceChain::new(face);

    // Plain "i" (no combining mark): the round-15 ccmp pass MUST be a
    // no-op (no chained-context match because there's no following
    // mark glyph).
    let plain = chain
        .shape("i", 16.0)
        .expect("plain 'i' shape doesn't error");
    assert_eq!(plain.len(), 1, "plain 'i' is one glyph");
    assert_eq!(
        plain[0].glyph_id, raw_i,
        "plain 'i' must NOT be substituted (no following mark)"
    );

    // "i" + COMBINING DOT ABOVE (U+0307): the round-15 ccmp pass MUST
    // fire and replace the base 'i' glyph with the font's
    // dotless-i-style substitute. The dot itself is preserved as the
    // second glyph (positioning is round-3 GPOS work; the round-15 win
    // is the base substitution).
    let combined = chain
        .shape("i\u{0307}", 16.0)
        .expect("i + combining mark shape doesn't error");
    assert_eq!(
        combined.len(),
        2,
        "i + mark stays two glyphs (mark is preserved)"
    );
    assert_ne!(
        combined[0].glyph_id, raw_i,
        "round-15: 'i' before a combining-above mark MUST be substituted by ccmp"
    );
}

#[test]
fn ccmp_substitutes_capital_dot_above_dejavu() {
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu Sans parses");
    let raw_combining_dot_above = cmap(&face, '\u{0307}');
    assert_ne!(
        raw_combining_dot_above, 0,
        "DejaVu Sans must have U+0307 in cmap"
    );

    let chain = FaceChain::new(face);

    // For uppercase 'I' followed by a combining-above mark, DejaVu Sans
    // ships a ccmp lookup that rewrites the MARK glyph (not the base —
    // the capital I already has no dot). After ccmp the second glyph
    // is the case-specific variant the font uses to position the mark
    // higher above the cap. The base 'I' is unchanged.
    let combined = chain
        .shape("I\u{0307}", 16.0)
        .expect("I + combining mark shape doesn't error");
    assert_eq!(combined.len(), 2, "I + mark stays two glyphs");
    assert_ne!(
        combined[1].glyph_id, raw_combining_dot_above,
        "round-15: combining-above on uppercase MUST be substituted by ccmp"
    );
}

#[test]
fn ccmp_is_noop_for_a_with_combining_acute_dejavu() {
    // Negative-control: DejaVu Sans publishes no `ccmp` rule for 'a' +
    // combining acute (U+0301). The base 'a' glyph and the combining
    // acute glyph both come out the cmap path unchanged. This proves
    // ccmp only fires when the font's coverage table matches — it
    // doesn't blanket-substitute every base+mark sequence.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu Sans parses");
    let raw_a = cmap(&face, 'a');
    let raw_acute = cmap(&face, '\u{0301}');
    assert_ne!(raw_a, 0);
    assert_ne!(raw_acute, 0);

    let chain = FaceChain::new(face);
    let combined = chain
        .shape("a\u{0301}", 16.0)
        .expect("a + combining acute shape doesn't error");
    assert_eq!(combined.len(), 2);
    assert_eq!(combined[0].glyph_id, raw_a, "'a' MUST pass through");
    assert_eq!(
        combined[1].glyph_id, raw_acute,
        "combining acute MUST pass through"
    );
}

#[test]
fn ccmp_does_not_disturb_ascii_only_runs() {
    // Round-1 ASCII shaping (Latin alphabetic + numerals + punctuation)
    // must not change between round-14 and round-15: the ccmp pre-pass
    // only matches glyphs in the lookup's coverage table, which
    // (in DejaVu Sans, for ccmp) is the "combining marks above"
    // coverage. Pure-ASCII runs never trigger the lookup.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu Sans parses");
    let chain = FaceChain::new(face);

    // Snapshot of pre-round-15 behaviour for "Hello, world!" — these
    // glyph IDs are stable across DejaVu Sans 2.37 (the vendored copy).
    let expected_ids: Vec<u16> = "Hello, world!"
        .chars()
        .map(|ch| cmap(chain.primary(), ch))
        .collect();

    let glyphs = chain.shape("Hello, world!", 16.0).unwrap();
    let got_ids: Vec<u16> = glyphs.iter().map(|g| g.glyph_id).collect();
    assert_eq!(got_ids, expected_ids);
}
